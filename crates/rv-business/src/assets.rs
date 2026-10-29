//! Persistent shared image cache. UI never loads remote image URLs.
use fs2::FileExt;
use rv_core::StorePaths;
use sha2::{Digest, Sha256};
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
    fetch_scoped(paths, None, url, None, None, false)
}
pub fn fetch_for_app(
    paths: &StorePaths,
    id: &rv_core::workspace::AppId,
    url: &str,
) -> Result<PathBuf, String> {
    fetch_scoped(paths, Some(id), url, None, None, false)
}
#[cfg(test)]
pub(crate) fn fetch_for_app_until(
    paths: &StorePaths,
    id: &rv_core::workspace::AppId,
    url: &str,
    deadline: std::time::Instant,
) -> Result<PathBuf, String> {
    fetch_scoped(paths, Some(id), url, Some(deadline), None, false)
}
/// Cache the current pull's images before the task is marked successful. Cache
/// hits do no network I/O; all misses use the same App's bound proxy and budget.
pub(crate) async fn cache_business_images_on_route(
    paths: &StorePaths,
    id: &rv_core::workspace::AppId,
    urls: Vec<String>,
    deadline: std::time::Instant,
    route: &crate::network::Route,
    interactive: bool,
) -> Result<(), String> {
    if urls.iter().all(|url| cached(paths, url).is_some()) {
        return Ok(());
    }
    let images = route.for_images(paths, id)?;
    cache_task_images_scoped(paths, id, urls, deadline, Some(images), interactive).await
}
pub(crate) async fn cache_business_images(
    paths: &StorePaths,
    id: &rv_core::workspace::AppId,
    urls: Vec<String>,
    deadline: std::time::Instant,
    interactive: bool,
) -> Result<(), String> {
    cache_task_images_scoped(paths, id, urls, deadline, None, interactive).await
}
async fn cache_task_images_scoped(
    paths: &StorePaths,
    id: &rv_core::workspace::AppId,
    urls: Vec<String>,
    deadline: std::time::Instant,
    route: Option<crate::network::Route>,
    interactive: bool,
) -> Result<(), String> {
    let urls: std::collections::HashSet<_> = urls.into_iter().collect();
    let missing: Vec<_> = urls
        .into_iter()
        .filter(|url| cached(paths, url).is_none())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let paths = paths.clone();
    let id = id.clone();
    tokio::task::spawn_blocking(move || {
        for url in missing {
            fetch_scoped(
                &paths,
                Some(&id),
                &url,
                Some(deadline),
                route.as_ref(),
                interactive,
            )?;
        }
        Ok(())
    })
    .await
    .map_err(|_| "图片缓存中断".to_string())?
}
fn fetch_scoped(
    paths: &StorePaths,
    id: Option<&rv_core::workspace::AppId>,
    url: &str,
    deadline: Option<std::time::Instant>,
    captured_route: Option<&crate::network::Route>,
    _interactive: bool,
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
    if let Some(deadline) = deadline {
        loop {
            if std::time::Instant::now() >= deadline {
                return Err("图片缓存等待超时".into());
            }
            match FileExt::try_lock_exclusive(&lock) {
                Ok(()) => break,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    } else {
        lock.lock_exclusive().map_err(|e| e.to_string())?;
    }
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
    // Preserve synchronous prerequisite errors before entering the blocking
    // image runtime. Actual occupancy/rotation happens asynchronously below.
    if captured_route.is_none() {
        if let Some(id) = id {
            crate::network::image_route(paths, id, client)?;
        }
    }
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
                let request = async {
                    let (route, _lease) = match (captured_route, id) {
                        (Some(route), _) => (Some(route.clone()), route.acquire().await),
                        (None, Some(id)) => {
                            let (route, lease) =
                                crate::network::acquire_image_route(paths, id, client).await?;
                            (Some(route), lease)
                        }
                        (None, None) => (None, None),
                    };
                    let client = route.as_ref().map(|route| &route.client).unwrap_or(client);
                    if let (Some(route), Some(id)) = (&route, id) {
                        route.validate_binding(paths, id)?;
                    }
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
                };
                if let Some(deadline) = deadline {
                    tokio::time::timeout(
                        deadline.saturating_duration_since(std::time::Instant::now()),
                        request,
                    )
                    .await
                    .map_err(|_| "图片缓存等待或下载超时".to_string())?
                } else {
                    request.await
                }
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
#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Read, net::TcpListener, sync::Arc, thread};
    #[test]
    fn image_deadline_bounds_waiting_for_the_cache_file_lock() {
        let dir = tempfile::tempdir().unwrap();
        let paths = StorePaths::in_dir(dir.path().to_owned());
        let url = "https://rv-fixture.invalid/locked-image";
        let path = target(&paths, url);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))
            .unwrap();
        FileExt::lock_exclusive(&lock).unwrap();
        let start = std::time::Instant::now();
        let result = fetch_scoped(
            &paths,
            None,
            url,
            Some(start + Duration::from_millis(50)),
            None,
            false,
        );
        assert!(result.unwrap_err().contains("超时"));
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(!path.exists());
    }
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
