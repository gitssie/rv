use super::*;
use rv_core::{AddressBook, Connection, StorePaths, workspace::*};
use serde_json::json;
pub(super) fn fixture() -> (tempfile::TempDir, StorePaths, WorkspaceStore, Vec<AppId>) {
    let dir = tempfile::tempdir().unwrap();
    let paths = StorePaths::in_dir(dir.path().into());
    let mut book = AddressBook::load(paths.clone()).unwrap();
    let phone = Connection::new("phone", "192.0.2.1", 5900);
    book.upsert(phone.clone());
    book.save().unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let ids = vec![
        AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap(),
        AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F4").unwrap(),
    ];
    store.sync_backups(phone.id,"lt.manodrabuziai.fr",&json!({"bundle_id":"lt.manodrabuziai.fr","backups":ids.iter().enumerate().map(|(i,id)|json!({"kind":"snapshot","idfv":id.as_str(),"backup_id":format!("snapshot:{i}"),"display_name":format!("Shop {i}")})).collect::<Vec<_>>()})).unwrap();
    for id in &ids {
        let mut app = store.load(id).unwrap();
        app.metadata = json!({"user_id":123}).to_string();
        store.save(&app).unwrap();
        store
            .save_auth(
                id,
                &AppAuth {
                    access_token: "fixture-token".into(),
                    refresh_token: None,
                    headers: Default::default(),
                    ..AppAuth::default()
                },
            )
            .unwrap();
    }
    (dir, paths, store, ids)
}
pub(super) const REMOTE: &str = "01a0d934-4000-7cdd-b5ab-a42ed00a00dc";
#[test]
fn account_unread_quantities_come_from_server_projection_not_cached_rows() {
    let (_dir, paths, mut store, ids) = fixture();
    let mut app = store.load(&ids[0]).unwrap();
    app.metadata =
        json!({"user_id":123,"unread_count":31,"notification_unread_count":7}).to_string();
    store.save(&app).unwrap();
    let batch = vinted::VintedChat
        .parse_detail(&app, REMOTE, &detail())
        .unwrap();
    store
        .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
        .unwrap();
    assert_eq!(conversations(&paths).unwrap().len(), 1);
    let rows = accounts(&paths).unwrap();
    let a = rows.iter().find(|a| a.id == ids[0]).unwrap();
    assert_eq!(a.unread, Some(31));
    assert_eq!(a.notification_unread, Some(7));
    let unknown = rows.iter().find(|a| a.id == ids[1]).unwrap();
    assert_eq!(unknown.unread, None);
    assert_eq!(unknown.notification_unread, None);
    let mut app = store.load(&ids[0]).unwrap();
    app.metadata =
        json!({"user_id":123,"unread_count":0,"notification_unread_count":0}).to_string();
    store.save(&app).unwrap();
    let rows = accounts(&paths).unwrap();
    let a = rows.iter().find(|a| a.id == ids[0]).unwrap();
    assert_eq!(a.unread, Some(0));
    assert_eq!(a.notification_unread, Some(0));
}
pub(super) fn detail() -> Value {
    json!({"id":REMOTE,"conversation_type":"private","allow_reply":true,"created_at":"2026-10-13T10:00:00Z","is_unread_by_current_user":true,"data":{"subject":"Item","inventory_items":[]},"opposite_users":[{"type":"user","data":{"id":"456","login":"Emma"}}],"messages":[{"id":"m1","conversation_id":REMOTE,"sender_id":"123","created_at":"2026-10-13T10:01:00Z","message_type":"text","data":{"content":"你好世界","unknown":true}}],"pagination":{"has_next":false}})
}
#[test]
fn same_conversation_and_user_in_different_accounts_remain_isolated() {
    let (_dir, paths, mut store, ids) = fixture();
    let plugin = vinted::VintedChat;
    for id in &ids {
        let app = store.load(id).unwrap();
        let batch = plugin.parse_detail(&app, REMOTE, &detail()).unwrap();
        store
            .business_transaction(|db| repository::persist_detail(db, id, &batch))
            .unwrap();
    }
    let conversations = conversations(&paths).unwrap();
    assert_eq!(conversations.len(), 2);
    assert_ne!(conversations[0].id, conversations[1].id);
    for c in conversations {
        let t = thread(&paths, &c.id).unwrap();
        assert_eq!(t.messages.len(), 1);
        assert!(t.messages[0].outgoing);
        assert_eq!(t.messages[0].body, "你好世界");
        assert_eq!(t.messages[0].payload["data"]["unknown"], true);
    }
    store
        .business_transaction(|db| {
            let count: i64 =
                db.query_row("SELECT COUNT(*) FROM chat_customers", [], |r| r.get(0))?;
            assert_eq!(count, 2);
            Ok(())
        })
        .unwrap();
}
#[test]
fn system_notices_and_unknown_variants_never_enable_send() {
    let (_dir, _paths, store, ids) = fixture();
    let app = store.load(&ids[0]).unwrap();
    let plugin = vinted::VintedChat;
    let mut reply = detail();
    reply["allow_reply"] = json!(false);
    reply["opposite_users"] = json!([{"type":"system","data":{}}]);
    reply["messages"][0]["message_type"] = json!("legacy_portal_message");
    reply["messages"][0]["data"] = json!({"entity":{"body":"通知文本","is_hidden":false}});
    let batch = plugin.parse_detail(&app, REMOTE, &reply).unwrap();
    assert_eq!(batch.conversation.allow_reply, Some(false));
    assert!(batch.participants.is_empty());
    assert_eq!(batch.messages[0].body, "通知文本");
    reply["messages"][0]["message_type"] = json!("future_variant");
    let batch = plugin.parse_detail(&app, REMOTE, &reply).unwrap();
    assert_eq!(batch.messages[0].kind, "future_variant");
    assert_eq!(batch.messages[0].body, "此消息类型暂不支持展示");
    reply["messages"][0]["conversation_id"] = json!("wrong");
    assert!(plugin.parse_detail(&app, REMOTE, &reply).is_err());
}
#[test]
fn server_cursor_and_reply_body_use_new_gateway_contract() {
    let (_dir, _paths, store, ids) = fixture();
    let app = store.load(&ids[0]).unwrap();
    let plugin = vinted::VintedChat;
    assert_eq!(
        plugin.inbox(None).unwrap().url,
        "https://api.vinted.com/messaging/main/inbox"
    );
    let request = plugin.inbox(Some("a&b=2")).unwrap();
    assert!(request.url.contains("next_cursor=a%26b%3D2"));
    assert!(!request.url.contains("per_page"));
    assert!(
        plugin
            .parse_inbox(
                &app,
                &json!({"conversations":[],"pagination":{"has_next":true}})
            )
            .is_err()
    );
    let plan = plugin.reply(REMOTE, "Hello").unwrap();
    assert!(plan.url.ends_with("/replies"));
    assert_eq!(
        plan.body.unwrap(),
        json!({"content":"Hello","is_personal_data_sharing_check_skipped":false})
    );
    assert!(plugin.capabilities().upload);
}
#[test]
fn pending_send_is_persistent_scoped_and_never_auto_replayed() {
    let (_dir, paths, mut store, ids) = fixture();
    let plugin = vinted::VintedChat;
    let batch = plugin
        .parse_detail(&store.load(&ids[0]).unwrap(), REMOTE, &detail())
        .unwrap();
    store
        .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
        .unwrap();
    let c = conversations(&paths).unwrap().remove(0);
    save_draft(&paths, &c.id, "My draft").unwrap();
    assert_eq!(draft(&paths, &c.id).unwrap(), "My draft");
    let pending = store
        .business_transaction(|db| repository::begin_send(db, &ids[0], REMOTE, "Hello"))
        .unwrap();
    assert_eq!(draft(&paths, &c.id).unwrap(), "");
    store
        .business_transaction(|db| repository::finish_send(db, &pending, "unknown", None))
        .unwrap();
    assert!(
        store
            .business_transaction(|db| repository::begin_send(db, &ids[0], REMOTE, "Hello again"))
            .is_err()
    );
    let t = thread(&paths, &c.id).unwrap();
    assert_eq!(t.pending[0].status, "unknown");
    assert_eq!(t.pending[0].body, "Hello");
    assert!(
        store
            .business_transaction(|db| repository::begin_send(db, &ids[1], REMOTE, "wrong account"))
            .is_err()
    );
    dismiss_pending(&paths, &c.id, &pending).unwrap();
    assert!(thread(&paths, &c.id).unwrap().pending.is_empty());
}
#[test]
fn latest_inbox_can_revoke_reply_permission_and_absence_preserves_known_permission() {
    let (_dir, _paths, mut store, ids) = fixture();
    let plugin = vinted::VintedChat;
    let app = store.load(&ids[0]).unwrap();
    let batch = plugin.parse_detail(&app, REMOTE, &detail()).unwrap();
    store
        .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
        .unwrap();
    let mut row = detail();
    row.as_object_mut().unwrap().remove("allow_reply");
    let inbox = plugin
        .parse_inbox(
            &app,
            &json!({"conversations":[row],"pagination":{"has_next":false}}),
        )
        .unwrap();
    store
        .business_transaction(|db| {
            repository::persist_conversations(db, &ids[0], &inbox.rows, false)
        })
        .unwrap();
    store
        .business_transaction(|db| {
            let key = repository::key(db, &ids[0], REMOTE)?;
            let allowed: bool = db.query_row(
                "SELECT allow_reply FROM chat_conversations WHERE id=?1",
                [key],
                |r| r.get(0),
            )?;
            assert!(allowed);
            Ok(())
        })
        .unwrap();
    let mut row = detail();
    row["allow_reply"] = json!(false);
    let inbox = plugin
        .parse_inbox(
            &app,
            &json!({"conversations":[row],"pagination":{"has_next":false}}),
        )
        .unwrap();
    store
        .business_transaction(|db| {
            repository::persist_conversations(db, &ids[0], &inbox.rows, false)
        })
        .unwrap();
    assert!(
        store
            .business_transaction(|db| repository::begin_send(db, &ids[0], REMOTE, "must not send"))
            .is_err()
    );
}
pub(super) struct Mock {
    base: String,
}
impl Mock {
    fn map(&self, mut request: ChatRequest) -> ChatRequest {
        request.url = request.url.replace("https://api.vinted.com", &self.base);
        request
    }
}
impl ChatStrategy for Mock {
    fn capabilities(&self) -> Capabilities {
        vinted::VintedChat.capabilities()
    }
    fn inbox(&self, c: Option<&str>) -> Result<ChatRequest, String> {
        vinted::VintedChat.inbox(c).map(|r| self.map(r))
    }
    fn detail(&self, id: &str, c: Option<&str>) -> Result<ChatRequest, String> {
        vinted::VintedChat.detail(id, c).map(|r| self.map(r))
    }
    fn parse_inbox(&self, app: &AppInstance, v: &Value) -> Result<Batch<Conversation>, String> {
        vinted::VintedChat.parse_inbox(app, v)
    }
    fn parse_detail(&self, app: &AppInstance, id: &str, v: &Value) -> Result<DetailBatch, String> {
        vinted::VintedChat.parse_detail(app, id, v)
    }
    fn reply(&self, id: &str, text: &str) -> Result<ChatRequest, String> {
        vinted::VintedChat.reply(id, text).map(|r| self.map(r))
    }
    fn image_upload(&self) -> Result<ImageUpload, String> {
        let mut request = vinted::VintedChat.image_upload()?;
        request.url = request.url.replace("https://www.vinted.com", &self.base);
        Ok(request)
    }
    fn image_reference(&self, reply: &Value) -> Result<String, String> {
        vinted::VintedChat.image_reference(reply)
    }
    fn reply_image(&self, id: &str, reference: &str) -> Result<ChatRequest, String> {
        vinted::VintedChat
            .reply_image(id, reference)
            .map(|r| self.map(r))
    }
    fn sent_message_id(&self, id: &str, v: &Value) -> Option<String> {
        vinted::VintedChat.sent_message_id(id, v)
    }
    fn mark_read(&self, id: &str) -> Result<ChatRequest, String> {
        vinted::VintedChat.mark_read(id).map(|r| self.map(r))
    }
}
pub(super) async fn server(
    responses: Vec<(u16, String)>,
) -> (Mock, tokio::task::JoinHandle<Vec<String>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0; 4096];
                let n = socket.read(&mut buffer).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(header_end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..header_end]);
                    let len = header
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|s| s.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= header_end + 4 + len {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8_lossy(&bytes).into_owned());
            socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    (Mock { base }, task)
}
#[tokio::test]
async fn paged_inbox_failure_preserves_previous_snapshot_and_success_commits_every_page() {
    let (_dir, paths, mut store, ids) = fixture();
    let app = store.load(&ids[0]).unwrap();
    let auth = store.auth(&ids[0]).unwrap().unwrap();
    let runner = crate::OfflineRunner::new(None).unwrap();
    let mut first = detail();
    first["last_message"] = first["messages"][0].clone();
    first.as_object_mut().unwrap().remove("messages");
    let mut second = first.clone();
    second["id"] = json!("01a0d934-4000-7cdd-b5ab-a42ed00a00dd");
    let page1=json!({"conversations":[first],"pagination":{"has_next":true,"next_cursor":"server-token"}}).to_string();
    let page2 = json!({"conversations":[second],"pagination":{"has_next":false}}).to_string();
    let (mock, task) = server(vec![(200, page1.clone()), (503, "{}".into())]).await;
    assert!(
        runner
            .chat_with(&mut store, &app, &auth, &mock, Action::Inbox)
            .await
            .is_err()
    );
    assert!(conversations(&paths).unwrap().is_empty());
    task.await.unwrap();
    let (mock, task) = server(vec![(200, page1), (200, page2)]).await;
    runner
        .chat_with(&mut store, &app, &auth, &mock, Action::Inbox)
        .await
        .unwrap();
    let requests = task.await.unwrap();
    assert!(requests[1].contains("next_cursor=server-token"));
    assert_eq!(conversations(&paths).unwrap().len(), 2);
}
#[tokio::test]
async fn numeric_send_id_is_reconciled_by_authoritative_detail_without_replay() {
    let (_dir, paths, mut store, ids) = fixture();
    let app = store.load(&ids[0]).unwrap();
    let auth = store.auth(&ids[0]).unwrap().unwrap();
    let plugin = vinted::VintedChat;
    let batch = plugin.parse_detail(&app, REMOTE, &detail()).unwrap();
    store
        .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
        .unwrap();
    let (mock, server) = server(vec![(
        200,
        json!({"message":{"id":123,"conversation_id":REMOTE}}).to_string(),
    )])
    .await;
    let runner = crate::OfflineRunner::new(None).unwrap();
    runner
        .chat_with(
            &mut store,
            &app,
            &auth,
            &mock,
            Action::Send {
                conversation: REMOTE.into(),
                text: "Hello".into(),
            },
        )
        .await
        .unwrap();
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("/replies"));
    assert!(requests[0].contains("\"content\":\"Hello\""));
    assert!(!requests[0].contains("photo_temp_uuids"));
    let c = conversations(&paths).unwrap().remove(0);
    let pending = thread(&paths, &c.id).unwrap().pending;
    assert_eq!(pending[0].remote_id.as_deref(), Some("123"));
    assert_eq!(pending[0].status, "submitted");
    let mut reply = detail();
    reply["messages"][0]["id"] = json!(123);
    reply["messages"][0]["data"]["content"] = json!("Hello");
    let batch = plugin.parse_detail(&app, REMOTE, &reply).unwrap();
    store
        .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
        .unwrap();
    assert!(thread(&paths, &c.id).unwrap().pending.is_empty());
}
#[tokio::test]
async fn ambiguous_send_never_retries_or_loses_draft_content() {
    let (_dir, paths, mut store, ids) = fixture();
    let app = store.load(&ids[0]).unwrap();
    let auth = store.auth(&ids[0]).unwrap().unwrap();
    let batch = vinted::VintedChat
        .parse_detail(&app, REMOTE, &detail())
        .unwrap();
    store
        .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
        .unwrap();
    let c = conversations(&paths).unwrap().remove(0);
    save_draft(&paths, &c.id, "Keep this").unwrap();
    let (mock, server) = server(vec![(200, "not-json".into())]).await;
    let runner = crate::OfflineRunner::new(None).unwrap();
    assert!(
        runner
            .chat_with(
                &mut store,
                &app,
                &auth,
                &mock,
                Action::Send {
                    conversation: REMOTE.into(),
                    text: "Keep this".into()
                }
            )
            .await
            .is_err()
    );
    server.await.unwrap();
    assert!(
        runner
            .chat_with(
                &mut store,
                &app,
                &auth,
                &mock,
                Action::Send {
                    conversation: REMOTE.into(),
                    text: "Keep this".into()
                }
            )
            .await
            .is_err()
    );
    let pending = thread(&paths, &c.id).unwrap().pending;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].body, "Keep this");
    assert_eq!(pending[0].status, "unknown");
}

#[test]
fn live_inquiry_member_and_item_fields_are_parsed() {
    let (_dir, _paths, store, ids) = fixture();
    let app = store.load(&ids[0]).unwrap();
    let reply = json!({
        "id":REMOTE,"conversation_type":"inquiry","allow_reply":true,
        "created_at":"2026-10-28T12:18:33Z",
        "data":{"items":[{"inquiry_item_id":"inquiry-1","inventory_item_id":"item-1","title":"Funko Pop","photos":[]}]},
        "opposite_users":[{"type":"member","data":{"id":"456","login_name":"newmanx","photo":{"url":"https://example.test/avatar.jpg"}}}],
        "messages":[],"pagination":{"has_next":false}
    });
    let batch = vinted::VintedChat
        .parse_detail(&app, REMOTE, &reply)
        .unwrap();
    assert_eq!(batch.conversation.title, "newmanx");
    assert_eq!(batch.conversation.allow_reply, Some(true));
    assert_eq!(batch.participants[0].username.as_deref(), Some("newmanx"));
    assert!(batch.conversation.avatar.is_some());
    assert_eq!(batch.conversation.items[0].remote_id, "item-1");
    assert_eq!(batch.conversation.items[0].title, "Funko Pop");
}

#[test]
fn live_reply_receipt_and_legacy_reply_text_are_parsed() {
    let (_dir, _paths, store, ids) = fixture();
    let app = store.load(&ids[0]).unwrap();
    let plugin = vinted::VintedChat;
    let receipt = json!({"id":"platform-message-1","conversation_id":REMOTE,"message_type":"reply_plain","data":{"content":"Bonjour !","id":"1234"},"sender_id":"123"});
    assert_eq!(
        plugin.sent_message_id(REMOTE, &receipt).as_deref(),
        Some("reply:1234")
    );
    assert!(
        plugin
            .sent_message_id("different-conversation", &receipt)
            .is_none()
    );
    let mut reply = detail();
    reply["messages"][0]["message_type"] = json!("legacy_reply");
    reply["messages"][0]["data"] = json!({"entity":{"body":"Bonjour !","is_hidden":false,"photos":[]},"entity_type":"message"});
    let batch = plugin.parse_detail(&app, REMOTE, &reply).unwrap();
    assert_eq!(batch.messages[0].body, "Bonjour !");
    assert!(batch.messages[0].outgoing);
    reply["messages"][0]["data"]["entity"]["is_hidden"] = json!(true);
    assert_eq!(
        plugin.parse_detail(&app, REMOTE, &reply).unwrap().messages[0].body,
        "消息已隐藏"
    );
}

#[test]
fn bridge_reply_identity_survives_gateway_uuid_changes_and_matches_receipt() {
    let (_dir, _paths, mut store, ids) = fixture();
    let app = store.load(&ids[0]).unwrap();
    let plugin = vinted::VintedChat;
    let receipt = json!({"id":"gateway-receipt","conversation_id":REMOTE,"message_type":"reply_plain","data":{"id":"1234","content":"Bonjour !"}});
    let stable = plugin.sent_message_id(REMOTE, &receipt).unwrap();
    assert_eq!(stable, "reply:1234");
    let mut reply = detail();
    reply["messages"][0]["message_type"] = json!("legacy_reply");
    reply["messages"][0]["data"] =
        json!({"entity":{"id":1234,"body":"Bonjour !","is_hidden":false}});
    for wrapper in ["gateway-read-1", "gateway-read-2"] {
        reply["messages"][0]["id"] = json!(wrapper);
        let batch = plugin.parse_detail(&app, REMOTE, &reply).unwrap();
        assert_eq!(
            batch.messages[0].remote_id.as_deref(),
            Some(stable.as_str())
        );
        store
            .business_transaction(|db| repository::persist_detail(db, &app.id, &batch))
            .unwrap();
    }
    store.business_transaction(|db|{
        let count:i64=db.query_row("SELECT COUNT(*) FROM chat_messages",[],|r|r.get(0))?;
        assert_eq!(count,1);
        let conversation=repository::key(db,&app.id,REMOTE)?;
        db.execute("INSERT INTO chat_outbox(id,conversation_id,body,created_at,status,remote_id,updated_at) VALUES('verified-send',?1,'Bonjour !',1,'submitted',?2,1)",rusqlite::params![conversation,stable])?;
        Ok(())
    }).unwrap();
    let batch = plugin.parse_detail(&app, REMOTE, &reply).unwrap();
    store
        .business_transaction(|db| {
            repository::persist_detail(db, &app.id, &batch)?;
            let count: i64 = db.query_row("SELECT COUNT(*) FROM chat_outbox", [], |r| r.get(0))?;
            assert_eq!(count, 0);
            Ok(())
        })
        .unwrap();
}

#[tokio::test]
async fn interactive_inbox_and_thread_do_not_create_or_wake_tasks() {
    let (_dir,paths,store,ids)=fixture();
    crate::tasks::initialize(&paths,&ids[0]).unwrap();
    for task in store.configured_tasks(&ids[0]).unwrap(){crate::tasks::set_task_enabled(&paths,&task.id,false).unwrap();}
    let before=store.tasks(&ids[0]).unwrap();
    let mut conversation=detail();conversation["last_message"]=conversation["messages"][0].clone();conversation.as_object_mut().unwrap().remove("messages");
    let first=json!({"conversations":[conversation],"pagination":{"has_next":true,"next_cursor":"cursor"}});
    let second=json!({"conversations":[],"pagination":{"has_next":false}});
    let (mock,http)=server(vec![(200,first.to_string()),(200,second.to_string()),(200,detail().to_string())]).await;
    let runner=crate::OfflineRunner::new(None).unwrap();
    runner.chat_interactive_with(&paths,&ids[0],Action::Inbox,&mock).await.unwrap();
    runner.chat_interactive_with(&paths,&ids[0],Action::Thread(REMOTE.into()),&mock).await.unwrap();
    let requests=http.await.unwrap();
    let session=|request:&String|request.lines().find_map(|line|line.split_once(':').filter(|(k,_)|k.eq_ignore_ascii_case("x-session-id")).map(|(_,v)|v.trim().to_string())).unwrap();
    assert_eq!(session(&requests[0]),session(&requests[1]));assert_eq!(session(&requests[0]),session(&requests[2]));
    assert_eq!(store.tasks(&ids[0]).unwrap(),before);
    let list=conversations(&paths).unwrap();assert_eq!(list.len(),1);
    assert_eq!(thread(&paths,&list[0].id).unwrap().messages.len(),1);
}

#[tokio::test]
async fn interactive_chat_error_preserves_server_reason_and_redacts_token() {
    let (_dir,paths,store,ids)=fixture();
    let (mock,http)=server(vec![(400,json!({"error":"invalid_scope","error_description":"request rejected for fixture-token"}).to_string())]).await;
    let error=crate::OfflineRunner::new(None).unwrap().chat_interactive_with(&paths,&ids[0],Action::Inbox,&mock).await.unwrap_err();
    assert_eq!(http.await.unwrap().len(),1);
    assert!(error.contains("HTTP 400"));assert!(error.contains("invalid_scope"));assert!(error.contains("request rejected"));assert!(!error.contains("fixture-token"));
    assert!(store.tasks(&ids[0]).unwrap().is_empty());
}

#[test]
fn repeated_legacy_system_detail_uses_stable_identity() {
    let (_dir, paths, mut store, ids) = fixture();
    let app=store.load(&ids[0]).unwrap();
    for round in 0..3 {
        let mut reply=detail();
        reply["messages"]=json!([
            {"conversation_id":REMOTE,"id":format!("portal-wrapper-{round}"),"message_type":"legacy_portal_message","sender_id":"0","created_at":"2026-10-13T10:01:00Z","data":{"created_time_ago":format!("{round} minutes"),"entity":{"id":73543542694u64,"body":"<p>Notice</p>"}}},
            {"conversation_id":REMOTE,"id":format!("action-wrapper-{round}"),"message_type":"legacy_action","sender_id":"0","created_at":"2026-10-13T10:01:00Z","data":{"created_time_ago":format!("{round} minutes"),"event_group":"dsa_appeals","event_type":"appeal_action_link_available","display_group":40,"entity":{"subtitle":"Appeal","actions":[]}}}
        ]);
        let batch=vinted::VintedChat.parse_detail(&app,REMOTE,&reply).unwrap();
        assert_eq!(batch.conversation.preview,"Notice");
        assert_eq!(batch.messages[1].body,"Appeal");
        store.business_transaction(|db|repository::persist_detail(db,&ids[0],&batch)).unwrap();
    }
    let c=conversations(&paths).unwrap().remove(0);
    assert_eq!(c.preview,"Notice");
    assert_eq!(thread(&paths,&c.id).unwrap().messages.len(),2);
}

#[test]
fn send_receipt_and_thread_reconcile_in_either_order_by_id() {
    for receipt_first in [true,false] {
        let (_dir,paths,mut store,ids)=fixture();
        let app=store.load(&ids[0]).unwrap();
        let mut reply=detail();reply["messages"]=json!([]);
        let empty=vinted::VintedChat.parse_detail(&app,REMOTE,&reply).unwrap();
        let local=store.business_transaction(|db|{
            repository::persist_detail(db,&ids[0],&empty)?;
            repository::begin_send(db,&ids[0],REMOTE,"Same text")
        }).unwrap();
        reply["messages"]=json!([{"id":"changing-wrapper","conversation_id":REMOTE,"sender_id":"123","created_at":"2026-10-13T10:01:00Z","message_type":"reply_plain","data":{"id":"9001","content":"Same text"}}]);
        let batch=vinted::VintedChat.parse_detail(&app,REMOTE,&reply).unwrap();
        store.business_transaction(|db|{
            db.execute("UPDATE chat_outbox SET prepared_body='Same text',body='相同文字',source_language='zh',target_language='en' WHERE id=?1",[&local])?;
            if receipt_first {repository::finish_send(db,&local,"submitted",Some("reply:9001"))?;}
            repository::persist_detail(db,&ids[0],&batch)?;
            if !receipt_first {repository::finish_send(db,&local,"submitted",Some("reply:9001"))?;}
            repository::persist_detail(db,&ids[0],&batch)?;
            Ok(())
        }).unwrap();
        let c=conversations(&paths).unwrap().remove(0);
        let t=thread(&paths,&c.id).unwrap();
        assert!(t.pending.is_empty());assert_eq!(t.messages.len(),1);
        assert_eq!(t.messages[0].id,local);
        assert_eq!(t.messages[0].translation.as_ref().unwrap().original,"相同文字");
        assert_eq!(t.messages[0].translation.as_ref().unwrap().translated,"Same text");
        assert_eq!(t.messages[0].remote_id.as_deref(),Some("reply:9001"));
        reply["messages"][0]["data"]["id"]=json!("9002");
        let second=vinted::VintedChat.parse_detail(&app,REMOTE,&reply).unwrap();
        store.business_transaction(|db|repository::persist_detail(db,&ids[0],&second)).unwrap();
        assert_eq!(thread(&paths,&c.id).unwrap().messages.len(),2,"identical text with different platform IDs must remain distinct");
    }
}

#[test]
fn old_portal_wrappers_migrate_by_entity_id_preserving_local_identity() {
    let (_dir,paths,mut store,ids)=fixture();
    let app=store.load(&ids[0]).unwrap();
    let mut reply=detail();
    reply["messages"][0]["message_type"]=json!("legacy_portal_message");
    reply["messages"][0]["data"]=json!({"entity":{"id":73543542694u64,"body":"Notice"}});
    let batch=vinted::VintedChat.parse_detail(&app,REMOTE,&reply).unwrap();
    store.business_transaction(|db|{
        repository::persist_detail(db,&ids[0],&batch)?;
        let c=repository::key(db,&ids[0],REMOTE)?;
        db.execute("DELETE FROM chat_messages",[])?;
        for n in 0..3 {
            let mut m=batch.messages[0].clone();m.id=format!("local-{n}");m.remote_id=Some(format!("old-wrapper-{n}"));
            db.execute("INSERT INTO chat_messages VALUES(?1,?2,?3,?4,?5,?6)",rusqlite::params![m.id,c,m.remote_id,m.created_at,m.body,serde_json::to_string(&m)?])?;
        }
        db.execute("UPDATE chat_schema SET version=2",[])?;
        repository::schema(db)?;
        repository::schema(db)?;
        Ok(())
    }).unwrap();
    let c=conversations(&paths).unwrap().remove(0);
    let t=thread(&paths,&c.id).unwrap();assert_eq!(t.messages.len(),1);
    assert_eq!(t.messages[0].id,"local-0");
    assert_eq!(t.messages[0].remote_id.as_deref(),Some("portal:73543542694"));
}

#[test]
fn local_read_updates_sqlite_immediately_and_survives_old_inbox() {
    let (_dir,paths,mut store,ids)=fixture();
    let app=store.load(&ids[0]).unwrap();
    let batch=vinted::VintedChat.parse_detail(&app,REMOTE,&detail()).unwrap();
    store.business_transaction(|db|repository::persist_detail(db,&ids[0],&batch)).unwrap();
    let c=conversations(&paths).unwrap().remove(0);
    request_read(&paths,&ids[0],&c.id).unwrap();request_read(&paths,&ids[0],&c.id).unwrap();
    assert!(!conversations(&paths).unwrap()[0].unread);
    store.business_transaction(|db|repository::persist_conversations(db,&ids[0],std::slice::from_ref(&batch.conversation),false)).unwrap();
    assert!(!conversations(&paths).unwrap()[0].unread);
    let count:i64=store.business_transaction(|db|db.query_row("SELECT count(*) FROM chat_read_requests",[],|r|r.get(0)).map_err(Into::into)).unwrap();assert_eq!(count,1);
    let mut newer=batch.conversation;newer.updated_at=super::service::now()+1;newer.unread=true;
    store.business_transaction(|db|repository::persist_conversations(db,&ids[0],&[newer],false)).unwrap();
    assert!(conversations(&paths).unwrap()[0].unread,"new messages can become unread");
}
