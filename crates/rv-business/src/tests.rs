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
        assert_eq!(store.tasks(&id).unwrap().len(),1,"direct sync must not create Tasks");
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
#[tokio::test]
async fn vinted_queries_require_the_virtual_apps_proxy_binding() {
    let (_dir, paths, mut store, id) = fixture();
    let mut auth = store.auth(&id).unwrap().unwrap();
    auth.refresh_token = Some("fixture-refresh".into());
    store.save_auth(&id, &auth).unwrap();
    assert!(store.proxy_binding(&id).unwrap().is_none());
    let before = store.tasks(&id).unwrap().len();
    let runner = OfflineRunner::new(None).unwrap();
    for operation in [
        Operation::Session,
        Operation::Inbox,
        Operation::NotificationStats,
        Operation::Refresh,
        Operation::Products,
        Operation::Orders,
    ] {
        let error = runner.run(&paths, &id, operation).await.unwrap_err();
        assert!(error.contains("关联代理"), "{operation:?}: {error}");
    }
    assert!(
        runner
            .chat(&paths, &id, chat::Action::Inbox)
            .await
            .unwrap_err()
            .contains("关联代理")
    );
    assert_eq!(store.tasks(&id).unwrap().len(), before);
    assert!(
        assets::fetch_for_app(&paths, &id, "https://images.vinted.net/avatar.jpg")
            .unwrap_err()
            .contains("关联代理")
    );
}
#[test]
fn request_headers_use_the_persisted_session_without_replacement() {
    let mut auth=AppAuth::default();let session=uuid::Uuid::new_v4().to_string();auth.headers.insert("X-Session-Id".into(),session.clone());
    let client=reqwest::Client::new();
    for path in ["first","next","another-operation"] {
        let request=device_headers(client.get(format!("http://127.0.0.1/{path}")),&auth).build().unwrap();
        assert_eq!(request.headers()["x-session-id"],session);
    }
}
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
    fn task_result(&self, operation: Operation, result: &Value) -> Value {
        vinted::Vinted.task_result(operation, result)
    }
}
#[tokio::test]
async fn direct_request_completes_without_phone_and_result_survives_reopen_without_tasks() {
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
        let body = r#"{"user":{"id":123,"login":"fixture shop","country_code":"FR","country_title":"France","photo":{"url":"https://images.vinted.net/avatar.jpg"},"feedback_reputation":0.98,"feedback_count":12,"item_count":6,"email":"private-email"},"access_token":"must-not-be-saved"}"#;
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
    assert!(tasks.is_empty(),"direct account reads must not create Tasks");
    let data: Value = serde_json::from_str(&reopened.load(&id).unwrap().metadata).unwrap();
    assert_eq!(data["session_status"], "有效");
    assert_eq!(data["login"], "fixture shop");
    assert_eq!(data["shop"]["country_title"], "France");
    assert_eq!(data["shop"]["feedback_count"], 12);
    assert_eq!(data["shop"]["item_count"], 6);
    assert_eq!(
        data["shop"]["avatar_url"],
        "https://images.vinted.net/avatar.jpg"
    );
    assert!(!data.to_string().contains("private-email"));
    assert!(!data.to_string().contains("must-not-be-saved"));
}

#[tokio::test]
async fn execute_now_bypasses_future_schedules_and_preserves_stopped_tasks() {
    let (_dir,paths,mut store,id)=fixture();
    for kind in ["products","chat_threads","chat_send","chat_read"] {
        let task=store.ensure_task(&id,kind,&TaskSpec::new(kind,"0 0 * * * *"),None,chrono::Utc::now().timestamp()+86400).unwrap();
        store.set_task_enabled(&task.id,false).unwrap();
    }
    let before=store.tasks(&id).unwrap();
    // Empty consumers finish immediately, even with future, stopped schedules.
    for kind in ["chat_send","chat_read"] {
        tokio::time::timeout(Duration::from_secs(2),shop_sync::execute_now(paths.clone(),id.clone(),kind.into()))
            .await.unwrap().unwrap();
    }
    // A real read enters request preflight now; it does not wait for the cron.
    for kind in ["products","chat_threads"] {
        let error=tokio::time::timeout(Duration::from_secs(2),shop_sync::execute_now(paths.clone(),id.clone(),kind.into()))
            .await.unwrap().unwrap_err();
        assert!(error.contains("关联代理"),"{error}");
    }
    assert_eq!(store.tasks(&id).unwrap(),before);
}

#[tokio::test]
async fn direct_refresh_saves_rotated_pair_without_creating_or_restarting_tasks() {
    struct DirectRefresh(String);
    impl BusinessStrategy for DirectRefresh {
        fn request(&self,operation:Operation,auth:&AppAuth)->Result<RequestPlan,String> {
            let mut plan=vinted::Vinted.request(operation,auth)?;
            plan.url=self.0.clone();Ok(plan)
        }
        fn project(&self,operation:Operation,status:u16,response:&Value)->Value {
            vinted::Vinted.project(operation,status,response)
        }
    }
    for success in [true,false] {
        let (_dir,paths,mut store,id)=fixture();
        let mut auth=store.auth(&id).unwrap().unwrap();
        auth.refresh_token=Some("fixture-refresh".into());
        store.save_auth(&id,&auth).unwrap();
        store.reject_access_token(&id,&auth.access_token).unwrap();
        let keepalive=store.ensure_task(&id,"keepalive",&TaskSpec::new("refresh","0 0 * * * *"),None,chrono::Utc::now().timestamp()+3600).unwrap();
        store.set_task_enabled(&keepalive.id,false).unwrap();
        let before=store.tasks(&id).unwrap();
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url=format!("http://{}",listener.local_addr().unwrap());
        let server=tokio::spawn(async move {
            let (mut socket,_)=listener.accept().await.unwrap();
            let mut bytes=Vec::new();
            loop {
                let mut buf=[0;8192];let n=socket.read(&mut buf).await.unwrap();assert!(n>0);
                bytes.extend_from_slice(&buf[..n]);
                if String::from_utf8_lossy(&bytes).contains("refresh_token=fixture-refresh") {break;}
            }
            let request=String::from_utf8_lossy(&bytes);
            assert!(request.starts_with("POST / "));
            assert!(!request.to_ascii_lowercase().contains("authorization:"));
            assert!(request.contains("grant_type=refresh_token&client_id=ios&scope=user"));
            let (status,body)=if success {(200,json!({"access_token":"rotated-access","refresh_token":"rotated-refresh","expires_in":3600}))}
                else {(400,json!({"error":"invalid_grant","error_description":"fixture invalid refresh"}))};
            let body=body.to_string();
            socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        });
        let outcome=OfflineRunner::new(None).unwrap().run_with_strategy(&paths,&id,Operation::Refresh,&DirectRefresh(url)).await;
        server.await.unwrap();
        assert_eq!(outcome.is_ok(),success);
        assert_eq!(store.tasks(&id).unwrap(),before);
        let saved=store.auth(&id).unwrap().unwrap();
        assert_eq!(saved.access_token,if success {"rotated-access"}else{"fixture"});
        assert_eq!(saved.refresh_token.as_deref(),Some(if success {"rotated-refresh"}else{"fixture-refresh"}));
        assert_eq!(saved.access_token_rejected(),!success);
    }
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
        protocol: Default::default(),
        front_proxy_id: None,
        id: uuid::Uuid::new_v4().to_string(),
        name: "exclusive".into(),
        host: "127.0.0.1".into(),
        port,
        username: String::new(),
        password: String::new(),
        mode: ProxyMode::Exclusive,
        ip: Default::default(),
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
    assert!(store.tasks(&id).unwrap().is_empty());
}

struct ProfileWithAvatar;
impl BusinessStrategy for ProfileWithAvatar {
    fn requires_proxy(&self) -> bool {
        true
    }
    fn request(&self, _: Operation, _: &AppAuth) -> Result<RequestPlan, String> {
        Ok(RequestPlan {
            method: reqwest::Method::GET,
            url: "http://rv-fixture.invalid/user".into(),
            body: None,
        })
    }
    fn project(&self, op: Operation, status: u16, reply: &Value) -> Value {
        vinted::Vinted.project(op, status, reply)
    }
    fn image_urls(&self, op: Operation, result: &Value) -> Vec<String> {
        let urls = vinted::Vinted.image_urls(op, result);
        assert_eq!(urls, ["https://rv-fixture.invalid/avatar"]);
        // Local SOCKS fixture serves HTTP; production keeps the original HTTPS URL.
        urls.into_iter()
            .map(|url| url.replacen("https://", "http://", 1))
            .collect()
    }
}
#[tokio::test]
async fn profile_task_caches_its_avatar_before_success_and_reuses_it_on_the_next_pull() {
    let (_dir, paths, mut store, id) = fixture();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = SocksProxy {
        protocol: Default::default(),
        front_proxy_id: None,
        id: uuid::Uuid::new_v4().to_string(),
        name: "fixture".into(),
        host: "127.0.0.1".into(),
        port: listener.local_addr().unwrap().port(),
        username: String::new(),
        password: String::new(),
        mode: ProxyMode::Exclusive,
        ip: Default::default(),
    };
    store.save_proxy(&proxy).unwrap();
    store.bind_proxy(&id, Some(&proxy.id)).unwrap();
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png = png.into_inner();
    let server = tokio::spawn(async move {
        let mut images = 0;
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            assert_eq!(socket.read_u8().await.unwrap(), 5);
            let count = socket.read_u8().await.unwrap();
            let mut methods = vec![0; usize::from(count)];
            socket.read_exact(&mut methods).await.unwrap();
            socket.write_all(&[5, 0]).await.unwrap();
            let mut connect = [0; 4];
            socket.read_exact(&mut connect).await.unwrap();
            assert_eq!(&connect[..3], &[5, 1, 0]);
            match connect[3] {
                3 => {
                    let n = socket.read_u8().await.unwrap();
                    let mut host = vec![0; usize::from(n)];
                    socket.read_exact(&mut host).await.unwrap();
                }
                1 => {
                    let mut host = [0; 4];
                    socket.read_exact(&mut host).await.unwrap();
                }
                _ => panic!("unexpected address type"),
            }
            let _port = socket.read_u16().await.unwrap();
            socket
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                .await
                .unwrap();
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
            let avatar = String::from_utf8_lossy(&request).starts_with("GET /avatar ");
            if avatar {
                assert!(
                    !String::from_utf8_lossy(&request)
                        .to_lowercase()
                        .contains("authorization:")
                );
            }
            let body = if avatar {
                images += 1;
                png.clone()
            } else {
                json!({"code":0,"user":{"id":123,"login":"fixture","photo":{"url":"https://rv-fixture.invalid/avatar"}}}).to_string().into_bytes()
            };
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(&body).await.unwrap();
        }
        images
    });
    let runner = OfflineRunner::new(None).unwrap();
    for _ in 0..2 {
        tokio::time::timeout(
            Duration::from_secs(3),
            runner.run_with_strategy(&paths, &id, Operation::Session, &ProfileWithAvatar),
        )
        .await
        .unwrap()
        .unwrap();
    }
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap(),
        1
    );
    assert!(assets::cached(&paths, "http://rv-fixture.invalid/avatar").is_some());
    assert!(
        store
            .tasks(&id)
            .unwrap()
            .iter()
            .all(|t| t.status == "completed")
    );
}

#[test]
fn http_error_details_preserve_cause_and_strip_credentials() {
    let auth=AppAuth{access_token:"access-secret".into(),refresh_token:Some("refresh-secret".into()),..Default::default()};
    let value=redact_http_detail("failed https://user:password@host/path?token=other-secret connection refused access-secret refresh-secret".into(),&auth);
    assert!(value.contains("connection refused"));
    for secret in ["password","other-secret","access-secret","refresh-secret"]{assert!(!value.contains(secret));}
}

#[tokio::test]
async fn manual_shop_sync_reuses_pagination_and_session_without_touching_tasks() {
    for fail_second in [false,true] {
        let (_dir,paths,mut store,id)=fixture();
        tasks::initialize(&paths,&id).unwrap();
        for task in store.configured_tasks(&id).unwrap(){tasks::set_task_enabled(&paths,&task.id,false).unwrap();}
        // An already-running scheduler task must not force this read into its queue.
        store.start_task(&id,"session").unwrap();
        let before=store.tasks(&id).unwrap();
        let bus=rv_core::events::workspace_bus(&paths).unwrap();
        let events=bus.subscribe::<rv_core::events::WorkspaceChanged>();
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint=format!("http://{}",listener.local_addr().unwrap());
        let server=tokio::spawn(async move {
            let mut session=None;
            for page in 1..=2 {
                let (mut socket,_)=listener.accept().await.unwrap();
                let mut bytes=vec![0;8192];let n=socket.read(&mut bytes).await.unwrap();
                let request=String::from_utf8_lossy(&bytes[..n]);
                assert!(request.starts_with(&format!("GET /{page} ")));
                let current=request.lines().find_map(|line|line.split_once(':').filter(|(key,_)|key.eq_ignore_ascii_case("x-session-id")).map(|(_,value)|value.trim().to_string())).unwrap();
                if let Some(first)=&session {assert_eq!(first,&current)}else{assert_eq!(uuid::Uuid::parse_str(&current).unwrap().get_version_num(),4);session=Some(current);}
                let body=json!({"code":0,"items":[{"id":page,"user_id":123,"title":format!("page {page}")}],"pagination":{"current_page":page,"total_pages":2}}).to_string();
                let status=if fail_second&&page==2 {503}else{200};
                socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        });
        let result=OfflineRunner::new(None).unwrap().execute_business(&paths,&id,Operation::Products,&PagedFixture(endpoint),None).await;
        server.await.unwrap();
        assert_eq!(result.is_err(),fail_second);
        assert_eq!(store.tasks(&id).unwrap(),before);
        assert_eq!(data::read_data(&paths,&id,Dataset::Products,"",0).unwrap().total,if fail_second {0}else{2});
        let changes:Vec<_>=std::iter::from_fn(||events.try_recv().ok()).flat_map(|event|event.changes.clone()).collect();
        assert!(!changes.iter().any(|c|c.domain==rv_core::events::ChangeDomain::Tasks));
        assert_eq!(changes.iter().any(|c|c.domain==rv_core::events::ChangeDomain::Products),!fail_second);
    }
}

#[tokio::test]
async fn manual_sync_can_continue_when_ownership_changes_but_token_is_unchanged() {
    let (_dir,paths,store,id)=fixture();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint=format!("http://{}",listener.local_addr().unwrap());
    let phone_paths=paths.clone();let phone_id=id.clone();
    let server=tokio::spawn(async move {
        let (mut socket,_)=listener.accept().await.unwrap();let mut request=vec![0;8192];socket.read(&mut request).await.unwrap();
        let mut owner=WorkspaceStore::open(&phone_paths).unwrap();
        owner.business_transaction(|db| {db.execute("INSERT INTO app_execution(app_id,mode,operation_id,updated_at) VALUES(?1,'to_phone','test-handoff',0)",[phone_id.as_str()])?;Ok(())}).unwrap();
        let body=json!({"code":0,"items":[{"id":1,"user_id":123,"title":"first"}],"pagination":{"current_page":1,"total_pages":2}}).to_string();
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        let (mut second,_)=tokio::time::timeout(Duration::from_secs(2),listener.accept()).await.unwrap().unwrap();
        second.read(&mut request).await.unwrap();
        let body=json!({"code":0,"items":[{"id":2,"user_id":123,"title":"second"}],"pagination":{"current_page":2,"total_pages":2}}).to_string();
        second.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
    });
    OfflineRunner::new(None).unwrap().execute_business(&paths,&id,Operation::Products,&PagedFixture(endpoint),None).await.unwrap();
    server.await.unwrap();
    assert!(store.tasks(&id).unwrap().is_empty());
    assert_eq!(data::read_data(&paths,&id,Dataset::Products,"",0).unwrap().total,2);
}

#[tokio::test]
async fn manual_401_marks_shared_token_invalid_without_creating_a_task() {
    let (_dir,paths,store,id)=fixture();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint=format!("http://{}",listener.local_addr().unwrap());
    let server=tokio::spawn(async move {
        let (mut socket,_)=listener.accept().await.unwrap();let mut request=vec![0;8192];socket.read(&mut request).await.unwrap();
        let body=r#"{"message":"Authorization failed."}"#;
        socket.write_all(format!("HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
    });
    let plugin=PagedFixture(endpoint);let runner=OfflineRunner::new(None).unwrap();
    let error=runner.execute_business(&paths,&id,Operation::Products,&plugin,None).await.unwrap_err();
    assert!(error.contains("401"));server.await.unwrap();
    assert!(store.auth(&id).unwrap().unwrap().access_token_rejected());
    assert!(store.tasks(&id).unwrap().is_empty());
    assert!(runner.execute_business(&paths,&id,Operation::Products,&plugin,None).await.unwrap_err().contains("访问令牌已失效"));
}

#[tokio::test]
async fn refresh_rechecks_rv_ownership_and_latest_token_pair_before_sending() {
    for changed_source in [true,false] {
        let (_dir,paths,mut store,id)=fixture();
        let mut original=store.auth(&id).unwrap().unwrap();
        original.refresh_token=Some("original-refresh".into());store.save_auth(&id,&original).unwrap();
        let runner=OfflineRunner {credentials:Some((paths.clone(),id.clone())),..OfflineRunner::new(None).unwrap()};
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let request=runner.client.post(format!("http://{}/refresh",listener.local_addr().unwrap())).body("fixture");
        if changed_source {
            let device=AddressBook::load(paths.clone()).unwrap().connections()[0].id;
            store.set_token_source(&id,TokenSource::Phone(Some(device))).unwrap();
        } else {
            let mut current=original.clone();current.refresh_token=Some("newer-refresh".into());store.save_auth(&id,&current).unwrap();
        }
        let error=runner.send_authenticated(request,&original,true).await.unwrap_err();
        assert!(error.contains(if changed_source {"RV 不刷新"} else {"不使用旧 refresh token"}),"{error}");
        assert!(tokio::time::timeout(std::time::Duration::from_millis(30),listener.accept()).await.is_err(),"no refresh HTTP request may leave after ownership/credentials change");
    }
}

#[tokio::test]
async fn wire_headers_contain_only_phone_context_and_http_fields() {
    for with_phone_agent in [true, false] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let capture = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0; 1024];
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.windows(4).any(|v| v == b"\r\n\r\n") { break; }
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").await.unwrap();
            String::from_utf8(bytes).unwrap()
        });
        let mut auth = AppAuth { access_token: "test-token".into(), ..Default::default() };
        let agent = "vinted-ios Vinted/26.41.1 (lt.manodrabuziai.fr; build:30253; iOS 15.8.3) iPhone9,2";
        if with_phone_agent { auth.headers.insert("User-Agent".into(), agent.into()); }
        auth.headers.insert("X-Session-Id".into(), "fixture-session".into());
        auth.headers.insert("Accept-Language".into(), "fr".into());
        // These must not escape the allowlist, even if present in stored input.
        auth.headers.insert("X-RV-Version".into(), "desktop".into());
        auth.headers.insert("X-Operating-System".into(), "macOS".into());
        let runner = OfflineRunner::new(None).unwrap();
        runner.exchange(RequestPlan { method: reqwest::Method::GET, url: format!("http://{addr}/item"), body: None }, &auth, Operation::Products).await.unwrap();
        let wire = capture.await.unwrap();
        let headers: std::collections::BTreeMap<_, _> = wire.split("\r\n").skip(1)
            .filter_map(|line| line.split_once(": "))
            .map(|(k, v)| (k.to_ascii_lowercase(), v.to_owned())).collect();
        let mut expected = std::collections::BTreeMap::from([
            ("host".to_owned(), addr.to_string()),
            ("accept".to_owned(), "*/*".to_owned()),
            ("accept-encoding".to_owned(), "gzip, deflate, br".to_owned()),
            ("authorization".to_owned(), "Bearer test-token".to_owned()),
            ("x-session-id".to_owned(), "fixture-session".to_owned()),
            ("accept-language".to_owned(), "fr".to_owned()),
        ]);
        if with_phone_agent { expected.insert("user-agent".into(), agent.into()); }
        assert_eq!(headers, expected);
    }
}

#[test]
fn single_product_refresh_updates_only_requested_id_without_scheduler_tasks() {
    let (_dir, paths, mut store, id) = fixture();
    commit(&mut store, &id, Operation::Products, &json!({"code":0,"items":[
        {"id":77,"user_id":123,"title":"Old","is_hidden":true},
        {"id":88,"user_id":123,"title":"Keep"}
    ]})).unwrap();
    let before = store.tasks(&id).unwrap().len();
    let plugin = vinted::Vinted.product_refresh("77").unwrap();
    let plan = plugin.request(Operation::Products, &AppAuth::default()).unwrap();
    assert_eq!(plan.method, reqwest::Method::GET);
    assert_eq!(plan.url, "https://www.vinted.com/api/v2/items/77/details");
    assert!(plan.body.is_none());
    assert!(plugin.requires_proxy());
    let mut combined = Value::Null;
    assert!(!plugin.collect_page(Operation::Products, json!({"code":0,"item":{"id":77,"user":{"id":123},"title":"Updated","is_hidden":false,"access_token":"secret"}}), &mut combined, 1, None).unwrap());
    let result = plugin.project(Operation::Products, 200, &combined);
    store.commit_business_data(&id, &json!({}), |db| plugin.persist(db, &id, Operation::Products, &result)).unwrap();
    let page = data::read_data(&paths, &id, Dataset::Products, "", 0).unwrap();
    assert_eq!(page.total, 2);
    let updated = page.rows.iter().find(|row| row.id == "77").unwrap();
    assert_eq!(updated.title, "Updated");
    assert_eq!(updated.payload["is_hidden"], false);
    assert!(updated.payload.get("access_token").is_none());
    assert_eq!(page.rows.iter().find(|row| row.id == "88").unwrap().title, "Keep");
    assert_eq!(store.tasks(&id).unwrap().len(), before);
    assert!(plugin.collect_page(Operation::Products, json!({"item":{"id":88}}), &mut Value::Null, 1, None).is_err());
    assert!(plugin.collect_page(Operation::Products, json!({"code":99}), &mut Value::Null, 1, None).is_err());
    assert!(vinted::Vinted.product_refresh("77/path").is_err());
    let mut wrong_owner = Value::Null;
    plugin.collect_page(Operation::Products, json!({"item":{"id":77,"user_id":999}}), &mut wrong_owner, 1, None).unwrap();
    let result = plugin.project(Operation::Products, 200, &wrong_owner);
    assert!(store.commit_business_data(&id, &json!({}), |db| plugin.persist(db, &id, Operation::Products, &result)).is_err());
    assert_eq!(data::read_data(&paths, &id, Dataset::Products, "Updated", 0).unwrap().total, 1);
}

#[tokio::test]
async fn refresh_invalid_grant_stops_schedule_but_transient_failure_keeps_it() {
    for (http, code, should_stop) in [(400, "invalid_grant", true), (400, "invalid_scope", false), (503, "unavailable", false)] {
        let (_dir, paths, mut store, id) = fixture();
        let mut auth = store.auth(&id).unwrap().unwrap();
        auth.refresh_token = Some("refresh-fixture".into());
        store.save_auth(&id, &auth).unwrap();
        store.set_token_source(&id, TokenSource::Rv).unwrap();
        let scheduled = store.ensure_task(&id, "keepalive", &TaskSpec::new("refresh", "0 0 * * * *"), None, 0).unwrap();
        let scheduled = store.run_task_now(&scheduled.id).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint: &'static str = Box::leak(format!("http://{}/refresh", listener.local_addr().unwrap()).into_boxed_str());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0; 8192];
            stream.read(&mut buf).await.unwrap();
            let body = json!({"error":code,"error_description":"server rejection"}).to_string();
            stream.write_all(format!("HTTP/1.1 {http} Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let error = OfflineRunner::new(None).unwrap().run_admitted(&paths, &id, Operation::Refresh, &FixtureStrategy(endpoint), Some(&scheduled)).await.unwrap_err();
        assert!(error.contains(code));
        server.await.unwrap();
        let saved = store.task(&scheduled.id).unwrap();
        assert_eq!(saved.enabled, !should_stop);
        assert_eq!(saved.next_run_at.is_none(), should_stop);
        assert_eq!(saved.status, if should_stop {"failed"} else {"pending"});
        assert_eq!(saved.error_code.as_deref(), Some(code));
        assert_eq!(store.auth(&id).unwrap().unwrap().refresh_token, auth.refresh_token);
    }
}

#[test]
fn late_invalid_refresh_result_does_not_disable_new_credentials_schedule() {
    let (_dir, _paths, mut store, id) = fixture();
    store.set_token_source(&id, TokenSource::Rv).unwrap();
    let original = store.auth(&id).unwrap().unwrap();
    let task = store.ensure_task(&id, "keepalive", &TaskSpec::new("refresh", "0 0 * * * *"), None, 0).unwrap();
    let task = store.run_task_now(&task.id).unwrap();
    store.claim_task_when(&task, |_| Ok(())).unwrap();
    let mut fresh = original.clone();
    fresh.access_token = "fresh-phone-access".into();
    fresh.refresh_token = Some("fresh-phone-refresh".into());
    store.save_auth(&id, &fresh).unwrap();
    let stopped = store.complete_invalid_refresh_task(&id, &task.id, &json!({"http_status":400,"error_code":"invalid_grant"}), &original).unwrap();
    assert!(!stopped);
    assert!(store.task(&task.id).unwrap().enabled);
    assert!(store.task(&task.id).unwrap().next_run_at.is_some());
    assert_eq!(store.auth(&id).unwrap().unwrap().access_token, fresh.access_token);
}

#[test]
fn single_product_deleted_is_retained_until_details_confirm_existence() {
    let (_dir, paths, mut store, id) = fixture();
    let original=json!({"id":77,"user_id":123,"title":"Keep deleted bag","photo":{"url":"https://example.invalid/bag.jpg"},"price":{"amount":"360.00","currency_code":"EUR"},"is_hidden":false,"transaction_permitted":true});
    commit(&mut store,&id,Operation::Products,&json!({"code":0,"items":[original.clone(),{"id":88,"user_id":123,"title":"Other"}]})).unwrap();
    let before=store.tasks(&id).unwrap().len();
    let plugin=vinted::Vinted.product_refresh("77").unwrap();
    for (status,code) in [(401,104),(403,104),(500,104),(404,99)] {
        let result=plugin.project(Operation::Products,status,&json!({"code":code,"message":"Error"}));
        assert!(!plugin.is_success(Operation::Products,status,&result));
        store.commit_business_data(&id,&json!({}),|db|plugin.persist(db,&id,Operation::Products,&result)).unwrap();
        assert!(data::read_data(&paths,&id,Dataset::Products,"bag",0).unwrap().rows[0].payload.get("_rv_deleted").is_none());
    }
    let deleted=plugin.project(Operation::Products,404,&json!({"code":104,"message":"Contenu non trouvé"}));
    assert!(plugin.is_success(Operation::Products,404,&deleted));
    store.commit_business_data(&id,&json!({}),|db|plugin.persist(db,&id,Operation::Products,&deleted)).unwrap();
    drop(store);
    let mut store=WorkspaceStore::open(&paths).unwrap();
    let page=data::read_data(&paths,&id,Dataset::Products,"",0).unwrap();
    assert_eq!(page.total,2);
    let row=page.rows.iter().find(|r|r.id=="77").unwrap();
    for key in ["title","price","photo"] {assert_eq!(row.payload[key],original[key]);}
    assert_eq!(row.payload["_rv_deleted"],true);
    assert!(row.summary.contains("已删除"));
    assert_eq!(crate::presentation::vinted(&row.payload,Dataset::Products).badges,["已删除"]);
    assert!(page.rows.iter().find(|r|r.id=="88").unwrap().payload.get("_rv_deleted").is_none());
    assert_eq!(store.tasks(&id).unwrap().len(),before);
    // A list that omits the product retains it, and a stale list cannot clear its status.
    for items in [json!([]),json!([original.clone()]),json!([{"id":77,"user_id":123,"title":"Stale title"}])] {
        let result=vinted::Vinted.project(Operation::Products,200,&json!({"code":0,"items":items}));
        store.commit_business_data(&id,&json!({}),|db|vinted::Vinted.persist(db,&id,Operation::Products,&result)).unwrap();
        let saved=&data::read_data(&paths,&id,Dataset::Products,"bag",0).unwrap().rows[0].payload;
        assert_eq!(saved["_rv_deleted"],true);
        for key in ["title","price","photo"] {assert_eq!(saved[key],original[key]);}
    }
    let mut combined=Value::Null;
    plugin.collect_page(Operation::Products,json!({"code":0,"item":original}),&mut combined,1,None).unwrap();
    let result=plugin.project(Operation::Products,200,&combined);
    store.commit_business_data(&id,&json!({}),|db|plugin.persist(db,&id,Operation::Products,&result)).unwrap();
    let row=&data::read_data(&paths,&id,Dataset::Products,"bag",0).unwrap().rows[0];
    assert_eq!(row.payload["_rv_deleted"],false);
    assert!(row.payload.get("_rv_deleted_at").is_none());
    assert_eq!(crate::presentation::vinted(&row.payload,Dataset::Products).badges,["激活","可交易"]);
}

#[tokio::test]
async fn single_product_deleted_http_result_commits_as_success_without_tasks() {
    struct LocalDetails { url:String, inner:Box<dyn BusinessStrategy> }
    impl BusinessStrategy for LocalDetails {
        fn request(&self,op:Operation,auth:&AppAuth)->Result<RequestPlan,String> {
            let mut plan=self.inner.request(op,auth)?;plan.url=self.url.clone();Ok(plan)
        }
        fn project(&self,op:Operation,status:u16,response:&Value)->Value {self.inner.project(op,status,response)}
        fn is_success(&self,op:Operation,status:u16,result:&Value)->bool {self.inner.is_success(op,status,result)}
        fn persist(&self,db:&rusqlite::Connection,id:&AppId,op:Operation,result:&Value)->Result<(),rv_core::StoreError> {
            self.inner.persist(db,id,op,result)
        }
    }
    let (_dir,paths,mut store,id)=fixture();
    commit(&mut store,&id,Operation::Products,&json!({"code":0,"items":[{"id":77,"user_id":123,"title":"Deleted bag"}]})).unwrap();
    let before=store.tasks(&id).unwrap().len();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let strategy=LocalDetails{url:format!("http://{}/details",listener.local_addr().unwrap()),inner:vinted::Vinted.product_refresh("77").unwrap()};
    let server=tokio::spawn(async move {
        let (mut socket,_)=listener.accept().await.unwrap();
        let mut bytes=[0;8192];socket.read(&mut bytes).await.unwrap();
        let body=r#"{"code":104,"message":"Contenu non trouvé"}"#;
        socket.write_all(format!("HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
    });
    OfflineRunner::new(None).unwrap().run_with_strategy(&paths,&id,Operation::Products,&strategy).await.unwrap();
    server.await.unwrap();
    assert_eq!(store.tasks(&id).unwrap().len(),before);
    let page=data::read_data(&paths,&id,Dataset::Products,"bag",0).unwrap();
    assert_eq!(page.total,1);
    assert_eq!(page.rows[0].payload["_rv_deleted"],true);
    assert_eq!(page.rows[0].title,"Deleted bag");
}
