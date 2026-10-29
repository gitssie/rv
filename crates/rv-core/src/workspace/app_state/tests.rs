use super::*;
use crate::workspace::{ExecutionMode, TaskFinishPolicy, TaskSpec, task_cron_at};
use crate::{AddressBook, Connection, StorePaths};
use serde_json::json;

const BUNDLE: &str = "lt.manodrabuziai.fr";
const ID: &str = "C9EB7804-2062-4519-AC8D-6F68972574F3";
fn dict(values: Vec<(&str, Plist)>) -> Plist {
    Plist::Dictionary(
        values
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}
fn string(s: &str) -> Plist {
    Plist::String(s.into())
}
fn record(service: &str, account: &str, value: &str) -> Plist {
    dict(vec![
        ("svce", string(service)),
        ("acct", string(account)),
        ("agrp", string("fixture.group")),
        ("v_Data", Plist::Data(value.as_bytes().to_vec())),
        ("future", Plist::Data(vec![1, 2, 3])),
    ])
}
fn fixture_state() -> ShadowAppState {
    let install = "7A70B21A-2F9D-4F66-81C4-6D21355758D7";
    let keychain = Plist::Array(vec![
        record("Vintedfr", "token", "original-access"),
        record("Vintedfr", "refresh_token", "original-refresh"),
        record(
            "deviceIdentifierService",
            "deviceIdentifierKey",
            "original-device",
        ),
        record(
            "VintedSessionStorage",
            "user",
            r#"{"anonId":"original-anon"}"#,
        ),
        record(
            &format!("DeviceToken-{BUNDLE}-{install}"),
            "DeviceTokenKeychainValue",
            "install-token",
        ),
        record(
            &format!("DeviceToken.{BUNDLE}"),
            "DeviceTokenKeychainValue",
            "permanent-token",
        ),
        record("unrelated-service", "unrelated-account", "preserve-unknown"),
    ]);
    let identity = dict(vec![
        (
            "data",
            dict(vec![
                ("VendorIdentifier", string(ID)),
                ("future", Plist::Data(vec![9, 8, 7])),
            ]),
        ),
        (
            "configuration",
            dict(vec![("future", Plist::Boolean(true))]),
        ),
    ]);
    let preferences = dict(vec![
        ("version", Plist::Integer(1.into())),
        (
            "records",
            Plist::Array(vec![dict(vec![
                ("container", string("data")),
                ("domain", string(BUNDLE)),
                ("host", string("any")),
                (
                    "values",
                    dict(vec![
                        ("DeviceTokenSessionUUID", string(install)),
                        ("ISOLocaleDefaultKey", string("en_FR")),
                        ("CurrentLanguageCodeDefaultsKey", string("en")),
                        ("token_scope", string("user")),
                        ("unknown", Plist::Data(vec![4, 5, 6])),
                    ]),
                ),
            ])]),
        ),
    ]);
    let payloads = [
        encode(&keychain).unwrap(),
        encode(&identity).unwrap(),
        encode(&preferences).unwrap(),
    ];
    let entries = Plist::Array(
        NAMES
            .iter()
            .map(|name| {
                dict(vec![
                    ("path", string(name)),
                    ("type", string("file")),
                    ("mode", Plist::Integer(0o600.into())),
                    ("uid", Plist::Integer(501.into())),
                    ("gid", Plist::Integer(501.into())),
                    ("mtime", Plist::Integer(0.into())),
                    ("mtimeNsec", Plist::Integer(0.into())),
                    ("birthtime", Plist::Integer(0.into())),
                    ("birthtimeNsec", Plist::Integer(0.into())),
                ])
            })
            .collect(),
    );
    let manifest = dict(vec![
        ("format", string("ShadowAppState2")),
        ("bundleID", string(BUNDLE)),
        ("idfv", string(ID)),
        ("version", string("26.41.1")),
        ("build", string("30253")),
        ("future", Plist::Data(vec![8, 9])),
        (
            "archiveInfo",
            dict(vec![("sha256", string("untouched-data-archive"))]),
        ),
        (
            "stateArchiveInfo",
            dict(vec![
                ("format", string("pax")),
                ("compression", string("zstd")),
                ("compressionLevel", Plist::Integer(3.into())),
                ("sourceRoots", dict(vec![])),
                ("entries", entries),
            ]),
        ),
    ]);
    let (manifest, archive) = repack(&encode(&manifest).unwrap(), &payloads).unwrap();
    ShadowAppState::decode(
        &AppId::from_idfv(ID).unwrap(),
        BUNDLE,
        manifest,
        archive,
        AppDeviceInfo {
            model: "iPhone9,2".into(),
            os_version: "15.8.8".into(),
        },
    )
    .unwrap()
}
fn store_fixture() -> (tempfile::TempDir, WorkspaceStore, AppId, ConnectionId) {
    let dir = tempfile::tempdir().unwrap();
    let paths = StorePaths::in_dir(dir.path().into());
    let mut book = AddressBook::load(paths.clone()).unwrap();
    let phone = Connection::new("phone", "fixture", 5900);
    book.upsert(phone.clone());
    book.save().unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let id = AppId::from_idfv(ID).unwrap();
    store.sync_backups(phone.id,BUNDLE,&serde_json::json!({"bundle_id":BUNDLE,"backups":[{"backup_id":format!("snapshot:{ID}"),"kind":"snapshot","idfv":ID}]})).unwrap();
    (dir, store, id, phone.id)
}
#[test]
fn state_builds_request_context_from_actual_scoped_records() {
    let state = fixture_state();
    let auth = state.request_auth().unwrap();
    assert_eq!(auth.access_token, "original-access");
    assert_eq!(auth.headers["X-Device-UUID"], "original-device");
    assert_eq!(auth.headers["X-Anon-Id"], "original-anon");
    assert_eq!(auth.headers["X-V-Udt"], "install-token");
    assert_eq!(auth.headers["Locale"], "en-FR");
    assert_eq!(auth.headers["Accept-Language"], "en");
    assert_eq!(
        auth.headers["User-Agent"],
        "vinted-ios Vinted/26.41.1 (lt.manodrabuziai.fr; build:30253; iOS 15.8.8) iPhone9,2"
    );
    assert!(!auth.headers.contains_key("X-Session-Id"));
    assert_eq!(auth.request_context().unwrap().idfv, ID);
    assert!(!format!("{state:?}").contains("original-access"));
}
#[test]
fn token_update_is_atomic_and_export_preserves_unknown_state() {
    let (_dir, mut store, id, phone) = store_fixture();
    let state = fixture_state();
    store.import_app_state(&id, Some(phone), &state).unwrap();
    let stored: Vec<u8> = store
        .db
        .query_row(
            "SELECT keychain_blob FROM app_states WHERE app_id=?1",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !stored
            .windows(b"original-access".len())
            .any(|v| v == b"original-access")
    );
    store
        .update_app_tokens(
            &id,
            "new-access".into(),
            Some("new-refresh".into()),
            Some(2_000_000_000),
        )
        .unwrap();
    let current = store.app_state(&id).unwrap().unwrap();
    assert_eq!(current.archive(), state.archive());
    assert_eq!(store.auth(&id).unwrap().unwrap().access_token, "new-access");
    let exported = store.prepare_state_upload(&id).unwrap();
    assert_ne!(exported.archive(), state.archive());
    let decoded = ShadowAppState::decode(
        &id,
        BUNDLE,
        exported.manifest.clone(),
        exported.archive.clone(),
        exported.device.clone(),
    )
    .unwrap();
    assert_eq!(decoded.request_auth().unwrap().access_token, "new-access");
    assert_eq!(decoded.payloads[1], state.payloads[1]);
    assert_eq!(
        context::keychain_value(
            &parse(&decoded.payloads[0]).unwrap(),
            "unrelated-service",
            "unrelated-account"
        )
        .unwrap()
        .unwrap(),
        b"preserve-unknown"
    );
    assert_eq!(
        at(&parse(&decoded.manifest).unwrap(), "archiveInfo").unwrap(),
        at(&parse(&state.manifest).unwrap(), "archiveInfo").unwrap()
    );
    assert_eq!(
        store.prepare_state_upload(&id).unwrap().archive(),
        exported.archive()
    );
    store.import_app_state(&id, Some(phone), &state).unwrap();
    assert_eq!(store.auth(&id).unwrap().unwrap().access_token, "new-access");
    assert_eq!(
        store
            .prepare_state_upload(&id)
            .unwrap()
            .request_auth()
            .unwrap()
            .access_token,
        "new-access"
    );
}
#[test]
fn auth_reads_and_updates_never_load_or_decompress_archive_blob() {
    let (_dir, mut store, id, _) = store_fixture();
    store.import_app_state(&id, None, &fixture_state()).unwrap();
    store
        .db
        .execute(
            "UPDATE app_states SET archive_blob=x'00' WHERE app_id=?1",
            [id.as_str()],
        )
        .unwrap();
    assert_eq!(
        store.auth(&id).unwrap().unwrap().access_token,
        "original-access"
    );
    store
        .update_app_tokens(&id, "new-access".into(), Some("new-refresh".into()), None)
        .unwrap();
    assert_eq!(store.auth(&id).unwrap().unwrap().access_token, "new-access");
}
#[test]
fn invalid_archive_and_identity_are_rejected_without_creating_state() {
    let (_dir, mut store, id, _) = store_fixture();
    let state = fixture_state();
    let mut archive = state.archive.clone();
    archive[3] ^= 1;
    assert!(
        ShadowAppState::decode(
            &id,
            BUNDLE,
            state.manifest.clone(),
            archive,
            state.device.clone()
        )
        .is_err()
    );
    assert!(
        ShadowAppState::decode(
            &AppId::from_idfv("7A70B21A-2F9D-4F66-81C4-6D21355758D7").unwrap(),
            BUNDLE,
            state.manifest.clone(),
            state.archive.clone(),
            state.device.clone()
        )
        .is_err()
    );
    assert!(store.app_state(&id).unwrap().is_none());
    store.import_app_state(&id, None, &state).unwrap();
    let old = store.auth(&id).unwrap().unwrap().access_token;
    assert!(
        store
            .update_app_tokens(&id, "".into(), Some("bad".into()), None)
            .is_err()
    );
    assert_eq!(store.auth(&id).unwrap().unwrap().access_token, old);
    assert_eq!(
        store
            .app_state(&id)
            .unwrap()
            .unwrap()
            .request_auth()
            .unwrap()
            .access_token,
        old
    );
}
#[test]
fn clearing_tokens_and_expiry_cannot_leave_old_credentials_in_state() {
    let (_dir, mut store, id, _) = store_fixture();
    store.import_app_state(&id, None, &fixture_state()).unwrap();
    store
        .update_app_tokens(
            &id,
            "new-access".into(),
            Some("new-refresh".into()),
            Some(2_000_000_000),
        )
        .unwrap();
    store
        .update_app_tokens(&id, "newest-access".into(), None, None)
        .unwrap();
    let auth = store.auth(&id).unwrap().unwrap();
    assert!(auth.refresh_token.is_none());
    assert!(auth.expires_at.is_none());
    let exported = store
        .prepare_state_upload(&id)
        .unwrap()
        .request_auth()
        .unwrap();
    assert_eq!(auth.access_token, exported.access_token);
    assert!(exported.refresh_token.is_none());
    assert!(exported.expires_at.is_none());
}

#[test]
fn unchanged_import_preserves_fractional_expiry_and_other_preference_scopes() {
    let mut state = fixture_state();
    let mut preferences = parse(&state.payloads[2]).unwrap();
    let records = preferences
        .as_dictionary_mut()
        .unwrap()
        .get_mut("records")
        .unwrap()
        .as_array_mut()
        .unwrap();
    let date = Plist::Date(
        (std::time::UNIX_EPOCH + std::time::Duration::new(1_800_000_000, 123_456_000)).into(),
    );
    records[0]
        .as_dictionary_mut()
        .unwrap()
        .get_mut("values")
        .unwrap()
        .as_dictionary_mut()
        .unwrap()
        .insert("token_expiration".into(), date);
    let mut other = records[0].clone();
    other
        .as_dictionary_mut()
        .unwrap()
        .insert("host".into(), string("current"));
    other
        .as_dictionary_mut()
        .unwrap()
        .get_mut("values")
        .unwrap()
        .as_dictionary_mut()
        .unwrap()
        .remove("token_expiration");
    records.push(other);
    state.payloads[2] = encode(&preferences).unwrap();
    (state.manifest, state.archive) = repack(&state.manifest, &state.payloads).unwrap();
    let (_dir, mut store, id, source) = store_fixture();
    store.import_app_state(&id, Some(source), &state).unwrap();
    let exported = store.prepare_state_upload(&id).unwrap();
    assert!(
        exported.archive() == state.archive(),
        "unchanged state must retain the exact downloaded archive"
    );
    assert!(
        exported.payload("preferences/records.plist") == state.payload("preferences/records.plist")
    );
}

#[test]
fn phone_handoff_fences_tasks_waits_for_rotation_and_persists_verified_owner() {
    let (dir, mut store, id, device) = store_fixture();
    store
        .import_app_state(&id, Some(device), &fixture_state())
        .unwrap();
    let spec = TaskSpec::new("refresh", task_cron_at(super::now()).unwrap());
    let task = store.enqueue_task(&id, &spec).unwrap();
    store.claim_task_when(&task, |_| Ok(())).unwrap();
    let transfer = store.begin_phone_handoff(&id, device).unwrap();
    assert_eq!(transfer.mode, ExecutionMode::ToPhone);
    assert!(!store.task(&task.id).unwrap().enabled);
    assert!(matches!(
        store.prepare_phone_handoff(&id, &transfer.operation_id),
        Err(StoreError::Conflict)
    ));
    let independent = store.enqueue_task(&id, &TaskSpec::new("products", "* * * * * *")).unwrap();
    store.set_task_enabled(&independent.id, false).unwrap();
    store.set_task_enabled(&task.id, true).unwrap();
    store.set_task_enabled(&task.id, false).unwrap();
    assert!(store.run_task_now(&task.id).is_err());
    let mut auth = store.auth(&id).unwrap().unwrap();
    auth.access_token = "last-running-access".into();
    auth.refresh_token = Some("last-running-refresh".into());
    store
        .complete_task_with_policy(
            &id,
            &task.id,
            "completed",
            &json!({}),
            Some(&auth),
            None,
            TaskFinishPolicy::default(),
            |_| Ok(()),
        )
        .unwrap();
    let exported = store
        .prepare_phone_handoff(&id, &transfer.operation_id)
        .unwrap();
    assert_eq!(
        exported.request_auth().unwrap().access_token,
        "last-running-access"
    );
    assert!(
        store
            .complete_phone_handoff(&id, &transfer.operation_id, device, "wrong")
            .is_err()
    );
    assert_eq!(store.execution(&id).unwrap().mode, ExecutionMode::ToPhone);
    let retry = store.begin_phone_handoff(&id, device).unwrap();
    assert_eq!(retry.operation_id, transfer.operation_id);
    store
        .complete_phone_handoff(
            &id,
            &transfer.operation_id,
            device,
            &state_sha256(exported.archive()),
        )
        .unwrap();
    assert_eq!(store.execution(&id).unwrap().mode, ExecutionMode::Phone);
    assert!(
        store
            .import_app_state(&id, Some(device), &fixture_state())
            .is_err()
    );
    assert!(store.start_task(&id, "products").is_ok());
    assert!(store.begin_phone_handoff(&id, device).is_err());
    drop(store);
    let paths = StorePaths::in_dir(dir.path().into());
    assert_eq!(
        WorkspaceStore::open(&paths)
            .unwrap()
            .execution(&id)
            .unwrap()
            .mode,
        ExecutionMode::Phone
    );
}

#[test]
fn uncertain_refresh_cannot_be_exported_as_a_completed_phone_handoff() {
    let (_dir, mut store, id, device) = store_fixture();
    store
        .import_app_state(&id, Some(device), &fixture_state())
        .unwrap();
    let task = store
        .enqueue_task(
            &id,
            &TaskSpec::new("refresh", task_cron_at(super::now()).unwrap()),
        )
        .unwrap();
    store.claim_task_when(&task, |_| Ok(())).unwrap();
    let transfer = store.begin_phone_handoff(&id, device).unwrap();
    store.finish_task(&task.id, "unknown", &json!({})).unwrap();
    assert!(
        store
            .prepare_phone_handoff(&id, &transfer.operation_id)
            .is_err()
    );
    assert_eq!(store.execution(&id).unwrap().mode, ExecutionMode::ToPhone);
}

#[test]
fn rv_takeover_adopts_phone_tokens_and_preserves_stopped_task_schedules() {
    let (_dir, mut store, id, device) = store_fixture();
    store
        .import_app_state(&id, Some(device), &fixture_state())
        .unwrap();
    store
        .update_app_tokens(
            &id,
            "rv-access".into(),
            Some("rv-refresh".into()),
            Some(1_900_000_000),
        )
        .unwrap();
    let mut previous = store.auth(&id).unwrap().unwrap();
    previous
        .extra
        .insert("refresh_token_expires_at".into(), json!(2_000_000_000));
    store.save_auth(&id, &previous).unwrap();
    let task = store
        .enqueue_task(&id, &TaskSpec::new("products", "0 */20 * * * *"))
        .unwrap();
    store.set_token_source(&id, super::super::TokenSource::Rv).unwrap();
    let transfer = store.begin_phone_handoff(&id, device).unwrap();
    let exported = store
        .prepare_phone_handoff(&id, &transfer.operation_id)
        .unwrap();
    store
        .complete_phone_handoff(
            &id,
            &transfer.operation_id,
            device,
            &state_sha256(exported.archive()),
        )
        .unwrap();
    assert_eq!(store.token_source(&id).unwrap(), super::super::TokenSource::Phone(Some(device)));
    let (_phone_dir, mut phone, phone_id, phone_device) = store_fixture();
    phone
        .import_app_state(&phone_id, Some(phone_device), &fixture_state())
        .unwrap();
    phone
        .update_app_tokens(
            &phone_id,
            "phone-access".into(),
            Some("phone-refresh".into()),
            Some(2_050_000_000),
        )
        .unwrap();
    let source = phone.prepare_state_upload(&phone_id).unwrap();
    store
        .complete_rv_takeover(&id, device, &transfer.operation_id, &source)
        .unwrap();
    let auth = store.auth(&id).unwrap().unwrap();
    assert_eq!(auth.access_token, "phone-access");
    assert_eq!(auth.refresh_token.as_deref(), Some("phone-refresh"));
    assert_eq!(auth.expires_at, Some(2_050_000_000));
    assert_eq!(
        auth.token_expirations().refresh,
        None,
        "old RV refresh expiry must not survive phone credentials"
    );
    assert_eq!(store.execution(&id).unwrap().mode, ExecutionMode::Rv);
    assert_eq!(store.token_source(&id).unwrap(), super::super::TokenSource::Rv);
    let stopped = store.task(&task.id).unwrap();
    assert!(!stopped.enabled && stopped.next_run_at.is_none());
    assert_eq!(stopped.cron_expression, "0 */20 * * * *");
    store
        .import_app_state(&id, Some(device), &fixture_state())
        .unwrap();
    assert_eq!(
        store.auth(&id).unwrap().unwrap().access_token,
        "phone-access",
        "ordinary backup import still protects RV credentials"
    );
    assert!(
        store
            .complete_rv_takeover(&id, device, &transfer.operation_id, &source)
            .is_err(),
        "completed takeover cannot be replayed"
    );
}

#[test]
fn rv_takeover_failure_preserves_phone_owner_and_current_credentials() {
    let (_dir, mut store, id, device) = store_fixture();
    store
        .import_app_state(&id, Some(device), &fixture_state())
        .unwrap();
    let transfer = store.begin_phone_handoff(&id, device).unwrap();
    let state = store
        .prepare_phone_handoff(&id, &transfer.operation_id)
        .unwrap();
    store
        .complete_phone_handoff(
            &id,
            &transfer.operation_id,
            device,
            &state_sha256(state.archive()),
        )
        .unwrap();
    let mut invalid = fixture_state();
    let mut manifest = parse(invalid.manifest()).unwrap();
    manifest.as_dictionary_mut().unwrap().insert(
        "idfv".into(),
        string("A65D96D4-7BD2-4FFF-BF95-0CB2930CAB53"),
    );
    invalid.manifest = encode(&manifest).unwrap();
    assert!(
        store
            .complete_rv_takeover(&id, device, &transfer.operation_id, &invalid)
            .is_err()
    );
    assert_eq!(store.execution(&id).unwrap().mode, ExecutionMode::Phone);
    assert_eq!(store.token_source(&id).unwrap(), super::super::TokenSource::Phone(Some(device)));
    assert_eq!(
        store.auth(&id).unwrap().unwrap().access_token,
        "original-access"
    );
    assert!(
        store
            .enqueue_task(&id, &TaskSpec::new("products", "* * * * * *"))
            .is_ok()
    );
    assert!(
        store
            .complete_rv_takeover(&id, device, "old-operation", &state)
            .is_err()
    );
    assert_eq!(store.execution(&id).unwrap().mode, ExecutionMode::Phone);
    store
        .db
        .execute(
            "UPDATE app_execution SET device_id=NULL WHERE app_id=?1",
            [id.as_str()],
        )
        .unwrap();
    store.db.execute("UPDATE app_token_sources SET device_id=NULL WHERE app_id=?1", [id.as_str()]).unwrap();
    assert!(
        store
            .complete_rv_takeover(&id, device, &transfer.operation_id, &state)
            .is_err(),
        "missing phone owner cannot authorize a different source"
    );
    assert_eq!(store.execution(&id).unwrap().mode, ExecutionMode::Phone);
}

#[test]
fn inactive_phone_backup_takeover_adopts_its_own_tokens_and_wakes_only_token_waiters() {
    use super::super::TokenSource;
    let (_dir, mut store, id, device) = store_fixture();
    store.import_app_state(&id, Some(device), &fixture_state()).unwrap();
    // The environment is phone-sourced without an earlier RV -> phone handoff.
    assert_eq!(store.execution(&id).unwrap().mode, ExecutionMode::Rv);
    store.update_app_tokens(&id, "old-rv-access".into(), Some("old-rv-refresh".into()), Some(1)).unwrap();
    assert!(store.reject_access_token(&id, "old-rv-access").unwrap());
    let waiting = store.enqueue_task(&id, &TaskSpec::new("products", "0 */20 * * * *")).unwrap();
    let stopped = store.enqueue_task(&id, &TaskSpec::new("orders", "0 */30 * * * *")).unwrap();
    store.set_task_enabled(&stopped.id, false).unwrap();
    store.wait_for_access_token(&id).unwrap();
    let operation = store.execution(&id).unwrap().operation_id;
    store.sync_backups(device, BUNDLE, &json!({
        "bundle_id": BUNDLE,
        "current_backup_id": "snapshot:A65D96D4-7BD2-4FFF-BF95-0CB2930CAB53",
        "backups": [{"kind":"snapshot", "backup_id":format!("snapshot:{ID}"), "idfv":ID}]
    })).unwrap();
    store.complete_rv_takeover(&id, device, &operation, &fixture_state()).unwrap();
    let auth = store.auth(&id).unwrap().unwrap();
    assert_eq!(auth.access_token, "original-access");
    assert_eq!(auth.refresh_token.as_deref(), Some("original-refresh"));
    assert_eq!(store.token_source(&id).unwrap(), TokenSource::Rv);
    assert!(!store.auth(&id).unwrap().unwrap().access_token_rejected());
    let resumed = store.task(&waiting.id).unwrap();
    assert!(resumed.enabled);
    assert!(resumed.error_code.is_none());
    assert_eq!(resumed.cron_expression, "0 */20 * * * *");
    assert!(!store.task(&stopped.id).unwrap().enabled);
    assert!(store.complete_rv_takeover(&id, device, &operation, &fixture_state()).is_err());
}

#[test]
fn repeated_backup_catalog_refresh_does_not_replace_credentials_source_or_tasks() {
    use super::super::TokenSource;
    let (_dir, mut store, id, device) = store_fixture();
    store.import_app_state(&id, Some(device), &fixture_state()).unwrap();
    let operation = store.execution(&id).unwrap().operation_id;
    store.complete_rv_takeover(&id, device, &operation, &fixture_state()).unwrap();
    store.update_app_tokens(&id, "latest-rv-access".into(), Some("latest-rv-refresh".into()), Some(2_050_000_000)).unwrap();
    let task = store.enqueue_task(&id, &TaskSpec::new("products", "0 */20 * * * *")).unwrap();
    store.set_task_enabled(&task.id, false).unwrap();
    let state_hash = store.app_state_info(&id).unwrap().unwrap().archive_sha256;
    for source in [TokenSource::Rv, TokenSource::Phone(Some(device))] {
        store.set_token_source(&id, source).unwrap();
        for _ in 0..2 {
            store.sync_backups(device, BUNDLE, &json!({"bundle_id": BUNDLE,
                "backups":[{"kind":"snapshot", "backup_id":format!("snapshot:{ID}"), "idfv":ID,
                "display_name":"updated phone backup", "modified_at":2_060_000_000}]
            })).unwrap();
            let auth = store.auth(&id).unwrap().unwrap();
            assert_eq!(auth.access_token, "latest-rv-access");
            assert_eq!(auth.refresh_token.as_deref(), Some("latest-rv-refresh"));
            assert_eq!(store.token_source(&id).unwrap(), source);
            assert_eq!(store.app_state_info(&id).unwrap().unwrap().archive_sha256, state_hash);
            let unchanged = store.task(&task.id).unwrap();
            assert!(!unchanged.enabled);
            assert_eq!(unchanged.cron_expression, "0 */20 * * * *");
        }
    }
}

#[test]
fn failed_takeover_commit_rolls_back_credentials_source_state_and_waiter_changes() {
    use super::super::TokenSource;
    let (_dir, mut store, id, device) = store_fixture();
    store.import_app_state(&id, Some(device), &fixture_state()).unwrap();
    store.update_app_tokens(&id, "keep-access".into(), Some("keep-refresh".into()), Some(1)).unwrap();
    store.reject_access_token(&id, "keep-access").unwrap();
    let task = store.enqueue_task(&id, &TaskSpec::new("products", "0 */20 * * * *")).unwrap();
    store.wait_for_access_token(&id).unwrap();
    let before_task = store.task(&task.id).unwrap();
    let before_hash = store.app_state_info(&id).unwrap().unwrap().archive_sha256;
    // Force failure after importing state/auth and deleting the phone source.
    store.db.execute_batch("CREATE TEMP TRIGGER fail_takeover BEFORE INSERT ON app_execution BEGIN SELECT RAISE(ABORT,'fixture commit failure'); END;").unwrap();
    assert!(store.complete_rv_takeover(&id, device, "", &fixture_state()).is_err());
    let auth = store.auth(&id).unwrap().unwrap();
    assert_eq!(auth.access_token, "keep-access");
    assert_eq!(auth.refresh_token.as_deref(), Some("keep-refresh"));
    assert!(auth.access_token_rejected());
    assert_eq!(store.token_source(&id).unwrap(), TokenSource::Phone(Some(device)));
    assert_eq!(store.app_state_info(&id).unwrap().unwrap().archive_sha256, before_hash);
    assert_eq!(store.task(&task.id).unwrap(), before_task);
}

#[test]
fn takeover_requires_the_phone_backups_complete_token_pair() {
    let (_dir, mut store, id, device) = store_fixture();
    store.import_app_state(&id, Some(device), &fixture_state()).unwrap();
    store.update_app_tokens(&id, "keep-access".into(), Some("keep-refresh".into()), None).unwrap();
    let mut state = fixture_state();
    let mut keychain = parse(&state.payloads[0]).unwrap();
    keychain.as_array_mut().unwrap().retain(|row| text(row, "acct") != Some("refresh_token"));
    state.payloads[0] = encode(&keychain).unwrap();
    let (manifest, archive) = repack(&state.manifest, &state.payloads).unwrap();
    let state = ShadowAppState::decode(&id, BUNDLE, manifest, archive, state.device).unwrap();
    let error = store.complete_rv_takeover(&id, device, "", &state).unwrap_err();
    assert!(error.to_string().contains("缺少 refresh token"));
    assert_eq!(store.token_source(&id).unwrap(), super::super::TokenSource::Phone(Some(device)));
    let auth = store.auth(&id).unwrap().unwrap();
    assert_eq!(auth.access_token, "keep-access");
    assert_eq!(auth.refresh_token.as_deref(), Some("keep-refresh"));
}

#[test]
fn takeover_uses_current_phone_source_instead_of_legacy_phone_owner() {
    use super::super::TokenSource;
    let (dir, mut store, id, phone_a) = store_fixture();
    store.import_app_state(&id, Some(phone_a), &fixture_state()).unwrap();
    let transfer = store.begin_phone_handoff(&id, phone_a).unwrap();
    let exported = store.prepare_phone_handoff(&id, &transfer.operation_id).unwrap();
    store.complete_phone_handoff(&id, &transfer.operation_id, phone_a, &state_sha256(exported.archive())).unwrap();
    let mut book = AddressBook::load(StorePaths::in_dir(dir.path().into())).unwrap();
    let phone_b = Connection::new("phone B", "fixture-b", 5900);
    book.upsert(phone_b.clone());
    book.save().unwrap();
    store.sync_backups(phone_b.id, BUNDLE, &json!({"bundle_id":BUNDLE,
        "backups":[{"kind":"snapshot","backup_id":format!("snapshot:{ID}"),"idfv":ID}]
    })).unwrap();
    // A was the former handoff owner; B now supplies this environments tokens.
    store.set_token_source(&id, TokenSource::Phone(Some(phone_b.id))).unwrap();
    assert!(store.validate_rv_takeover(&id, phone_a, &transfer.operation_id).is_err());
    assert!(store.complete_rv_takeover(&id, phone_a, &transfer.operation_id, &fixture_state()).is_err());
    assert_eq!(store.token_source(&id).unwrap(), TokenSource::Phone(Some(phone_b.id)));
    store.complete_rv_takeover(&id, phone_b.id, &transfer.operation_id, &fixture_state()).unwrap();
    assert_eq!(store.token_source(&id).unwrap(), TokenSource::Rv);
    assert_eq!(store.execution(&id).unwrap().device_id, Some(phone_b.id));
}

#[test]
fn takeover_serializes_with_an_in_flight_phone_token_writer() {
    let (dir, mut store, id, device) = store_fixture();
    store.import_app_state(&id, Some(device), &fixture_state()).unwrap();
    let mut other = WorkspaceStore::open(&StorePaths::in_dir(dir.path().into())).unwrap();
    store.db.execute_batch("BEGIN IMMEDIATE").unwrap();
    store.db.execute("UPDATE app_instances SET note='concurrent phone update' WHERE id=?1", [id.as_str()]).unwrap();
    let ready = std::sync::Arc::new(std::sync::Barrier::new(2));
    let worker_ready=ready.clone();
    let worker_id=id.clone();
    let worker=std::thread::spawn(move || {
        worker_ready.wait();
        other.complete_rv_takeover(&worker_id, device, "", &fixture_state())
    });
    ready.wait();
    std::thread::sleep(std::time::Duration::from_millis(100));
    store.db.execute_batch("COMMIT").unwrap();
    worker.join().unwrap().unwrap();
    assert_eq!(store.token_source(&id).unwrap(), super::super::TokenSource::Rv);
}

#[test]
fn token_rotation_uses_returned_scope_in_both_preference_hosts() {
    let (_dir, mut store, id, source) = store_fixture();
    let mut original = fixture_state();
    let mut preferences = parse(&original.payloads[2]).unwrap();
    let rows = preferences
        .as_dictionary_mut()
        .unwrap()
        .get_mut("records")
        .unwrap()
        .as_array_mut()
        .unwrap();
    let mut current = rows[0].clone();
    set(&mut current, "host", "current").unwrap();
    rows.push(current);
    let mut unrelated = rows[0].clone();
    set(&mut unrelated, "domain", "unrelated.domain").unwrap();
    rows.push(unrelated);
    original.payloads[2] = encode(&preferences).unwrap();
    (original.manifest, original.archive) = repack(&original.manifest, &original.payloads).unwrap();
    store
        .import_app_state(&id, Some(source), &original)
        .unwrap();
    let mut auth = store.auth(&id).unwrap().unwrap();
    auth.access_token = "new-access".into();
    auth.expires_at = Some(2_000_000_000);
    auth.extra
        .insert("scope".into(), json!("returned-server-scope"));
    store.save_auth(&id, &auth).unwrap();
    let state = store.prepare_state_upload(&id).unwrap();
    let state =
        ShadowAppState::decode(&id, BUNDLE, state.manifest, state.archive, state.device).unwrap();
    let prefs = parse(state.payload("preferences/records.plist").unwrap()).unwrap();
    let rows = at(&prefs, "records").unwrap().as_array().unwrap();
    for row in &rows[..2] {
        let values = at(row, "values").unwrap();
        assert_eq!(text(values, "token_scope"), Some("returned-server-scope"));
        let expiry = at(values, "token_expiration").unwrap().as_date().unwrap();
        assert_eq!(
            std::time::SystemTime::from(expiry)
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            2_000_000_000
        );
        assert_eq!(
            at(values, "unknown").unwrap().as_data(),
            Some(&[4, 5, 6][..])
        );
    }
    assert_eq!(
        text(at(&rows[2], "values").unwrap(), "token_scope"),
        Some("user")
    );
    auth.extra.remove("scope");
    store.save_auth(&id, &auth).unwrap();
    let state = store.prepare_state_upload(&id).unwrap();
    let prefs = parse(state.payload("preferences/records.plist").unwrap()).unwrap();
    for row in &at(&prefs, "records").unwrap().as_array().unwrap()[..2] {
        assert_eq!(
            text(at(row, "values").unwrap(), "token_scope"),
            Some("returned-server-scope")
        );
    }
}

#[test]
fn access_rejection_is_shared_notifies_and_new_credentials_resume_requests() {
    let (_dir,mut store,id,phone)=store_fixture();
    store.import_app_state(&id,Some(phone),&fixture_state()).unwrap();
    let bus=store.events.clone();
    let changes=bus.subscribe::<crate::events::WorkspaceChanged>();
    let original=store.auth(&id).unwrap().unwrap();
    assert!(store.reject_access_token(&id,&original.access_token).unwrap());
    assert!(WorkspaceStore::require_access_token(&store.db,id.as_str()).is_err());
    assert!(changes.try_recv().unwrap().changes.iter().any(|c|c.domain==crate::events::ChangeDomain::Credentials && c.app_id.as_deref()==Some(id.as_str())));
    // A header-only save does not make the rejected token valid again.
    store.save_auth(&id,&original).unwrap();
    assert!(store.auth(&id).unwrap().unwrap().access_token_rejected());
    let mut replacement=store.auth(&id).unwrap().unwrap();
    replacement.access_token="replacement-access".into();
    store.save_auth(&id,&replacement).unwrap();
    assert!(!store.auth(&id).unwrap().unwrap().access_token_rejected());
    WorkspaceStore::require_access_token(&store.db,id.as_str()).unwrap();
    // A late failure from an old request cannot reject the replacement token.
    assert!(!store.reject_access_token(&id,&original.access_token).unwrap());
    assert!(!store.auth(&id).unwrap().unwrap().access_token_rejected());
}

#[test]
fn concurrent_same_token_save_cannot_erase_a_rejection() {
    let (_dir,mut store,id,phone)=store_fixture();
    store.import_app_state(&id,Some(phone),&fixture_state()).unwrap();
    let path:String=store.db.query_row("SELECT file FROM pragma_database_list WHERE name='main'",[],|r|r.get(0)).unwrap();
    let paths=crate::StorePaths::in_dir(std::path::Path::new(&path).parent().unwrap().to_path_buf());
    let original=store.auth(&id).unwrap().unwrap();
    let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));
    let b=barrier.clone();let id2=id.clone();let old=original.clone();
    let save=std::thread::spawn(move || {
        let mut writer=WorkspaceStore::open(&paths).unwrap();b.wait();
        for _ in 0..10 {writer.save_auth(&id2,&old).unwrap();}
    });
    barrier.wait();store.reject_access_token(&id,&original.access_token).unwrap();
    save.join().unwrap();
    assert!(store.auth(&id).unwrap().unwrap().access_token_rejected());
}

#[test]
fn session_id_is_plain_token_bound_and_survives_restart_and_token_reuse() {
    let (dir,mut store,id,phone)=store_fixture();store.import_app_state(&id,Some(phone),&fixture_state()).unwrap();
    let paths=crate::StorePaths::in_dir(dir.path().to_path_buf());
    let mut auth=store.auth(&id).unwrap().unwrap();let first=auth.headers["X-Session-Id"].clone();
    assert_eq!(uuid::Uuid::parse_str(&first).unwrap().get_version_num(),4);
    let plain:String=store.db.query_row("SELECT session_id FROM app_request_sessions WHERE app_id=?1",[id.as_str()],|r|r.get(0)).unwrap();assert_eq!(plain,first);
    let blob:Vec<u8>=store.db.query_row("SELECT encrypted FROM app_auth WHERE app_id=?1",[id.as_str()],|r|r.get(0)).unwrap();
    let encrypted_auth:AppAuth=serde_json::from_slice(&crate::store::credentials::unseal(&format!("rv-app-auth|{}|{BUNDLE}",id.as_str()),blob).unwrap()).unwrap();
    assert!(!encrypted_auth.headers.contains_key("X-Session-Id"));
    // Caller-supplied session headers cannot change this token's saved binding.
    auth.headers.insert("X-Session-Id".into(),uuid::Uuid::new_v4().to_string());store.save_auth(&id,&auth).unwrap();
    assert_eq!(store.auth(&id).unwrap().unwrap().headers["X-Session-Id"],first);
    drop(store);let mut store=WorkspaceStore::open(&paths).unwrap();assert_eq!(store.request_auth(&id).unwrap().unwrap().headers["X-Session-Id"],first);
    let original=auth.access_token.clone();auth.access_token="rotated-token".into();store.save_auth(&id,&auth).unwrap();
    assert_ne!(store.auth(&id).unwrap().unwrap().headers["X-Session-Id"],first);
    auth.access_token=original;store.save_auth(&id,&auth).unwrap();assert_eq!(store.auth(&id).unwrap().unwrap().headers["X-Session-Id"],first);
}
#[test]
fn legacy_session_initialization_is_concurrent_and_does_not_rewrite_credentials_or_state() {
    let (dir,mut store,id,phone)=store_fixture();store.import_app_state(&id,Some(phone),&fixture_state()).unwrap();
    let state_before:String=store.db.query_row("SELECT archive_sha256 FROM app_states WHERE app_id=?1",[id.as_str()],|r|r.get(0)).unwrap();
    store.business_transaction(|db|{db.execute_batch("DROP TABLE app_request_sessions; PRAGMA user_version=12;")?;Ok(())}).unwrap();
    let blob:Vec<u8>=store.db.query_row("SELECT encrypted FROM app_auth WHERE app_id=?1",[id.as_str()],|r|r.get(0)).unwrap();
    let paths=crate::StorePaths::in_dir(dir.path().to_path_buf());let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut workers=Vec::new();
    for _ in 0..2 {let paths=paths.clone();let id=id.clone();let b=barrier.clone();workers.push(std::thread::spawn(move||{let mut s=WorkspaceStore::open(&paths).unwrap();b.wait();s.request_auth(&id).unwrap().unwrap().headers["X-Session-Id"].clone()}));}
    let a=workers.remove(0).join().unwrap();let b=workers.remove(0).join().unwrap();assert_eq!(a,b);
    let after:Vec<u8>=store.db.query_row("SELECT encrypted FROM app_auth WHERE app_id=?1",[id.as_str()],|r|r.get(0)).unwrap();assert_eq!(after,blob);
    let state_after:String=store.db.query_row("SELECT archive_sha256 FROM app_states WHERE app_id=?1",[id.as_str()],|r|r.get(0)).unwrap();assert_eq!(state_before,state_after);
    assert_eq!(store.db.query_row("PRAGMA user_version",[],|r|r.get::<_,i64>(0)).unwrap(),14);
}

#[test]
fn attached_session_does_not_reduce_the_existing_saved_header_limit() {
    let (_dir,mut store,id,phone)=store_fixture();store.import_app_state(&id,Some(phone),&fixture_state()).unwrap();
    let mut auth=store.auth(&id).unwrap().unwrap();auth.headers.clear();
    for index in 0..32 {auth.headers.insert(format!("X-Fixture-{index}"),"value".into());}
    store.save_auth(&id,&auth).unwrap();
    let loaded=store.auth(&id).unwrap().unwrap();assert_eq!(loaded.headers.len(),33);
    store.save_auth(&id,&loaded).unwrap();
    assert_eq!(store.auth(&id).unwrap().unwrap().headers["X-Session-Id"],loaded.headers["X-Session-Id"]);
}

#[test]
fn first_phone_state_import_uses_its_token_and_phone_source_without_requesting_a_replacement() {
    let (dir,mut store,id,device)=store_fixture();
    let bus=crate::events::workspace_bus(&StorePaths::in_dir(dir.path().into())).unwrap();
    let reads=bus.subscribe::<crate::events::PhoneTokenRequested>();
    let state=fixture_state();let imported=state.request_auth().unwrap();
    store.import_app_state(&id,Some(device),&state).unwrap();
    assert_eq!(store.token_source(&id).unwrap(),super::super::TokenSource::Phone(Some(device)));
    assert_eq!(store.auth(&id).unwrap().unwrap().access_token,imported.access_token);
    assert!(reads.try_recv().is_err(),"import does not force a new live-token read");
    store.set_token_source(&id,super::super::TokenSource::Rv).unwrap();
    store.import_app_state(&id,Some(device),&state).unwrap();
    assert_eq!(store.token_source(&id).unwrap(),super::super::TokenSource::Rv,"later backups do not reset the chosen source");
}
