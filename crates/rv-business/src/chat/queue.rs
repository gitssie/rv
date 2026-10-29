//! Durable user intent. These entry points never perform HTTP or allocate a task per message.
use rusqlite::{Connection, OptionalExtension, params};
use rv_core::{StoreError, StorePaths, workspace::*};
pub(super) fn writable(store: &mut WorkspaceStore, id: &AppId) -> Result<(), StoreError> {
    store.business_transaction(|db| crate::tasks::admit_access_token(db, id.as_str(), None))?;
    Ok(())
}
pub(super) fn wake(db: &Connection, app: &AppId, key: &str) -> Result<(), StoreError> {
    let timestamp = super::service::now();
    db.execute("UPDATE tasks SET status='pending',next_run_at=CASE WHEN status='pending' THEN MIN(next_run_at,?3) ELSE ?3 END,attempt_count=0,updated_at=?3 WHERE app_id=?1 AND task_key=?2 AND enabled=1 AND status IN ('pending','completed','failed')",params![app.as_str(),key,timestamp])?;
    Ok(())
}
pub fn queue_message(
    paths: &StorePaths,
    app: &AppId,
    conversation: &str,
    text: &str,
) -> Result<String, String> {
    if text.trim().is_empty() || text.len() > 16384 {
        return Err("消息为空或过长".into());
    }
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    writable(&mut store, app).map_err(|e| e.to_string())?;
    store.business_transaction(|db|{
  super::repository::schema(db)?;
  let allowed:Option<bool>=db.query_row("SELECT allow_reply FROM chat_conversations WHERE id=?1 AND app_id=?2",params![conversation,app.as_str()],|r|r.get(0)).optional()?;
  if allowed!=Some(true){return Err(StoreError::Invalid("此会话不允许回复".into()));}
  crate::tasks::admit_access_token(db,app.as_str(),None)?;
  let timestamp=super::service::now();let id=uuid::Uuid::new_v4().to_string();
  let language=super::customers::for_conversation(db,conversation)?.and_then(|c|c.language);
  db.execute("INSERT INTO chat_outbox(id,conversation_id,body,created_at,status,target_language,available_at,updated_at) VALUES(?1,?2,?3,?4,'pending',?5,?4,?4)",params![id,conversation,text,timestamp,language])?;
  db.execute("DELETE FROM chat_drafts WHERE conversation_id=?1",[conversation])?;
  Ok(id)
 }).map_err(|e|e.to_string())
}
pub fn request_sync(
    paths: &StorePaths,
    app: &AppId,
    conversation: Option<&str>,
) -> Result<(), String> {
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    writable(&mut store, app).map_err(|e| e.to_string())?;
    store.business_transaction(|db|{super::repository::schema(db)?;
  if let Some(id)=conversation {let owned:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM chat_conversations WHERE id=?1 AND app_id=?2)",params![id,app.as_str()],|r|r.get(0))?;if !owned{return Err(StoreError::Invalid("会话不属于当前店铺".into()));}
   db.execute("INSERT INTO chat_view_state(app_id,conversation_id) VALUES(?1,?2) ON CONFLICT(app_id) DO UPDATE SET conversation_id=excluded.conversation_id",params![app.as_str(),id])?;
  }
  Ok(())
 }).map_err(|e|e.to_string())
}
pub fn request_read(paths: &StorePaths, app: &AppId, conversation: &str) -> Result<(), String> {
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    writable(&mut store, app).map_err(|e| e.to_string())?;
    store.business_transaction(|db|{super::repository::schema(db)?;db.execute("INSERT INTO chat_read_requests(conversation_id,requested_at) SELECT id,?3 FROM chat_conversations WHERE id=?1 AND app_id=?2 AND unread=1 ON CONFLICT(conversation_id) DO UPDATE SET requested_at=excluded.requested_at",params![conversation,app.as_str(),super::service::now()])?;Ok(())}).map_err(|e|e.to_string())
}
pub fn cancel_queued(paths: &StorePaths, conversation: &str, id: &str) -> Result<(), String> {
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    store.business_transaction(|db|{super::repository::schema(db)?;
  if db.execute("UPDATE chat_outbox SET status='cancelled',updated_at=?3 WHERE id=?1 AND conversation_id=?2 AND status IN ('pending','failed','unknown')",params![id,conversation,super::service::now()])?!=1{return Err(StoreError::Invalid("正在发送或已提交的消息不能取消".into()));}
  Ok(())
 }).map_err(|e|e.to_string())
}
pub(super) fn recover(db: &Connection) -> Result<(), StoreError> {
    let timestamp = super::service::now();
    db.execute("UPDATE chat_outbox SET status='unknown',updated_at=?1,error='消息发送中断，请核对是否送达' WHERE status='sending' AND ((task_id IS NULL AND COALESCE(started_at,created_at)<=?1-90) OR (task_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM tasks t WHERE t.id=chat_outbox.task_id AND t.status='running' AND t.deadline>?1)))",[timestamp])?;
    Ok(())
}
