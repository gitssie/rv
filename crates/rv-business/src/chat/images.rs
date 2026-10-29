//! Immutable, conversation-scoped image intent. No HTTP at selection or queue time.
use super::*;
use image::{GenericImageView, ImageDecoder, codecs::jpeg::JpegEncoder, imageops::FilterType};
use rusqlite::{OptionalExtension, params};
use rv_core::{StoreError, StorePaths, workspace::*};
use std::{
    io::{Cursor, Read},
    path::Path,
};

#[derive(Clone)]
pub struct PreparedImage {
    jpeg: Vec<u8>,
}
impl PreparedImage {
    pub fn bytes(&self) -> &[u8] {
        &self.jpeg
    }
}
pub fn prepare(path: &Path) -> Result<PreparedImage, String> {
    let file = std::fs::File::open(path).map_err(|_| "无法读取图片")?;
    let mut bytes = Vec::new();
    file.take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取图片失败")?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("请选择 8 MB 以内的图片".into());
    }
    prepare_bytes(&bytes)
}
fn prepare_bytes(bytes: &[u8]) -> Result<PreparedImage, String> {
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
    let image = if image.width() > 1600 || image.height() > 1600 {
        image.resize(1600, 1600, FilterType::Lanczos3)
    } else {
        image
    };
    let (width, height) = image.dimensions();
    let rgba = image.into_rgba8();
    let rgb = image::RgbImage::from_fn(width, height, |x, y| {
        let p = rgba.get_pixel(x, y);
        let a = u32::from(p[3]);
        image::Rgb(std::array::from_fn(|i| {
            ((u32::from(p[i]) * a + 255 * (255 - a) + 127) / 255) as u8
        }))
    });
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 90)
        .encode_image(&rgb)
        .map_err(|_| "图片编码失败")?;
    Ok(PreparedImage { jpeg })
}
pub fn queue(
    paths: &StorePaths,
    app: &AppId,
    conversation: &str,
    image: &PreparedImage,
) -> Result<String, String> {
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    let instance = store.load(app).map_err(|e| e.to_string())?;
    if !strategy(&instance.bundle_id).is_some_and(|p| p.capabilities().upload) {
        return Err("此 App 不支持发送图片".into());
    }
    queue::writable(&mut store, app).map_err(|e| e.to_string())?;
    store.business_transaction(|db|{
        repository::schema(db)?;
        let allowed:Option<bool>=db.query_row("SELECT c.allow_reply FROM chat_conversations c JOIN app_instances a ON a.id=c.app_id WHERE c.id=?1 AND c.app_id=?2 AND a.archived=0",params![conversation,app.as_str()],|r|r.get(0)).optional()?;
        if allowed!=Some(true) {return Err(StoreError::Invalid("此会话不允许发送图片".into()));}
        crate::tasks::admit_access_token(db,app.as_str(),None)?;
        let now=service::now();let id=uuid::Uuid::new_v4().to_string();
        db.execute("INSERT INTO chat_outbox(id,conversation_id,body,created_at,status,available_at,updated_at) VALUES(?1,?2,'',?3,'pending',?3,?3)",params![id,conversation,now])?;
        db.execute("INSERT INTO chat_outbox_images(outbox_id,jpeg) VALUES(?1,?2)",params![id,image.jpeg])?;
        Ok(id)
    }).map_err(|e|e.to_string())
}

#[cfg(test)]
mod tests;
