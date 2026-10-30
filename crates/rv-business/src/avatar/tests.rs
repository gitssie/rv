use super::*;
use rv_core::{AddressBook, Connection};

fn fixture() -> (tempfile::TempDir, StorePaths, AppId) {
    let dir = tempfile::tempdir().unwrap();
    let paths = StorePaths::in_dir(dir.path().into());
    let mut book = AddressBook::load(paths.clone()).unwrap();
    let phone = Connection::new("fixture", "192.0.2.1", 5900);
    book.upsert(phone.clone());
    book.save().unwrap();
    let id = AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    store.sync_backups(phone.id,"lt.manodrabuziai.fr",&json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"kind":"snapshot","idfv":id.as_str(),"backup_id":"snapshot:1"}]})).unwrap();
    store
        .save_auth(
            &id,
            &AppAuth {
                access_token: "fixture".into(),
                ..Default::default()
            },
        )
        .unwrap();
    tasks::initialize(&paths, &id).unwrap();
    (dir, paths, id)
}
fn image() -> PreparedAvatar {
    let image = image::RgbaImage::from_fn(100, 60, |x, _| {
        if x < 20 || x >= 80 {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 255, 255])
        }
    });
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    prepare_bytes(png.get_ref()).unwrap()
}
#[test]
fn prepare_centers_square_image_and_composites_alpha() {
    let prepared = image();
    let decoded = image::load_from_memory(prepared.bytes())
        .unwrap()
        .into_rgb8();
    assert_eq!(decoded.dimensions(), (800, 800));
    assert!(decoded.get_pixel(400, 400)[2] > 240);
    assert!(decoded.get_pixel(2, 400)[2] > 240);
    let source = image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 0, 0]));
    let mut png = Cursor::new(Vec::new());
    source.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let decoded = image::load_from_memory(prepare_bytes(png.get_ref()).unwrap().bytes())
        .unwrap()
        .into_rgb8();
    assert!(decoded.get_pixel(0, 0)[0] > 250);
    assert!(prepare_bytes(b"not an image").is_err());
}
#[test]
fn avatar_readback_uses_image_identity_not_the_different_profile_photo_id() {
    let upload = json!({"id":36800,"temp_uuid":"upload-reference","high_resolution":{"id":"image-uuid"},"code":0});
    let (temporary, image) = upload_reference(&upload).unwrap();
    assert_eq!(temporary, "upload-reference");
    let readback = json!({"user":{"id":42,"login":"fixture","photo":{"id":34700,"url":"https://images.vinted.net/avatar.jpg","high_resolution":{"id":"image-uuid"}}}});
    assert_eq!(
        profile(&readback, 42, Some(image))
            .unwrap()
            .avatar_url
            .as_deref(),
        Some("https://images.vinted.net/avatar.jpg")
    );
    assert!(profile(&readback, 43, Some(image)).is_err());
    assert!(profile(&readback, 42, Some("other-image")).is_err());
    assert!(upload_reference(&json!({"id":1,"code":0})).is_err());
}

#[tokio::test]
async fn interactive_avatar_updates_profile_with_shared_session_and_no_task_records() {
    use tokio::io::{AsyncReadExt,AsyncWriteExt};use sha2::{Digest,Sha256};
    for (phone_source,keepalive_enabled) in [(false,true),(false,false),(true,false)] {
    let (_dir,paths,id)=fixture();let mut store=WorkspaceStore::open(&paths).unwrap();
    let mut app=store.load(&id).unwrap();app.metadata=json!({"user_id":42}).to_string();store.save(&app).unwrap();
    if !keepalive_enabled {
        for task in store.configured_tasks(&id).unwrap() {store.set_task_enabled(&task.id,false).unwrap();}
    }
    if phone_source {
        let device=AddressBook::load(paths.clone()).unwrap().connections()[0].id;
        store.set_token_source(&id,rv_core::workspace::TokenSource::Phone(Some(device))).unwrap();
    }
    let before=store.tasks(&id).unwrap();let prepared=image();
    let image_url="https://images.vinted.net/new-avatar.jpg";
    let cache=paths.root.join("business-images");std::fs::create_dir_all(&cache).unwrap();std::fs::write(cache.join(format!("{:x}.img",Sha256::digest(image_url.as_bytes()))),prepared.bytes()).unwrap();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let base=format!("http://{}",listener.local_addr().unwrap());
    let verified=json!({"user":{"id":42,"login":"shop","photo":{"url":image_url,"high_resolution":{"id":"image-id","url":image_url}}}});
    let replies=vec![json!({"user":{"id":42,"login":"shop"}}),json!({"temp_uuid":"4eedea8f-dd41-4458-8127-d04a5de14858","high_resolution":{"id":"image-id"}}),verified.clone(),verified];
    let http=tokio::spawn(async move {
        let mut requests=Vec::new();
        for body in replies {
            let (mut socket,_)=listener.accept().await.unwrap();let mut bytes=Vec::new();
            loop {
                let mut buf=[0;4096];let n=socket.read(&mut buf).await.unwrap();assert!(n>0);bytes.extend_from_slice(&buf[..n]);
                if let Some(end)=bytes.windows(4).position(|w|w==b"\r\n\r\n"){
                    let head=String::from_utf8_lossy(&bytes[..end]);let len=head.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v|v.trim().parse::<usize>().ok())).unwrap_or(0);
                    if bytes.len()>=end+4+len {break;}
                }
            }
            requests.push(String::from_utf8_lossy(&bytes).into_owned());let body=body.to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    let current=format!("{base}/current");let photos=format!("{base}/photos");let api=AvatarApi{current:&current,photos:&photos,requires_proxy:false};
    OfflineRunner::new(None).unwrap().avatar_execute_with(&paths,&id,None,prepared.jpeg,&api).await.unwrap();
    let requests=http.await.unwrap();assert_eq!(requests.len(),4);assert!(requests[0].starts_with("GET /current "));assert!(requests[1].starts_with("POST /photos "));assert!(requests[2].starts_with("PUT /current "));assert!(requests[3].starts_with("GET /current "));
    let session=store.auth(&id).unwrap().unwrap().headers["X-Session-Id"].clone();
    for request in &requests {assert!(request.contains(&format!("x-session-id: {session}")));}
    assert!(requests[2].contains("photo_temp_uuid"));assert_eq!(store.tasks(&id).unwrap(),before);
    assert_eq!(crate::shop::ShopProfile::from_app(&store.load(&id).unwrap()).unwrap().avatar_url.as_deref(),Some(image_url));
    }
}

#[tokio::test]
async fn archived_shop_cannot_start_interactive_avatar_requests() {
    let (_dir,paths,id)=fixture();let mut store=WorkspaceStore::open(&paths).unwrap();
    let mut app=store.load(&id).unwrap();app.metadata=json!({"user_id":42}).to_string();app.archived=true;store.save(&app).unwrap();
    let before=store.tasks(&id).unwrap();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let base=format!("http://{}",listener.local_addr().unwrap());
    let api=AvatarApi{current:&base,photos:&base,requires_proxy:false};
    let runner=OfflineRunner::new(None).unwrap();
    let result=tokio::time::timeout(Duration::from_secs(1),runner.avatar_execute_with(&paths,&id,None,image().jpeg,&api)).await.expect("archived account must reject before HTTP");
    assert!(result.unwrap_err().contains("归档"));assert_eq!(store.tasks(&id).unwrap(),before);
    assert!(tokio::time::timeout(Duration::from_millis(20),listener.accept()).await.is_err());
}

#[test]
fn avatars_cannot_create_new_scheduler_rows() {
    let (_dir,paths,id)=fixture();
    let store=WorkspaceStore::open(&paths).unwrap();let before=store.tasks(&id).unwrap();
    let mut spec=TaskSpec::new("set_avatar","* * * * * *");
    spec.payload=json!({"image_id":uuid::Uuid::new_v4().to_string()});
    assert!(tasks::enqueue(&paths,&id,&spec).unwrap_err().contains("直接"));
    assert_eq!(store.tasks(&id).unwrap(),before);
}
