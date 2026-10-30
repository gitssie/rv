use super::*;
use crate::{AddressBook, Connection, StorePaths, workspace::AppAuth};
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
    json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"backup_id":"snapshot:1","kind":"snapshot","idfv":id}]})
}

#[test]
fn login_credentials_survive_profile_token_and_backup_updates_and_reopen() {
    let (mut store, device, dir) = fixture();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    let id = AppId::from_idfv(ID).unwrap();
    assert!(store.login_credentials(&id).unwrap().is_none());
    store
        .commit_business_data(&id, &json!({"login":"shop.name"}), |_| Ok(()))
        .unwrap();
    assert_eq!(
        store.login_credentials(&id).unwrap(),
        Some(AppLoginCredentials {
            username: "shop.name".into(),
            password: None
        })
    );
    let credentials = AppLoginCredentials {
        username: "shop.name".into(),
        password: Some(" known-password ".into()),
    };
    store.save_login_credentials(&id, &credentials).unwrap();
    assert!(!format!("{credentials:?}").contains("known-password"));
    store
        .commit_business_data(&id, &json!({"login":"new.shop.name"}), |_| Ok(()))
        .unwrap();
    store
        .commit_business_data(&id, &json!({"login":null,"unread_count":4}), |_| Ok(()))
        .unwrap();
    store
        .save_auth(
            &id,
            &AppAuth {
                access_token: "fixture-new-token".into(),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    drop(store);
    let paths = StorePaths::in_dir(dir.path().into());
    let mut store = WorkspaceStore::open(&paths).unwrap();
    assert_eq!(
        store.login_credentials(&id).unwrap(),
        Some(AppLoginCredentials {
            username: "new.shop.name".into(),
            password: credentials.password
        })
    );
    assert_eq!(
        store.auth(&id).unwrap().unwrap().access_token,
        "fixture-new-token"
    );
    assert!(!store.load(&id).unwrap().metadata.contains("known-password"));
    store.delete_app(&id).unwrap();
    assert!(store.login_credentials(&id).unwrap().is_none());
}

#[test]
fn login_credentials_are_scoped_and_invalid_updates_do_not_overwrite_saved_data() {
    let (mut store, device, _dir) = fixture();
    let first = AppId::from_idfv(ID).unwrap();
    let second = AppId::from_idfv("B9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
    for id in [&first, &second] {
        store
            .sync_backups(device.id, "lt.manodrabuziai.fr", &result(id.as_str()))
            .unwrap();
    }
    let a = AppLoginCredentials {
        username: "first".into(),
        password: Some("password-one".into()),
    };
    let b = AppLoginCredentials {
        username: "second".into(),
        password: Some("password-two".into()),
    };
    store.save_login_credentials(&first, &a).unwrap();
    store.save_login_credentials(&second, &b).unwrap();
    for invalid in [
        AppLoginCredentials {
            username: " ".into(),
            password: None,
        },
        AppLoginCredentials {
            username: "first".into(),
            password: Some(String::new()),
        },
    ] {
        assert!(store.save_login_credentials(&first, &invalid).is_err());
    }
    assert!(
        store
            .commit_business_data(
                &first,
                &json!({"login":"bad\nname","unread_count":9}),
                |_| Ok(())
            )
            .is_err()
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(&store.load(&first).unwrap().metadata)
            .unwrap()
            .get("unread_count")
            .is_none()
    );
    assert_eq!(store.login_credentials(&first).unwrap(), Some(a));
    assert_eq!(store.login_credentials(&second).unwrap(), Some(b));
}

#[test]
fn login_credentials_migration_backfills_names_without_guessing_passwords() {
    let (mut store, device, dir) = fixture();
    let id = AppId::from_idfv(ID).unwrap();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(ID))
        .unwrap();
    let mut app = store.load(&id).unwrap();
    app.metadata = json!({"login":"existing.shop","shop":{"login":"fallback"}}).to_string();
    store.save(&app).unwrap();
    let other = AppId::from_idfv("B9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
    store
        .sync_backups(device.id, "lt.manodrabuziai.fr", &result(other.as_str()))
        .unwrap();
    let mut other_app = store.load(&other).unwrap();
    other_app.metadata = json!({"shop":{"login":"nested.shop"}}).to_string();
    store.save(&other_app).unwrap();
    store
        .db
        .execute_batch("DROP TABLE app_login_credentials; PRAGMA user_version=14;")
        .unwrap();
    drop(store);
    let paths = StorePaths::in_dir(dir.path().into());
    let store = WorkspaceStore::open(&paths).unwrap();
    assert_eq!(
        store.login_credentials(&id).unwrap(),
        Some(AppLoginCredentials {
            username: "existing.shop".into(),
            password: None
        })
    );
    assert_eq!(
        store.login_credentials(&other).unwrap(),
        Some(AppLoginCredentials {
            username: "nested.shop".into(),
            password: None
        })
    );
    assert_eq!(store.load(&id).unwrap().metadata, app.metadata);
    assert_eq!(
        store
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        15
    );
    drop(store);
    let store = WorkspaceStore::open(&paths).unwrap();
    assert_eq!(
        store.login_credentials(&id).unwrap().unwrap().username,
        "existing.shop"
    );
}

#[test]
fn login_editor_save_is_atomic_and_password_is_plain_text() {
    let (mut store,device,_dir)=fixture();
    let id=AppId::from_idfv(ID).unwrap();
    store.sync_backups(device.id,"lt.manodrabuziai.fr",&result(ID)).unwrap();
    let original=store.load(&id).unwrap();
    let mut edited=original.clone();edited.note="edited note".into();
    let credentials=AppLoginCredentials{username:"fixture.shop".into(),password:Some("plain-visible-password".into())};
    assert!(store.save_with_proxy_and_login(&edited,Some("missing"),Some(&credentials)).is_err());
    assert_eq!(store.load(&id).unwrap().note,original.note);
    assert!(store.login_credentials(&id).unwrap().is_none());
    let invalid=AppLoginCredentials{username:String::new(),password:Some("never-saved".into())};
    assert!(store.save_with_proxy_and_login(&edited,None,Some(&invalid)).is_err());
    assert_eq!(store.load(&id).unwrap().note,original.note);
    assert!(store.login_credentials(&id).unwrap().is_none());
    let saved=store.save_with_proxy_and_login(&edited,None,Some(&credentials)).unwrap();
    assert_eq!(saved.note,"edited note");
    let plain:(String,String)=store.db.query_row("SELECT typeof(login_password),login_password FROM app_login_credentials WHERE app_id=?1",[id.as_str()],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(plain,("text".into(),"plain-visible-password".into()));
    store.save_with_proxy_and_login(&saved,None,Some(&AppLoginCredentials{username:"fixture.shop".into(),password:None})).unwrap();
    assert!(store.login_credentials(&id).unwrap().unwrap().password.is_none());
}

#[test]
fn login_editor_never_restores_stale_read_only_username_after_profile_sync() {
    let (mut store,device,_dir)=fixture();
    let id=AppId::from_idfv(ID).unwrap();
    store.sync_backups(device.id,"lt.manodrabuziai.fr",&result(ID)).unwrap();
    store.commit_business_data(&id,&json!({"login":"old.name"}),|_|Ok(())).unwrap();
    let mut stale_input=store.login_credentials(&id).unwrap().unwrap();
    store.commit_business_data(&id,&json!({"login":"new.name"}),|_|Ok(())).unwrap();
    // The UI refreshes its editing revision, while its read-only input stays stale.
    let mut edited=store.load(&id).unwrap();edited.note="local edit".into();
    stale_input.password=Some("visible-password".into());
    store.save_with_proxy_and_login(&edited,None,Some(&stale_input)).unwrap();
    assert_eq!(store.login_credentials(&id).unwrap(),Some(AppLoginCredentials{username:"new.name".into(),password:Some("visible-password".into())}));
}
