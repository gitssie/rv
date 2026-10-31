use super::*;
use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
    service::service_fn,
};
use rv_core::workspace::state_sha256;
use std::sync::{Arc, Mutex};

const ID: &str = "C9EB7804-2062-4519-AC8D-6F68972574F3";
const BUNDLE: &str = "lt.manodrabuziai.fr";

fn encode(value: &Value) -> Vec<u8> {
    let mut bytes = Vec::new();
    plist::to_writer_binary(&mut bytes, value).unwrap();
    bytes
}
fn fixture(owner: Option<&str>) -> (Vec<u8>, Vec<u8>) {
    let payloads = [
        ("keychain/records.plist", encode(&json!({"records":[]}))),
        (
            "identity/state.plist",
            encode(&json!({"data":{"VendorIdentifier":ID}})),
        ),
        ("preferences/records.plist", encode(&json!({"records":[]}))),
    ];
    let mut tar = tar::Builder::new(zstd::stream::Encoder::new(Vec::new(), 3).unwrap());
    for (name, data) in &payloads {
        let mut header = tar::Header::new_ustar();
        header.set_size(data.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(&mut header, name, data.as_slice()).unwrap();
    }
    let archive = tar.into_inner().unwrap().finish().unwrap();
    let entries: Vec<_> = payloads
        .iter()
        .map(|(path, data)| json!({"path":path,"size":data.len(),"sha256":state_sha256(data)}))
        .collect();
    let mut manifest = json!({"format":"ShadowAppState2","bundleID":BUNDLE,"idfv":ID,
        "stateArchiveInfo":{"format":"pax","compression":"zstd","archiveSize":archive.len(),
            "sha256":state_sha256(&archive),"entries":entries}});
    if let Some(owner) = owner {
        manifest["owner"] = json!(owner);
    }
    (encode(&manifest), archive)
}

// Every fixture serves the public read path through real HTTP/2 and real tar.zst decoding.
async fn read_case(
    owner: Option<&str>,
    failure: Option<&str>,
) -> (Result<ShadowAppState, String>, Vec<String>) {
    let (manifest, archive) = fixture(owner);
    let expected_owner = owner.unwrap_or("phone").to_owned();
    let failure = failure.map(str::to_owned);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let service = service_fn(move |request: Request<Incoming>| {
            let (manifest, archive, expected_owner, failure, calls) = (
                manifest.clone(),
                archive.clone(),
                expected_owner.clone(),
                failure.clone(),
                calls.clone(),
            );
            async move {
                assert_eq!(request.version(), hyper::Version::HTTP_2);
                assert_eq!(request.headers()["authorization"], "Bearer fixture");
                let path = request.uri().path().to_owned();
                let input: Value = serde_json::from_slice(
                    &request.into_body().collect().await.unwrap().to_bytes(),
                )
                .unwrap();
                let mut calls = calls.lock().unwrap();
                let mut status = 200;
                let output = match path.as_str() {
                    "/api/app-state/ownership" => {
                        assert_eq!(
                            input,
                            json!({"bundle_id":BUNDLE,"backup_id":format!("snapshot:{ID}")})
                        );
                        json!({"operation_id":"owner-op","method":"ownership","status":"running"})
                            .to_string()
                            .into_bytes()
                    }
                    "/api/app-state/result" => {
                        assert_eq!(input["operation_id"], "owner-op");
                        let mut record = json!({"operation_id":"owner-op","method":"ownership","status":"completed",
                            "result":{"owner":expected_owner}});
                        if failure.as_deref() == Some("query") {
                            record["error"] = json!({"error_code":"snapshot_unavailable"});
                        }
                        record.to_string().into_bytes()
                    }
                    "/api/app-state/download" => {
                        assert_eq!(input["requester"], expected_owner);
                        assert_eq!(input["bundle_id"], BUNDLE);
                        assert_eq!(input["backup_id"], format!("snapshot:{ID}"));
                        if input["part"] == "manifest" {
                            manifest
                        } else {
                            assert_eq!(input["part"], "state");
                            assert_eq!(input["state_sha256"], state_sha256(&archive));
                            match failure.as_deref() {
                                Some("ownership_conflict" | "snapshot_changed") => {
                                    status = 409;
                                    json!({"error_code":failure,"error":"fixture conflict"})
                                        .to_string()
                                        .into_bytes()
                                }
                                Some("corrupt") => b"corrupted archive".to_vec(),
                                _ => archive,
                            }
                        }
                    }
                    "/api/device/info" => json!({"model":"iPhone14,5","os_version":"16.3"})
                        .to_string()
                        .into_bytes(),
                    _ => panic!("Ordinary sync must never claim, release, upload or stop: {path}"),
                };
                calls.push(format!("{path}:{}", input["part"].as_str().unwrap_or("")));
                Ok::<_, std::convert::Infallible>(
                    Response::builder()
                        .status(status)
                        .body(Full::new(Bytes::from(output)))
                        .unwrap(),
                )
            }
        });
        let _ = hyper::server::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
            .serve_connection(hyper_util::rt::TokioIo::new(socket), service)
            .await;
    });
    let client = PhoneHttpClient::new("127.0.0.1", port, "fixture".into()).unwrap();
    let result = read_shadow_state(
        &client,
        &AppId::from_idfv(ID).unwrap(),
        BUNDLE,
        &format!("snapshot:{ID}"),
    )
    .await;
    let calls = observed.lock().unwrap().clone();
    server.abort();
    (result, calls)
}

#[tokio::test]
async fn sync_reads_phone_rv_and_legacy_backups_without_transferring_ownership() {
    for owner in [Some("phone"), Some("rv"), None] {
        let (result, calls) = read_case(owner, None).await;
        let state = result.unwrap();
        assert!(state.payload("identity/state.plist").is_some());
        assert_eq!(
            calls,
            [
                "/api/app-state/ownership:",
                "/api/app-state/result:",
                "/api/app-state/download:manifest",
                "/api/app-state/download:state",
                "/api/device/info:"
            ]
        );
        let manifest = plist::Value::from_reader(std::io::Cursor::new(state.manifest())).unwrap();
        assert_eq!(
            manifest
                .as_dictionary()
                .unwrap()
                .get("owner")
                .and_then(plist::Value::as_string),
            owner
        );
    }
}
#[tokio::test]
async fn ownership_change_mid_download_stops_without_retry_or_switching_requester() {
    for owner in [Some("phone"), Some("rv")] {
        let (result, calls) = read_case(owner, Some("ownership_conflict")).await;
        assert!(result.unwrap_err().contains("归属已改变"));
        assert_eq!(calls.len(), 4);
        assert_eq!(calls.last().unwrap(), "/api/app-state/download:state");
    }
}
#[tokio::test]
async fn unavailable_or_invalid_owner_never_downloads_a_backup() {
    for (owner, failure) in [(Some("phone"), Some("query")), (Some("unexpected"), None)] {
        let (result, calls) = read_case(owner, failure).await;
        assert!(result.is_err());
        assert_eq!(calls.len(), 2);
    }
}
#[tokio::test]
async fn changing_or_corrupt_state_never_returns_a_profile() {
    for failure in ["snapshot_changed", "corrupt"] {
        let (result, calls) = read_case(Some("rv"), Some(failure)).await;
        assert!(result.is_err());
        assert_eq!(
            calls
                .iter()
                .filter(|path| path.starts_with("/api/app-state/download"))
                .count(),
            2
        );
    }
}
