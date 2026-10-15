//! Shared on-device App artwork, cached once per Bundle ID across cards and sessions.
use gpui::RenderImage;
use image::{ImageBuffer, Rgba};
use rv_core::StorePaths;
use smallvec::SmallVec;
use std::{path::PathBuf, sync::Arc};

fn path(paths: &StorePaths, bundle: &str) -> PathBuf {
    let key: String = bundle
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    paths.root.join("app-icons").join(format!("{key}.png"))
}
pub(crate) fn load(paths: &StorePaths, bundle: &str) -> Option<Arc<RenderImage>> {
    let bytes = std::fs::read(path(paths, bundle)).ok()?;
    if bytes.len() > 350_000 {
        return None;
    }
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(128);
    limits.max_image_height = Some(128);
    limits.max_alloc = Some(1024 * 1024);
    reader.limits(limits);
    let mut pixels = reader.decode().ok()?.into_rgba8();
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(SmallVec::from_elem(
        image::Frame::new(pixels),
        1,
    ))))
}
pub(crate) fn save(
    paths: &StorePaths,
    bundle: &str,
    width: u32,
    height: u32,
    bgra: &[u8],
) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > 128
        || height > 128
        || bgra.len() != (width * height * 4) as usize
    {
        return Err("Invalid App icon dimensions".into());
    }
    if load(paths, bundle).is_some() {
        return Ok(());
    }
    let destination = path(paths, bundle);
    std::fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
    let mut rgba = bgra.to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let image = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba).unwrap();
    let mut encoded = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    let temporary = destination.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = std::fs::write(&temporary, encoded.into_inner())
        .and_then(|_| std::fs::rename(&temporary, &destination));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn app_artwork_survives_reload_and_is_shared_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let paths = StorePaths::in_dir(dir.path().into());
        let pixels = [11, 22, 33, 255];
        save(&paths, "lt.manodrabuziai.fr", 1, 1, &pixels).unwrap();
        let reopened = StorePaths::in_dir(dir.path().into());
        assert_eq!(
            load(&reopened, "lt.manodrabuziai.fr")
                .unwrap()
                .as_bytes(0)
                .unwrap(),
            &pixels
        );
        save(&reopened, "lt.manodrabuziai.fr", 1, 1, &[44, 55, 66, 255]).unwrap();
        assert_eq!(
            load(&paths, "lt.manodrabuziai.fr")
                .unwrap()
                .as_bytes(0)
                .unwrap(),
            &pixels
        );
        assert!(load(&paths, "other.bundle").is_none());
    }
    #[test]
    fn corrupt_cache_is_replaced_and_invalid_pixels_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let paths = StorePaths::in_dir(dir.path().into());
        std::fs::create_dir_all(path(&paths, "bundle").parent().unwrap()).unwrap();
        std::fs::write(path(&paths, "bundle"), b"corrupt").unwrap();
        assert!(save(&paths, "bundle", 129, 1, &[]).is_err());
        assert!(save(&paths, "bundle", 1, 1, &[]).is_err());
        save(&paths, "bundle", 1, 1, &[0, 1, 2, 255]).unwrap();
        assert!(load(&paths, "bundle").is_some());
    }
}
