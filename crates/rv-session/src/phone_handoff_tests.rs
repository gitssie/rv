use super::*;
use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
    service::service_fn,
};
use std::sync::{Arc, Mutex};

const ID: &str = "C9EB7804-2062-4519-AC8D-6F68972574F3";
const BUNDLE: &str = "lt.manodrabuziai.fr";
fn manifest(data_hash: &str, archive: &[u8]) -> Vec<u8> {
    let mut info = plist::Dictionary::new();
    info.insert("sha256".into(), plist::Value::String(data_hash.into()));
    let mut state = plist::Dictionary::new();
    state.insert("sha256".into(), plist::Value::String(state_sha256(archive)));
    state.insert(
        "archiveSize".into(),
        plist::Value::Integer((archive.len() as u64).into()),
    );
    let mut root = plist::Dictionary::new();
    for (key, value) in [
        ("format", "ShadowAppState2"),
        ("bundleID", BUNDLE),
        ("idfv", ID),
    ] {
        root.insert(key.into(), plist::Value::String(value.into()));
    }
    root.insert("archiveInfo".into(), plist::Value::Dictionary(info));
    root.insert("stateArchiveInfo".into(), plist::Value::Dictionary(state));
    let mut bytes = Vec::new();
    plist::Value::Dictionary(root)
        .to_writer_binary(&mut bytes)
        .unwrap();
    bytes
}
#[derive(Default)]
struct Phone {
    manifest: Vec<u8>,
    state: Vec<u8>,
    writes: Vec<String>,
}
async fn run_case(corrupt_readback: bool, different_data: bool) {
    let archive = vec![42; 450_000];
    let expected = manifest(&"a".repeat(64), &archive);
    let phone = Arc::new(Mutex::new(Phone {
        manifest: manifest(&if different_data { "b" } else { "a" }.repeat(64), b"old"),
        state: b"old".to_vec(),
        writes: Vec::new(),
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = phone.clone();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let service = service_fn(move |request: Request<Incoming>| {
            let phone = state.clone();
            async move {
                assert_eq!(request.version(), hyper::Version::HTTP_2);
                assert_eq!(request.headers()["authorization"], "Bearer fixture");
                let path = request.uri().path().to_owned();
                let body = request.into_body().collect().await.unwrap().to_bytes();
                let input: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let mut phone = phone.lock().unwrap();
                let output = if path == "/api/app-state/upload" {
                    let part = input["part"].as_str().unwrap().to_string();
                    phone.writes.push(part.clone());
                    let offset = input["offset"].as_u64().unwrap() as usize;
                    let bytes = STANDARD.decode(input["data"].as_str().unwrap()).unwrap();
                    let target = if part == "state" {
                        &mut phone.state
                    } else {
                        &mut phone.manifest
                    };
                    if offset == 0 {
                        target.clear();
                    }
                    assert_eq!(target.len(), offset);
                    target.extend_from_slice(&bytes);
                    json!({"ok":true,"received":target.len(),"complete":target.len() as u64==input["total"].as_u64().unwrap()}).to_string().into_bytes()
                } else {
                    assert_eq!(path, "/api/app-state/download");
                    if input["part"] == "manifest" {
                        phone.manifest.clone()
                    } else if corrupt_readback {
                        b"corrupted".to_vec()
                    } else {
                        phone.state.clone()
                    }
                };
                Ok::<_, std::convert::Infallible>(Response::new(Full::new(Bytes::from(output))))
            }
        });
        let _ = hyper::server::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
            .serve_connection(hyper_util::rt::TokioIo::new(socket), service)
            .await;
    });
    let client = PhoneHttpClient::new("127.0.0.1", port, "fixture".into()).unwrap();
    let result = write_parts(
        &client,
        &AppId::from_idfv(ID).unwrap(),
        BUNDLE,
        "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        &expected,
        &archive,
    )
    .await;
    let phone = phone.lock().unwrap();
    if different_data {
        assert!(result.is_err());
        assert!(phone.writes.is_empty());
        assert_eq!(phone.state, b"old");
    } else {
        assert_eq!(result.is_ok(), !corrupt_readback);
        assert_eq!(phone.writes, vec!["state", "state", "state", "manifest"]);
        assert_eq!(phone.manifest, expected);
        assert_eq!(phone.state.len(), archive.len());
    }
    server.abort();
}
#[tokio::test]
async fn phone_handoff_writes_state_then_manifest_and_reads_both_back() {
    run_case(false, false).await;
}
#[tokio::test]
async fn phone_handoff_rejects_corrupt_readback() {
    run_case(true, false).await;
}
#[tokio::test]
async fn phone_handoff_rejects_other_data_before_first_write() {
    run_case(false, true).await;
}
