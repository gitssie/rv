//! Persistent shared image cache. UI never loads remote image URLs.
use rv_core::StorePaths;
use sha2::{Digest, Sha256};
use fs2::FileExt;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::OnceLock,
    time::Duration,
};
fn target(paths: &StorePaths, url: &str) -> PathBuf {
    paths
        .root
        .join("business-images")
        .join(format!("{:x}.img", Sha256::digest(url.as_bytes())))
}
pub fn cached(paths: &StorePaths, url: &str) -> Option<PathBuf> {
    let path = target(paths, url);
    path.is_file().then_some(path)
}
/// Exclusive per-URL OS lock covers concurrent windows and RV processes;
/// atomic publication means readers only see completely validated images.
#[cfg(test)]
pub fn fetch(paths: &StorePaths, url: &str) -> Result<PathBuf, String> {
    fetch_scoped(paths, None, url)
}
pub fn fetch_for_app(
    paths: &StorePaths,
    id: &rv_core::workspace::AppId,
    url: &str,
) -> Result<PathBuf, String> {
    fetch_scoped(paths, Some(id), url)
}
fn fetch_scoped(
    paths: &StorePaths,
    id: Option<&rv_core::workspace::AppId>,
    url: &str,
) -> Result<PathBuf, String> {
    let url_parsed = reqwest::Url::parse(url).map_err(|_| "图片地址无效")?;
    if !matches!(url_parsed.scheme(), "http" | "https")
        || !url_parsed.username().is_empty()
        || url_parsed.password().is_some()
    {
        return Err("图片地址不受支持".into());
    }
    let path = target(paths, url);
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))
        .map_err(|e| e.to_string())?;
    lock.lock_exclusive().map_err(|e| e.to_string())?;
    if path.is_file() {
        return Ok(path);
    }
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::limited(3))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .expect("image HTTP client")
    });
    let route = id
        .map(|id| crate::network::image_route(paths, id, client))
        .transpose()?;
    let client = route.as_ref().map(|route| &route.client).unwrap_or(client);
    let bytes = {
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        RUNTIME
            .get_or_init(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("image runtime")
            })
            .block_on(async {
                let _lease = match &route {
                    Some(route) => route.acquire().await,
                    None => None,
                };
                let mut reply = client.get(url).send().await.map_err(|_| "图片下载失败")?;
                if !reply.status().is_success() {
                    return Err("图片服务未成功".to_string());
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = reply.chunk().await.map_err(|_| "图片下载中断")? {
                    if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                        return Err("图片过大".into());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(bytes)
            })
    }?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| "图片格式无效")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|_| "图片内容无效")?;
    let temporary = path.with_extension("part");
    let result = (|| {
        let mut output = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        output.write_all(&bytes)?;
        output.sync_all()?;
        fs::rename(&temporary, &path)?;
        std::fs::File::open(path.parent().unwrap())?.sync_all()
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(temporary);
        return Err(error.to_string());
    }
    Ok(path)
}
pub fn local(paths: &StorePaths, rows: &mut [crate::data::BusinessRow]) {
    for row in rows {
        row.visual.image_path = row
            .visual
            .image_url
            .as_ref()
            .and_then(|url| cached(paths, url));
    }
}
pub fn hydrate(
    paths: &StorePaths,
    id: &rv_core::workspace::AppId,
    rows: &mut [crate::data::BusinessRow],
) {
    let chunk = rows.len().div_ceil(4).max(1);
    std::thread::scope(|scope| {
        for rows in rows.chunks_mut(chunk) {
            scope.spawn(move || {
                for row in rows {
                    if let Some(url) = &row.visual.image_url {
                        row.visual.image_path = fetch_for_app(paths, id, url).ok();
                    }
                }
            });
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Read, net::TcpListener, sync::Arc, thread};
    #[test]
    fn concurrent_readers_and_restart_reuse_one_download() {
        let directory = tempfile::tempdir().unwrap();
        let paths = Arc::new(StorePaths::in_dir(directory.path().to_owned()));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/image", listener.local_addr().unwrap());
        let mut image = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut image, image::ImageFormat::Png)
            .unwrap();
        let bytes = image.into_inner();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let size = stream.read(&mut request).unwrap();
            assert!(
                !String::from_utf8_lossy(&request[..size])
                    .to_lowercase()
                    .contains("authorization:")
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .unwrap();
            stream.write_all(&bytes).unwrap();
        });
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let paths = paths.clone();
                let url = url.clone();
                thread::spawn(move || fetch(&paths, &url).unwrap())
            })
            .collect();
        let files: Vec<_> = readers.into_iter().map(|r| r.join().unwrap()).collect();
        server.join().unwrap();
        assert!(files.iter().all(|p| p == &files[0]));
        // Server is gone: a fresh StorePaths must still succeed without a request.
        assert_eq!(
            fetch(&StorePaths::in_dir(directory.path().to_owned()), &url).unwrap(),
            files[0]
        );
        assert!(cached(&paths, &url).is_some());
    }
    #[test]
    fn malformed_image_is_never_published() {
        let directory = tempfile::tempdir().unwrap();
        let paths = StorePaths::in_dir(directory.path().to_owned());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/bad", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            stream.read(&mut request).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nbad")
                .unwrap();
        });
        assert!(fetch(&paths, &url).is_err());
        server.join().unwrap();
        assert!(cached(&paths, &url).is_none());
        assert!(!target(&paths, &url).with_extension("part").exists());
    }
}
