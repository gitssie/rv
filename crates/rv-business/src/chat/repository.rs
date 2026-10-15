use super::*;
use rusqlite::{Connection, OptionalExtension, params};
use rv_core::{StoreError, StorePaths, workspace::*};
use uuid::Uuid;
pub(super) fn schema(db: &Connection) -> Result<(), StoreError> {
    db.execute_batch(include_str!("schema.sql"))?;
    let version: i64 = db.query_row(
        "SELECT version FROM chat_schema WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    if version != 1 {
        return Err(StoreError::Invalid("聊天数据版本不受支持".into()));
    }
    Ok(())
}
pub fn accounts(paths: &StorePaths) -> Result<Vec<ChatAccount>, String> {
    let store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    store
        .list(&AppQuery::default())
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|app| strategy(&app.bundle_id).is_some())
        .map(|app| {
            let has_auth = store.auth(&app.id).map_err(|e| e.to_string())?.is_some();
            let metadata: Value = serde_json::from_str(&app.metadata).unwrap_or_default();
            Ok(ChatAccount {
                id: app.id,
                title: app.title,
                bundle: app.bundle_id,
                has_auth,
                session_status: metadata["session_status"]
                    .as_str()
                    .unwrap_or("尚未检测")
                    .into(),
                market: metadata["market"]
                    .as_str()
                    .or_else(|| metadata["profile"]["market"].as_str())
                    .unwrap_or("")
                    .into(),
            })
        })
        .collect()
}

pub fn conversations(paths: &StorePaths) -> Result<Vec<Conversation>, String> {
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    store.business_transaction(|db| {
        schema(db)?;
        let mut statement = db.prepare("SELECT c.id,c.app_id,c.remote_id,c.title,c.preview,c.updated_at,c.unread,c.allow_reply,c.data,a.title,a.bundle_id FROM chat_conversations c JOIN app_instances a ON a.id=c.app_id WHERE a.archived=0 ORDER BY c.updated_at DESC,c.id")?;
        let mut rows = statement.query([])?; let mut output = Vec::new();
        while let Some(row) = rows.next()? {
            let mut c: Conversation = serde_json::from_str(&row.get::<_,String>(8)?)?;
            c.id=row.get(0)?; c.app_id=AppId::from_idfv(&row.get::<_,String>(1)?)?; c.remote_id=row.get(2)?; c.title=row.get(3)?; c.preview=row.get(4)?; c.updated_at=row.get(5)?; c.unread=row.get(6)?; c.allow_reply=row.get(7)?; c.account=row.get(9)?; c.bundle=row.get(10)?; output.push(c);
        }
        Ok(output)
    }).map_err(|e|e.to_string())
}
pub(super) fn persist_conversations(
    db: &Connection,
    app: &AppId,
    rows: &[Conversation],
    detail: bool,
) -> Result<(), StoreError> {
    schema(db)?;
    for c in rows {
        if c.app_id != *app || c.remote_id.is_empty() {
            return Err(StoreError::Invalid("会话账号归属无效".into()));
        }
        let existing: Option<(String,String,bool,i64)> = db.query_row("SELECT id,data,allow_reply,updated_at FROM chat_conversations WHERE app_id=?1 AND remote_id=?2",params![app.as_str(),c.remote_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let mut data = c.clone();
        if let Some((id, old, allow_reply, _)) = &existing {
            data.id = id.clone();
            if !detail {
                let old: Conversation = serde_json::from_str(old)?;
                if data.allow_reply.is_none() {
                    data.allow_reply = Some(*allow_reply);
                }
                data.items = old.items;
            }
        } else {
            data.id = Uuid::new_v4().to_string();
        }
        // Old history pages must not regress the inbox's latest message or unread state.
        if detail
            && existing
                .as_ref()
                .is_some_and(|(_, _, _, updated)| *updated > data.updated_at)
        {
            let old: Conversation = serde_json::from_str(&existing.as_ref().unwrap().1)?;
            data.updated_at = old.updated_at;
            data.preview = old.preview;
            data.unread = old.unread;
        }
        db.execute("INSERT INTO chat_conversations(id,app_id,remote_id,title,preview,updated_at,unread,allow_reply,detail_loaded,data) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(app_id,remote_id) DO UPDATE SET title=excluded.title,preview=excluded.preview,updated_at=excluded.updated_at,unread=excluded.unread,allow_reply=excluded.allow_reply,detail_loaded=MAX(chat_conversations.detail_loaded,excluded.detail_loaded),data=excluded.data",params![data.id,app.as_str(),data.remote_id,data.title,data.preview,data.updated_at,data.unread,data.allow_reply.unwrap_or(false),detail,data.to_string()])?;
    }
    Ok(())
}
trait JsonText {
    fn to_string(&self) -> String;
}
impl JsonText for Conversation {
    fn to_string(&self) -> String {
        serde_json::to_string(self).expect("conversation serialization")
    }
}
pub(super) fn key(db: &Connection, app: &AppId, remote: &str) -> Result<String, StoreError> {
    Ok(db.query_row(
        "SELECT id FROM chat_conversations WHERE app_id=?1 AND remote_id=?2",
        params![app.as_str(), remote],
        |r| r.get(0),
    )?)
}
pub(super) fn persist_detail(
    db: &Connection,
    app: &AppId,
    batch: &DetailBatch,
) -> Result<(), StoreError> {
    persist_conversations(db, app, std::slice::from_ref(&batch.conversation), true)?;
    let conversation = key(db, app, &batch.conversation.remote_id)?;
    for m in &batch.messages {
        let remote = m
            .remote_id
            .as_deref()
            .ok_or_else(|| StoreError::Invalid("消息缺少平台 ID".into()))?;
        let id = db
            .query_row(
                "SELECT id FROM chat_messages WHERE conversation_id=?1 AND remote_id=?2",
                params![conversation, remote],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let mut m = m.clone();
        m.id = id;
        db.execute("INSERT INTO chat_messages VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(conversation_id,remote_id) DO UPDATE SET created_at=excluded.created_at,body=excluded.body,data=excluded.data",params![m.id,conversation,remote,m.created_at,m.body,serde_json::to_string(&m)?])?;
        db.execute(
            "DELETE FROM chat_outbox WHERE conversation_id=?1 AND remote_id=?2",
            params![conversation, remote],
        )?;
    }
    for p in &batch.participants {
        db.execute("INSERT INTO chat_participants VALUES(?1,?2,?3,?4) ON CONFLICT(app_id,remote_id) DO UPDATE SET name=excluded.name,data=excluded.data",params![app.as_str(),p.remote_id,p.name,p.payload.to_string()])?;
    }
    Ok(())
}
pub fn thread(paths: &StorePaths, conversation: &str) -> Result<Thread, String> {
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    store.business_transaction(|db| {
        schema(db)?;
        db.execute("UPDATE chat_outbox SET status='unknown' WHERE status='sending' AND created_at<?1",[super::service::now()-90])?;
        let mut query=db.prepare("SELECT data FROM chat_messages WHERE conversation_id=?1 ORDER BY created_at,id")?;
        let messages=query.query_map([conversation],|r|r.get::<_,String>(0))?.map(|r|serde_json::from_str(&r?).map_err(StoreError::from)).collect::<Result<_,_>>()?;
        let mut query=db.prepare("SELECT id,body,created_at,status,remote_id FROM chat_outbox WHERE conversation_id=?1 ORDER BY created_at,id")?;
        let pending=query.query_map([conversation],|r| Ok(Message{id:r.get(0)?,remote_id:r.get(4)?,outgoing:true,sender_id:String::new(),created_at:r.get(2)?,kind:"text".into(),body:r.get(1)?,status:r.get(3)?,images:vec![],payload:Value::Null}))?.collect::<Result<_,_>>()?;
        Ok(Thread{messages,pending})
    }).map_err(|e|e.to_string())
}
pub fn save_draft(paths: &StorePaths, conversation: &str, body: &str) -> Result<(), String> {
    if body.len() > 65536 {
        return Err("草稿过长".into());
    }
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    store.business_transaction(|db| {schema(db)?;db.execute("INSERT INTO chat_drafts VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET body=excluded.body",params![conversation,body])?;Ok(())}).map_err(|e|e.to_string())
}
pub fn draft(paths: &StorePaths, conversation: &str) -> Result<String, String> {
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    store
        .business_transaction(|db| {
            schema(db)?;
            Ok(db
                .query_row(
                    "SELECT body FROM chat_drafts WHERE conversation_id=?1",
                    [conversation],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or_default())
        })
        .map_err(|e| e.to_string())
}
pub(super) fn begin_send(
    db: &Connection,
    app: &AppId,
    remote: &str,
    text: &str,
) -> Result<String, StoreError> {
    schema(db)?;
    let conversation = key(db, app, remote)?;
    let allowed: bool = db.query_row(
        "SELECT allow_reply FROM chat_conversations WHERE id=?1",
        [&conversation],
        |r| r.get(0),
    )?;
    if !allowed {
        return Err(StoreError::Invalid("此会话不允许回复".into()));
    }
    // Crash recovery never resubmits; unresolved outcomes block another send.
    db.execute("UPDATE chat_outbox SET status='unknown' WHERE conversation_id=?1 AND status='sending' AND created_at<?2",params![conversation,super::service::now()-90])?;
    let id = Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO chat_outbox VALUES(?1,?2,?3,?4,'sending',NULL)",
        params![id, conversation, text, super::service::now()],
    )?;
    db.execute(
        "DELETE FROM chat_drafts WHERE conversation_id=?1",
        [&conversation],
    )?;
    Ok(id)
}
pub(super) fn finish_send(
    db: &Connection,
    id: &str,
    status: &str,
    remote: Option<&str>,
) -> Result<(), StoreError> {
    if db.execute(
        "UPDATE chat_outbox SET status=?2,remote_id=?3 WHERE id=?1 AND status='sending'",
        params![id, status, remote],
    )? != 1
    {
        return Err(StoreError::Conflict);
    }
    Ok(())
}
pub fn dismiss_pending(paths: &StorePaths, conversation: &str, id: &str) -> Result<(), String> {
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    store
        .business_transaction(|db| {
            schema(db)?;
            db.execute(
                "DELETE FROM chat_outbox WHERE id=?1 AND conversation_id=?2 AND status!='sending'",
                params![id, conversation],
            )?;
            Ok(())
        })
        .map_err(|e| e.to_string())
}
