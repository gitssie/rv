use super::*;
use crate::{AddressBook, Connection, StorePaths};
use serde_json::json;
const ID: &str = "C9EB7804-2062-4519-AC8D-6F68972574F3";
fn fixture() -> (WorkspaceStore, Connection, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let paths = StorePaths::in_dir(dir.path().into());
    let mut book = AddressBook::load(paths.clone()).unwrap();
    let device = Connection::new("phone", "phone", 5900);
    book.upsert(device.clone());
    book.save().unwrap();
    (WorkspaceStore::open(&paths).unwrap(), device, dir)
}
fn result(id: &str) -> serde_json::Value {
    json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"backup_id":"snapshot:1","kind":"snapshot","display_name":"店铺法国","note":"每日检查商品与消息","idfv":id,"future_field":{"a":1}}]})
}
#[test]
fn backup_is_only_creation_source_idfv_deduplicates_and_keeps_local_edits() {
    let (mut store, device, _) = fixture();
    let r = store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    assert_eq!(r.created, 1);
    let id = AppId::from_idfv(ID).unwrap();
    let mut app = store.load(&id).unwrap();
    app.tags = vec!["工作".into(), "工作".into()];
    app.note = "本地备注".into();
    app.metadata = json!({"market":"FR","extra":42}).to_string();
    store.save(&app).unwrap();
    let r = store
        .sync_backups(
            device.id,
            "lt.manodrabuziai.fr",
            &result(&ID.to_lowercase()),
        )
        .unwrap();
    assert_eq!(r.created, 0);
    let app = store.load(&id).unwrap();
    assert_eq!(app.tags, vec!["工作"]);
    assert_eq!(app.note, "本地备注");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&app.metadata).unwrap()["extra"],
        42
    );
    let backups = store.backups(&id).unwrap();
    assert_eq!(backups.len(), 1);
    assert!(backups[0].metadata.contains("future_field"));
}
#[test]
fn same_phone_same_bundle_can_have_multiple_idfv_and_references() {
    let (mut store, device, _) = fixture();
    let mut r = result(ID);
    let mut row = r["backups"][0].clone();
    row["idfv"] = json!("7A70B21A-2F9D-4F66-81C4-6D21355758D7");
    row["backup_id"] = json!("snapshot:2");
    r["backups"].as_array_mut().unwrap().push(row);
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &r)
        .unwrap();
    assert_eq!(store.list(&AppQuery::default()).unwrap().len(), 2);
}
#[test]
fn baselines_and_missing_idfv_do_not_create_cards_and_failure_rolls_back() {
    let (mut store, device, _) = fixture();
    let mut r = result(ID);
    r["backups"][0]["idfv"] = serde_json::Value::Null;
    r["backups"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind":"baseline","backup_id":"baseline:1"}));
    let report = store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &r)
        .unwrap();
    assert_eq!(report.missing_idfv, 1);
    assert_eq!(report.baselines, 1);
    assert!(store.list(&AppQuery::default()).unwrap().is_empty());
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    let mut broken = result(ID);
    broken["backups"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind":"bad"}));
    assert!(
        store
            .sync_backups(device.id, "lt.manodrabuziai.fr", &broken)
            .is_err()
    );
    assert_eq!(
        store.backups(&AppId::from_idfv(ID).unwrap()).unwrap().len(),
        1
    );
}
#[test]
fn full_text_search_supports_chinese_short_queries_tags_and_literal_operators() {
    let (mut store, device, _) = fixture();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    let mut app = store.load(&AppId::from_idfv(ID).unwrap()).unwrap();
    app.tags = vec!["法国".into()];
    app.metadata = json!({"market":"France","nested":"独立策略业务信息"}).to_string();
    app.favorite = true;
    store.save(&app).unwrap();
    for q in ["检查商品", "店铺", "France", "策略业务", "\""] {
        let found = store
            .list(&AppQuery {
                text: q,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(found.len(), 1, "{q}");
    }
    assert_eq!(
        store
            .list(&AppQuery {
                tag: Some("法国"),
                filter: AppFilter::Favorites,
                ..Default::default()
            })
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .list(&AppQuery {
                text: "%",
                ..Default::default()
            })
            .unwrap()
            .is_empty()
    );
}
#[test]
fn tag_choices_remain_available_when_cards_are_filtered_out() {
    let (mut store, device, _) = fixture();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    let mut app = store.load(&AppId::from_idfv(ID).unwrap()).unwrap();
    app.tags = vec!["工作".into(), "法国".into(), "工作".into()];
    app.archived = true;
    store.save(&app).unwrap();
    assert!(store.list(&AppQuery::default()).unwrap().is_empty());
    assert_eq!(store.tags().unwrap(), vec!["工作", "法国"]);
}
#[test]
fn stale_updates_and_cross_bundle_identity_collisions_are_rejected() {
    let (mut store, device, _) = fixture();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    let app = store.load(&AppId::from_idfv(ID).unwrap()).unwrap();
    store.save(&app).unwrap();
    assert!(matches!(store.save(&app), Err(crate::StoreError::Conflict)));
    let mut r = result(ID);
    r["bundle_id"] = json!("other.app");
    assert!(store.sync_backups(device.id, "other.app", &r).is_err());
}
#[test]
fn profile_credentials_are_encrypted_and_tasks_do_not_require_a_device_link() {
    let (mut store, device, dir) = fixture();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    let id = AppId::from_idfv(ID).unwrap();
    let profile = json!({"bundle_id":"lt.manodrabuziai.fr","idfv":ID,"supported":true,"metadata":{"user_id":123,"market":"FR","session":"unknown"},"credentials":{"access_token":"fixture-access-secret","refresh_token":"fixture-refresh-secret","headers":{}}});
    store.import_profile(&id, &profile).unwrap();
    assert_eq!(
        store.auth(&id).unwrap().unwrap().access_token,
        "fixture-access-secret"
    );
    assert!(
        !store
            .load(&id)
            .unwrap()
            .metadata
            .contains("fixture-access-secret")
    );
    assert!(
        store
            .list(&AppQuery {
                text: "fixture-access-secret",
                ..Default::default()
            })
            .unwrap()
            .is_empty()
    );
    let bytes: Vec<u8> = store
        .db
        .query_row(
            "SELECT encrypted FROM app_auth WHERE app_id=?1",
            [ID],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!bytes.windows(21).any(|s| s == b"fixture-access-secret"));
    store
        .sync_backups(
            device.id,
            "lt.manodrabuziai.fr",
            &json!({"bundle_id":"lt.manodrabuziai.fr","backups":[]}),
        )
        .unwrap();
    assert!(store.backups(&id).unwrap().is_empty());
    let task = store.start_task(&id, "session").unwrap();
    assert!(store.start_task(&id, "inbox").is_err());
    store
        .finish_task(&task, "completed", &json!({"session_status":"有效"}))
        .unwrap();
    let reopened = WorkspaceStore::open(&StorePaths::in_dir(dir.path().into())).unwrap();
    assert_eq!(reopened.tasks(&id).unwrap()[0].status, "completed");
    assert!(reopened.auth(&id).unwrap().is_some());
    let mut mismatch = profile;
    mismatch["idfv"] = json!("7A70B21A-2F9D-4F66-81C4-6D21355758D7");
    assert!(store.import_profile(&id, &mismatch).is_err());
}
#[test]
fn plugin_fallback_and_vinted_projection_never_fabricate_business_state() {
    let (mut store, device, _) = fixture();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    let app = store.load(&AppId::from_idfv(ID).unwrap()).unwrap();
    let registry = strategies::StrategyRegistry::default();
    let vinted = registry.for_bundle(&app.bundle_id).project(&app, 1);
    assert_eq!(vinted.action_label, "查看店铺");
    assert_eq!(vinted.facts[0].value, "尚未检测");
    assert_eq!(vinted.facts[1].value, "—");
    assert!(vinted.facts.iter().all(|fact| fact.label != "备份"));
    assert_eq!(
        registry
            .for_bundle("unknown.app")
            .project(&app, 1)
            .action_label,
        "查看环境"
    );
}

#[test]
fn task_commit_rolls_back_credentials_and_projection_when_terminal_write_fails() {
    let (mut store, device, _) = fixture();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    let id = AppId::from_idfv(ID).unwrap();
    let auth = AppAuth {
        access_token: "old".into(),
        refresh_token: Some("old-refresh".into()),
        headers: Default::default(),
        ..AppAuth::default()
    };
    store.save_auth(&id, &auth).unwrap();
    let task = store.start_task(&id, "refresh").unwrap();
    let rotated = AppAuth {
        access_token: "new".into(),
        ..auth
    };
    let revision = store.load(&id).unwrap().revision;
    assert!(
        store
            .complete_task(
                &id,
                &task,
                "invalid",
                &json!({}),
                Some(&rotated),
                Some(&json!({"login":"changed"}))
            )
            .is_err()
    );
    assert_eq!(store.auth(&id).unwrap().unwrap().access_token, "old");
    assert_eq!(store.load(&id).unwrap().revision, revision);
    assert_eq!(store.tasks(&id).unwrap()[0].status, "running");
    store
        .complete_task(
            &id,
            &task,
            "completed",
            &json!({}),
            Some(&rotated),
            Some(&json!({"login":"changed"})),
        )
        .unwrap();
    assert_eq!(store.auth(&id).unwrap().unwrap().access_token, "new");
    assert_eq!(store.tasks(&id).unwrap()[0].status, "completed");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&store.load(&id).unwrap().metadata).unwrap()["login"],
        "changed"
    );
}

#[test]
fn unknown_refresh_blocks_replay_beyond_recent_history_and_import_preserves_rotated_credentials() {
    let (mut store, device, _) = fixture();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    let id = AppId::from_idfv(ID).unwrap();
    let auth = AppAuth {
        access_token: "rotated".into(),
        refresh_token: Some("rotated-refresh".into()),
        headers: Default::default(),
        ..AppAuth::default()
    };
    store.save_auth(&id, &auth).unwrap();
    store.import_profile(&id,&json!({"bundle_id":"lt.manodrabuziai.fr","idfv":ID,"supported":true,"metadata":{},"credentials":{"access_token":"historical","refresh_token":"historical-refresh"}})).unwrap();
    assert_eq!(store.auth(&id).unwrap().unwrap().access_token, "rotated");
    let task = store.start_task(&id, "refresh").unwrap();
    store
        .db
        .execute("UPDATE app_tasks SET deadline=0 WHERE id=?1", [&task])
        .unwrap();
    assert!(store.start_task(&id, "refresh").is_err());
    assert_eq!(store.tasks(&id).unwrap()[0].status, "unknown");
    for _ in 0..51 {
        let query = store.start_task(&id, "session").unwrap();
        store.finish_task(&query, "completed", &json!({})).unwrap();
    }
    assert!(
        store
            .tasks(&id)
            .unwrap()
            .iter()
            .all(|task| task.kind != "refresh")
    );
    assert!(store.start_task(&id, "refresh").is_err());
}

#[test]
fn explicit_card_deletion_removes_links_and_prevents_phone_sync_resurrection() {
    let (mut store, device, _dir) = fixture();
    let id = AppId::from_idfv(ID).unwrap();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    store.delete_app(&id).unwrap();
    assert!(store.load(&id).is_err());
    assert!(store.backups(&id).unwrap().is_empty());
    assert!(store.list(&AppQuery::default()).unwrap().is_empty());
    let report = store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    assert_eq!(report.created, 0);
    assert!(store.list(&AppQuery::default()).unwrap().is_empty());
    let other = "7A70B21A-2F9D-4F66-81C4-6D21355758D7";
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(other))
        .unwrap();
    assert!(store.load(&AppId::from_idfv(other).unwrap()).is_ok());
}

#[test]
fn browsing_records_usage_without_changing_content_timestamp_or_revision() {
    let (mut store, device, _dir) = fixture();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    let id = AppId::from_idfv(ID).unwrap();
    let before = store.load(&id).unwrap();
    store.record_use(&id, before.updated_at + 100).unwrap();
    store.record_use(&id, before.updated_at + 200).unwrap();
    let after = store.load(&id).unwrap();
    assert_eq!(after.last_used_at, Some(before.updated_at + 200));
    assert_eq!(after.updated_at, before.updated_at);
    assert_eq!(after.revision, before.revision);
    // Reading never invalidates a concurrently opened editor.
    store.save(&before).unwrap();
    assert_eq!(store.load(&id).unwrap().last_used_at, after.last_used_at);
}

#[test]
fn proxy_binding_is_persistent_idfv_scoped_and_bound_proxy_cannot_be_deleted() {
    let (mut store, device, dir) = fixture();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    let id = AppId::from_idfv(ID).unwrap();
    let proxy = SocksProxy {
        id: uuid::Uuid::new_v4().to_string(),
        name: "France".into(),
        host: "127.0.0.1".into(),
        port: 1080,
        username: "account".into(),
        password: "proxy-secret".into(),
        mode: ProxyMode::Exclusive,
    };
    store.save_proxy(&proxy).unwrap();
    store.bind_proxy(&id, Some(&proxy.id)).unwrap();
    assert!(store.delete_proxy(&proxy.id).is_err());
    let encrypted: Vec<u8> = store
        .db
        .query_row("SELECT encrypted FROM socks_proxies", [], |r| r.get(0))
        .unwrap();
    assert!(!String::from_utf8_lossy(&encrypted).contains("proxy-secret"));
    drop(store);
    let paths = StorePaths::in_dir(dir.path().into());
    let mut store = WorkspaceStore::open(&paths).unwrap();
    assert_eq!(
        store.proxy_for_app(&id).unwrap().unwrap().password,
        "proxy-secret"
    );
    // Re-syncing snapshots does not change the fixed network exit.
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    assert_eq!(store.proxy_binding(&id).unwrap(), Some(proxy.id.clone()));
    let mut app = store.load(&id).unwrap();
    let revision = app.revision;
    app.title = "rollback".into();
    assert!(store.save_with_proxy(&app, Some("missing")).is_err());
    assert_eq!(store.load(&id).unwrap().revision, revision);
    store.bind_proxy(&id, None).unwrap();
    store.delete_proxy(&proxy.id).unwrap();
    assert!(store.proxy_for_app(&id).unwrap().is_none());
}
#[test]
fn profile_preserves_expiry_and_unknown_credentials_only_in_encrypted_auth() {
    let (mut store, device, _) = fixture();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    let id = AppId::from_idfv(ID).unwrap();
    store.import_profile(&id,&json!({"bundle_id":"lt.manodrabuziai.fr","idfv":ID,"supported":true,
        "metadata":{"exp":2000000000},"credentials":{"access_token":"token-private","refresh_token":"refresh-private",
        "future":{"nested":"extra-private"},"expire_time":"raw-expiry"}})).unwrap();
    let auth = store.auth(&id).unwrap().unwrap();
    assert_eq!(auth.expires_at, Some(2000000000));
    assert_eq!(auth.extra["future"]["nested"], "extra-private");
    assert_eq!(
        serde_json::to_value(&auth).unwrap()["expire_time"],
        "raw-expiry"
    );
    let metadata = store.load(&id).unwrap().metadata;
    assert!(!metadata.contains("private"));
    let encrypted: Vec<u8> = store
        .db
        .query_row("SELECT encrypted FROM app_auth", [], |r| r.get(0))
        .unwrap();
    assert!(!String::from_utf8_lossy(&encrypted).contains("extra-private"));
}

#[test]
fn profile_supplements_matching_tokens_but_never_attaches_old_expiry_to_rotated_tokens() {
    let (mut store, device, _) = fixture();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    let id = AppId::from_idfv(ID).unwrap();
    store
        .save_auth(
            &id,
            &AppAuth {
                access_token: "same".into(),
                refresh_token: Some("same-refresh".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let profile = json!({"bundle_id":"lt.manodrabuziai.fr","idfv":ID,"supported":true,"metadata":{"exp":"2000000000"},
        "credentials":{"access_token":"same","refresh_token":"same-refresh","extra":{"version":1}}});
    store.import_profile(&id, &profile).unwrap();
    let mut rotated = store.auth(&id).unwrap().unwrap();
    assert_eq!(rotated.expires_at, Some(2000000000));
    assert_eq!(rotated.extra["extra"]["version"], 1);
    rotated.access_token = "rotated".into();
    rotated.refresh_token = Some("rotated-refresh".into());
    rotated.expires_at = Some(2100000000);
    store.save_auth(&id, &rotated).unwrap();
    store.import_profile(&id, &profile).unwrap();
    let retained = store.auth(&id).unwrap().unwrap();
    assert_eq!(retained.access_token, "rotated");
    assert_eq!(retained.expires_at, Some(2100000000));
}
