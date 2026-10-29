//! Interactive avatar uploads and scheduled compatibility share the shop HTTP pipeline.
use crate::*;
use image::{GenericImageView, ImageDecoder, codecs::jpeg::JpegEncoder, imageops::FilterType};
use rusqlite::params;
use std::{io::Cursor, path::Path};

#[derive(Clone)]
pub struct PreparedAvatar {
    jpeg: Vec<u8>,
}
impl PreparedAvatar {
    pub fn bytes(&self) -> &[u8] {
        &self.jpeg
    }
}
pub fn prepare(path: &Path) -> Result<PreparedAvatar, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| "无法读取头像图片")?;
    let mut bytes = Vec::new();
    file.take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取图片失败")?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("请选择 8 MB 以内的图片".into());
    }
    prepare_bytes(&bytes)
}
fn prepare_bytes(bytes: &[u8]) -> Result<PreparedAvatar, String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "图片格式无法识别")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| "无法读取图片内容")?;
    let orientation = decoder.orientation().map_err(|_| "无法读取图片方向")?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|_| "无法读取图片内容")?;
    image.apply_orientation(orientation);
    let (width, height) = image.dimensions();
    let side = width.min(height);
    let rgba = image
        .crop_imm((width - side) / 2, (height - side) / 2, side, side)
        .resize_exact(800, 800, FilterType::Lanczos3)
        .into_rgba8();
    // JPEG has no alpha: composite transparent source pixels onto white.
    let rgb = image::RgbImage::from_fn(800, 800, |x, y| {
        let p = rgba.get_pixel(x, y);
        let a = u32::from(p[3]);
        image::Rgb(std::array::from_fn(|i| {
            ((u32::from(p[i]) * a + 255 * (255 - a) + 127) / 255) as u8
        }))
    });
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 90)
        .encode_image(&rgb)
        .map_err(|_| "头像编码失败")?;
    Ok(PreparedAvatar { jpeg })
}
pub async fn set(paths:StorePaths,id:AppId,image:PreparedAvatar)->Result<(),String> {
    crate::executor::runtime()?.spawn(async move {
        use tracing::Instrument;
        let span=tracing::info_span!("shop_avatar",app_id=%id.as_str());
        async move {OfflineRunner::new(None)?.avatar_execute(&paths,&id,None,image.jpeg).await}.instrument(span).await
    }).await.map_err(|e|format!("头像设置中断：{e}"))?
}
fn schema(db: &rusqlite::Connection) -> Result<(), rv_core::StoreError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS vinted_avatar_uploads (
        image_id TEXT PRIMARY KEY, app_id TEXT NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,
        jpeg BLOB NOT NULL
    );
    CREATE UNIQUE INDEX IF NOT EXISTS vinted_avatar_one_active_task ON tasks(app_id)
      WHERE kind='set_avatar' AND status IN ('pending','running');")?;
    Ok(())
}
pub fn enqueue(paths: &StorePaths, id: &AppId, image: &PreparedAvatar) -> Result<AppTask, String> {
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    let app = store.load(id).map_err(|e| e.to_string())?;
    if app.bundle_id != "lt.manodrabuziai.fr" {
        return Err("此 App 不支持设置头像".into());
    }
    let image_id = uuid::Uuid::new_v4().to_string();
    store.business_transaction(|db| {
        schema(db)?;
        tasks::admit_access_token(db,id.as_str(),None)?;
        let active:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE app_id=?1 AND kind='set_avatar' AND status IN ('pending','running'))",[id.as_str()],|r|r.get(0))?;
        if active {return Err(rv_core::StoreError::Invalid("已有头像任务，请在任务列表查看结果".into()));}
        db.execute("INSERT INTO vinted_avatar_uploads VALUES(?1,?2,?3)",params![image_id,id.as_str(),image.jpeg])?;
        Ok(())
    }).map_err(|e|e.to_string())?;
    let mut spec = TaskSpec::new(
        "set_avatar",
        task_cron_at(chrono::Utc::now().timestamp() + 1).map_err(|e| e.to_string())?,
    );
    spec.payload = json!({"image_id":image_id});
    let result = tasks::enqueue(paths, id, &spec);
    if result.is_err() {
        let _ = store.business_transaction(|db| {
            db.execute(
                "DELETE FROM vinted_avatar_uploads WHERE image_id=?1",
                [image_id],
            )?;
            Ok(())
        });
    }
    result
}

async fn response(request:reqwest::RequestBuilder,runner:&OfflineRunner,auth:&AppAuth)->Result<Value,String> {
    let mut reply=runner.send_authenticated(request,auth,false).await?;
    let status = reply.status();
    let mut endpoint=reply.url().clone();
    let _=endpoint.set_username("");let _=endpoint.set_password(None);endpoint.set_query(None);endpoint.set_fragment(None);
    let endpoint=endpoint.to_string();
    if !status.is_success() {
        return Err(format!("头像接口返回 HTTP {}", status.as_u16()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = reply
        .chunk()
        .await
        .map_err(|error|runner.http_error("response",&endpoint,"body",auth,error))?
    {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("头像接口回复过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let reply: Value =
        serde_json::from_slice(&bytes).map_err(|_| "头像回复无法解析，结果待确认")?;
    if reply.get("code").is_some_and(|code| code != 0) {
        return Err("平台未接受头像请求".into());
    }
    Ok(reply)
}
fn upload_reference(reply: &Value) -> Result<(&str, &str), String> {
    reply["temp_uuid"]
        .as_str()
        .filter(|s| !s.is_empty())
        .zip(
            reply["high_resolution"]["id"]
                .as_str()
                .filter(|s| !s.is_empty()),
        )
        .ok_or_else(|| "头像上传回执不完整，结果待确认".into())
}
fn profile(
    reply: &Value,
    user: u64,
    image: Option<&str>,
) -> Result<crate::shop::ShopProfile, String> {
    let p = crate::shop::ShopProfile::from_user(&reply["user"]).ok_or("未取得账号资料")?;
    if p.user_id != user {
        return Err("头像回复与店铺账号不一致".into());
    }
    if let Some(image) = image {
        if reply["user"]["photo"]["high_resolution"]["id"] != image || p.avatar_url.is_none() {
            return Err("账号头像尚未确认更新，请核对平台资料".into());
        }
    }
    Ok(p)
}
struct AvatarApi<'a>{current:&'a str,photos:&'a str,requires_proxy:bool}
const AVATAR_API:AvatarApi<'static>=AvatarApi{current:"https://www.vinted.com/api/v2/users/current",photos:"https://www.vinted.com/api/v2/photos",requires_proxy:true};
impl OfflineRunner {
    pub(crate) async fn avatar_task(
        &self,
        paths: &StorePaths,
        id: &AppId,
        task: &AppTask,
        image_id: &str,
    ) -> Result<(), String> {
        let mut store=WorkspaceStore::open(paths).map_err(|e|e.to_string())?;
        let bytes = store
            .business_transaction(|db| {
                schema(db)?;
                db.query_row(
                    "SELECT jpeg FROM vinted_avatar_uploads WHERE image_id=?1 AND app_id=?2",
                    params![image_id, id.as_str()],
                    |r| r.get::<_, Vec<u8>>(0),
                )
                .map_err(Into::into)
            })
            .map_err(|e| e.to_string())?;
        self.avatar_execute(paths,id,Some(task),bytes).await
    }
    async fn avatar_execute(&self,paths:&StorePaths,id:&AppId,task:Option<&AppTask>,bytes:Vec<u8>)->Result<(),String> {
        self.avatar_execute_with(paths,id,task,bytes,&AVATAR_API).await
    }
    async fn avatar_execute_with(&self,paths:&StorePaths,id:&AppId,task:Option<&AppTask>,bytes:Vec<u8>,api:&AvatarApi<'_>)->Result<(),String> {
        let mut store=WorkspaceStore::open(paths).map_err(|e|e.to_string())?;
        let app=store.load(id).map_err(|e|e.to_string())?;
        if app.archived{return Err("账号已归档".into())}
        if app.bundle_id!="lt.manodrabuziai.fr"{return Err("此 App 不支持设置头像".into())}
        let expected=crate::shop::ShopProfile::from_app(&app).map(|p|p.user_id)
            .or_else(||serde_json::from_str::<Value>(&app.metadata).ok()?["user_id"].as_u64()).ok_or("请先刷新店铺资料")?;
        let (route, _lease) = network::acquire_route(paths, id, &self.client, api.requires_proxy).await?;
        if store.load(id).map_err(|e|e.to_string())?.archived{return Err("账号已归档".into())}
        let auth = store
            .request_auth(id)
            .map_err(|e| e.to_string())?
            .ok_or("缺少账号凭据")?;
        let claimed=if let Some(task)=task {
            Some(store.claim_task_when(task,|db|tasks::admit_access_token(db,id.as_str(),Some(&task.id))).map_err(|e|e.to_string())?)
        }else{
None
        };
        let runner=Self{client:route.client.clone(),credentials:Some((paths.clone(),id.clone())),interactive:task.is_none()};
        let budget=Duration::from_secs(task.map_or(75,|task|task.timeout_seconds.into()));
        let deadline=std::time::Instant::now()+budget;
        let mut submitted = false;
        let mut assigned = false;
        let request = |method, url: &str| {
            device_headers(route.client.request(method, url), &auth)
                .bearer_auth(&auth.access_token)
                .header("Accept", "application/json")
                .header("Cache-Control", "no-cache")
        };
        let result =
            tokio::time::timeout(budget, async {
                let current_url=api.current;
                let current = response(request(reqwest::Method::GET, current_url), &runner, &auth).await?;
                profile(&current, expected, None)?;
                route.validate_binding(paths, id)?;
                let form = reqwest::multipart::Form::new()
                    .text("photo[type]", "user")
                    .part(
                        "photo[file]",
                        reqwest::multipart::Part::bytes(bytes)
                            .file_name("avatar.jpg")
                            .mime_str("image/jpeg")
                            .map_err(|_| "图片编码错误")?,
                    );
                submitted = true;
                let uploaded = response(
                    request(
                        reqwest::Method::POST,
                        api.photos,
                    )
                    .multipart(form),
                &runner, &auth)
                .await?;
                let (temporary, image) = upload_reference(&uploaded)?;
                route.validate_binding(paths, id)?;
                let assignment = response(
                    request(reqwest::Method::PUT, current_url)
                        .json(&json!({"user":{"photo_temp_uuid":temporary}})),
                &runner, &auth)
                .await?;
                assigned = true;
                profile(&assignment, expected, Some(image))?;
                route.validate_binding(paths, id)?;
                let verified = response(request(reqwest::Method::GET, current_url), &runner, &auth).await?;
                let profile = profile(&verified, expected, Some(image))?;
                assets::cache_business_images_on_route(
                    paths,
                    id,
                    profile.avatar_url.iter().cloned().collect(),
                    deadline,
                    &route,
                    task.is_none(),
                )
                .await?;
                Ok::<_, String>(profile)
            })
            .await
            .unwrap_or_else(|_| Err("头像任务超时，结果待确认".into()));
        match result {
            Ok(profile) => {
                let saved=if let Some(claimed)=&claimed {
                    store.complete_task_with(id,claimed,"completed",&json!({"avatar_updated":true}),None,Some(&json!({"shop":profile})),|_|Ok(()))
                }else{store.commit_business_data(id,&json!({"shop":profile}),|_|Ok(()))};
                if let Err(error)=saved {
                    tracing::error!(app_id=%id.as_str(),%error,"Avatar result persistence failed");
                    let message="头像已提交，本地保存失败，请刷新资料核对";
                    if let Some(claimed)=&claimed{let _=store.finish_task(claimed,"unknown",&json!({"error":message}));}
                    return Err(message.into());
                }
                Ok(())
            }
            Err(message) => {
                let http = message
                    .strip_prefix("头像接口返回 HTTP ")
                    .and_then(|s| s.parse::<u16>().ok());
                let rejected = !assigned
                    && (http.is_some_and(|status| (400..500).contains(&status))
                        || message == "平台未接受头像请求");
                if let Some(claimed)=&claimed {store
                    .finish_task_with_policy(
                        claimed,
                        if submitted && !rejected {
                            "unknown"
                        } else {
                            "failed"
                        },
                        &json!({"error":message}),
                        TaskFinishPolicy {
                            retryable: false,
                            stop_on_failure: matches!(http, Some(401 | 403)),
                        },
                    )
                    .map_err(|e| e.to_string())?;}
                Err(message)
            }
        }
    }
}

#[cfg(test)]
mod tests;
