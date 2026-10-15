use super::*;
use crate::data::Dataset;
use rv_core::{AddressBook, Connection};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
fn fixture() -> (tempfile::TempDir, StorePaths, WorkspaceStore, AppId) {
    let dir = tempfile::tempdir().unwrap();
    let paths = StorePaths::in_dir(dir.path().into());
    let mut book = AddressBook::load(paths.clone()).unwrap();
    let phone = Connection::new("offline", "192.0.2.1", 5900);
    book.upsert(phone.clone());
    book.save().unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let id = AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
    store.sync_backups(phone.id,"lt.manodrabuziai.fr",&json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"kind":"snapshot","backup_id":"snapshot:20261006-01","idfv":id.as_str()}]})).unwrap();
    let mut app = store.load(&id).unwrap();
    app.metadata = json!({"user_id":123}).to_string();
    store.save(&app).unwrap();
    store
        .save_auth(
            &id,
            &AppAuth {
                access_token: "fixture".into(),
                refresh_token: None,
                headers: Default::default(),
                ..AppAuth::default()
            },
        )
        .unwrap();
    (dir, paths, store, id)
}
fn commit(
    store: &mut WorkspaceStore,
    id: &AppId,
    op: Operation,
    response: &Value,
) -> Result<(), rv_core::StoreError> {
    let task = store.start_task(id, op.key())?;
    let result = vinted::Vinted.project(op, 200, response);
    store.complete_task_with(
        id,
        &task,
        "completed",
        &vinted::Vinted.task_result(op, &result),
        None,
        None,
        |db| vinted::Vinted.persist(db, id, op, &result),
    )
}
#[test]
fn products_are_upserted_searched_scoped_and_keep_exact_money_and_unknown_fields() {
    let (_dir, paths, mut store, id) = fixture();
    let item = json!({"id":77,"user_id":123,"title":"Coach 手袋","price":{"amount":"174.9900","currency_code":"EUR"},"is_hidden":true,"status":"Very good","brand":"Coach","future_field":{"new_flag":true},"access_token":"never-store"});
    commit(
        &mut store,
        &id,
        Operation::Products,
        &json!({"code":0,"items":[item]}),
    )
    .unwrap();
    let data = data::read_data(&paths, &id, Dataset::Products, "手袋", 0).unwrap();
    assert_eq!(data.total, 1);
    assert_eq!(data.rows[0].payload["price"]["amount"], "174.9900");
    assert_eq!(data.rows[0].payload["future_field"]["new_flag"], true);
    assert!(!data.rows[0].payload.to_string().contains("never-store"));
    assert!(data.rows[0].summary.contains("隐藏"));
    commit(&mut store,&id,Operation::Products,&json!({"code":0,"items":[{"id":77,"user_id":123,"title":"New listing","price":{"amount":"1.00","currency_code":"GBP"}}]})).unwrap();
    assert_eq!(
        data::read_data(&paths, &id, Dataset::Products, "Coach", 0)
            .unwrap()
            .total,
        0
    );
    let data = data::read_data(&paths, &id, Dataset::Products, "", 0).unwrap();
    assert_eq!(data.total, 1);
    assert_eq!(data.rows[0].title, "New listing");
    assert!(data.rows[0].summary.contains("状态待确认"));
    commit(
        &mut store,
        &id,
        Operation::Products,
        &json!({"code":0,"items":[]}),
    )
    .unwrap();
    assert_eq!(
        data::read_data(&paths, &id, Dataset::Products, "", 0)
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        data::read_data(&paths, &id, Dataset::Products, "\" OR *", 0)
            .unwrap()
            .total,
        0
    );
}
#[test]
fn malformed_collection_rolls_back_business_rows_and_keeps_task_unfinished() {
    let (_dir, paths, mut store, id) = fixture();
    assert!(commit(&mut store,&id,Operation::Products,&json!({"code":0,"items":[{"id":1,"user_id":123,"title":"valid"},{"id":2,"user_id":999,"title":"wrong shop"}]})).is_err());
    assert_eq!(
        data::read_data(&paths, &id, Dataset::Products, "", 0)
            .unwrap()
            .total,
        0
    );
    assert_eq!(store.tasks(&id).unwrap()[0].status, "running");
}
#[test]
fn orders_preserve_direction_and_distinct_ids_and_support_full_text_search() {
    let (_dir, paths, mut store, id) = fixture();
    let orders = json!({"my_orders":[{"escrow_order_id":"order-1","transaction_id":555,"conversation_id":"conversation-uuid","title":"鞋子","status":"needs_action","price":{"amount":"10.50","currency_code":"EUR"},"_rv_direction":"sold"},{"escrow_order_id":"order-1","title":"买入包裹","_rv_direction":"purchased"}]});
    commit(&mut store, &id, Operation::Orders, &orders).unwrap();
    commit(&mut store, &id, Operation::Orders, &orders).unwrap();
    assert_eq!(
        data::read_data(&paths, &id, Dataset::Orders, "", 0)
            .unwrap()
            .total,
        2
    );
    let data = data::read_data(&paths, &id, Dataset::Orders, "needs_action", 0).unwrap();
    assert_eq!(data.total, 1);
    assert_eq!(data.rows[0].payload["transaction_id"], 555);
    assert_eq!(data.rows[0].id, "order-1");
}
#[test]
fn schedule_clock_starts_fresh_and_never_catches_up_after_restart() {
    let (_dir, paths, _store, id) = fixture();
    let mut clock = schedules::ScheduleClock::default();
    schedules::set(&paths, &id, Operation::Products, true, 900).unwrap();
    assert!(clock.due(&paths, 100).unwrap().is_empty());
    assert!(clock.due(&paths, 999).unwrap().is_empty());
    assert_eq!(clock.due(&paths, 1000).unwrap().len(), 1);
    assert!(clock.due(&paths, 1000).unwrap().is_empty());
    let mut restarted = schedules::ScheduleClock::default();
    assert!(restarted.due(&paths, 10000).unwrap().is_empty());
    assert!(restarted.due(&paths, 10899).unwrap().is_empty());
    assert_eq!(restarted.due(&paths, 10900).unwrap().len(), 1);
    schedules::set(&paths, &id, Operation::Products, false, 900).unwrap();
    assert!(restarted.due(&paths, 20000).unwrap().is_empty());
    assert!(schedules::set(&paths, &id, Operation::Refresh, true, 900).is_err());
}
#[test]
fn disabled_or_archived_schedule_never_starts_a_task() {
    let (_dir, paths, mut store, id) = fixture();
    let job = schedules::ScheduledJob {
        app_id: id.clone(),
        operation: Operation::Products,
    };
    schedules::set(&paths, &id, Operation::Products, false, 900).unwrap();
    assert!(
        store
            .start_task_when(&id, "products", |db| schedules::authorize(db, &job))
            .is_err()
    );
    schedules::set(&paths, &id, Operation::Products, true, 900).unwrap();
    let mut app = store.load(&id).unwrap();
    app.archived = true;
    store.save(&app).unwrap();
    assert!(
        store
            .start_task_when(&id, "products", |db| schedules::authorize(db, &job))
            .is_err()
    );
    assert!(store.tasks(&id).unwrap().is_empty());
}
#[test]
fn vinted_collection_contract_requires_consistent_pagination_and_separates_buy_sell() {
    let mut combined = Value::Null;
    assert!(
        vinted::Vinted
            .collect_page(
                Operation::Products,
                json!({"items":[],"code":0,"pagination":{"current_page":1,"total_pages":2}}),
                &mut combined,
                1,
                None
            )
            .unwrap()
    );
    assert!(
        vinted::Vinted
            .collect_page(
                Operation::Products,
                json!({"items":[],"code":0,"pagination":{"current_page":1,"total_pages":2}}),
                &mut combined,
                2,
                None
            )
            .is_err()
    );
    let mut orders = Value::Null;
    for direction in ["sold", "purchased"] {
        assert!(!vinted::Vinted.collect_page(Operation::Orders,json!({"my_orders":[{"escrow_order_id":"o"}],"pagination":{"current_page":1,"total_pages":1}}),&mut orders,1,Some(direction)).unwrap());
    }
    assert_eq!(orders["my_orders"].as_array().unwrap().len(), 2);
    assert_eq!(orders["my_orders"][1]["_rv_direction"], "purchased");
}
struct PagedFixture(String);
impl BusinessStrategy for PagedFixture {
    fn request(&self, _: Operation, _: &AppAuth) -> Result<RequestPlan, String> {
        Err("page required".into())
    }
    fn request_page(
        &self,
        _: Operation,
        _: &AppAuth,
        _: &AppInstance,
        page: u32,
        _: Option<&str>,
    ) -> Result<RequestPlan, String> {
        Ok(RequestPlan {
            method: reqwest::Method::GET,
            url: format!("{}/{page}", self.0),
            body: None,
        })
    }
    fn project(&self, op: Operation, status: u16, response: &Value) -> Value {
        vinted::Vinted.project(op, status, response)
    }
    fn collect_page(
        &self,
        op: Operation,
        response: Value,
        combined: &mut Value,
        page: u32,
        direction: Option<&str>,
    ) -> Result<bool, String> {
        vinted::Vinted.collect_page(op, response, combined, page, direction)
    }
    fn task_result(&self, op: Operation, result: &Value) -> Value {
        vinted::Vinted.task_result(op, result)
    }
    fn persist(
        &self,
        db: &rusqlite::Connection,
        id: &AppId,
        op: Operation,
        result: &Value,
    ) -> Result<(), rv_core::StoreError> {
        vinted::Vinted.persist(db, id, op, result)
    }
}
#[tokio::test]
async fn paged_http_sync_commits_all_pages_or_preserves_previous_inventory() {
    for fail_second in [false, true] {
        let (_dir, paths, mut store, id) = fixture();
        commit(
            &mut store,
            &id,
            Operation::Products,
            &json!({"code":0,"items":[{"id":900,"user_id":123,"title":"previous"}]}),
        )
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for page in 1..=2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = vec![0; 8192];
                let n = socket.read(&mut bytes).await.unwrap();
                assert!(String::from_utf8_lossy(&bytes[..n]).starts_with(&format!("GET /{page} ")));
                let body=json!({"code":0,"items":[{"id":page,"user_id":123,"title":format!("page {page}")}],"pagination":{"current_page":page,"total_pages":2}}).to_string();
                let status = if fail_second && page == 2 { 503 } else { 200 };
                socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        });
        let outcome = OfflineRunner::new(None)
            .unwrap()
            .run_with_strategy(&paths, &id, Operation::Products, &PagedFixture(endpoint))
            .await;
        server.await.unwrap();
        assert_eq!(outcome.is_err(), fail_second);
        let data = data::read_data(&paths, &id, Dataset::Products, "", 0).unwrap();
        assert_eq!(data.total, if fail_second { 1 } else { 3 });
        assert_eq!(
            store.tasks(&id).unwrap()[0].status,
            if fail_second { "failed" } else { "completed" }
        );
    }
}
#[test]
fn same_platform_product_id_in_two_shops_never_shares_records_or_search_results() {
    let (_dir, paths, mut store, id) = fixture();
    let other = AppId::from_idfv("3272E520-9D25-4531-AE00-4A2A67045BDD").unwrap();
    let phone = AddressBook::load(paths.clone()).unwrap().connections()[0].id;
    store.sync_backups(phone,"lt.manodrabuziai.fr",&json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"kind":"snapshot","backup_id":"snapshot:20261006-02","idfv":other.as_str()}]})).unwrap();
    let mut app = store.load(&other).unwrap();
    app.metadata = json!({"user_id":456}).to_string();
    store.save(&app).unwrap();
    commit(
        &mut store,
        &id,
        Operation::Products,
        &json!({"code":0,"items":[{"id":1,"user_id":123,"title":"first shop"}]}),
    )
    .unwrap();
    commit(
        &mut store,
        &other,
        Operation::Products,
        &json!({"code":0,"items":[{"id":1,"user_id":456,"title":"second shop"}]}),
    )
    .unwrap();
    assert_eq!(
        data::read_data(&paths, &id, Dataset::Products, "", 0)
            .unwrap()
            .rows[0]
            .title,
        "first shop"
    );
    assert_eq!(
        data::read_data(&paths, &other, Dataset::Products, "first shop", 0)
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        data::read_data(&paths, &other, Dataset::Products, "", 0)
            .unwrap()
            .rows[0]
            .title,
        "second shop"
    );
}
struct FixtureStrategy(&'static str);
impl BusinessStrategy for FixtureStrategy {
    fn request(&self, _: Operation, _: &AppAuth) -> Result<RequestPlan, String> {
        Ok(RequestPlan {
            url: self.0.into(),
            method: reqwest::Method::GET,
            body: None,
        })
    }
    fn project(&self, operation: Operation, status: u16, response: &Value) -> Value {
        vinted::Vinted.project(operation, status, response)
    }
}
#[tokio::test]
async fn task_completes_on_host_without_phone_and_result_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let paths = StorePaths::in_dir(dir.path().into());
    let mut book = AddressBook::load(paths.clone()).unwrap();
    let phone = Connection::new("disconnected phone", "192.0.2.1", 5900);
    book.upsert(phone.clone());
    book.save().unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let id = AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
    store.sync_backups(phone.id,"lt.manodrabuziai.fr",&json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"kind":"snapshot","backup_id":"snapshot:20261006-01","idfv":id.as_str()}]})).unwrap();
    store
        .save_auth(
            &id,
            &AppAuth {
                access_token: "fixture-access".into(),
                refresh_token: None,
                headers: Default::default(),
                ..AppAuth::default()
            },
        )
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint: &'static str =
        Box::leak(format!("http://{}/session", listener.local_addr().unwrap()).into_boxed_str());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = vec![0; 8192];
        let read = socket.read(&mut bytes).await.unwrap();
        let request = String::from_utf8_lossy(&bytes[..read]);
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer fixture-access")
        );
        let body = r#"{"user":{"id":123,"login":"fixture shop","country_code":"FR"},"access_token":"must-not-be-saved"}"#;
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
    OfflineRunner::new(None)
        .unwrap()
        .run_with_strategy(&paths, &id, Operation::Session, &FixtureStrategy(endpoint))
        .await
        .unwrap();
    server.await.unwrap();
    let reopened = WorkspaceStore::open(&paths).unwrap();
    let tasks = reopened.tasks(&id).unwrap();
    assert_eq!(tasks[0].status, "completed");
    assert!(
        !tasks[0]
            .result
            .as_ref()
            .unwrap()
            .to_string()
            .contains("must-not-be-saved")
    );
    let data: Value = serde_json::from_str(&reopened.load(&id).unwrap().metadata).unwrap();
    assert_eq!(data["session_status"], "有效");
    assert_eq!(data["login"], "fixture shop");
}
#[test]
fn compact_detail_reads_all_local_rows_and_preview_uses_newest_product_id() {
    let (_dir, paths, mut store, id) = fixture();
    let items: Vec<_> = (1..=67).rev().map(|id| json!({"id":id,"user_id":123,"title":format!("Product {id}"),"is_hidden":true,"is_reserved":true})).collect();
    commit(
        &mut store,
        &id,
        Operation::Products,
        &json!({"code":0,"items":items}),
    )
    .unwrap();
    let page = data::read_all(&paths, &id, Dataset::Products).unwrap();
    assert_eq!(page.rows.len(), 67);
    assert_eq!(page.rows[0].id, "67");
    assert_eq!(
        page.rows
            .iter()
            .map(|r| &r.id)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        67
    );
    assert_eq!(page.rows[0].visual.badges, ["隐藏", "预留"]);
}

#[tokio::test]
async fn waiting_for_another_idfv_does_not_create_or_expire_a_running_task() {
    let (_dir, paths, mut store, id) = fixture();
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let proxy = SocksProxy {
        id: uuid::Uuid::new_v4().to_string(),
        name: "exclusive".into(),
        host: "127.0.0.1".into(),
        port,
        username: String::new(),
        password: String::new(),
        mode: ProxyMode::Exclusive,
    };
    store.save_proxy(&proxy).unwrap();
    store.bind_proxy(&id, Some(&proxy.id)).unwrap();
    let other = AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F4").unwrap();
    let mut book = AddressBook::load(paths.clone()).unwrap();
    let device = Connection::new("other", "192.0.2.2", 5900);
    book.upsert(device.clone());
    book.save().unwrap();
    store.sync_backups(device.id,"lt.manodrabuziai.fr",&json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"kind":"snapshot","backup_id":"snapshot:2","idfv":other.as_str()}]})).unwrap();
    store.bind_proxy(&other, Some(&proxy.id)).unwrap();
    let runner = OfflineRunner::new(None).unwrap();
    let occupied = network::route(&paths, &other, &runner.client).unwrap();
    let lease = occupied.acquire().await;
    let mut pending = Box::pin(runner.run(&paths, &id, Operation::Session));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut pending)
            .await
            .is_err()
    );
    assert!(store.tasks(&id).unwrap().is_empty());
    drop(lease);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), pending)
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(store.tasks(&id).unwrap().len(), 1);
}
