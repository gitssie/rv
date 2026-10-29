//! Explicit live validation using the existing RV database and production phone runtime.
//! Never prints tokens; commands: inspect, phone-token, takeover, handoff.
use rv_core::{AddressBook,ConnectRequest,StorePaths,events::*,workspace::*};
use rv_session::{AppCommand,AppEvent,FunctionCommand,FunctionEvent,SessionEvent,SessionHandle};
use std::time::{Duration,Instant};
const BUNDLE:&str="lt.manodrabuziai.fr";
type Error=Box<dyn std::error::Error>;
async fn event(session:&SessionHandle,deadline:Instant)->Result<SessionEvent,Error>{
    loop {
        if Instant::now()>deadline{return Err("phone operation timeout; no replay performed".into());}
        if let Some(event)=session.try_recv(){return Ok(event);}
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
#[tokio::main]
async fn main()->Result<(),Error>{
    let args:Vec<_>=std::env::args().skip(1).collect();
    let action=args.first().ok_or("action required")?;
    if !matches!(action.as_str(),"inspect"|"phone-token"|"takeover"|"handoff"){return Err("unsupported action".into());}
    let id=AppId::from_idfv(args.get(1).ok_or("IDFV required")?)?;
    let paths=StorePaths::default_dir()?;
    let book=AddressBook::load(paths.clone())?;
    let device=book.connections().iter().find(|d|d.id.to_string()=="8e36d3fe-5dea-4394-877a-252b217b522a").ok_or("7Plus not configured")?;
    if device.host!="172.16.19.176"{return Err("7Plus endpoint changed; verify identity first".into());}
    let session=SessionHandle::spawn_control_with_store(ConnectRequest::from_connection(device,paths.load_password(device.id)?),paths.clone());
    let result=async {
        let deadline=Instant::now()+Duration::from_secs(20);
        loop{if matches!(event(&session,deadline).await?,SessionEvent::ControlState(true)){break;}}
        println!("7Plus authenticated through production HTTP/2 controller");
        let mut store=WorkspaceStore::open(&paths)?;
        store.load(&id)?;
        if action=="phone-token"{
            let bus=workspace_bus(&paths)?;let results=bus.subscribe::<PhoneTokenResult>();
            store.set_token_source(&id,TokenSource::Phone(Some(device.id)))?;
            store.request_phone_token(&id,true)?;
            let result=tokio::time::timeout(Duration::from_secs(30),results.recv()).await??;
            result.result.clone().map_err(|e|format!("live phone token read: {e}"))?;
            let auth=store.auth(&id)?.ok_or("missing phone token")?;
            println!("Phone token source/live read PASS; access expiry={:?}",auth.token_expirations().access);
            return Ok::<_,Error>(());
        }
        if action=="inspect"{
            let before=store.auth(&id)?.map(|a|serde_json::to_value(a)).transpose()?;
            let source=store.token_source(&id)?;let tasks=store.tasks(&id)?;
            for _ in 0..2 {
                session.function(FunctionCommand::Invoke{id:"app-list".into(),text:format!("App={BUNDLE}")});
                let deadline=Instant::now()+Duration::from_secs(25);
                let catalog=loop {
                    if let SessionEvent::Functions(mut ev)=event(&session,deadline).await?{
                        while let FunctionEvent::ForOperation{event:inner,..}=ev{ev=*inner;}
                        match ev {
                            FunctionEvent::State{task:32,record} if record["status"]=="completed"=>{
                                if record.get("error").is_some(){return Err("catalog failed".into());}
                                break record["result"].clone();
                            },
                            FunctionEvent::Error(e)|FunctionEvent::Uncertain(e)=>return Err(e.into()),_=>{}
                        }
                    }
                };
                store.sync_backups(device.id,BUNDLE,&catalog)?;
                assert_eq!(before,store.auth(&id)?.map(|a|serde_json::to_value(a)).transpose()?);
                assert_eq!(source,store.token_source(&id)?);assert_eq!(tasks,store.tasks(&id)?);
            }
            println!("Repeated real-phone backup catalogue refresh PASS; credentials/source/tasks unchanged");
            session.app(AppCommand::ReadProfile{app_id:id.clone(),bundle:BUNDLE.into(),backup:format!("snapshot:{}",id.as_str())});
            let deadline=Instant::now()+Duration::from_secs(60);
            loop{match event(&session,deadline).await?{
                SessionEvent::Apps(AppEvent::Profile(profile))=>{
                    let phone=profile.state.request_auth()?;
                    let local=store.auth(&id)?;
                    println!("Backup state read PASS; refresh present={}, expiry={:?}, scope={:?}, access matches RV={}, refresh matches RV={}, source={:?}",phone.refresh_token.is_some(),phone.token_expirations(),phone.extra.get("scope"),local.as_ref().is_some_and(|a|a.access_token==phone.access_token),local.as_ref().is_some_and(|a|a.refresh_token==phone.refresh_token),source);
                    break;
                },SessionEvent::Apps(AppEvent::Failed{message,..})=>return Err(message.into()),_=>{}
            }}
            return Ok(());
        }
        let command=if action=="takeover"{
            // Explicitly move refresh responsibility to the phone before reading its new backup.
            // Do not start any scheduler; this probe sends only the requested handover.
            store.set_token_source(&id,TokenSource::Phone(Some(device.id)))?;
            let execution=store.execution(&id)?;
            AppCommand::TakeoverBackup{app_id:id.clone(),bundle:BUNDLE.into(),backup:format!("snapshot:{}",id.as_str()),store_root:paths.root.clone(),device_id:device.id,operation_id:execution.operation_id}
        }else{
            let execution=store.begin_phone_handoff(&id,device.id)?;
            AppCommand::HandoffBackup{app_id:id.clone(),bundle:BUNDLE.into(),backup:format!("snapshot:{}",id.as_str()),store_root:paths.root.clone(),device_id:device.id,operation_id:execution.operation_id}
        };
        session.app(command);
        let deadline=Instant::now()+Duration::from_secs(120);
        loop{match event(&session,deadline).await?{
            SessionEvent::Apps(AppEvent::Finished(AppCommand::TakeoverBackup{..}|AppCommand::HandoffBackup{..}))=>break,
            SessionEvent::Apps(AppEvent::Failed{message,..})=>return Err(message.into()),_=>{}
        }}
        println!("{} PASS; execution={:?}, token_source={:?}",action,store.execution(&id)?.mode,store.token_source(&id)?);
        Ok(())
    }.await;
    session.close();tokio::time::sleep(Duration::from_millis(100)).await;
    result
}
