//! Read-only Shadow catalog test using RV's production HTTP control/runtime.
//! No VNC session. --state reads state; --write-back-unchanged explicitly verifies same-content writeback into the selected phone backup using an isolated local DB.
//! --import-existing-rv explicitly imports into an existing RV card; --stop-app stops the phone App first.
use rv_core::workspace::{AppId, WorkspaceStore};
use rv_core::{AddressBook, ConnectRequest, Connection, StorePaths};
use rv_session::{
    AppCommand, AppEvent, FunctionCommand, FunctionEvent, SessionEvent, SessionHandle,
};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let host = args
        .first()
        .ok_or("usage: live_app_state_backup_probe HOST [HTTP_PORT] [--password-stdin]")?;
    let port = args
        .get(1)
        .filter(|arg| !arg.starts_with("--"))
        .map(|arg| arg.parse::<u16>())
        .transpose()?
        .unwrap_or(6000);
    let password = if args.iter().any(|arg| arg == "--password-stdin") {
        let mut value = String::new();
        std::io::stdin().read_line(&mut value)?;
        Some(value.trim_end_matches(['\r', '\n']).to_string())
    } else {
        None
    };
    let mut device = Connection::new("Shadow catalog probe", host, 5900);
    device.zxtouch_port = Some(port);
    let request = ConnectRequest::from_connection(&device, password);
    let session = SessionHandle::spawn_control(request);
    let result=async {
        let deadline=Instant::now()+Duration::from_secs(20);
        loop {
            if Instant::now()>=deadline{return Err("HTTP control authentication timed out".into());}
            if matches!(session.try_recv(),Some(SessionEvent::ControlState(true))){break;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if args.iter().any(|arg|arg=="--stop-app") {
            session.app(AppCommand::Terminate("lt.manodrabuziai.fr".into()));
            let deadline=Instant::now()+Duration::from_secs(20);
            loop {
                if Instant::now()>=deadline {return Err("phone App stop timed out".into());}
                match session.try_recv() {
                    Some(SessionEvent::Apps(AppEvent::Finished(AppCommand::Terminate(_))))=>break,
                    Some(SessionEvent::Apps(AppEvent::Failed{command:AppCommand::Terminate(_),message}))=>return Err(message.into()),
                    _=>{},
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            println!("Phone App terminated through authenticated HTTP App control API");
        }
        let started=Instant::now();
        session.function(FunctionCommand::Invoke{id:"app-list".into(),text:"App=lt.manodrabuziai.fr".into()});
        let deadline=Instant::now()+Duration::from_secs(20);
        let response=loop {
            if Instant::now()>=deadline{return Err("Shadow list timed out".into());}
            if let Some(SessionEvent::Functions(mut event))=session.try_recv() {
                while let FunctionEvent::ForOperation{event:inner,..}=event {event=*inner;}
                match event {
                    FunctionEvent::State{task:32,record} if record["status"]=="completed"=>{
                        if record.get("error").is_some(){return Err("Shadow list failed".into());}
                        break record["result"].clone();
                    },
                    FunctionEvent::Error(message)|FunctionEvent::Uncertain(message)=>return Err(message.into()),
                    _=>{},
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(response["format"],"ShadowAppState2");
        let rows=response["backups"].as_array().ok_or("missing backups")?;
        let elapsed=started.elapsed().as_millis();
        let dir=tempfile::tempdir()?;let data_dir=args.iter().find_map(|arg|arg.strip_prefix("--state-dir=")).map(std::path::PathBuf::from).unwrap_or_else(||dir.path().into());let paths=StorePaths::in_dir(data_dir);
        let mut book=AddressBook::load(paths.clone())?;book.upsert(device.clone());book.save()?;
        let mut store=WorkspaceStore::open(&paths)?;
        let report=store.sync_backups(device.id,"lt.manodrabuziai.fr",&response)?;
        assert_eq!(report.created,rows.len());
        for row in rows {
            let id=AppId::from_idfv(row["idfv"].as_str().ok_or("missing IDFV")?)?;
            assert_eq!(store.backups(&id)?[0].backup_id,row["backup_id"].as_str().unwrap());
        }
        assert_eq!(store.sync_backups(device.id,"lt.manodrabuziai.fr",&response)?.created,0);
        println!("RV Rust HTTP control -> function stream -> Shadow catalog -> SQLite PASS: {} App, query {}ms, repeat import no duplicates",report.created,elapsed);
        if args.iter().any(|arg|arg=="--edit-metadata-roundtrip") {
            let row=rows.first().ok_or("no Shadow backup")?;
            let backup=row["backup_id"].as_str().ok_or("no backup id")?;
            let title=row["display_name"].as_str().ok_or("no title")?;
            let note=row["note"].as_str().unwrap_or("");
            if title.contains(['\n','\r']) || note.contains(['\n','\r']) {return Err("roundtrip probe requires single-line original metadata".into());}
            for name in ["RV metadata probe",title] {
                let text=format!("App=lt.manodrabuziai.fr\n备份={backup}\n名称={name}\n备注={note}");
                session.function(FunctionCommand::Invoke{id:"app-edit".into(),text});
                let deadline=Instant::now()+Duration::from_secs(20);
                loop {
                    if Instant::now()>=deadline{return Err("Shadow metadata edit timed out; query metadata before retrying".into());}
                    if let Some(SessionEvent::Functions(mut event))=session.try_recv() {
                        while let FunctionEvent::ForOperation{event:inner,..}=event {event=*inner;}
                        match event {
                            FunctionEvent::State{task:32,record} if record["status"]=="completed"=>{
                                if record.get("error").is_some(){return Err("Shadow metadata edit returned failure".into());}
                                let edited=record["result"]["backups"].as_array().and_then(|items|items.iter().find(|item|item["backup_id"]==backup)).ok_or("edited row absent")?;
                                if edited["display_name"]!=name || edited["note"]!=note{return Err("metadata readback mismatch".into());}
                                break;
                            },
                            FunctionEvent::Error(message)|FunctionEvent::Uncertain(message)=>return Err(message.into()),
                            _=>{},
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
            println!("Production HTTP2 metadata edit -> updated catalog -> original title/note restored PASS");
        }
        if args.iter().any(|arg|arg=="--state") {
            for row in rows {
                let id=AppId::from_idfv(row["idfv"].as_str().unwrap())?;
                session.app(AppCommand::ReadProfile{app_id:id.clone(),bundle:"lt.manodrabuziai.fr".into(),backup:row["backup_id"].as_str().unwrap().into()});
                let deadline=Instant::now()+Duration::from_secs(60);
                let downloaded=loop {
                    if Instant::now()>=deadline{return Err("state read timed out".into());}
                    match session.try_recv() {
                        Some(SessionEvent::Apps(AppEvent::Profile(profile)))=>break profile,
                        Some(SessionEvent::Apps(AppEvent::Failed{message,..}))=>return Err(message.into()),
                        _=>tokio::time::sleep(Duration::from_millis(10)).await,
                    }
                };
                store.import_app_state(&id,Some(device.id),&downloaded.state)?;
                let auth=store.auth(&id)?.ok_or("missing imported auth")?;
                assert!(!auth.access_token.is_empty());assert!(auth.request_context().is_some());
                assert!(auth.headers.contains_key("User-Agent"));assert!(!auth.headers.contains_key("X-Session-Id"));
                let exported=store.prepare_state_upload(&id)?;
                if exported.archive()!=downloaded.state.archive() {return Err("unchanged imported state was unexpectedly repacked".into());}
                println!("Rust state download/import/direct credential read/export PASS: {} bytes, request context fields={:?}",exported.archive().len(),auth.headers.keys().collect::<Vec<_>>());
                if args.iter().any(|arg|arg=="--write-back-unchanged") {
                    let handoff=store.begin_phone_handoff(&id,device.id)?;
                    session.app(AppCommand::HandoffBackup {app_id:id.clone(),bundle:"lt.manodrabuziai.fr".into(),backup:row["backup_id"].as_str().unwrap().into(),store_root:paths.root.clone(),device_id:device.id,operation_id:handoff.operation_id});
                    let deadline=Instant::now()+Duration::from_secs(120);
                    loop {
                        if Instant::now()>=deadline{return Err("backup writeback timed out".into());}
                        match session.try_recv() {
                            Some(SessionEvent::Apps(AppEvent::Finished(AppCommand::HandoffBackup{..})))=>break,
                            Some(SessionEvent::Apps(AppEvent::Failed{command:AppCommand::HandoffBackup{..},message}))=>return Err(message.into()),
                            _=>tokio::time::sleep(Duration::from_millis(10)).await,
                        }
                    }
                    assert_eq!(store.execution(&id)?.mode,rv_core::workspace::ExecutionMode::Phone);
                    println!("Same-content phone backup state -> manifest -> readback verification -> persisted phone ownership PASS (isolated RV database)");
                    if args.iter().any(|arg|arg=="--return-to-rv") {
                        let execution=store.execution(&id)?;
                        session.app(AppCommand::TakeoverBackup {app_id:id.clone(),bundle:"lt.manodrabuziai.fr".into(),backup:row["backup_id"].as_str().unwrap().into(),store_root:paths.root.clone(),device_id:device.id,operation_id:execution.operation_id});
                        let deadline=Instant::now()+Duration::from_secs(60);
                        loop {
                            if Instant::now()>=deadline{return Err("RV takeover timed out".into());}
                            match session.try_recv() {
                                Some(SessionEvent::Apps(AppEvent::Finished(AppCommand::TakeoverBackup{..})))=>break,
                                Some(SessionEvent::Apps(AppEvent::Failed{command:AppCommand::TakeoverBackup{..},message}))=>return Err(message.into()),
                                _=>tokio::time::sleep(Duration::from_millis(10)).await,
                            }
                        }
                        assert_eq!(store.execution(&id)?.mode,rv_core::workspace::ExecutionMode::Rv);
                        if store.auth(&id)?.ok_or("missing returned credentials")?.access_token!=auth.access_token{return Err("phone credential adoption mismatch".into());}
                        println!("Phone backup -> fresh state read -> credential adoption -> persisted RV ownership PASS (isolated database; no business request or phone App operation)");
                    }

                }

                if args.iter().any(|arg|arg=="--import-existing-rv") {
                    let live_paths=StorePaths::default_dir()?;
                    if !live_paths.database.is_file() {return Err("existing RV database not found".into());}
                    let mut live=WorkspaceStore::open(&live_paths)?;live.load(&id)?;
                    let links=live.backups(&id)?;
                    live.import_app_state(&id,links.first().map(|link|link.device_id),&downloaded.state)?;
                    println!("Existing RV SQLite state/auth import PASS; proxy associated={}",live.proxy_binding(&id)?.is_some());
                }
            }
        }

        Ok::<_,Box<dyn std::error::Error>>(())
    }.await;
    session.close();
    tokio::time::sleep(Duration::from_millis(150)).await;
    result
}
