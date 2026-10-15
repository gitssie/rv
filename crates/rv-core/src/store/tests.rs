use super::*;
use crate::{ConnectRequest, ThemePref, UnlockCode};
fn book() -> (AddressBook, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (
        AddressBook::load(StorePaths::in_dir(dir.path().into())).unwrap(),
        dir,
    )
}
#[test]
fn device_and_preference_roundtrip_ignore_legacy_files() {
    let (mut book, dir) = book();
    fs::write(dir.path().join("addressbook.json"), b"{broken old data}").unwrap();
    fs::write(dir.path().join("prefs.json"), b"{broken old data}").unwrap();
    let mut device = Connection::new("phone", "10.8.0.7", 5901);
    device.guid = Some("a-real-device-guid".into());
    device.model = Some("iPhone X".into());
    device.os_version = Some("16.7.10".into());
    device.labels = vec!["工作".into()];
    book.upsert(device.clone());
    book.prefs_mut().theme = ThemePref::Dark;
    book.save().unwrap();
    let loaded = AddressBook::load(book.paths.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&loaded.connections()[0]).unwrap(),
        serde_json::to_value(&device).unwrap()
    );
    assert_eq!(loaded.prefs().theme, ThemePref::Dark);
    assert!(dir.path().join("addressbook.json").exists());
    assert!(
        fs::read_dir(dir.path()).unwrap().all(|v| !v
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with("broken"))
    );
}
#[test]
fn true_guid_is_unique_but_not_primary_key_and_unknown_devices_can_coexist() {
    let (mut book, _) = book();
    let mut a = Connection::new("one", "a", 5900);
    a.guid = Some("real-guid".into());
    let mut b = Connection::new("two", "b", 5900);
    b.guid = a.guid.clone();
    book.upsert(a.clone());
    book.upsert(b.clone());
    assert!(book.save().is_err());
    assert!(
        AddressBook::load(book.paths.clone())
            .unwrap()
            .connections()
            .is_empty()
    );
    b.guid = None;
    book.upsert(b);
    book.save().unwrap();
    assert_ne!(a.id.to_string(), a.guid.unwrap());
}
#[test]
fn credentials_are_encrypted_bound_to_id_and_do_not_read_legacy_files() {
    let (book, dir) = book();
    let id = ConnectionId::new();
    let other = ConnectionId::new();
    fs::create_dir_all(dir.path().join("passwords")).unwrap();
    fs::write(
        dir.path().join("passwords").join(format!("{id}.bin")),
        b"old password",
    )
    .unwrap();
    assert!(book.paths.load_password(id).unwrap().is_none());
    book.paths.save_password(id, "a secret 密码").unwrap();
    let db = crate::database::open(&book.paths.database).unwrap();
    let first: Vec<u8> = db
        .query_row(
            "SELECT encrypted FROM credentials WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !first
            .windows("a secret 密码".len())
            .any(|w| w == "a secret 密码".as_bytes())
    );
    book.paths.save_password(id, "a secret 密码").unwrap();
    let second: Vec<u8> = db
        .query_row(
            "SELECT encrypted FROM credentials WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(first, second);
    db.execute(
        "INSERT INTO credentials VALUES(?1,?2)",
        rusqlite::params![other.to_string(), second],
    )
    .unwrap();
    assert!(book.paths.load_password(other).is_err());
    let mut corrupt = second;
    *corrupt.last_mut().unwrap() ^= 1;
    db.execute(
        "UPDATE credentials SET encrypted=?2 WHERE id=?1",
        rusqlite::params![id.to_string(), corrupt],
    )
    .unwrap();
    assert!(book.paths.load_password(id).is_err());
}
#[test]
fn password_and_unlock_clear_independently_and_device_delete_clears_both() {
    let (mut book, _) = book();
    let device = Connection::new("phone", "a", 5900);
    let id = device.id;
    let mut req = ConnectRequest::from_connection(&device, None);
    book.upsert(device);
    book.save().unwrap();
    book.paths.save_password(id, "vnc").unwrap();
    book.paths
        .save_unlock_code(&req, &UnlockCode::new("001234".into()).unwrap())
        .unwrap();
    req.host = "b".into();
    assert!(book.paths.load_unlock_code(&req).unwrap().is_none());
    req.host = "a".into();
    book.paths.delete_password(id).unwrap();
    assert!(book.paths.load_unlock_code(&req).unwrap().is_some());
    book.paths.save_password(id, "again").unwrap();
    book.paths.delete_unlock_code(&req).unwrap();
    assert_eq!(
        book.paths.load_password(id).unwrap().as_deref(),
        Some("again")
    );
    book.remove(id).unwrap();
    assert!(book.paths.load_password(id).unwrap().is_none());
    assert!(
        AddressBook::load(book.paths.clone())
            .unwrap()
            .connections()
            .is_empty()
    );
}
#[test]
fn concurrent_credential_updates_preserve_both_fields() {
    let (book, _) = book();
    let device = Connection::new("phone", "a", 5900);
    let id = device.id;
    let req = ConnectRequest::from_connection(&device, None);
    let paths = book.paths.clone();
    let writer = std::thread::spawn(move || {
        for _ in 0..8 {
            paths
                .save_unlock_code(&req, &UnlockCode::new("012345".into()).unwrap())
                .unwrap();
        }
    });
    for _ in 0..8 {
        book.paths.save_password(id, "vnc").unwrap();
    }
    writer.join().unwrap();
    assert_eq!(
        book.paths.load_password(id).unwrap().as_deref(),
        Some("vnc")
    );
    assert_eq!(
        book.paths
            .load_unlock_code(&ConnectRequest::from_connection(&device, None))
            .unwrap()
            .unwrap()
            .digits(),
        b"012345"
    );
}
#[test]
fn ad_hoc_credentials_are_endpoint_scoped_in_database() {
    let (book, _) = book();
    let mut req = ConnectRequest::from_connection(&Connection::new("a", "a", 5900), None);
    req.connection_id = None;
    book.paths
        .save_unlock_code(&req, &UnlockCode::new("012345".into()).unwrap())
        .unwrap();
    req.port = 5901;
    assert!(book.paths.load_unlock_code(&req).unwrap().is_none());
    req.port = 5900;
    book.paths.delete_unlock_code(&req).unwrap();
    assert!(book.paths.load_unlock_code(&req).unwrap().is_none());
}
