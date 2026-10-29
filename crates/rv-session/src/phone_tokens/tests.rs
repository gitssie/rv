use super::*;
use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
    service::service_fn,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn live_h2_read_uses_current_identity_keeps_refresh_and_manual_request_wakes_waiters() {
    let dir = tempfile::tempdir().unwrap();
    let paths = StorePaths::in_dir(dir.path().into());
    let mut book = rv_core::AddressBook::load(paths.clone()).unwrap();
    let device = rv_core::Connection::new("phone", "127.0.0.1", 5900);
    book.upsert(device.clone());
    book.save().unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let id = AppId::from_idfv("C148FA97-37B0-4BA1-9DBB-AF619062F420").unwrap();
    store.sync_backups(device.id,"lt.manodrabuziai.fr",&serde_json::json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"kind":"snapshot","backup_id":"snapshot:fixture","idfv":id.as_str()}]})).unwrap();
    store
        .save_auth(
            &id,
            &AppAuth {
                access_token: "old".into(),
                refresh_token: Some("keep-rv-refresh".into()),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .set_token_source(&id, TokenSource::Phone(Some(device.id)))
        .unwrap();
    let task = store
        .enqueue_task(&id, &TaskSpec::new("products", "0 */15 * * * *"))
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let response_id = id.as_str().to_owned();
    let server = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let calls = calls.clone();
            let response_id = response_id.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<Incoming>| {
                    let calls = calls.clone();
                    let response_id = response_id.clone();
                    async move {
                        assert_eq!(request.version(), hyper::Version::HTTP_2);
                        assert_eq!(request.uri().path(), "/api/apps/access-token");
                        assert_eq!(request.headers()["authorization"], "Bearer control-token");
                        let body = request.into_body().collect().await.unwrap().to_bytes();
                        assert_eq!(
                            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
                            serde_json::json!({"bundle_id":"lt.manodrabuziai.fr"})
                        );
                        let n = calls.fetch_add(1, Ordering::SeqCst);
                        let reply = serde_json::json!({"bundle_id":"lt.manodrabuziai.fr","idfv":response_id,"access_token":if n==0 {"old"}else{"new"},"expires_at":2000000000,"headers":{},"refresh_token":"must-not-import"});
                        Ok::<_, std::convert::Infallible>(Response::new(Full::new(Bytes::from(
                            reply.to_string(),
                        ))))
                    }
                });
                let _ =
                    hyper::server::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
                        .serve_connection(hyper_util::rt::TokioIo::new(socket), service)
                        .await;
            });
        }
    });
    let client = PhoneHttpClient::new("127.0.0.1", port, "control-token".into()).unwrap();
    let bus = workspace_bus(&paths).unwrap();
    let results = bus.subscribe::<PhoneTokenResult>();
    let sync = tokio::spawn(run(paths.clone(), device.id, client));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(count.load(Ordering::SeqCst),0,"a valid imported token is used without reading the phone on connect");
    store.request_phone_token(&id,true).unwrap();
    tokio::time::timeout(Duration::from_secs(2), results.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    store.reject_access_token(&id, "old").unwrap();
    store.wait_for_access_token(&id).unwrap();
    for _ in 0..10 {
        store.request_phone_token(&id, false).unwrap();
    }
    store.request_phone_token(&id, true).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let result = results.recv().await.unwrap();
            if result.manual {
                result.result.as_ref().unwrap();
                break;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let auth = store.auth(&id).unwrap().unwrap();
    assert_eq!(auth.access_token, "new");
    assert_eq!(auth.refresh_token.as_deref(), Some("keep-rv-refresh"));
    assert!(store.task(&task.id).unwrap().error_code.is_none());
    sync.abort();
    server.abort();
}
