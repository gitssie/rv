use super::*;
use crate::{BusinessStrategy, Operation, vinted::Vinted};
use serde_json::json;

fn fixture() -> (tempfile::TempDir, StorePaths, WorkspaceStore, Vec<AppId>) {
    let dir = tempfile::tempdir().unwrap();
    let paths = StorePaths::in_dir(dir.path().into());
    let mut book = rv_core::AddressBook::load(paths.clone()).unwrap();
    let phone = rv_core::Connection::new("fixture", "192.0.2.1", 5900);
    book.upsert(phone.clone());
    book.save().unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let ids = [
        "C9EB7804-2062-4519-AC8D-6F68972574F3",
        "C9EB7804-2062-4519-AC8D-6F68972574F4",
    ]
    .iter()
    .map(|id| AppId::from_idfv(id).unwrap())
    .collect::<Vec<_>>();
    store.sync_backups(phone.id, "lt.manodrabuziai.fr", &json!({"bundle_id":"lt.manodrabuziai.fr","backups":ids.iter().map(|id|json!({"kind":"snapshot","idfv":id.as_str(),"backup_id":format!("snapshot:{}",id.as_str())})).collect::<Vec<_>>()})).unwrap();
    for id in &ids {
        store
            .save_auth(
                id,
                &AppAuth {
                    access_token: "fixture".into(),
                    ..Default::default()
                },
            )
            .unwrap();
    }
    (dir, paths, store, ids)
}

fn result(rows: Value) -> Value {
    json!({"http_status":200,"response":{"code":0,"notifications":rows}})
}
#[test]
fn notifications_use_verified_read_only_endpoint_and_collect_all_pages() {
    let (_dir, _paths, store, ids) = fixture();
    let app = store.load(&ids[0]).unwrap();
    let auth = store.auth(&ids[0]).unwrap().unwrap();
    let request = Vinted
        .request_page(Operation::Notifications, &auth, &app, 2, None)
        .unwrap();
    assert_eq!(request.method, reqwest::Method::GET);
    assert_eq!(
        request.url,
        "https://api.vinted.com/inbox-notifications/v1/notifications?page=2&per_page=20"
    );
    assert!(request.body.is_none());
    let mut combined = Value::Null;
    assert!(Vinted.collect_page(Operation::Notifications,
        json!({"code":0,"notifications":[{"id":1}],"pagination":{"current_page":1,"total_pages":2}}),
        &mut combined, 1, None).unwrap());
    assert!(!Vinted.collect_page(Operation::Notifications,
        json!({"code":0,"notifications":[{"id":2}],"pagination":{"current_page":2,"total_pages":2}}),
        &mut combined, 2, None).unwrap());
    assert_eq!(combined["notifications"].as_array().unwrap().len(), 2);
    // Empty response captured from the current mobile API.
    assert!(!Vinted.collect_page(Operation::Notifications,
        json!({"code":0,"notifications":[],"pagination":{"current_page":1,"per_page":20,"total_entries":0,"total_pages":1}}),
        &mut Value::Null, 1, None).unwrap());
    let summary = Vinted.task_result(
        Operation::Notifications,
        &result(json!([{"id":1,"body":"private notice"}])),
    );
    assert_eq!(summary, json!({"http_status":200,"record_count":1}));
}

#[test]
fn shop_isolation_deduplication_unread_and_html_are_preserved_offline() {
    let (_dir, paths, mut store, ids) = fixture();
    for id in &ids {
        store.business_transaction(|db| persist(db, id, &result(json!([
            {"id":"same-id","updated_at":"2026-10-28T10:32:00Z","read":false,"body":"<p>Bonjour &amp; merci</p><p>新通知</p><script>hidden()</script>","note":"Note","item_id":42,"order_id":"order","link":"vinted://items/42","future_field":true,"access_token":"never-store"}
        ])))).unwrap();
    }
    let rows = list(&paths).unwrap();
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].app_id, rows[1].app_id);
    assert_eq!(rows[0].body, "Bonjour & merci\n新通知");
    assert_eq!(rows[0].unread, Some(true));
    assert_eq!(rows[0].item_id.as_deref(), Some("42"));
    assert!(rows[0].created_at.is_some());
    store
        .business_transaction(|db| {
            let payload: String = db.query_row(
                "SELECT payload FROM vinted_notifications LIMIT 1",
                [],
                |r| r.get(0),
            )?;
            assert!(payload.contains("future_field"));
            assert!(!payload.contains("never-store"));
            Ok(())
        })
        .unwrap();
    let tasks_before: Vec<_> = ids
        .iter()
        .map(|id| store.tasks(id).unwrap().len())
        .collect();
    // Reading/selecting does not enqueue work or mark anything remotely/locally read.
    assert_eq!(list(&paths).unwrap()[0].unread, Some(true));
    assert_eq!(
        tasks_before,
        ids.iter()
            .map(|id| store.tasks(id).unwrap().len())
            .collect::<Vec<_>>()
    );
    store
        .business_transaction(|db| {
            persist(
                db,
                &ids[0],
                &result(json!([
                    {"id":"same-id","updated_at":1793190000_i64,"read":true,"body":"Changed"}
                ])),
            )
        })
        .unwrap();
    let rows = list(&paths).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter().find(|n| n.app_id == ids[0]).unwrap().unread,
        Some(false)
    );
    assert_eq!(
        rows.iter().find(|n| n.app_id == ids[1]).unwrap().unread,
        Some(true)
    );
    // An empty latest page is not a deletion instruction for cached history.
    store
        .business_transaction(|db| persist(db, &ids[0], &result(json!([]))))
        .unwrap();
    assert_eq!(list(&paths).unwrap().len(), 2);
    let mut app = store.load(&ids[0]).unwrap();
    app.archived = true;
    store.save(&app).unwrap();
    assert_eq!(list(&paths).unwrap().len(), 1);
    store.delete_app(&ids[1]).unwrap();
    assert!(list(&paths).unwrap().is_empty());
}

#[test]
fn unknown_notice_types_stay_readable_and_partial_batch_rolls_back() {
    let (_dir, paths, mut store, ids) = fixture();
    store
        .business_transaction(|db| {
            persist(
                db,
                &ids[0],
                &result(json!([
                    {"id":1,"entry_type":"future_variant","new_content":{"text":"new format"}}
                ])),
            )
        })
        .unwrap();
    let rows = list(&paths).unwrap();
    assert!(rows[0].body.contains("new format"));
    assert_eq!(rows[0].unread, None);
    assert!(rows[0].created_at.is_none());
    assert!(
        store
            .business_transaction(|db| persist(
                db,
                &ids[0],
                &result(json!([
                    {"id":2,"body":"should roll back"},{"body":"missing identity"}
                ]))
            ))
            .is_err()
    );
    assert_eq!(list(&paths).unwrap().len(), 1);
    store
        .business_transaction(|db| persist(db, &ids[0], &json!({"http_status":401})))
        .unwrap();
    assert_eq!(list(&paths).unwrap().len(), 1);
}
