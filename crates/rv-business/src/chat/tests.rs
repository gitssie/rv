use super::*;
use rv_core::{AddressBook, Connection, StorePaths, workspace::*};
use serde_json::json;
fn fixture() -> (tempfile::TempDir, StorePaths, WorkspaceStore, Vec<AppId>) {
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
const REMOTE: &str = "01a0d934-4000-7cdd-b5ab-a42ed00a00dc";
fn detail() -> Value {
    json!({"id":REMOTE,"conversation_type":"private","allow_reply":true,"created_at":"2026-10-13T10:00:00Z","is_unread_by_current_user":true,"data":{"subject":"Item","inventory_items":[]},"opposite_users":[{"type":"user","data":{"id":"456","login":"Emma"}}],"messages":[{"id":"m1","conversation_id":REMOTE,"sender_id":"123","created_at":"2026-10-13T10:01:00Z","message_type":"text","data":{"content":"你好世界","unknown":true}}],"pagination":{"has_next":false}})
}
#[test]
fn same_conversation_and_user_in_different_accounts_remain_isolated_and_searchable() {
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
    store.business_transaction(|db|{let count:i64=db.query_row("SELECT COUNT(*) FROM chat_message_search WHERE chat_message_search MATCH '\"你好世界\"'",[],|r|r.get(0))?;assert_eq!(count,2);Ok(())}).unwrap();
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
    assert!(!plugin.capabilities().upload);
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
struct Mock {
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
    fn sent_message_id(&self, id: &str, v: &Value) -> Option<String> {
        vinted::VintedChat.sent_message_id(id, v)
    }
    fn mark_read(&self, id: &str) -> Result<ChatRequest, String> {
        vinted::VintedChat.mark_read(id).map(|r| self.map(r))
    }
}
async fn server(responses: Vec<(u16, String)>) -> (Mock, tokio::task::JoinHandle<Vec<String>>) {
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
