use super::*;
use rv_core::{AddressBook, Connection};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn fixture() -> (StorePaths, AppId, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let paths = StorePaths::in_dir(dir.path().into());
    let mut book = AddressBook::load(paths.clone()).unwrap();
    let device = Connection::new("phone", "phone", 5900);
    book.upsert(device.clone());
    book.save().unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let id = AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
    store.sync_backups(device.id,"lt.manodrabuziai.fr",&json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"idfv":id.as_str(),"backup_id":"snapshot:1","kind":"snapshot"}]})).unwrap();
    store
        .save_auth(
            &id,
            &AppAuth {
                access_token: "old-fixture-token".into(),
                ..Default::default()
            },
        )
        .unwrap();
    (paths, id, dir)
}
struct Mock {
    url: String,
}
impl BusinessStrategy for Mock {
    fn request(&self, _: Operation, _: &AppAuth) -> Result<RequestPlan, String> {
        Ok(RequestPlan {
            method: reqwest::Method::GET,
            url: self.url.clone(),
            body: None,
        })
    }
    fn project(&self, _: Operation, status: u16, response: &Value) -> Value {
        json!({"http_status":status,"response":response})
    }
}
async fn peer(status: Option<u16>) -> (Mock, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut b = [0; 1024];
            let n = socket.read(&mut b).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&b[..n]);
            if request.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        if status.is_some() {
            assert!(
                String::from_utf8(request)
                    .unwrap()
                    .contains("Bearer fresh-fixture-token")
            );
        }
        if let Some(status) = status {
            let body = "{\"ok\":true}";
            let response = format!(
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (
        Mock {
            url: format!("http://{address}/fixture"),
        },
        task,
    )
}
#[tokio::test]
async fn due_task_uses_latest_auth_and_updates_original_row() {
    let (paths, id, _dir) = fixture();
    let job = enqueue(&paths, &id, &TaskSpec::new("session", "* * * * * * *")).unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    store
        .save_auth(
            &id,
            &AppAuth {
                access_token: "fresh-fixture-token".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let (plugin, server) = peer(Some(200)).await;
    let runner = OfflineRunner::new(None).unwrap();
    runner
        .run_admitted(&paths, &id, Operation::Session, &plugin, Some(&job))
        .await
        .unwrap();
    server.await.unwrap();
    let saved = store.task(&job.id).unwrap();
    assert_eq!(saved.status, "pending");
    assert_eq!(saved.last_run_status.as_deref(), Some("completed"));
    assert!(saved.next_run_at.is_some());
    assert_eq!(store.tasks(&id).unwrap().len(), 1);
}
#[tokio::test]
async fn retryable_http_failure_stays_on_same_task() {
    let (paths, id, _dir) = fixture();
    let mut spec = TaskSpec::new("session", "* * * * * * *");
    spec.max_attempts = 2;
    let job = enqueue(&paths, &id, &spec).unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    store
        .save_auth(
            &id,
            &AppAuth {
                access_token: "fresh-fixture-token".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let (plugin, server) = peer(Some(503)).await;
    assert!(
        OfflineRunner::new(None)
            .unwrap()
            .run_admitted(&paths, &id, Operation::Session, &plugin, Some(&job))
            .await
            .is_err()
    );
    server.await.unwrap();
    let saved = store.task(&job.id).unwrap();
    assert_eq!(saved.status, "pending");
    assert_eq!(saved.attempt_count, 1);
    assert!(saved.next_run_at.unwrap() >= chrono::Utc::now().timestamp() + 29);
    assert_eq!(store.tasks(&id).unwrap().len(), 1);
}
#[tokio::test]
async fn interrupted_request_keeps_its_enabled_periodic_schedule() {
    let (paths, id, _dir) = fixture();
    let job = enqueue(&paths, &id, &TaskSpec::new("refresh", "* * * * * * *")).unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    store
        .save_auth(
            &id,
            &AppAuth {
                access_token: "fresh-fixture-token".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let (plugin, server) = peer(None).await;
    assert!(
        OfflineRunner::new(None)
            .unwrap()
            .run_admitted(&paths, &id, Operation::Refresh, &plugin, Some(&job))
            .await
            .is_err()
    );
    server.await.unwrap();
    let saved = store.task(&job.id).unwrap();
    assert_eq!(saved.status, "pending");
    assert!(saved.enabled);
    assert_eq!(saved.last_run_status.as_deref(), Some("unknown"));
    assert!(saved.next_run_at.is_some());
    assert!(
        store.due_tasks(timestamp() + 3600)
            .unwrap()
            .iter()
            .any(|task| task.id == job.id)
    );
}
#[test]
fn parameters_are_validated_and_writes_cannot_enable_retries() {
    let (paths, id, _dir) = fixture();
    let mut spec = TaskSpec::new("chat_send", "0 0 0 * * * *");
    spec.payload = json!({"conversation_id":"conv","text":"hello"});
    spec.max_attempts = 2;
    assert!(enqueue(&paths, &id, &spec).is_err());
    spec.max_attempts = 1;
    spec.payload["access_token"] = json!("not a task parameter");
    assert!(enqueue(&paths, &id, &spec).is_err());
    spec.kind = "unregistered".into();
    spec.payload = json!({});
    assert!(enqueue(&paths, &id, &spec).is_err());
    let task = enqueue(&paths, &id, &TaskSpec::new("inbox", "0 0 0 * * * *")).unwrap();
    assert_eq!(task.kind, "chat_threads");
}
#[tokio::test]
async fn preflight_failure_becomes_visible_without_starting_another_task() {
    let (paths, id, _dir) = fixture();
    let job = enqueue(&paths, &id, &TaskSpec::new("products", "* * * * * * *")).unwrap();
    assert!(
        OfflineRunner::new(None)
            .unwrap()
            .run_task(&paths, &job)
            .await
            .is_err()
    );
    let store = WorkspaceStore::open(&paths).unwrap();
    let saved = store.task(&job.id).unwrap();
    assert_eq!(saved.status, "pending");
    assert_eq!(saved.last_run_status.as_deref(), Some("failed"));
    assert!(saved.next_run_at.unwrap() > chrono::Utc::now().timestamp() - 1);
    assert_eq!(saved.attempt_count, 0);
    assert_eq!(store.tasks(&id).unwrap().len(), 1);
}

#[test]
fn workflow_provisions_once_and_waits_for_the_apps_proxy_without_spending_attempts() {
    let (paths, id, _dir) = fixture();
    let before = timestamp();
    initialize(&paths, &id).unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let tasks = store.tasks(&id).unwrap();
    assert_eq!(tasks.len(), 8);
    assert!(tasks.iter().all(|t|t.kind!="session" && t.task_key.as_deref()!=Some("profile")));
    assert!(tasks.iter().all(|t|t.depends_on.is_none()));
    let product=tasks.iter().find(|t|t.kind=="products").unwrap();
    assert!((before+10..=timestamp()+10).contains(&product.next_run_at.unwrap()));
    let keepalive = tasks
        .iter()
        .find(|task| task.task_key.as_deref() == Some(KEEPALIVE_KEY))
        .unwrap();
    assert!(due(&paths, before + 20).unwrap().is_empty());
    let blocked = store.task(&keepalive.id).unwrap();
    let refresh_due=keepalive.next_run_at.unwrap()<=before+20;
    assert_eq!(blocked.error_code.as_deref(),if refresh_due {Some("proxy")} else {None});
    assert_eq!(store.task(&product.id).unwrap().error_code.as_deref(),Some("proxy"));
    assert_eq!(blocked.attempt_count, 0);
    assert_eq!(blocked.status, "pending");
    let proxy = rv_core::workspace::SocksProxy {
        protocol: Default::default(),
        front_proxy_id: None,
        id: uuid::Uuid::new_v4().to_string(),
        name: "fixture".into(),
        host: "127.0.0.1".into(),
        port: 12345,
        username: String::new(),
        password: String::new(),
        mode: rv_core::workspace::ProxyMode::Shared,
        ip: Default::default(),
    };
    store.save_proxy(&proxy).unwrap();
    store.bind_proxy(&id, Some(&proxy.id)).unwrap();
    let ready = due(&paths, before + 20).unwrap();
    assert_eq!(ready.len(), if refresh_due {8} else {7});
    assert_eq!(ready.iter().any(|task|task.id==keepalive.id),refresh_due);
    assert!(ready.iter().any(|task|task.id==product.id));
    assert!(ready.iter().all(|task|task.error_code.is_none()));
    let product = tasks.iter().find(|t| t.kind == "products").unwrap();
    store.cancel_task(&product.id).unwrap();
    drop(store);
    initialize(&paths, &id).unwrap();
    let store = WorkspaceStore::open(&paths).unwrap();
    assert_eq!(store.tasks(&id).unwrap().len(), 8);
    assert_eq!(store.task(&product.id).unwrap().status, "cancelled");
}
#[test]
fn manual_request_wakes_the_existing_periodic_task_instead_of_creating_another() {
    let (paths, id, _dir) = fixture();
    initialize(&paths, &id).unwrap();
    let store = WorkspaceStore::open(&paths).unwrap();
    let task = store
        .tasks(&id)
        .unwrap()
        .into_iter()
        .find(|t| t.kind == "products")
        .unwrap();
    let manual = enqueue(
        &paths,
        &id,
        &TaskSpec::new(
            "products",
            rv_core::workspace::task_cron_at(timestamp()).unwrap(),
        ),
    )
    .unwrap();
    assert_eq!(manual.id, task.id);
    assert_eq!(manual.cron_expression, task.cron_expression);
    assert_eq!(store.tasks(&id).unwrap().len(), 8);
}

#[test]
fn blocked_accounts_do_not_hide_ready_tasks_beyond_the_first_sixty_four() {
    let (paths, id, _dir) = fixture();
    let phone = AddressBook::load(paths.clone()).unwrap().connections()[0].id;
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let ids: Vec<_> = (0..64)
        .map(|_| AppId::from_idfv(&uuid::Uuid::new_v4().to_string()).unwrap())
        .collect();
    let rows:Vec<_>=ids.iter().map(|id|json!({"idfv":id.as_str(),"backup_id":format!("snapshot:{}",id.as_str()),"kind":"snapshot"})).collect();
    store
        .sync_backups(
            phone,
            "lt.manodrabuziai.fr",
            &json!({"bundle_id":"lt.manodrabuziai.fr","backups":rows}),
        )
        .unwrap();
    let spec = TaskSpec::new("session", "* * * * * *");
    let time = timestamp();
    for blocked in &ids {
        store
            .ensure_task(blocked, "profile", &spec, None, time - 20)
            .unwrap();
    }
    let ready = store
        .ensure_task(&id, "profile", &spec, None, time)
        .unwrap();
    let proxy = rv_core::workspace::SocksProxy {
        protocol: Default::default(),
        front_proxy_id: None,
        id: uuid::Uuid::new_v4().to_string(),
        name: "fixture".into(),
        host: "127.0.0.1".into(),
        port: 12345,
        username: String::new(),
        password: String::new(),
        mode: rv_core::workspace::ProxyMode::Shared,
        ip: Default::default(),
    };
    store.save_proxy(&proxy).unwrap();
    store.bind_proxy(&id, Some(&proxy.id)).unwrap();
    let due = due(&paths, time).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, ready.id);
}

#[test]
fn catalogue_without_credentials_does_not_create_api_tasks() {
    let (paths, id, _dir) = fixture();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    store
        .business_transaction(|db| {
            db.execute("DELETE FROM app_auth WHERE app_id=?1", [id.as_str()])?;
            Ok(())
        })
        .unwrap();
    initialize(&paths, &id).unwrap();
    assert!(store.tasks(&id).unwrap().is_empty());
    store
        .save_auth(
            &id,
            &AppAuth {
                access_token: "imported-fixture-token".into(),
                ..Default::default()
            },
        )
        .unwrap();
    initialize(&paths, &id).unwrap();
    assert_eq!(store.tasks(&id).unwrap().len(), 8);
}

#[test]
fn profiles_are_manual_and_custom_schedules_and_stops_survive_reinitialization() {
    let (paths, id, _dir) = fixture();
    initialize(&paths, &id).unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let initial = store.configured_tasks(&id).unwrap();
    assert_eq!(initial.len(), 8);
    assert!(initial.iter().all(|t| t.kind != "images"));
    assert!(initial.iter().all(|t|t.kind!="session"));
    let keeper = initial
        .iter()
        .find(|task| task.task_key.as_deref() == Some(KEEPALIVE_KEY))
        .unwrap();
    let keepalive_ready = store.run_task_now(&keeper.id).unwrap();
    store.claim_task_when(&keepalive_ready, |_| Ok(())).unwrap();
    store
        .finish_task(&keeper.id, "completed", &json!({}))
        .unwrap();
    let product = initial.iter().find(|t| t.kind == "products").unwrap();
    store.set_task_enabled(&product.id, false).unwrap();
    store
        .reschedule_task(&product.id, "0 */20 * * * *")
        .unwrap();
    initialize(&paths, &id).unwrap();
    let product = store.task(&product.id).unwrap();
    assert!(!product.enabled);
    assert_eq!(product.cron_expression, "0 */20 * * * *");
    assert!(enqueue(&paths, &id, &TaskSpec::new("images", "* * * * * *")).is_err());
}

struct InvalidImage(Mock);
impl BusinessStrategy for InvalidImage {
    fn request(&self, op: Operation, auth: &AppAuth) -> Result<RequestPlan, String> {
        self.0.request(op, auth)
    }
    fn project(&self, op: Operation, status: u16, response: &Value) -> Value {
        self.0.project(op, status, response)
    }
    fn image_urls(&self, _: Operation, _: &Value) -> Vec<String> {
        vec!["file:///invalid-image".into()]
    }
}
#[tokio::test]
async fn definite_image_failure_remains_failed_and_keeps_a_periodic_tasks_next_run() {
    let (paths, id, _dir) = fixture();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    store
        .save_auth(
            &id,
            &AppAuth {
                access_token: "fresh-fixture-token".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let job = enqueue(&paths, &id, &TaskSpec::new("session", "* * * * * *")).unwrap();
    let (plugin, server) = peer(Some(200)).await;
    let result = OfflineRunner::new(None)
        .unwrap()
        .run_admitted(
            &paths,
            &id,
            Operation::Session,
            &InvalidImage(plugin),
            Some(&job),
        )
        .await;
    server.await.unwrap();
    assert!(result.unwrap_err().starts_with("图片缓存失败："));
    let saved = store.task(&job.id).unwrap();
    assert_eq!(saved.status, "pending");
    assert_eq!(saved.last_run_status.as_deref(), Some("failed"));
    assert!(saved.next_run_at.is_some());
    assert_eq!(store.tasks(&id).unwrap().len(), 1);
}

#[tokio::test]
async fn rotating_to_opaque_refresh_without_expiry_clears_old_expiry_metadata() {
    let before = timestamp();
    let (paths, id, _dir) = fixture();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let mut auth = store.auth(&id).unwrap().unwrap();
    auth.refresh_token = Some("old-refresh".into());
    auth.extra
        .insert("refresh_token_expires_at".into(), json!(1_900_000_000));
    auth.extra
        .insert("refresh_token_expires_in".into(), json!(3600));
    auth.extra
        .insert("credentials_received_at".into(), json!(1_800_000_000));
    store.save_auth(&id, &auth).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/refresh", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 8192];
        let _ = socket.read(&mut bytes).await.unwrap();
        let body=json!({"access_token":"new-access","refresh_token":"new-opaque-refresh","expires_in":60}).to_string();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let job = enqueue(&paths, &id, &TaskSpec::new("refresh", "* * * * * *")).unwrap();
    OfflineRunner::new(None)
        .unwrap()
        .run_admitted(&paths, &id, Operation::Refresh, &Mock { url }, Some(&job))
        .await
        .unwrap();
    server.await.unwrap();
    let auth = store.auth(&id).unwrap().unwrap();
    assert_eq!(auth.token_expirations().refresh, None);
    assert!(!auth.extra.contains_key("refresh_token_expires_at"));
    assert!(!auth.extra.contains_key("refresh_token_expires_in"));
    assert!(
        (before..=timestamp()).contains(&auth.extra["credentials_received_at"].as_i64().unwrap())
    );
}

#[test]
fn stopped_keepalive_allows_other_tasks_and_preserves_individual_switches_and_crons() {
    let (paths, id, _dir) = fixture();
    initialize(&paths, &id).unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let tasks = store.configured_tasks(&id).unwrap();
    let keeper = tasks
        .iter()
        .find(|task| task.task_key.as_deref() == Some(KEEPALIVE_KEY))
        .unwrap();
    assert_eq!(keeper.kind, "refresh");
    assert_eq!(keeper.cron_expression, "0 0 * * * *");
    assert_eq!(keeper.max_attempts, 3);
    let first = keeper.next_run_at.unwrap();
    let next = rv_core::workspace::next_task_time(&keeper.cron_expression, first)
        .unwrap()
        .unwrap();
    assert!(next > first && next <= first + 3600);
    assert_eq!(
        rv_core::workspace::next_task_time(&keeper.cron_expression, next).unwrap(),
        Some(next + 3600)
    );
    let product = tasks.iter().find(|task| task.kind == "products").unwrap();
    store
        .reschedule_task(&product.id, "0 */20 * * * *")
        .unwrap();
    store.run_task_now(&product.id).unwrap();
    let order = tasks.iter().find(|task| task.kind == "orders").unwrap();
    set_task_enabled(&paths, &order.id, false).unwrap();
    set_task_enabled(&paths, &keeper.id, false).unwrap();
    assert!(!store.task(&keeper.id).unwrap().enabled);
    assert!(store.task(&product.id).unwrap().enabled);
    assert!(!store.task(&order.id).unwrap().enabled);
    assert!(set_task_enabled(&paths, &product.id, true).is_ok());
    assert!(run_task_now(&paths, &product.id).is_ok());
    assert!(enqueue(&paths, &id, &TaskSpec::new("products", "* * * * * *")).is_ok());
    let candidate=store.task(&product.id).unwrap();
    let running=store.claim_task_when(&candidate,|db|admit_access_token(db,id.as_str(),Some(&candidate.id))).unwrap();
    store.finish_task(&running,"completed",&json!({})).unwrap();
    store.business_transaction(|db|admit_access_token(db,id.as_str(),Some(&product.id))).unwrap();
    initialize(&paths, &id).unwrap();
    assert!(!store.task(&keeper.id).unwrap().enabled);
    assert!(store.task(&product.id).unwrap().enabled);
    let keeper = set_task_enabled(&paths, &keeper.id, true).unwrap();
    assert!(keeper.next_run_at.unwrap() <= timestamp());
    assert!(store.task(&product.id).unwrap().enabled);
    assert!(!store.task(&order.id).unwrap().enabled);
    assert_eq!(
        store.task(&product.id).unwrap().cron_expression,
        "0 */20 * * * *"
    );
}

#[test]
fn stopping_keepalive_does_not_restart_a_request_already_sent() {
    let (paths, id, _dir) = fixture();
    initialize(&paths, &id).unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let keeper = store
        .configured_tasks(&id)
        .unwrap()
        .into_iter()
        .find(|task| task.task_key.as_deref() == Some(KEEPALIVE_KEY))
        .unwrap();
    let ready = store.run_task_now(&keeper.id).unwrap();
    store
        .claim_task_when(&ready, |db| {
            admit_access_token(db, id.as_str(), Some(&ready.id))
        })
        .unwrap();
    set_task_enabled(&paths, &keeper.id, false).unwrap();
    store
        .finish_task(&keeper.id, "completed", &json!({}))
        .unwrap();
    let stopped = store.task(&keeper.id).unwrap();
    assert!(!stopped.enabled);
    assert!(stopped.next_run_at.is_none());
    assert_eq!(stopped.status, "completed");
}

#[test]
fn unknown_keepalive_stays_stopped_until_user_explicitly_retries() {
    let (paths, id, _dir) = fixture();
    initialize(&paths, &id).unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let keeper = store
        .configured_tasks(&id)
        .unwrap()
        .into_iter()
        .find(|task| task.task_key.as_deref() == Some(KEEPALIVE_KEY))
        .unwrap();
    let ready = store.run_task_now(&keeper.id).unwrap();
    store.claim_task_when(&ready, |_| Ok(())).unwrap();
    store
        .finish_task(&keeper.id, "unknown", &json!({}))
        .unwrap();
    set_task_enabled(&paths, &keeper.id, false).unwrap();
    assert!(!store.task(&keeper.id).unwrap().enabled);
    assert!(due(&paths, timestamp() + 3600).unwrap().is_empty());
    let restarted = set_task_enabled(&paths, &keeper.id, true).unwrap();
    assert!(restarted.enabled);
    assert_eq!(restarted.status, "pending");
    assert!(restarted.next_run_at.unwrap() <= timestamp());
    let ready = run_task_now(&paths, &keeper.id).unwrap();
    store
        .claim_task_when(&ready, |db| {
            admit_access_token(db, id.as_str(), Some(&ready.id))
        })
        .unwrap();
    store
        .finish_task(&keeper.id, "completed", &json!({}))
        .unwrap();
    assert_eq!(
        store.task(&keeper.id).unwrap().last_run_status.as_deref(),
        Some("completed")
    );
}

#[tokio::test]
async fn keepalive_network_failure_can_retry_and_recover_without_closing_gate() {
    let (paths, id, _dir) = fixture();
    initialize(&paths, &id).unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let keeper = store
        .configured_tasks(&id)
        .unwrap()
        .into_iter()
        .find(|t| t.task_key.as_deref() == Some(KEEPALIVE_KEY))
        .unwrap();
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/fixture", closed.local_addr().unwrap());
    drop(closed);
    let job = store.run_task_now(&keeper.id).unwrap();
    let runner = OfflineRunner::new(None).unwrap();
    let error = runner
        .run_admitted(&paths, &id, Operation::Refresh, &Mock { url }, Some(&job))
        .await
        .unwrap_err();
    assert!(error.starts_with(crate::CONNECT_FAILURE),"{error}");
    assert!(error.contains("127.0.0.1"));
    assert!(error.contains("Connection refused") || error.contains("connection refused"),"{error}");
    let failed = store.task(&keeper.id).unwrap();
    assert!(failed.enabled);
    assert_eq!(failed.status, "pending");
    assert_eq!(failed.last_run_status.as_deref(), Some("failed"));
    assert!(failed.next_run_at.unwrap() > timestamp());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/fixture", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = vec![];
        while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            bytes.push(socket.read_u8().await.unwrap());
        }
        let body=json!({"access_token":"recovered-access","refresh_token":"recovered-refresh","scope":"user","expires_in":7200}).to_string();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let retry = run_task_now(&paths, &keeper.id).unwrap();
    runner
        .run_admitted(&paths, &id, Operation::Refresh, &Mock { url }, Some(&retry))
        .await
        .unwrap();
    server.await.unwrap();
    let recovered = store.task(&keeper.id).unwrap();
    assert!(recovered.enabled);
    assert!(recovered.next_run_at.is_some());
    assert_eq!(recovered.last_run_status.as_deref(), Some("completed"));
    assert_eq!(
        store.auth(&id).unwrap().unwrap().access_token,
        "recovered-access"
    );
}

#[tokio::test]
async fn first_401_blocks_later_requests_and_new_token_allows_them() {
    let (paths,id,_dir)=fixture();
    let mut store=WorkspaceStore::open(&paths).unwrap();
    let mut auth=store.auth(&id).unwrap().unwrap();auth.access_token="fresh-fixture-token".into();store.save_auth(&id,&auth).unwrap();
    let (mock,server)=peer(Some(401)).await;
    let runner=OfflineRunner::new(None).unwrap();
    assert!(runner.run_with_strategy(&paths,&id,Operation::Session,&mock).await.is_err());
    server.await.unwrap();
    assert!(store.auth(&id).unwrap().unwrap().access_token_rejected());
    // The peer is closed. The next operation must stop on shared token state,
    // before reaching transport, and remain enabled for later recovery.
    let error=runner.run_with_strategy(&paths,&id,Operation::Products,&mock).await.unwrap_err();
    assert!(error.contains("访问令牌已失效"),"{error}");
    auth.access_token="replacement".into();store.save_auth(&id,&auth).unwrap();
    let current=store.auth(&id).unwrap().unwrap();assert!(!current.access_token_rejected());
}

#[test]
fn rejected_token_waits_without_stopping_tasks_and_new_token_resumes_due_work() {
    let (paths,id,_dir)=fixture();initialize(&paths,&id).unwrap();
    let mut store=WorkspaceStore::open(&paths).unwrap();
    let tasks=store.configured_tasks(&id).unwrap();
    let keeper=tasks.iter().find(|t|t.kind=="refresh").unwrap();
    let product=tasks.iter().find(|t|t.kind=="products").unwrap();
    set_task_enabled(&paths,&keeper.id,false).unwrap();
    let proxy=rv_core::workspace::SocksProxy{id:uuid::Uuid::new_v4().to_string(),name:"fixture".into(),host:"127.0.0.1".into(),port:12345,protocol:Default::default(),front_proxy_id:None,username:String::new(),password:String::new(),mode:rv_core::workspace::ProxyMode::Shared,ip:Default::default()};
    store.save_proxy(&proxy).unwrap();store.bind_proxy(&id,Some(&proxy.id)).unwrap();
    run_task_now(&paths,&product.id).unwrap();
    let mut auth=store.auth(&id).unwrap().unwrap();
    store.reject_access_token(&id,&auth.access_token).unwrap();
    assert!(due(&paths,timestamp()).unwrap().iter().all(|t|t.id!=product.id));
    let waiting=store.task(&product.id).unwrap();
    assert!(waiting.enabled);assert_eq!(waiting.status,"pending");assert_eq!(waiting.error_code.as_deref(),Some("access_token"));assert_eq!(waiting.attempt_count,0);
    auth.access_token="new-valid-candidate".into();store.save_auth(&id,&auth).unwrap();
    let ready=due(&paths,timestamp()).unwrap();
    assert!(ready.iter().any(|t|t.id==product.id && t.error_code.is_none()));
    assert!(!store.task(&keeper.id).unwrap().enabled);
}

#[test]
fn legacy_profile_tasks_are_removed_and_dependants_keep_their_settings() {
    let (paths,id,_dir)=fixture();
    let mut store=WorkspaceStore::open(&paths).unwrap();
    let old=store.ensure_task(&id,"profile",&TaskSpec::new("session","0 * * * * *"),None,timestamp()).unwrap();
    let product=store.ensure_task(&id,"products",&TaskSpec::new("products","0 */20 * * * *"),Some(&old.id),timestamp()).unwrap();
    store.set_task_enabled(&product.id,false).unwrap();
    initialize(&paths,&id).unwrap();initialize(&paths,&id).unwrap();
    let tasks=store.configured_tasks(&id).unwrap();assert_eq!(tasks.len(),8);
    assert!(tasks.iter().all(|t|t.kind!="session"));
    let current=store.task(&product.id).unwrap();
    assert!(current.depends_on.is_none());assert!(!current.enabled);
    assert_eq!(current.cron_expression,"0 */20 * * * *");
}

#[test]
fn message_sync_merges_legacy_tasks_preserving_plan_and_enabled_intent() {
    for (details_enabled,inbox_enabled) in [(false,false),(true,false),(false,true),(true,true)] {
        let (paths,id,_dir)=fixture();
        initialize(&paths,&id).unwrap();
        let mut store=WorkspaceStore::open(&paths).unwrap();
        let primary=store.configured_tasks(&id).unwrap().into_iter().find(|task|task.kind=="chat_threads").unwrap();
        store.reschedule_task(&primary.id,"0 */7 * * * *").unwrap();
        store.set_task_enabled(&primary.id,details_enabled).unwrap();
        let old=store.ensure_task(&id,"inbox",&TaskSpec::new("chat_inbox","0 */5 * * * *"),None,timestamp()+300).unwrap();
        store.set_task_enabled(&old.id,inbox_enabled).unwrap();
        let other_before:Vec<_>=store.configured_tasks(&id).unwrap().into_iter().filter(|task|task.id!=primary.id&&task.id!=old.id).collect();
        initialize(&paths,&id).unwrap();
        let saved=store.task(&primary.id).unwrap();
        assert_eq!(saved.kind,"chat_threads");
        assert_eq!(saved.cron_expression,"0 */7 * * * *");
        assert_eq!(saved.enabled,details_enabled||inbox_enabled);
        assert_eq!(saved.next_run_at.is_some(),details_enabled||inbox_enabled);
        assert!(store.task(&old.id).is_err());
        assert_eq!(store.configured_tasks(&id).unwrap().len(),8);
        assert_eq!(store.configured_tasks(&id).unwrap().into_iter().filter(|task|task.id!=primary.id).collect::<Vec<_>>(),other_before);
        initialize(&paths,&id).unwrap();
        assert_eq!(store.task(&primary.id).unwrap(),saved);
    }
}

#[test]
fn message_sync_migration_waits_for_running_legacy_and_promotes_lone_inbox() {
    let (paths,id,_dir)=fixture();
    let mut store=WorkspaceStore::open(&paths).unwrap();
    let old=store.ensure_task(&id,"inbox",&TaskSpec::new("chat_inbox","0 */5 * * * *"),None,timestamp()).unwrap();
    let old=store.run_task_now(&old.id).unwrap();
    let running=store.claim_task_when(&old,|_|Ok(())).unwrap();
    initialize(&paths,&id).unwrap();
    assert_eq!(store.task(&old.id).unwrap().status,"running");
    assert_eq!(store.configured_tasks(&id).unwrap().len(),1);
    store.finish_task(&running,"completed",&json!({})).unwrap();
    initialize(&paths,&id).unwrap();
    let promoted=store.task(&old.id).unwrap();
    assert_eq!(promoted.kind,"chat_threads");
    assert_eq!(promoted.task_key.as_deref(),Some("chat_threads"));
    assert_eq!(store.configured_tasks(&id).unwrap().len(),8);
}
