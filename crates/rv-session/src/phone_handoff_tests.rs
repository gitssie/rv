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
    calls: Vec<String>,
    stopped: bool,
}
async fn run_case(corrupt_readback: bool, different_data: bool, stop_mode:u8) {
    let archive = vec![42; 450_000];
    let expected = manifest(&"a".repeat(64), &archive);
    let phone = Arc::new(Mutex::new(Phone {
        manifest: manifest(&if different_data { "b" } else { "a" }.repeat(64), b"old"),
        state: b"old".to_vec(),
        writes: Vec::new(),
        calls: Vec::new(),
        stopped: false,
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
                phone.calls.push(path.clone());
                let output = if path == "/api/app-state/stop" {
                    assert_eq!(input["bundle_id"],BUNDLE);
                    json!({"operation_id":"stop-op","method":"stop","status":"running"}).to_string().into_bytes()
                } else if path == "/api/app-state/result" {
                    assert_eq!(input["operation_id"],"stop-op");
                    phone.stopped=stop_mode==0;
                    let mut result=json!({"operation_id":"stop-op","method":"stop","status":"completed",
                        "result":{"bundle_id":BUNDLE,"running":stop_mode==2,"main_running":stop_mode==2,"extension_process_count":0}});
                    if stop_mode==1 {result["error"]=json!({"message":"fixture stop failed"});}
                    result.to_string().into_bytes()
                } else if path == "/api/app-state/upload" {
                    assert!(phone.stopped,"write must follow confirmed process exit");
                    let part = input["part"].as_str().unwrap().to_string();
                    let before=plist::Value::from_reader(std::io::Cursor::new(&phone.manifest)).unwrap();
                    assert_eq!(input["data_sha256"].as_str(),before.as_dictionary().unwrap()["archiveInfo"].as_dictionary().unwrap()["sha256"].as_string());
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
                    assert!(phone.stopped,"download must follow confirmed process exit");
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
    assert_eq!(result.is_ok(),stop_mode==0 && !corrupt_readback);
    assert_eq!(&phone.calls[..2],["/api/app-state/stop","/api/app-state/result"]);
    if stop_mode!=0 {
        assert_eq!(phone.calls.len(),2);
        assert!(phone.writes.is_empty());assert_eq!(phone.state,b"old");
        server.abort();return;
    }
    assert_eq!(phone.writes, vec!["state", "state", "state", "manifest"]);
    let actual=plist::Value::from_reader(std::io::Cursor::new(&phone.manifest)).unwrap();
    let local=plist::Value::from_reader(std::io::Cursor::new(&expected)).unwrap();
    let actual=actual.as_dictionary().unwrap();
    assert_eq!(actual["archiveInfo"].as_dictionary().unwrap()["sha256"].as_string(),Some(if different_data {"b"}else{"a"}.repeat(64).as_str()));
    assert_eq!(actual.get("stateArchiveInfo"),local.as_dictionary().unwrap().get("stateArchiveInfo"));
    assert_eq!(phone.state.len(),archive.len());
    server.abort();
}
#[tokio::test]
async fn phone_handoff_writes_state_then_manifest_and_reads_both_back() {
    run_case(false, false,0).await;
}
#[tokio::test]
async fn phone_handoff_rejects_corrupt_readback() {
    run_case(true, false,0).await;
}
#[tokio::test]
async fn phone_handoff_preserves_current_phone_data_when_local_archive_info_is_old() {
    run_case(false, true,0).await;
}

#[tokio::test]
async fn phone_handoff_never_reads_or_writes_state_when_stop_fails() {
    run_case(false,false,1).await;
}
#[tokio::test]
async fn phone_handoff_does_not_treat_stop_ack_or_running_process_as_exit() {
    run_case(false,false,2).await;
}

async fn live_identity_peer(body:serde_json::Value)->(PhoneHttpClient,Arc<Mutex<Vec<String>>>,tokio::task::JoinHandle<()>) {
    let calls=Arc::new(Mutex::new(Vec::new()));let observed=calls.clone();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port=listener.local_addr().unwrap().port();
    let server=tokio::spawn(async move {
        let (socket,_)=listener.accept().await.unwrap();
        let service=service_fn(move|request:Request<Incoming>| {
            let body=body.clone();let observed=observed.clone();
            async move {
                assert_eq!(request.version(),hyper::Version::HTTP_2);
                assert_eq!(request.headers()["authorization"],"Bearer fixture");
                let path=request.uri().path().to_string();observed.lock().unwrap().push(path.clone());
                assert_eq!(path,"/api/app-state/list","active detection must use independent catalog identity, never token or save interfaces");
                let input:serde_json::Value=serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
                assert_eq!(input,json!({"bundle_id":BUNDLE}));
                Ok::<_,std::convert::Infallible>(Response::new(Full::new(Bytes::from(json!({"operation_id":"list-op","method":"list","status":"completed","result":body}).to_string()))))
            }
        });
        let _=hyper::server::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
            .serve_connection(hyper_util::rt::TokioIo::new(socket),service).await;
    });
    (PhoneHttpClient::new("127.0.0.1",port,"fixture".into()).unwrap(),calls,server)
}
#[tokio::test]
async fn active_takeover_is_rejected_before_state_download_or_credential_changes() {
    use rv_core::{AddressBook,Connection};
    let dir=tempfile::tempdir().unwrap();let paths=StorePaths::in_dir(dir.path().into());
    let mut book=AddressBook::load(paths.clone()).unwrap();
    let device=Connection::new("fixture","192.0.2.1",5900);book.upsert(device.clone());book.save().unwrap();
    let mut store=WorkspaceStore::open(&paths).unwrap();
    // Matches the actual Shadow catalog shape: no current_backup_id field.
    store.sync_backups(device.id,BUNDLE,&json!({"bundle_id":BUNDLE,"format":"ShadowAppState2",
        "backups":[{"backup_id":format!("snapshot:{ID}"),"kind":"snapshot","idfv":ID}]})).unwrap();
    let id=AppId::from_idfv(ID).unwrap();let before=store.execution(&id).unwrap();
    let (client,calls,server)=live_identity_peer(json!({"bundle_id":BUNDLE,"format":"ShadowAppState2","current_idfv":ID,"backups":[]})).await;
    let command=AppCommand::TakeoverBackup{app_id:id.clone(),bundle:BUNDLE.into(),backup:format!("snapshot:{ID}"),
        store_root:paths.root.clone(),device_id:device.id,operation_id:before.operation_id.clone()};
    let error=execute(&client,&command).await.unwrap_err();
    assert!(error.contains("active") && error.contains("保存最新备份并切换"),"{error}");
    assert_eq!(calls.lock().unwrap().as_slice(),["/api/app-state/list"]);
    assert!(store.auth(&id).unwrap().is_none());
    assert!(store.app_state_info(&id).unwrap().is_none());
    assert_eq!(store.execution(&id).unwrap().operation_id,before.operation_id);
    server.abort();
}
#[tokio::test]
async fn inactive_identity_is_accepted_but_missing_identity_is_not_guessed() {
    for (current,allowed) in [(Some("7A70B21A-2F9D-4F66-81C4-6D21355758D7"),true),(Some("null"),true),(Some("invalid"),false),(None,false)] {
        let mut value=json!({"bundle_id":BUNDLE,"format":"ShadowAppState2","backups":[]});
        if let Some(current)=current {value["current_idfv"]=if current=="null" {serde_json::Value::Null}else{json!(current)};}
        let (client,_,server)=live_identity_peer(value).await;
        assert_eq!(require_inactive_backup(&client,&AppId::from_idfv(ID).unwrap(),BUNDLE).await.is_ok(),allowed);
        server.abort();
    }
}
