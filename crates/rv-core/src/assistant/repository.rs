use super::*;
use ring::digest::{Context, SHA256};
use rusqlite::{Connection, params};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("数据库错误：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("素材错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("草稿已在另一个窗口修改，请重新载入")]
    Conflict,
    #[error("{0}")]
    Invalid(String),
}

pub struct AssistantStore {
    db: Connection,
    root: PathBuf,
}
impl AssistantStore {
    pub fn open(root: &Path) -> Result<Self, StoreError> {
        fs::create_dir_all(root.join("assistant-assets"))?;
        let db = Connection::open(root.join("assistant.db"))?;
        db.busy_timeout(std::time::Duration::from_secs(3))?;
        db.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;",
        )?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 2 {
            return Err(StoreError::Invalid("数据库版本高于当前程序支持版本".into()));
        }
        if version == 0 {
            db.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE drafts(id TEXT PRIMARY KEY,title TEXT NOT NULL,description TEXT NOT NULL,price_minor INTEGER,currency TEXT NOT NULL,stock_note TEXT NOT NULL,revision INTEGER NOT NULL,updated_at INTEGER NOT NULL);
                CREATE TABLE images(id TEXT PRIMARY KEY,draft_id TEXT NOT NULL REFERENCES drafts(id) ON DELETE CASCADE,filename TEXT NOT NULL,path TEXT NOT NULL,sha256 TEXT NOT NULL,position INTEGER NOT NULL);
                CREATE INDEX draft_images ON images(draft_id,position);
                CREATE TABLE templates(id TEXT PRIMARY KEY,name TEXT NOT NULL,content TEXT NOT NULL,category TEXT NOT NULL,revision INTEGER NOT NULL);
                CREATE TABLE imports(id TEXT PRIMARY KEY,image_id TEXT NOT NULL REFERENCES images(id) ON DELETE CASCADE,target TEXT NOT NULL,status TEXT NOT NULL,asset_id TEXT,error TEXT,updated_at INTEGER NOT NULL);
                PRAGMA user_version=1; COMMIT;")?;
        }
        if version < 2 {
            db.execute_batch("BEGIN IMMEDIATE; ALTER TABLE drafts ADD COLUMN price_input TEXT; UPDATE imports SET status='unknown' WHERE status='failed'; PRAGMA user_version=2; COMMIT;")?;
        }
        // A crashed transfer must never silently restart and duplicate a photo.
        db.execute("UPDATE imports SET status='unknown',error='程序退出，导入结果待确认' WHERE status IN ('queued','running')", [])?;
        Ok(Self {
            db,
            root: root.to_owned(),
        })
    }
    pub fn list_drafts(&self) -> Result<Vec<Draft>, StoreError> {
        Ok(self.db.prepare("SELECT id,title,description,price_minor,currency,stock_note,revision,updated_at,price_input FROM drafts ORDER BY updated_at DESC,id")?.query_map([], read_draft)?.collect::<Result<_,_>>()?)
    }
    pub fn load_draft(&self, id: &str) -> Result<Draft, StoreError> {
        Ok(self.db.query_row("SELECT id,title,description,price_minor,currency,stock_note,revision,updated_at,price_input FROM drafts WHERE id=?1", [id], read_draft)?)
    }
    pub fn save_draft(&mut self, draft: &Draft) -> Result<Draft, StoreError> {
        if draft.title.len() > 4096
            || draft.description.len() > 65536
            || draft.stock_note.len() > 4096
            || draft.price_input.as_ref().is_some_and(|v| v.len() > 4096)
            || draft.price_minor.is_some_and(|v| v < 0)
            || draft.currency.len() != 3
            || !draft.currency.bytes().all(|b| b.is_ascii_uppercase())
        {
            return Err(StoreError::Invalid("草稿字段过长或金额、币种无效".into()));
        }
        let mut result = draft.clone();
        result.revision += 1;
        result.updated_at = now();
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if draft.revision == 0 {
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM drafts WHERE id=?1)", [&draft.id], |r| r.get(0))?;
            if exists { return Err(StoreError::Conflict); }
            tx.execute("INSERT INTO drafts VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![result.id,result.title,result.description,result.price_minor,result.currency,result.stock_note,result.revision,result.updated_at,result.price_input])?;
        } else if tx.execute("UPDATE drafts SET title=?2,description=?3,price_minor=?4,currency=?5,stock_note=?6,revision=?7,updated_at=?8,price_input=?9 WHERE id=?1 AND revision=?10",params![result.id,result.title,result.description,result.price_minor,result.currency,result.stock_note,result.revision,result.updated_at,result.price_input,draft.revision])? != 1 { return Err(StoreError::Conflict); }
        tx.commit()?;
        Ok(result)
    }
    pub fn delete_draft(&mut self, id: &str, revision: i64) -> Result<(), StoreError> {
        if self.db.execute(
            "DELETE FROM drafts WHERE id=?1 AND revision=?2",
            params![id, revision],
        )? != 1
        {
            return Err(StoreError::Conflict);
        }
        Ok(())
    }
    pub fn duplicate(&mut self, id: &str) -> Result<Draft, StoreError> {
        let source = self.load_draft(id)?;
        let images = self.images(id)?;
        let mut duplicate = source;
        duplicate.id = Uuid::new_v4().to_string();
        duplicate.title = format!("{}（副本）", duplicate.title);
        duplicate.revision = 0;
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT INTO drafts VALUES(?1,?2,?3,?4,?5,?6,1,?7,?8)",
            params![
                duplicate.id,
                duplicate.title,
                duplicate.description,
                duplicate.price_minor,
                duplicate.currency,
                duplicate.stock_note,
                now(),
                duplicate.price_input
            ],
        )?;
        for image in images {
            tx.execute(
                "INSERT INTO images VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    Uuid::new_v4().to_string(),
                    duplicate.id,
                    image.filename,
                    image.relative_path,
                    image.sha256,
                    image.position
                ],
            )?;
        }
        tx.commit()?;
        self.load_draft(&duplicate.id)
    }
    pub fn images(&self, id: &str) -> Result<Vec<DraftImage>, StoreError> {
        Ok(self.db.prepare("SELECT id,draft_id,filename,path,sha256,position FROM images WHERE draft_id=?1 ORDER BY position,id")?.query_map([id],|r|Ok(DraftImage{id:r.get(0)?,draft_id:r.get(1)?,filename:r.get(2)?,relative_path:r.get(3)?,sha256:r.get(4)?,position:r.get(5)?}))?.collect::<Result<_,_>>()?)
    }
    pub fn add_image(&mut self, draft_id: &str, source: &Path) -> Result<DraftImage, StoreError> {
        self.load_draft(draft_id)?;
        let extension = source
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !["png", "jpg", "jpeg", "heic", "heif"].contains(&extension.as_str()) {
            return Err(StoreError::Invalid("仅支持 PNG/JPEG/HEIC/HEIF 图片".into()));
        }
        let mut input = fs::File::open(source)?;
        let meta = input.metadata()?;
        if !meta.is_file() || meta.len() > 50 * 1024 * 1024 {
            return Err(StoreError::Invalid("图片必须为不超过 50 MB 的文件".into()));
        }
        let temp = self
            .root
            .join("assistant-assets")
            .join(format!("{}.part", Uuid::new_v4()));
        let result = (|| {
            let mut output = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?;
            let mut digest = Context::new(&SHA256);
            let mut buffer = [0; 65536];
            let mut total = 0;
            loop {
                let len = input.read(&mut buffer)?;
                if len == 0 {
                    break;
                }
                total += len;
                if total > 50 * 1024 * 1024 {
                    return Err(StoreError::Invalid("图片超过 50 MB".into()));
                }
                digest.update(&buffer[..len]);
                output.write_all(&buffer[..len])?;
            }
            output.sync_all()?;
            if total == 0 {
                return Err(StoreError::Invalid("图片文件为空".into()));
            }
            let sha256 = digest
                .finish()
                .as_ref()
                .iter()
                .map(|v| format!("{v:02x}"))
                .collect::<String>();
            let relative_path = format!("assistant-assets/{sha256}.{extension}");
            let final_path = self.root.join(&relative_path);
            if final_path.exists() {
                fs::remove_file(&temp)?;
            } else {
                fs::rename(&temp, final_path)?;
            }
            let position = self.db.query_row(
                "SELECT COALESCE(MAX(position),-1)+1 FROM images WHERE draft_id=?1",
                [draft_id],
                |r| r.get(0),
            )?;
            let image = DraftImage {
                id: Uuid::new_v4().to_string(),
                draft_id: draft_id.into(),
                filename: source
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                relative_path,
                sha256,
                position,
            };
            self.db.execute(
                "INSERT INTO images VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    image.id,
                    image.draft_id,
                    image.filename,
                    image.relative_path,
                    image.sha256,
                    image.position
                ],
            )?;
            Ok(image)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    pub fn remove_image(&mut self, id: &str) -> Result<(), StoreError> {
        self.db.execute("DELETE FROM images WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn reorder_images(&mut self, draft: &str, ids: &[String]) -> Result<(), StoreError> {
        let old = self.images(draft)?;
        let set = ids.iter().collect::<std::collections::HashSet<_>>();
        if ids.len() != old.len()
            || set.len() != ids.len()
            || old.iter().any(|v| !set.contains(&v.id))
        {
            return Err(StoreError::Invalid("图片顺序不包含完整图片列表".into()));
        }
        let tx = self.db.transaction()?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE images SET position=?2 WHERE id=?1 AND draft_id=?3",
                params![id, i as i64, draft],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn templates(&self) -> Result<Vec<Template>, StoreError> {
        Ok(self
            .db
            .prepare(
                "SELECT id,name,content,category,revision FROM templates ORDER BY category,name,id",
            )?
            .query_map([], |r| {
                Ok(Template {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    content: r.get(2)?,
                    category: r.get(3)?,
                    revision: r.get(4)?,
                })
            })?
            .collect::<Result<_, _>>()?)
    }
    pub fn save_template(&mut self, t: &Template) -> Result<Template, StoreError> {
        if t.name.trim().is_empty()
            || t.name.len() > 1024
            || t.content.len() > 65536
            || t.category.len() > 1024
        {
            return Err(StoreError::Invalid("文案名称为空或字段过长".into()));
        }
        let mut t = t.clone();
        if t.id.is_empty() {
            t.id = Uuid::new_v4().to_string();
        }
        let old = t.revision;
        t.revision += 1;
        if old==0 {self.db.execute("INSERT INTO templates VALUES(?1,?2,?3,?4,?5)",params![t.id,t.name,t.content,t.category,t.revision])?;}else if self.db.execute("UPDATE templates SET name=?2,content=?3,category=?4,revision=?5 WHERE id=?1 AND revision=?6",params![t.id,t.name,t.content,t.category,t.revision,old])?!=1{return Err(StoreError::Conflict);}
        Ok(t)
    }
    pub fn delete_template(&mut self, id: &str, revision: i64) -> Result<(), StoreError> {
        if self.db.execute(
            "DELETE FROM templates WHERE id=?1 AND revision=?2",
            params![id, revision],
        )? != 1
        {
            return Err(StoreError::Conflict);
        }
        Ok(())
    }
    /// Preserve a closing editor without overwriting a newer revision.
    pub fn save_closing_draft(&mut self, draft: &Draft) -> Result<Draft, StoreError> {
        match self.save_draft(draft) {
            Err(StoreError::Conflict) => {
                let mut recovered = draft.clone();
                recovered.id = Uuid::new_v4().to_string();
                recovered.revision = 0;
                recovered.title = format!(
                    "{}（恢复副本）",
                    draft.title.chars().take(1000).collect::<String>()
                );
                let saved = self.save_draft(&recovered)?;
                // Shared immutable assets survive a conflicting editor close.
                self.db.execute("INSERT INTO images SELECT lower(hex(randomblob(16))),?1,filename,path,sha256,position FROM images WHERE draft_id=?2", params![saved.id,draft.id])?;
                Ok(saved)
            }
            result => result,
        }
    }
    /// Atomically claim an image/phone pair across session windows.
    /// Returns the existing completion, a newly claimed task, or a recovery query.
    pub fn claim_import(
        &mut self,
        image: &str,
        target: &str,
        durable: bool,
    ) -> Result<(ImportReceipt, bool), StoreError> {
        self.claim_import_recovering(image, target, durable, false)
    }
    /// A missing remote task may be resumed only after the user checks Photos.
    /// Reuse its original UUID so a delayed remote receipt still deduplicates it.
    pub fn claim_import_recovering(
        &mut self,
        image: &str,
        target: &str,
        durable: bool,
        confirmed_missing: bool,
    ) -> Result<(ImportReceipt, bool), StoreError> {
        use rusqlite::OptionalExtension;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let prior = tx.query_row("SELECT id,image_id,target,status,asset_id,error FROM imports WHERE image_id=?1 AND target=?2 ORDER BY updated_at DESC,rowid DESC LIMIT 1",params![image,target], |r| Ok(ImportReceipt {id:r.get(0)?,image_id:r.get(1)?,target:r.get(2)?,status:r.get(3)?,asset_id:r.get(4)?,error:r.get(5)?})).optional()?;
        let (mut receipt, querying) = match prior {
            Some(r) if r.status == "complete" => {
                tx.commit()?;
                return Ok((r, false));
            }
            Some(r) if r.status == "running" || r.status == "queued" => {
                return Err(StoreError::Invalid(
                    "此图片正在其他窗口导入，请稍后再试".into(),
                ));
            }
            Some(r) if r.status == "not_found" => {
                if !durable || !confirmed_missing {
                    tx.commit()?;
                    return Ok((r, false));
                }
                (r, false)
            }
            Some(r) if r.status == "unknown" => {
                if !durable {
                    return Err(StoreError::Invalid(
                        "此前导入结果未知，请先在手机相册核实".into(),
                    ));
                }
                (r, true)
            }
            _ => (
                ImportReceipt {
                    id: Uuid::new_v4().to_string(),
                    image_id: image.into(),
                    target: target.into(),
                    status: "running".into(),
                    asset_id: None,
                    error: None,
                },
                false,
            ),
        };
        receipt.status = "running".into();
        tx.execute("INSERT INTO imports VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET status=excluded.status,error=NULL,updated_at=excluded.updated_at",params![receipt.id,receipt.image_id,receipt.target,receipt.status,receipt.asset_id,receipt.error,now()])?;
        tx.commit()?;
        Ok((receipt, querying))
    }
    pub fn record_import(&mut self, r: &ImportReceipt) -> Result<(), StoreError> {
        self.db.execute("INSERT INTO imports VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET status=excluded.status,asset_id=excluded.asset_id,error=excluded.error,updated_at=excluded.updated_at",params![r.id,r.image_id,r.target,r.status,r.asset_id,r.error,now()])?;
        Ok(())
    }
    pub fn receipts(&self, image: &str, target: &str) -> Result<Vec<ImportReceipt>, StoreError> {
        Ok(self.db.prepare("SELECT id,image_id,target,status,asset_id,error FROM imports WHERE image_id=?1 AND target=?2 ORDER BY updated_at DESC,rowid DESC")?.query_map(params![image,target],|r|Ok(ImportReceipt{id:r.get(0)?,image_id:r.get(1)?,target:r.get(2)?,status:r.get(3)?,asset_id:r.get(4)?,error:r.get(5)?}))?.collect::<Result<_,_>>()?)
    }
}
fn read_draft(r: &rusqlite::Row<'_>) -> rusqlite::Result<Draft> {
    Ok(Draft {
        id: r.get(0)?,
        title: r.get(1)?,
        description: r.get(2)?,
        price_minor: r.get(3)?,
        currency: r.get(4)?,
        stock_note: r.get(5)?,
        revision: r.get(6)?,
        updated_at: r.get(7)?,
        price_input: r.get(8)?,
    })
}
