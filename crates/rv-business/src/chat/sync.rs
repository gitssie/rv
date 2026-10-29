//! Interactive chat operations share HTTP/storage with cron consumers, without Tasks.
use super::*;
use rv_core::{StorePaths,workspace::{AppId,WorkspaceStore}};
use crate::{OfflineRunner,Operation};
use std::sync::{Arc,Mutex,OnceLock,Weak};
#[derive(Clone,Debug,PartialEq,Eq,Hash)]
pub enum ChatSync {Inbox,Thread(String),SendPending(String),MarkRead(String)}
impl ChatSync {
    pub fn label(&self)->&'static str {match self {Self::Inbox=>"刷新会话",Self::Thread(_)=>"刷新消息",Self::SendPending(_)=>"发送消息",Self::MarkRead(_)=>"标记已读"}}
}
pub async fn synchronize(paths:StorePaths,id:AppId,operation:ChatSync)->Result<(),String> {
    crate::executor::runtime()?.spawn(async move {
        use tracing::Instrument;
        let span=tracing::info_span!("chat_sync",app_id=%id.as_str(),operation=operation.label());
        async move {
            let store=WorkspaceStore::open(&paths).map_err(|e|e.to_string())?;
            let app=store.load(&id).map_err(|e|e.to_string())?;
            let plugin=strategy(&app.bundle_id).ok_or("此 App 尚未接入聊天")?;
            let runner=OfflineRunner::new(None)?;
            match operation {
                ChatSync::Inbox=>{
                    runner.chat_interactive(&paths,&id,Action::Inbox).await?;
                    runner.run(&paths,&id,Operation::NotificationStats).await.map_err(|e|format!("会话已更新，但未读数量刷新失败：{e}"))
                },
                ChatSync::Thread(key)=>{
                    let remote=conversation_remote(&paths,&id,&key)?;
                    remember(&paths,&id,&key)?;
                    runner.chat_interactive(&paths,&id,Action::Thread(remote)).await
                },
                ChatSync::SendPending(key)=>dispatch_with(&runner,&paths,&id,&key,plugin.as_ref()).await,
                ChatSync::MarkRead(key)=>{
                    request_read(&paths,&id,&key)?;
                    runner.chat_workflow_execute(&paths,&id,None,workflow::Workflow::Read,plugin.as_ref(),Some(&key)).await?;
                    runner.run(&paths,&id,Operation::NotificationStats).await.map_err(|e|format!("已标记已读，但未读数量刷新失败：{e}"))
                },
            }
        }.instrument(span).await
    }).await.map_err(|e|format!("聊天操作中断：{e}"))?
}
fn conversation_remote(paths:&StorePaths,id:&AppId,key:&str)->Result<String,String> {
    WorkspaceStore::open(paths).and_then(|mut s|s.business_transaction(|db| {
        repository::schema(db)?;
        db.query_row("SELECT remote_id FROM chat_conversations WHERE app_id=?1 AND id=?2",rusqlite::params![id.as_str(),key],|r|r.get(0)).map_err(Into::into)
    })).map_err(|error|format!("读取当前店铺会话失败：{error}"))
}
fn remember(paths:&StorePaths,id:&AppId,key:&str)->Result<(),String> {
    WorkspaceStore::open(paths).and_then(|mut s|s.business_transaction(|db| {
        db.execute("INSERT INTO chat_view_state(app_id,conversation_id) VALUES(?1,?2) ON CONFLICT(app_id) DO UPDATE SET conversation_id=excluded.conversation_id",rusqlite::params![id.as_str(),key])?;Ok(())
    })).map_err(|e|e.to_string())
}
// Serialize explicit sends within one conversation, including kicks from multiple windows.
fn send_gate(paths:&StorePaths,key:&str)->Result<Arc<tokio::sync::Mutex<()>>,String> {
    type Gates=std::collections::HashMap<(std::path::PathBuf,String),Weak<tokio::sync::Mutex<()>>>;
    static GATES:OnceLock<Mutex<Gates>>=OnceLock::new();
    let database=std::fs::canonicalize(&paths.database).map_err(|e|e.to_string())?;
    let mut gates=GATES.get_or_init(Default::default).lock().unwrap();gates.retain(|_,v|v.strong_count()>0);
    let key=(database,key.to_string());
    if let Some(gate)=gates.get(&key).and_then(Weak::upgrade){return Ok(gate)}
    let gate=Arc::new(tokio::sync::Mutex::new(()));gates.insert(key,Arc::downgrade(&gate));Ok(gate)
}
pub(super) async fn dispatch_with(runner:&OfflineRunner,paths:&StorePaths,id:&AppId,key:&str,plugin:&dyn ChatStrategy)->Result<(),String> {
    let gate=send_gate(paths,key)?;let _send=gate.lock().await;
    let remote=conversation_remote(paths,id,key)?;
    loop {
        let processed=runner.chat_workflow_execute(paths,id,None,workflow::Workflow::Send,plugin,Some(key)).await?;
        if processed==0{return Ok(())}
        // Reconcile the returned platform message ID before consuming the next send.
        runner.chat_interactive_with(paths,id,Action::Thread(remote.clone()),plugin).await
            .map_err(|e|format!("消息已提交，但刷新会话失败：{e}"))?;
    }
}
