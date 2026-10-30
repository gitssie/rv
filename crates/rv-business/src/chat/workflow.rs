//! Fixed per-store task consumers. HTTP, proxy leases and durable queue transitions stay here.
use super::service::{ChatResult, now};
use super::*;
use rusqlite::{OptionalExtension, params};
use rv_core::{StoreError, StorePaths, workspace::*};
use serde_json::json;
use std::time::{Duration, Instant};
#[derive(Clone, Copy)]
pub(crate) enum Workflow {
    Send,
    Sync,
    Read,
}
fn pending_threads(db:&rusqlite::Connection,id:&AppId)->Result<Vec<String>,StoreError> {
    let mut stmt=db.prepare("SELECT c.remote_id FROM chat_conversations c WHERE c.app_id=?1 AND (c.detail_loaded=0 OR c.updated_at>COALESCE((SELECT inbox_updated_at FROM chat_detail_sync WHERE conversation_id=c.id),0) OR c.id=(SELECT conversation_id FROM chat_view_state WHERE app_id=?1)) ORDER BY (c.id=(SELECT conversation_id FROM chat_view_state WHERE app_id=?1)) DESC,COALESCE((SELECT fetched_at FROM chat_detail_sync WHERE conversation_id=c.id),0),c.updated_at DESC LIMIT 4")?;
    Ok(stmt.query_map([id.as_str()],|row|row.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?)
}
struct Pending {
    id: String,
    conversation: String,
    body: String,
    image: Option<Vec<u8>>,
}
impl crate::OfflineRunner {
    pub(crate) async fn chat_workflow_direct(
        &self,paths:&StorePaths,id:&AppId,workflow:Workflow,
    )->Result<(),String> {
        let app=WorkspaceStore::open(paths).map_err(|e|e.to_string())?
            .load(id).map_err(|e|e.to_string())?;
        let plugin=strategy(&app.bundle_id).ok_or("此 App 尚未接入聊天")?;
        self.chat_workflow_execute(paths,id,None,workflow,plugin.as_ref(),None).await.map(|_|())
    }
    pub(crate) async fn chat_workflow(
        &self,
        paths: &StorePaths,
        id: &AppId,
        task: &AppTask,
        workflow: Workflow,
    ) -> Result<(), String> {
        let store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
        let app = store.load(id).map_err(|e| e.to_string())?;
        let plugin = strategy(&app.bundle_id).ok_or("此 App 尚未接入聊天")?;
        self.chat_workflow_with(paths, id, task, workflow, plugin.as_ref())
            .await
    }
    async fn chat_workflow_with(
        &self,
        paths: &StorePaths,
        id: &AppId,
        task: &AppTask,
        workflow: Workflow,
        plugin: &dyn ChatStrategy,
    ) -> Result<(), String> {
        self.chat_workflow_execute(paths,id,Some(task),workflow,plugin,None).await.map(|_|())
    }
    pub(super) async fn chat_workflow_execute(
        &self,paths:&StorePaths,id:&AppId,task:Option<&AppTask>,workflow:Workflow,
        plugin:&dyn ChatStrategy,conversation:Option<&str>,
    )->Result<usize,String> {
        let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
        let app = store.load(id).map_err(|e| e.to_string())?;
        if app.archived {
            return Err("账号已归档".into());
        }
        let mut work=store.business_transaction(|db|{
   repository::schema(db)?;queue::recover(db)?;
   match workflow {
    Workflow::Send=>{
     let pending=db.query_row("SELECT q.id,c.remote_id,COALESCE(q.prepared_body,q.body),i.jpeg FROM chat_outbox q JOIN chat_conversations c ON c.id=q.conversation_id LEFT JOIN chat_outbox_images i ON i.outbox_id=q.id WHERE c.app_id=?1 AND (?3 IS NULL OR c.id=?3) AND q.status='pending' AND q.available_at<=?2 AND c.allow_reply=1 AND NOT EXISTS(SELECT 1 FROM chat_outbox earlier WHERE earlier.conversation_id=q.conversation_id AND earlier.status='pending' AND (earlier.created_at<q.created_at OR (earlier.created_at=q.created_at AND earlier.rowid<q.rowid))) AND NOT EXISTS(SELECT 1 FROM chat_outbox active WHERE active.conversation_id=q.conversation_id AND active.status IN ('sending','submitted','unknown')) ORDER BY q.created_at,q.rowid LIMIT 1",params![id.as_str(),now(),conversation],|r|Ok(Pending{id:r.get(0)?,conversation:r.get(1)?,body:r.get(2)?,image:r.get(3)?})).optional()?;
     Ok((pending,Vec::new()))
    },
    Workflow::Read=>{let mut stmt=db.prepare("SELECT c.remote_id FROM chat_read_requests r JOIN chat_conversations c ON c.id=r.conversation_id WHERE c.app_id=?1 AND (?2 IS NULL OR c.id=?2) ORDER BY r.requested_at LIMIT 1")?;let list=stmt.query_map(params![id.as_str(),conversation],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;Ok((None,list))},
    Workflow::Sync=>Ok((None,Vec::new())),
   }
  }).map_err(|e|e.to_string())?;
        if work.0.is_none() && work.1.is_empty() && !matches!(workflow,Workflow::Sync) {
            if let Some(task)=task {
                let claimed=store.claim_task_when(task,|db|crate::tasks::admit_access_token(db,id.as_str(),Some(&task.id))).map_err(|e|e.to_string())?;
                store.finish_task(&claimed,"completed",&json!({"processed":0})).map_err(|e|e.to_string())?;
            }
            return Ok(0);
        }
        if let Some(pending)=&work.0 {
            if pending.image.is_some(){if !plugin.capabilities().upload{return Err("此会话不支持发送图片".into())}plugin.image_upload()?;}
            else {if !plugin.capabilities().text{return Err("此会话不支持发送文字".into())}plugin.reply(&pending.conversation,&pending.body)?;}
        }
        let (route, lease) = crate::network::acquire_route(paths,id,&self.client,plugin.requires_proxy()).await?;
        let auth=store.request_auth(id).map_err(|e|e.to_string())?.ok_or("账号凭据缺失")?;
        let app=store.load(id).map_err(|e|e.to_string())?;
        if app.archived{return Err("账号已归档".into())}
        if task.is_none(){queue::writable(&mut store,id).map_err(|e|e.to_string())?;}
        let admit=|db:&rusqlite::Connection| {

   crate::tasks::admit_access_token(db,id.as_str(),task.map(|t|t.id.as_str()))?;
   if let Some(pending)=&work.0 {
    let unchanged:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM chat_outbox q JOIN chat_conversations c ON c.id=q.conversation_id WHERE q.id=?1 AND c.app_id=?2 AND c.remote_id=?3 AND c.allow_reply=1 AND COALESCE(q.prepared_body,q.body)=?4 AND q.available_at<=?5)",params![pending.id,id.as_str(),pending.conversation,pending.body,now()],|r|r.get(0))?;
    if !unchanged { return Err(StoreError::Conflict); }
    let image:Option<Vec<u8>>=db.query_row("SELECT jpeg FROM chat_outbox_images WHERE outbox_id=?1",[&pending.id],|r|r.get(0)).optional()?;
    if image!=pending.image {return Err(StoreError::Conflict);}
    if db.execute("UPDATE chat_outbox SET status='sending',prepared_body=COALESCE(prepared_body,body),started_at=?2,updated_at=?2,task_id=?3,error=NULL WHERE id=?1 AND status='pending'",params![pending.id,now(),task.map(|t|t.id.as_str())])?!=1{return Err(StoreError::Conflict);}
   }
   Ok(())
        };
        let claimed=if let Some(task)=task {Some(store.claim_task_when(task,admit).map_err(|e|e.to_string())?)} else {store.business_transaction(admit).map_err(|e|e.to_string())?;None};
        let runner = Self {
            client: route.client.clone(),
            credentials: Some((paths.clone(),id.clone())),
            interactive: task.is_none(),
        };
        let budget = Duration::from_secs(task.map_or(75,|t|u64::from(t.timeout_seconds)));
        let deadline = Instant::now() + budget;
        let result = tokio::time::timeout(budget, async {
            if let Some(pending) = &work.0 {
                let plan = if let Some(image) = &pending.image {
                    if !plugin.capabilities().upload {
                        return Err("此会话不支持发送图片".into());
                    }
                    let reference = runner
                        .chat_upload_image(plugin, &auth, image.clone())
                        .await?;
                    route.validate_binding(paths, id)?;
                    plugin.reply_image(&pending.conversation, &reference)?
                } else {
                    if !plugin.capabilities().text {
                        return Err("此会话不支持发送文字".into());
                    }
                    plugin.reply(&pending.conversation, &pending.body)?
                };
                let reply = runner.chat_exchange(plan, &auth, true).await?;
                if plugin
                    .sent_message_id(&pending.conversation, &reply)
                    .is_none()
                {
                    return Err("消息回执缺少标识，结果未知，请核对是否送达".into());
                }
                return Ok((Some(reply), Vec::new()));
            }
            if matches!(workflow, Workflow::Sync) {
                let ChatResult::Inbox(rows) = runner.chat_fetch(plugin,&app,&auth,&service::Action::Inbox).await? else {
                    return Err("聊天列表返回无效".into());
                };
                let urls=rows.iter().flat_map(|row| {
                    row.avatar.iter().map(|image|image.url.clone())
                        .chain(row.items.iter().filter_map(|item|item.image.as_ref().map(|image|image.url.clone())))
                }).collect();
                crate::assets::cache_business_images_on_route(paths,id,urls,deadline,&route,task.is_none()).await?;
                // The same claimed task and network lease continue into details.
                // Persist the list before selecting details so new conversations
                // are included in this execution, rather than the next cron run.
                work.1=store.business_transaction(|db| {
                    repository::persist_conversations(db,id,&rows,false)?;
                    pending_threads(db,id)
                }).map_err(|error|error.to_string())?;
            }
            let mut batches = Vec::new();
            for remote in &work.1 {
                match workflow {
                    Workflow::Read => {
                        if !plugin.capabilities().mark_read {
                            return Err("此 App 不支持已读操作".into());
                        }
                        runner
                            .chat_exchange(plugin.mark_read(remote)?, &auth, true)
                            .await?;
                    }
                    Workflow::Sync => {
                        let result = runner
                            .chat_fetch(
                                plugin,
                                &app,
                                &auth,
                                &service::Action::Thread(remote.clone()),
                            )
                            .await?;
                        let ChatResult::Thread(batch) = result else {
                            return Err("消息详情返回无效".into());
                        };
                        let mut urls: Vec<String> = batch
                            .messages
                            .iter()
                            .flat_map(|m| m.images.iter().map(|i| i.url.clone()))
                            .collect();
                        if let Some(avatar) = &batch.conversation.avatar {
                            urls.push(avatar.url.clone());
                        }
                        urls.extend(
                            batch
                                .conversation
                                .items
                                .iter()
                                .filter_map(|i| i.image.as_ref().map(|i| i.url.clone())),
                        );
                        urls.extend(
                            batch
                                .participants
                                .iter()
                                .filter_map(|p| p.avatar.as_ref().map(|i| i.url.clone())),
                        );
                        crate::assets::cache_business_images_on_route(
                            paths, id, urls, deadline, &route,task.is_none(),
                        )
                        .await?;
                        batches.push(batch);
                    }
                    Workflow::Send => unreachable!(),
                }
            }
            Ok::<_, String>((None, batches))
        })
        .await
        .unwrap_or_else(|_| Err("聊天请求超时，结果未知".into()));
        drop(lease);
        let mut outcome = Ok(work.0.is_some() as usize+work.1.len());
        match result {
            Ok((reply, batches)) => {
                let remote = reply.as_ref().and_then(|reply| {
                    work.0
                        .as_ref()
                        .and_then(|pending| plugin.sent_message_id(&pending.conversation, reply))
                });
                let persist=|db:&rusqlite::Connection| {

     if let Some(pending)=&work.0{repository::finish_send(db,&pending.id,"submitted",remote.as_deref())?;if task.is_some(){queue::wake(db,id,"chat_threads")?;}}
     for batch in &batches{repository::persist_detail(db,id,batch)?;}
     if matches!(workflow,Workflow::Read){for remote in &work.1{let key=repository::key(db,id,remote)?;db.execute("UPDATE chat_conversations SET unread=0,data=json_set(data,'$.unread',json('false')) WHERE id=?1",[&key])?;db.execute("DELETE FROM chat_read_requests WHERE conversation_id=?1",[key])?;}if task.is_some(){queue::wake(db,id,"notifications")?;}}
     Ok(())
                };
                let committed=if let Some(claimed)=&claimed {
                    store.complete_task_with(id,claimed,"completed",&json!({"processed":work.0.is_some() as usize+work.1.len()}),None,None,persist)
                }else {store.commit_business_data(id,&json!({}),persist)};
                if let Err(error) = committed {
                    let _ = store.business_transaction(|db| {
                        if let Some(pending) = &work.0 {
                            let _ = repository::finish_send(db, &pending.id, "unknown", None);
                        }
                        Ok(())
                    });
                    if let Some(claimed)=&claimed {let _=store.finish_task(claimed,"unknown",&json!({"error":"消息结果保存未确认"}));}
                    outcome = Err(error.to_string());
                }
            }
            Err(error) => {
                let not_sent=error.starts_with("访问令牌已失效") || error.starts_with("账号凭据已更新") || error.starts_with("店铺已切换手机接管");
                let definite = error == "图片上传未成功"
                    || error
                        .strip_prefix("HTTP ")
                        .and_then(|s| s.split('：').next())
                        .and_then(|s| s.parse::<u16>().ok())
                        .is_some_and(|s| (400..500).contains(&s) && s != 408);
                let writing = matches!(workflow, Workflow::Send | Workflow::Read);
                let status = if writing && !definite {
                    "unknown"
                } else {
                    "failed"
                };
                let persist=|db:&rusqlite::Connection| {

                            if let Some(pending) = &work.0 {
                                repository::finish_send(db, &pending.id, if not_sent {"pending"}else{status}, None)?;
                                db.execute(
                                    "UPDATE chat_outbox SET error=?2,updated_at=?3 WHERE id=?1",
                                    params![pending.id, error, now()],
                                )?;
                            }
                            if matches!(workflow, Workflow::Read) && definite && !error.starts_with("HTTP 401：") {
                                for remote in &work.1 {
                                    let key = repository::key(db, id, remote)?;
                                    db.execute(
                                        "DELETE FROM chat_read_requests WHERE conversation_id=?1",
                                        [key],
                                    )?;
                                }
                            }
                            Ok(())
                };
                if let Some(claimed)=&claimed {
                    store.complete_task_with_policy(id,claimed,status,&json!({"error":error}),None,None,
                        TaskFinishPolicy{retryable:!writing,stop_on_failure:false},persist).map_err(|e|e.to_string())?;
                }else {store.commit_business_data(id,&json!({}),persist).map_err(|e|e.to_string())?;}
                outcome = Err(error);
            }
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{REMOTE, detail, fixture, server};
    use super::*;
    fn provision(paths: &StorePaths, store: &mut WorkspaceStore, id: &AppId) -> String {
        let app = store.load(id).unwrap();
        let batch = super::super::vinted::VintedChat
            .parse_detail(&app, REMOTE, &detail())
            .unwrap();
        store
            .business_transaction(|db| repository::persist_detail(db, id, &batch))
            .unwrap();
        crate::tasks::initialize(paths, id).unwrap();
        for key in [crate::tasks::KEEPALIVE_KEY] {
            let task = store
                .configured_tasks(id)
                .unwrap()
                .into_iter()
                .find(|t| t.task_key.as_deref() == Some(key))
                .unwrap();
            let queued = store.run_task_now(&task.id).unwrap();
            let running = store.claim_task_when(&queued, |_| Ok(())).unwrap();
            store
                .finish_task(&running, "completed", &json!({}))
                .unwrap();
        }
        super::super::conversations(paths)
            .unwrap()
            .into_iter()
            .find(|c| c.app_id == *id)
            .unwrap()
            .id
    }
    fn ready(store: &mut WorkspaceStore, id: &AppId, key: &str) -> AppTask {
        let task = store
            .configured_tasks(id)
            .unwrap()
            .into_iter()
            .find(|t| t.task_key.as_deref() == Some(key))
            .unwrap();
        store.run_task_now(&task.id).unwrap()
    }
    #[tokio::test]
    async fn message_sync_fetches_inbox_and_new_details_in_one_task_or_direct_execution() {
        for scheduled in [true,false] {
            let (_dir,paths,mut store,ids)=fixture();
            crate::tasks::initialize(&paths,&ids[0]).unwrap();
            let task=ready(&mut store,&ids[0],"chat_threads");
            if !scheduled {store.set_task_enabled(&task.id,false).unwrap();}
            let tasks_before=store.tasks(&ids[0]).unwrap();
            let mut summary=detail();summary.as_object_mut().unwrap().remove("messages");
            let inbox=json!({"conversations":[summary],"pagination":{"has_next":false}});
            let (mock,http)=server(vec![(200,inbox.to_string()),(200,detail().to_string())]).await;
            let runner=crate::OfflineRunner::new(None).unwrap();
            runner.chat_workflow_execute(&paths,&ids[0],scheduled.then_some(&task),Workflow::Sync,&mock,None).await.unwrap();
            let requests=http.await.unwrap();
            assert_eq!(requests.len(),2);
            assert!(requests[0].contains("/messaging/main/inbox"));
            assert!(requests[1].contains(REMOTE));
            let conversations=super::super::conversations(&paths).unwrap();
            assert_eq!(conversations.len(),1);
            let messages=super::super::thread(&paths,&conversations[0].id).unwrap();
            assert_eq!(messages.messages.len(),1);
            assert_eq!(messages.messages[0].body,"你好世界");
            assert_eq!(store.tasks(&ids[0]).unwrap().len(),tasks_before.len());
            if scheduled {
                let saved=store.task(&task.id).unwrap();
                assert_eq!(saved.last_run_status.as_deref(),Some("completed"));
                assert!(saved.next_run_at.is_some());
                assert_eq!(saved.result.unwrap()["processed"],1);
            } else {assert_eq!(store.tasks(&ids[0]).unwrap(),tasks_before);}
        }
    }
    #[tokio::test]
    async fn message_sync_inbox_failure_never_fetches_details_or_reports_success() {
        let (_dir,paths,mut store,ids)=fixture();
        crate::tasks::initialize(&paths,&ids[0]).unwrap();
        let task=ready(&mut store,&ids[0],"chat_threads");
        let (mock,http)=server(vec![(503,"{}".into())]).await;
        assert!(crate::OfflineRunner::new(None).unwrap().chat_workflow_with(&paths,&ids[0],&task,Workflow::Sync,&mock).await.is_err());
        assert_eq!(http.await.unwrap().len(),1);
        assert!(super::super::conversations(&paths).unwrap().is_empty());
        let saved=store.task(&task.id).unwrap();
        assert_eq!(saved.last_run_status.as_deref(),Some("failed"));
        assert!(saved.enabled);
    }
    #[tokio::test]
    async fn message_sync_detail_failure_keeps_new_list_but_marks_the_same_task_failed() {
        let (_dir,paths,mut store,ids)=fixture();
        crate::tasks::initialize(&paths,&ids[0]).unwrap();
        let task=ready(&mut store,&ids[0],"chat_threads");
        let mut summary=detail();summary.as_object_mut().unwrap().remove("messages");
        let inbox=json!({"conversations":[summary],"pagination":{"has_next":false}});
        let (mock,http)=server(vec![(200,inbox.to_string()),(503,"{}".into())]).await;
        assert!(crate::OfflineRunner::new(None).unwrap().chat_workflow_with(&paths,&ids[0],&task,Workflow::Sync,&mock).await.is_err());
        assert_eq!(http.await.unwrap().len(),2);
        let rows=super::super::conversations(&paths).unwrap();
        assert_eq!(rows.len(),1);
        assert!(super::super::thread(&paths,&rows[0].id).unwrap().messages.is_empty());
        assert_eq!(store.task(&task.id).unwrap().last_run_status.as_deref(),Some("failed"));
        assert_eq!(store.configured_tasks(&ids[0]).unwrap().iter().filter(|task|task.kind=="chat_threads").count(),1);
    }
    #[test]
    fn queued_messages_are_durable_without_creating_tasks_and_reads_are_inert() {
        let (_dir, paths, mut store, ids) = fixture();
        let conversation = provision(&paths, &mut store, &ids[0]);
        super::super::save_draft(&paths, &conversation, "第一条").unwrap();
        let count = store.tasks(&ids[0]).unwrap().len();
        let first = super::super::queue_message(&paths, &ids[0], &conversation, "第一条").unwrap();
        let second = super::super::queue_message(&paths, &ids[0], &conversation, "第二条").unwrap();
        assert_ne!(first, second);
        assert_eq!(store.tasks(&ids[0]).unwrap().len(), count);
        assert_eq!(count, 8);
        assert_eq!(super::super::draft(&paths, &conversation).unwrap(), "");
        let read = super::super::thread(&paths, &conversation).unwrap();
        assert_eq!(read.pending.len(), 2);
        assert!(
            read.pending
                .iter()
                .all(|m| m.status == "pending" && m.translation.is_none())
        );
        store
            .business_transaction(|db| {
                db.execute(
                    "UPDATE chat_outbox SET status='sending',started_at=1 WHERE id=?1",
                    [&first],
                )?;
                Ok(())
            })
            .unwrap();
        assert_eq!(
            super::super::thread(&paths, &conversation).unwrap().pending[0].status,
            "sending",
            "reading UI data cannot recover or mutate a queue row"
        );
        store.business_transaction(|db| queue::recover(db)).unwrap();
        assert_eq!(
            super::super::thread(&paths, &conversation).unwrap().pending[0].status,
            "unknown"
        );
        assert!(
            super::super::queue_message(&paths, &ids[1], &conversation, "wrong account").is_err()
        );
    }
    #[tokio::test]
    async fn image_send_uploads_once_sends_no_text_and_reconciles_receipt() {
        let (_dir, paths, mut store, ids) = fixture();
        let conversation = provision(&paths, &mut store, &ids[0]);
        let path = paths.root.join("fixture.png");
        image::RgbImage::from_pixel(20, 10, image::Rgb([255, 200, 0]))
            .save(&path)
            .unwrap();
        let image = super::super::images::prepare(&path).unwrap();
        let queued = super::super::images::queue(&paths, &ids[0], &conversation, &image).unwrap();
        std::fs::remove_file(path).unwrap();
        super::super::save_draft(&paths, &conversation, "keep this draft").unwrap();
        let reference = "4eedea8f-dd41-4458-8127-d04a5de14858";
        let (mock,http)=server(vec![(200,json!({"code":0,"temp_uuid":reference}).to_string()),(201,json!({"id":"gateway-id","conversation_id":REMOTE,"message_type":"reply_plain","data":{"id":"7001","content":""}}).to_string())]).await;
        let task = ready(&mut store, &ids[0], "chat_send");
        let runner = crate::OfflineRunner::new(None).unwrap();
        runner
            .chat_workflow_with(&paths, &ids[0], &task, Workflow::Send, &mock)
            .await
            .unwrap();
        let requests = http.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("POST /api/v2/photos "));
        assert!(requests[0].contains("name=\"photo[type]\""));
        assert!(requests[0].contains("user_msg"));
        assert!(requests[0].contains("name=\"photo[file]\""));
        let body: Value =
            serde_json::from_str(requests[1].split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["content"], "");
        assert_eq!(body["photo_temp_uuids"], json!([reference]));
        assert!(!requests[1].contains("keep this draft"));
        let thread = super::super::thread(&paths, &conversation).unwrap();
        assert_eq!(thread.pending[0].id, queued);
        assert_eq!(thread.pending[0].status, "submitted");
        assert_eq!(thread.pending[0].remote_id.as_deref(), Some("reply:7001"));
        let mut reply = detail();
        reply["messages"][0]["message_type"] = json!("legacy_reply");
        reply["messages"][0]["data"] = json!({"entity":{"id":7001,"body":"","photos":[{"url":"https://images.vinted.net/flower.jpg"}]}});
        let batch = super::super::vinted::VintedChat
            .parse_detail(&store.load(&ids[0]).unwrap(), REMOTE, &reply)
            .unwrap();
        store
            .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
            .unwrap();
        assert!(
            super::super::thread(&paths, &conversation)
                .unwrap()
                .pending
                .is_empty()
        );
        store
            .business_transaction(|db| {
                assert_eq!(
                    db.query_row("SELECT COUNT(*) FROM chat_outbox_images", [], |r| r
                        .get::<_, i64>(0))?,
                    0
                );
                Ok(())
            })
            .unwrap();
        assert_eq!(
            super::super::draft(&paths, &conversation).unwrap(),
            "keep this draft"
        );
    }
    #[tokio::test]
    async fn image_upload_unknown_receipt_never_posts_the_message_or_replays() {
        let (_dir, paths, mut store, ids) = fixture();
        let conversation = provision(&paths, &mut store, &ids[0]);
        let path = paths.root.join("fixture.png");
        image::RgbImage::from_pixel(10, 10, image::Rgb([0, 200, 0]))
            .save(&path)
            .unwrap();
        let image = super::super::images::prepare(&path).unwrap();
        super::super::images::queue(&paths, &ids[0], &conversation, &image).unwrap();
        let (mock, http) = server(vec![(200, json!({"code":0}).to_string())]).await;
        let task = ready(&mut store, &ids[0], "chat_send");
        let runner = crate::OfflineRunner::new(None).unwrap();
        assert!(
            runner
                .chat_workflow_with(&paths, &ids[0], &task, Workflow::Send, &mock)
                .await
                .is_err()
        );
        assert_eq!(http.await.unwrap().len(), 1);
        assert_eq!(
            super::super::thread(&paths, &conversation).unwrap().pending[0].status,
            "unknown"
        );
        let retry = store.run_task_now(&task.id).unwrap();
        crate::OfflineRunner::new(None)
            .unwrap()
            .chat_workflow_with(&paths, &ids[0], &retry, Workflow::Send, &mock)
            .await
            .unwrap();
        assert_eq!(
            super::super::thread(&paths, &conversation).unwrap().pending[0].status,
            "unknown"
        );
    }
    #[tokio::test]
    async fn fixed_send_claims_one_row_and_detail_reconciles_without_reposting() {
        let (_dir, paths, mut store, ids) = fixture();
        let conversation = provision(&paths, &mut store, &ids[0]);
        super::super::queue_message(&paths, &ids[0], &conversation, "你好").unwrap();
        super::super::queue_message(&paths, &ids[0], &conversation, "第二条").unwrap();
        let (mock, http) = server(vec![(
            200,
            json!({"message":{"id":123,"conversation_id":REMOTE}}).to_string(),
        )])
        .await;
        let task = ready(&mut store, &ids[0], "chat_send");
        let runner = crate::OfflineRunner::new(None).unwrap();
        runner
            .chat_workflow_with(&paths, &ids[0], &task, Workflow::Send, &mock)
            .await
            .unwrap();
        let requests = http.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].contains("你好"));
        assert!(!requests[0].contains("第二条"));
        let state = super::super::thread(&paths, &conversation).unwrap();
        assert_eq!(state.pending[0].status, "submitted");
        assert_eq!(state.pending[1].status, "pending");
        let mut reply = detail();
        reply["messages"][0]["id"] = json!(123);
        reply["messages"][0]["data"]["content"] = json!("你好");
        let batch = super::super::vinted::VintedChat
            .parse_detail(&store.load(&ids[0]).unwrap(), REMOTE, &reply)
            .unwrap();
        store
            .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
            .unwrap();
        let state = super::super::thread(&paths, &conversation).unwrap();
        assert_eq!(state.pending.len(), 1);
        assert_eq!(
            state
                .messages
                .iter()
                .filter(|m| m.remote_id.as_deref() == Some("123"))
                .count(),
            1
        );
        assert_eq!(store.configured_tasks(&ids[0]).unwrap().len(), 8);
    }
    #[tokio::test]
    async fn interrupted_send_is_not_replayed_and_phone_ownership_allows_valid_token() {
        let (_dir, paths, mut store, ids) = fixture();
        let conversation = provision(&paths, &mut store, &ids[0]);
        super::super::queue_message(&paths, &ids[0], &conversation, "保留原文").unwrap();
        let (mock, http) = server(vec![(200, "not-json".into())]).await;
        let task = ready(&mut store, &ids[0], "chat_send");
        let runner = crate::OfflineRunner::new(None).unwrap();
        assert!(
            runner
                .chat_workflow_with(&paths, &ids[0], &task, Workflow::Send, &mock)
                .await
                .is_err()
        );
        assert_eq!(http.await.unwrap().len(), 1);
        let state = super::super::thread(&paths, &conversation).unwrap();
        assert_eq!(state.pending[0].body, "保留原文");
        assert_eq!(state.pending[0].status, "unknown");
        assert_eq!(store.task(&task.id).unwrap().status, "pending");
        let retry = store.run_task_now(&task.id).unwrap();
        crate::OfflineRunner::new(None)
            .unwrap()
            .chat_workflow_with(&paths, &ids[0], &retry, Workflow::Send, &mock)
            .await
            .unwrap();
        assert_eq!(
            super::super::thread(&paths, &conversation).unwrap().pending[0].status,
            "unknown"
        );
        store.business_transaction(|db|{db.execute("INSERT INTO app_execution(app_id,mode,device_id,operation_id,updated_at) VALUES(?1,'phone',NULL,'test',?2)",params![ids[0].as_str(),now()])?;Ok(())}).unwrap();
        assert!(
            super::super::queue_message(&paths, &ids[0], &conversation, "queued with current token").is_ok()
        );
        assert!(super::super::request_sync(&paths, &ids[0], Some(&conversation)).is_ok());
    }

    #[tokio::test]
    async fn delayed_queue_head_preserves_order_and_definite_read_failure_consumes_intent() {
        let (_dir, paths, mut store, ids) = fixture();
        let conversation = provision(&paths, &mut store, &ids[0]);
        let first = super::super::queue_message(&paths, &ids[0], &conversation, "first").unwrap();
        super::super::queue_message(&paths, &ids[0], &conversation, "second").unwrap();
        store
            .business_transaction(|db| {
                db.execute(
                    "UPDATE chat_outbox SET available_at=?2 WHERE id=?1",
                    params![first, now() + 3600],
                )?;
                Ok(())
            })
            .unwrap();
        // Idle send must not need a proxy or execute a later message ahead of the first.
        let task = ready(&mut store, &ids[0], "chat_send");
        crate::OfflineRunner::new(None)
            .unwrap()
            .chat_workflow_with(
                &paths,
                &ids[0],
                &task,
                Workflow::Send,
                &super::super::vinted::VintedChat,
            )
            .await
            .unwrap();
        assert!(
            super::super::thread(&paths, &conversation)
                .unwrap()
                .pending
                .iter()
                .all(|m| m.status == "pending")
        );
        super::super::request_read(&paths, &ids[0], &conversation).unwrap();
        let (mock, http) = server(vec![(403, json!({"error":"denied"}).to_string())]).await;
        let task = ready(&mut store, &ids[0], "chat_read");
        assert!(
            crate::OfflineRunner::new(None)
                .unwrap()
                .chat_workflow_with(&paths, &ids[0], &task, Workflow::Read, &mock)
                .await
                .is_err()
        );
        assert_eq!(http.await.unwrap().len(), 1);
        store
            .business_transaction(|db| {
                assert_eq!(
                    db.query_row("SELECT count(*) FROM chat_read_requests", [], |r| r
                        .get::<_, i64>(0))?,
                    0
                );
                Ok(())
            })
            .unwrap();
    }

    #[tokio::test]
    async fn success_without_receipt_remains_resolvable_and_is_not_replayed() {
        let (_dir, paths, mut store, ids) = fixture();
        let conversation = provision(&paths, &mut store, &ids[0]);
        let queued =
            super::super::queue_message(&paths, &ids[0], &conversation, "original").unwrap();
        let (mock, http) = server(vec![(204, String::new())]).await;
        let task = ready(&mut store, &ids[0], "chat_send");
        assert!(
            crate::OfflineRunner::new(None)
                .unwrap()
                .chat_workflow_with(&paths, &ids[0], &task, Workflow::Send, &mock)
                .await
                .is_err()
        );
        assert_eq!(http.await.unwrap().len(), 1);
        assert_eq!(
            super::super::thread(&paths, &conversation).unwrap().pending[0].status,
            "unknown"
        );
        let retry = store.run_task_now(&task.id).unwrap();
        crate::OfflineRunner::new(None)
            .unwrap()
            .chat_workflow_with(&paths, &ids[0], &retry, Workflow::Send, &mock)
            .await
            .unwrap();
        assert_eq!(
            super::super::thread(&paths, &conversation).unwrap().pending[0].status,
            "unknown"
        );
        super::super::cancel_queued(&paths, &conversation, &queued).unwrap();
        assert!(store.task(&task.id).unwrap().enabled);
    }
    #[tokio::test]
    async fn empty_thread_snapshots_stop_requesting_detail() {
        let (_dir, paths, mut store, ids) = fixture();
        provision(&paths, &mut store, &ids[0]);
        let app = store.load(&ids[0]).unwrap();
        let mut empty = detail();
        empty["messages"] = json!([]);
        for n in 1..7 {
            let remote = uuid::Uuid::from_u128(200 + n).to_string();
            empty["id"] = json!(remote);
            let batch = super::super::vinted::VintedChat
                .parse_detail(&app, &remote, &empty)
                .unwrap();
            store
                .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
                .unwrap();
        }
        // Inbox still refreshes, but unchanged cached details need no HTTP.
        let (mock,http)=server(vec![(200,json!({"conversations":[],"pagination":{"has_next":false}}).to_string())]).await;
        let task = ready(&mut store, &ids[0], "chat_threads");
        crate::OfflineRunner::new(None)
            .unwrap()
            .chat_workflow_with(
                &paths,
                &ids[0],
                &task,
                Workflow::Sync,
                &mock,
            )
            .await
            .unwrap();
        assert_eq!(http.await.unwrap().len(),1);
        assert_eq!(
            store.task(&task.id).unwrap().result.unwrap()["processed"],
            0
        );
    }

    #[test]
    fn stale_detail_keeps_newer_inbox_dirty() {
        let (_dir, paths, mut store, ids) = fixture();
        let conversation = provision(&paths, &mut store, &ids[0]);
        let batch = super::super::vinted::VintedChat
            .parse_detail(&store.load(&ids[0]).unwrap(), REMOTE, &detail())
            .unwrap();
        store.business_transaction(|db|{
            db.execute("UPDATE chat_conversations SET updated_at=?2,data=json_set(data,'$.updated_at',?2) WHERE id=?1",params![conversation,batch.conversation.updated_at+100])?;
            repository::persist_detail(db,&ids[0],&batch)?;
            let (inbox,covered):(i64,i64)=db.query_row("SELECT c.updated_at,d.inbox_updated_at FROM chat_conversations c JOIN chat_detail_sync d ON d.conversation_id=c.id WHERE c.id=?1",[&conversation],|r|Ok((r.get(0)?,r.get(1)?)))?;
            assert!(inbox>covered,"stale detail must remain eligible for a later pull");
            Ok(())
        }).unwrap();
    }
    #[test]
    fn confirmed_translation_survives_repeated_remote_sync() {
        let (_dir, paths, mut store, ids) = fixture();
        let conversation = provision(&paths, &mut store, &ids[0]);
        let app = store.load(&ids[0]).unwrap();
        let mut batch = super::super::vinted::VintedChat
            .parse_detail(&app, REMOTE, &detail())
            .unwrap();
        let translated = super::super::translation::Translation {
            original: "原文".into(),
            translated: batch.messages[0].body.clone(),
            source_language: Some("zh".into()),
            target_language: "es".into(),
        };
        batch.messages[0].translation = Some(translated.clone());
        store
            .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
            .unwrap();
        batch.messages[0].translation = None;
        store
            .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
            .unwrap();
        let thread = super::super::thread(&paths, &conversation).unwrap();
        assert_eq!(
            thread
                .messages
                .iter()
                .find(|m| m.remote_id == batch.messages[0].remote_id)
                .unwrap()
                .translation
                .as_ref()
                .unwrap()
                .original,
            "原文"
        );
    }
    #[test]
    fn customers_are_account_scoped_and_manual_language_survives_profile_updates() {
        let (_dir, paths, mut store, ids) = fixture();
        for id in &ids {
            let app = store.load(id).unwrap();
            let mut response = detail();
            response["opposite_users"][0]["data"]["country_code"] = json!("ES");
            let batch = super::super::vinted::VintedChat
                .parse_detail(&app, REMOTE, &response)
                .unwrap();
            store
                .business_transaction(|db| repository::persist_detail(db, id, &batch))
                .unwrap();
        }
        let conversations = super::super::conversations(&paths).unwrap();
        let first = conversations.iter().find(|c| c.app_id == ids[0]).unwrap();
        let second = conversations.iter().find(|c| c.app_id == ids[1]).unwrap();
        let customer = super::super::thread(&paths, &first.id)
            .unwrap()
            .customer
            .unwrap();
        assert_eq!(customer.country_code.as_deref(), Some("ES"));
        assert_eq!(customer.language.as_deref(), Some("es"));
        super::super::set_customer_language(&paths, &first.id, "en").unwrap();
        let mut response = detail();
        response["opposite_users"][0]["data"]["country_code"] = json!("FR");
        response["opposite_users"][0]["data"]["language_code"] = json!("fr");
        let batch = super::super::vinted::VintedChat
            .parse_detail(&store.load(&ids[0]).unwrap(), REMOTE, &response)
            .unwrap();
        store
            .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
            .unwrap();
        let customer = super::super::thread(&paths, &first.id)
            .unwrap()
            .customer
            .unwrap();
        assert_eq!(customer.language.as_deref(), Some("en"));
        assert_eq!(customer.language_source.as_deref(), Some("manual"));
        assert_eq!(
            super::super::thread(&paths, &second.id)
                .unwrap()
                .customer
                .unwrap()
                .language
                .as_deref(),
            Some("es")
        );
    }
    #[tokio::test]
    async fn manual_outbox_dispatch_drains_in_order_without_tasks_and_duplicate_kicks_do_not_resend() {
        let (_dir,paths,mut store,ids)=fixture();let key=provision(&paths,&mut store,&ids[0]);
        for task in store.configured_tasks(&ids[0]).unwrap(){crate::tasks::set_task_enabled(&paths,&task.id,false).unwrap();}
        let before=store.tasks(&ids[0]).unwrap();
        super::super::queue_message(&paths,&ids[0],&key,"first").unwrap();
        super::super::queue_message(&paths,&ids[0],&key,"second").unwrap();
        let mut responses=Vec::new();
        for (id,text) in [(7001,"first"),(7002,"second")] {
            responses.push((201,json!({"message":{"id":id,"conversation_id":REMOTE}}).to_string()));
            let mut reply=detail();reply["messages"][0]["id"]=json!(id);reply["messages"][0]["data"]["content"]=json!(text);
            responses.push((200,reply.to_string()));
        }
        let (mock,http)=server(responses).await;let runner=crate::OfflineRunner::new(None).unwrap();
        let (a,b)=tokio::join!(super::super::sync::dispatch_with(&runner,&paths,&ids[0],&key,&mock),super::super::sync::dispatch_with(&runner,&paths,&ids[0],&key,&mock));
        a.unwrap();b.unwrap();let requests=http.await.unwrap();assert_eq!(requests.len(),4);
        assert!(requests[0].contains("\"content\":\"first\""));assert!(requests[2].contains("\"content\":\"second\""));
        assert!(super::super::thread(&paths,&key).unwrap().pending.is_empty());
        assert_eq!(store.tasks(&ids[0]).unwrap(),before);
    }
    #[test]
    fn manual_send_recovery_keeps_live_send_and_only_marks_expired_send_unknown() {
        let (_dir,paths,mut store,ids)=fixture();let key=provision(&paths,&mut store,&ids[0]);
        let queued=super::super::queue_message(&paths,&ids[0],&key,"pending").unwrap();
        store.business_transaction(|db|{
            db.execute("UPDATE chat_outbox SET status='sending',task_id=NULL,started_at=?2 WHERE id=?1",params![queued,now()])?;
            queue::recover(db)?;Ok(())
        }).unwrap();
        assert_eq!(super::super::thread(&paths,&key).unwrap().pending[0].status,"sending");
        store.business_transaction(|db|{db.execute("UPDATE chat_outbox SET started_at=?2 WHERE id=?1",params![queued,now()-91])?;queue::recover(db)?;Ok(())}).unwrap();
        assert_eq!(super::super::thread(&paths,&key).unwrap().pending[0].status,"unknown");
    }
    #[tokio::test]
    async fn manual_send_unknown_reply_is_not_replayed_and_creates_no_task() {
        let (_dir,paths,mut store,ids)=fixture();let key=provision(&paths,&mut store,&ids[0]);
        let before=store.tasks(&ids[0]).unwrap();
        super::super::queue_message(&paths,&ids[0],&key,"keep original").unwrap();
        let (mock,http)=server(vec![(200,"{}".into())]).await;let runner=crate::OfflineRunner::new(None).unwrap();
        assert!(super::super::sync::dispatch_with(&runner,&paths,&ids[0],&key,&mock).await.is_err());
        assert_eq!(http.await.unwrap().len(),1);
        assert_eq!(super::super::thread(&paths,&key).unwrap().pending[0].status,"unknown");
        super::super::sync::dispatch_with(&runner,&paths,&ids[0],&key,&mock).await.unwrap();
        assert_eq!(store.tasks(&ids[0]).unwrap(),before);
    }

    #[tokio::test]
    async fn manual_image_send_upload_and_reply_share_session_and_do_not_touch_tasks() {
        use sha2::{Digest,Sha256};
        let (_dir,paths,mut store,ids)=fixture();let key=provision(&paths,&mut store,&ids[0]);
        for task in store.configured_tasks(&ids[0]).unwrap(){crate::tasks::set_task_enabled(&paths,&task.id,false).unwrap();}
        let before=store.tasks(&ids[0]).unwrap();
        let file=paths.root.join("image.png");image::RgbImage::from_pixel(12,12,image::Rgb([200,120,20])).save(&file).unwrap();
        let image=super::super::images::prepare(&file).unwrap();
        super::super::images::queue(&paths,&ids[0],&key,&image).unwrap();
        let url="https://images.vinted.net/manual-fixture.jpg";
        let cache=paths.root.join("business-images");std::fs::create_dir_all(&cache).unwrap();std::fs::write(cache.join(format!("{:x}.img",Sha256::digest(url.as_bytes()))),image.bytes()).unwrap();
        let mut reply=detail();reply["messages"][0]["message_type"]=json!("legacy_reply");reply["messages"][0]["data"]=json!({"entity":{"id":7001,"body":"","photos":[{"url":url}]}});
        let (mock,http)=server(vec![(200,json!({"code":0,"temp_uuid":"4eedea8f-dd41-4458-8127-d04a5de14858"}).to_string()),(201,json!({"conversation_id":REMOTE,"message_type":"reply_plain","data":{"id":"7001","content":""}}).to_string()),(200,reply.to_string())]).await;
        super::super::sync::dispatch_with(&crate::OfflineRunner::new(None).unwrap(),&paths,&ids[0],&key,&mock).await.unwrap();
        let requests=http.await.unwrap();assert_eq!(requests.len(),3);assert!(requests[0].starts_with("POST /api/v2/photos "));assert!(requests[1].contains("photo_temp_uuids"));
        let session=|request:&String|request.lines().find_map(|line|line.split_once(':').filter(|(k,_)|k.eq_ignore_ascii_case("x-session-id")).map(|(_,v)|v.trim().to_string())).unwrap();
        assert_eq!(session(&requests[0]),session(&requests[1]));
        assert!(super::super::thread(&paths,&key).unwrap().pending.is_empty());
        assert_eq!(store.tasks(&ids[0]).unwrap(),before);
    }
    #[tokio::test]
    async fn manual_mark_read_only_consumes_selected_conversation_without_tasks() {
        let (_dir,paths,mut store,ids)=fixture();let first=provision(&paths,&mut store,&ids[0]);
        let other_remote="01a0d934-4000-7cdd-b5ab-a42ed00a00dd";let mut reply=detail();reply["id"]=json!(other_remote);reply["messages"]=json!([]);
        let batch=super::super::vinted::VintedChat.parse_detail(&store.load(&ids[0]).unwrap(),other_remote,&reply).unwrap();
        store.business_transaction(|db|repository::persist_detail(db,&ids[0],&batch)).unwrap();
        let second=super::super::conversations(&paths).unwrap().into_iter().find(|c|c.remote_id==other_remote).unwrap().id;
        let before=store.tasks(&ids[0]).unwrap();
        super::super::request_read(&paths,&ids[0],&first).unwrap();super::super::request_read(&paths,&ids[0],&second).unwrap();
        let (mock,http)=server(vec![(204,String::new())]).await;
        crate::OfflineRunner::new(None).unwrap().chat_workflow_execute(&paths,&ids[0],None,Workflow::Read,&mock,Some(&second)).await.unwrap();
        let requests=http.await.unwrap();assert!(requests[0].contains(other_remote));
        let rows=super::super::conversations(&paths).unwrap();assert!(!rows.iter().find(|c|c.id==first).unwrap().unread);assert!(!rows.iter().find(|c|c.id==second).unwrap().unread);
        let queued:Vec<String>=store.business_transaction(|db|{let mut q=db.prepare("SELECT conversation_id FROM chat_read_requests")?;let rows=q.query_map([],|r|r.get(0))?.collect::<Result<Vec<_>,_>>()?;Ok(rows)}).unwrap();assert_eq!(queued,vec![first]);
        assert_eq!(store.tasks(&ids[0]).unwrap(),before);
    }

    #[tokio::test]
    async fn manual_read_401_preserves_intent_and_new_token_can_finish_it() {
        let (_dir,paths,mut store,ids)=fixture();let key=provision(&paths,&mut store,&ids[0]);
        let before=store.tasks(&ids[0]).unwrap();
        super::super::request_read(&paths,&ids[0],&key).unwrap();
        let runner=crate::OfflineRunner::new(None).unwrap();
        let (denied,http)=server(vec![(401,json!({"message":"Authorization failed."}).to_string())]).await;
        assert!(runner.chat_workflow_execute(&paths,&ids[0],None,Workflow::Read,&denied,Some(&key)).await.is_err());http.await.unwrap();
        let remaining:i64=store.business_transaction(|db|db.query_row("SELECT COUNT(*) FROM chat_read_requests WHERE conversation_id=?1",[&key],|r|r.get(0)).map_err(Into::into)).unwrap();assert_eq!(remaining,1);
        let mut auth=store.auth(&ids[0]).unwrap().unwrap();assert!(auth.access_token_rejected());auth.access_token="replacement-read-token".into();store.save_auth(&ids[0],&auth).unwrap();
        let (accepted,http)=server(vec![(204,String::new())]).await;
        runner.chat_workflow_execute(&paths,&ids[0],None,Workflow::Read,&accepted,Some(&key)).await.unwrap();http.await.unwrap();
        assert!(!super::super::conversations(&paths).unwrap().iter().find(|c|c.id==key).unwrap().unread);
        let after=store.tasks(&ids[0]).unwrap();
        assert_eq!(after.len(),before.len());
        for previous in before {
            let current=after.iter().find(|task|task.id==previous.id).unwrap();
            if previous.task_key.as_deref()==Some(crate::tasks::KEEPALIVE_KEY) {
                assert!(current.enabled);assert_eq!(current.cron_expression,previous.cron_expression);
                assert!(current.next_run_at.unwrap()<=now()+1,"401 wakes the already-enabled RV refresh");
            } else { assert_eq!(current,&previous,"manual read creates no business tasks"); }
        }
    }

}
