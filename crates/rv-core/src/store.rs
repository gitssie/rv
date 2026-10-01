use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ConnectRequest, Connection, ConnectionId, Preferences, UnlockCode};
use std::sync::Mutex;

const PASSWORD_FILE_VERSION: &[u8; 4] = b"RVP2";
const LEGACY_PASSWORD_FILE_VERSION: &[u8; 4] = b"RVP1";
static CREDENTIAL_IO: Mutex<()> = Mutex::new(());

#[derive(Default, Serialize, Deserialize)]
struct ConnectionCredentials {
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    unlock: Option<SavedUnlock>,
}

#[derive(Serialize, Deserialize)]
struct SavedUnlock {
    host: String,
    port: u16,
    code: String,
}
const PASSWORD_KEY: [u8; 32] = [
    0x7a, 0xf7, 0x4d, 0x61, 0x24, 0xa6, 0x38, 0x77, 0xc4, 0x15, 0x3b, 0x9e, 0x1a, 0x05, 0xb0, 0xb2,
    0x68, 0x22, 0x16, 0x61, 0x3b, 0x8c, 0x19, 0xe5, 0x30, 0xe8, 0x34, 0x79, 0xfa, 0x72, 0x44, 0x06,
];

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("saved password error: {0}")]
    Credential(String),
    #[error("unknown connection")]
    UnknownConnection,
}

#[derive(Debug, Clone)]
pub struct StorePaths {
    pub root: PathBuf,
    pub address_book: PathBuf,
    pub prefs: PathBuf,
    pub thumbs: PathBuf,
}

impl StorePaths {
    /// Platform data directory, or `$RV_DATA_DIR` when set (handy for
    /// testing against a scratch address book).
    pub fn default_dir() -> Result<Self, StoreError> {
        if let Some(dir) = std::env::var_os("RV_DATA_DIR").filter(|d| !d.is_empty()) {
            return Ok(Self::in_dir(PathBuf::from(dir)));
        }
        let dirs = directories::ProjectDirs::from("app", "RV", "rv").ok_or_else(|| {
            StoreError::Io(std::io::Error::other("cannot resolve application data dir"))
        })?;
        Ok(Self::in_dir(dirs.data_dir().to_path_buf()))
    }

    pub fn in_dir(root: PathBuf) -> Self {
        Self {
            address_book: root.join("addressbook.json"),
            prefs: root.join("prefs.json"),
            thumbs: root.join("thumbs"),
            root,
        }
    }

    pub fn ensure(&self) -> Result<(), StoreError> {
        fs::create_dir_all(&self.root)?;
        fs::create_dir_all(&self.thumbs)?;
        Ok(())
    }

    pub fn thumb_path(&self, id: ConnectionId) -> PathBuf {
        self.thumbs.join(format!("{id}.png"))
    }

    fn password_path(&self, id: ConnectionId) -> PathBuf {
        self.root.join("passwords").join(format!("{id}.bin"))
    }

    pub fn save_password(&self, id: ConnectionId, password: &str) -> Result<(), StoreError> {
        let _guard = CREDENTIAL_IO.lock().unwrap_or_else(|e| e.into_inner());
        let mut credentials = self.load_credentials(id)?;
        credentials.password = Some(password.to_owned());
        self.save_credentials(id, &credentials)
    }

    fn save_credentials(
        &self,
        id: ConnectionId,
        credentials: &ConnectionCredentials,
    ) -> Result<(), StoreError> {
        if credentials.password.is_none() && credentials.unlock.is_none() {
            return self.delete_credentials(id);
        }
        let key = password_cipher()?;
        let mut nonce = [0; 12];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| StoreError::Credential("cannot generate nonce".into()))?;
        let mut encrypted = serde_json::to_vec(credentials)?;
        key.seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(id.to_string().as_bytes()),
            &mut encrypted,
        )
        .map_err(|_| StoreError::Credential("cannot encrypt password".into()))?;
        let mut data = Vec::with_capacity(4 + nonce.len() + encrypted.len());
        data.extend_from_slice(PASSWORD_FILE_VERSION);
        data.extend_from_slice(&nonce);
        data.extend_from_slice(&encrypted);
        let path = self.password_path(id);
        fs::create_dir_all(path.parent().expect("password file has a parent"))?;
        atomic_write(&path, &data)
    }

    pub fn load_password(&self, id: ConnectionId) -> Result<Option<String>, StoreError> {
        let _guard = CREDENTIAL_IO.lock().unwrap_or_else(|e| e.into_inner());
        Ok(self.load_credentials(id)?.password)
    }

    fn load_credentials(&self, id: ConnectionId) -> Result<ConnectionCredentials, StoreError> {
        let data = match fs::read(self.password_path(id)) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ConnectionCredentials::default());
            }
            Err(error) => return Err(error.into()),
        };
        if data.len() < 4 + 12 + aead::AES_256_GCM.tag_len()
            || (&data[..4] != PASSWORD_FILE_VERSION && &data[..4] != LEGACY_PASSWORD_FILE_VERSION)
        {
            return Err(StoreError::Credential("invalid password file".into()));
        }
        let nonce: [u8; 12] = data[4..16].try_into().expect("validated nonce length");
        let mut encrypted = data[16..].to_vec();
        let plaintext = password_cipher()?
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(id.to_string().as_bytes()),
                &mut encrypted,
            )
            .map_err(|_| StoreError::Credential("password authentication failed".into()))?;
        if &data[..4] == LEGACY_PASSWORD_FILE_VERSION {
            let password = String::from_utf8(plaintext.to_vec())
                .map_err(|_| StoreError::Credential("saved password is not UTF-8".into()))?;
            Ok(ConnectionCredentials {
                password: Some(password),
                unlock: None,
            })
        } else {
            Ok(serde_json::from_slice(plaintext)?)
        }
    }

    pub fn delete_password(&self, id: ConnectionId) -> Result<(), StoreError> {
        let _guard = CREDENTIAL_IO.lock().unwrap_or_else(|e| e.into_inner());
        let mut credentials = self.load_credentials(id)?;
        credentials.password = None;
        self.save_credentials(id, &credentials)
    }

    pub fn load_unlock_code(&self, req: &ConnectRequest) -> Result<Option<UnlockCode>, StoreError> {
        let _guard = CREDENTIAL_IO.lock().unwrap_or_else(|e| e.into_inner());
        let saved = self.load_credentials(credential_id(req))?.unlock;
        match saved {
            Some(saved) if saved.host == req.host && saved.port == req.port => {
                UnlockCode::new(saved.code).map(Some)
            }
            _ => Ok(None),
        }
    }

    pub fn save_unlock_code(
        &self,
        req: &ConnectRequest,
        code: &UnlockCode,
    ) -> Result<(), StoreError> {
        let _guard = CREDENTIAL_IO.lock().unwrap_or_else(|e| e.into_inner());
        let id = credential_id(req);
        let mut credentials = self.load_credentials(id)?;
        credentials.unlock = Some(SavedUnlock {
            host: req.host.clone(),
            port: req.port,
            code: std::str::from_utf8(code.digits())
                .expect("validated ASCII digits")
                .to_owned(),
        });
        self.save_credentials(id, &credentials)
    }

    pub fn delete_unlock_code(&self, req: &ConnectRequest) -> Result<(), StoreError> {
        let _guard = CREDENTIAL_IO.lock().unwrap_or_else(|e| e.into_inner());
        let id = credential_id(req);
        let mut credentials = self.load_credentials(id)?;
        credentials.unlock = None;
        self.save_credentials(id, &credentials)
    }

    fn delete_credentials(&self, id: ConnectionId) -> Result<(), StoreError> {
        match fs::remove_file(self.password_path(id)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

// Ad-hoc targets share the same credential store, identified by their endpoint.
fn credential_id(req: &ConnectRequest) -> ConnectionId {
    req.connection_id.unwrap_or_else(|| {
        let endpoint = format!("rv-ad-hoc|{}:{}", req.host, req.port);
        let digest = ring::digest::digest(&ring::digest::SHA256, endpoint.as_bytes());
        let bytes: [u8; 16] = digest.as_ref()[..16].try_into().unwrap();
        ConnectionId(uuid::Uuid::from_bytes(bytes))
    })
}

fn password_cipher() -> Result<aead::LessSafeKey, StoreError> {
    aead::UnboundKey::new(&aead::AES_256_GCM, &PASSWORD_KEY)
        .map(aead::LessSafeKey::new)
        .map_err(|_| StoreError::Credential("invalid encryption key".into()))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AddressBookFile {
    #[serde(default)]
    connections: Vec<Connection>,
}

#[derive(Debug, Clone)]
pub struct AddressBook {
    paths: StorePaths,
    connections: Vec<Connection>,
    prefs: Preferences,
}

impl AddressBook {
    /// Load strictly: any unreadable file is an error.
    pub fn load(paths: StorePaths) -> Result<Self, StoreError> {
        paths.ensure()?;
        let file: AddressBookFile = read_json(&paths.address_book)?.unwrap_or_default();
        let prefs = read_json(&paths.prefs)?.unwrap_or_default();
        Ok(Self {
            paths,
            connections: file.connections,
            prefs,
        })
    }

    /// Load, but survive corrupt JSON: the bad file is moved aside as
    /// `<name>.broken` so nothing is lost, and a warning describes what
    /// happened. I/O errors (unreadable directory) still fail.
    pub fn load_or_quarantine(paths: StorePaths) -> Result<(Self, Vec<String>), StoreError> {
        paths.ensure()?;
        let mut warnings = Vec::new();
        let file: AddressBookFile = read_json_or_quarantine(&paths.address_book, &mut warnings)?;
        let prefs = read_json_or_quarantine(&paths.prefs, &mut warnings)?;
        Ok((
            Self {
                paths,
                connections: file.connections,
                prefs,
            },
            warnings,
        ))
    }

    pub fn paths(&self) -> &StorePaths {
        &self.paths
    }

    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    pub fn prefs(&self) -> &Preferences {
        &self.prefs
    }

    pub fn prefs_mut(&mut self) -> &mut Preferences {
        &mut self.prefs
    }

    pub fn get(&self, id: ConnectionId) -> Option<&Connection> {
        self.connections.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: ConnectionId) -> Option<&mut Connection> {
        self.connections.iter_mut().find(|c| c.id == id)
    }

    pub fn upsert(&mut self, conn: Connection) {
        if let Some(existing) = self.connections.iter_mut().find(|c| c.id == conn.id) {
            *existing = conn;
        } else {
            self.connections.push(conn);
        }
    }

    pub fn remove(&mut self, id: ConnectionId) -> Result<(), StoreError> {
        if !self.connections.iter().any(|c| c.id == id) {
            return Err(StoreError::UnknownConnection);
        }
        self.paths.delete_credentials(id)?;
        self.connections.retain(|c| c.id != id);
        let thumb = self.paths.thumb_path(id);
        let _ = fs::remove_file(thumb);
        Ok(())
    }

    pub fn labels(&self) -> Vec<String> {
        let mut labels: Vec<String> = self
            .connections
            .iter()
            .flat_map(|c| c.labels.iter().cloned())
            .collect();
        labels.sort();
        labels.dedup();
        labels
    }

    pub fn recents(&self, n: usize) -> Vec<Connection> {
        let mut list: Vec<Connection> = self
            .connections
            .iter()
            .filter(|c| c.last_connected.is_some())
            .cloned()
            .collect();
        list.sort_by_key(|b| std::cmp::Reverse(b.last_connected));
        list.truncate(n);
        list
    }

    pub fn filtered(&self, query: &str, label: Option<&str>) -> Vec<Connection> {
        let q = query.trim().to_ascii_lowercase();
        self.connections
            .iter()
            .filter(|c| {
                if let Some(label) = label
                    && !c.labels.iter().any(|l| l == label)
                {
                    return false;
                }
                if q.is_empty() {
                    return true;
                }
                c.name.to_ascii_lowercase().contains(&q)
                    || c.host.to_ascii_lowercase().contains(&q)
                    || c.server_display().to_ascii_lowercase().contains(&q)
                    || c.labels.iter().any(|l| l.to_ascii_lowercase().contains(&q))
            })
            .cloned()
            .collect()
    }

    pub fn save(&self) -> Result<(), StoreError> {
        self.paths.ensure()?;
        let file = AddressBookFile {
            connections: self.connections.clone(),
        };
        atomic_write(&self.paths.address_book, &serde_json::to_vec_pretty(&file)?)?;
        atomic_write(&self.paths.prefs, &serde_json::to_vec_pretty(&self.prefs)?)?;
        Ok(())
    }

    pub fn forget_sensitive(&mut self) -> Result<(), StoreError> {
        for conn in &self.connections {
            self.paths.delete_credentials(conn.id)?;
            let _ = fs::remove_file(self.paths.thumb_path(conn.id));
        }
        for conn in &mut self.connections {
            conn.remember_password = false;
        }
        self.prefs.hide_screenshots = true;
        self.save()
    }
}

/// `Ok(None)` when the file does not exist.
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, StoreError> {
    if !path.exists() {
        return Ok(None);
    }
    let data = fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str(&data)?))
}

fn read_json_or_quarantine<T: serde::de::DeserializeOwned + Default>(
    path: &Path,
    warnings: &mut Vec<String>,
) -> Result<T, StoreError> {
    match read_json(path) {
        Ok(v) => Ok(v.unwrap_or_default()),
        Err(StoreError::Json(e)) => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let aside = path.with_extension("json.broken");
            fs::rename(path, &aside)?;
            warnings.push(format!(
                "{name} was unreadable ({e}) and moved to {}",
                aside.display()
            ));
            Ok(T::default())
        }
        Err(e) => Err(e),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        #[cfg(unix)]
        f.set_permissions(fs::Permissions::from_mode(0o600))?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_book() -> (AddressBook, tempfile_dir::Guard) {
        let guard = tempfile_dir::Guard::new();
        let paths = StorePaths::in_dir(guard.path().to_path_buf());
        (AddressBook::load(paths).unwrap(), guard)
    }

    // Tiny temp-dir helper so we don't take a tempfile crate dependency.
    mod tempfile_dir {
        use std::path::{Path, PathBuf};
        use std::time::{SystemTime, UNIX_EPOCH};

        pub struct Guard(PathBuf);
        impl Guard {
            pub fn new() -> Self {
                let nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let path = std::env::temp_dir().join(format!("rv-test-{nanos}"));
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn crud_and_filter() {
        let (mut book, _g) = tmp_book();
        let mut a = Connection::new("office", "10.0.0.8", 5900);
        a.labels.push("lab".into());
        let b = Connection::new("pi4", "192.0.2.4", 5900);
        book.upsert(a.clone());
        book.upsert(b);
        book.save().unwrap();

        let reloaded = AddressBook::load(book.paths.clone()).unwrap();
        assert_eq!(reloaded.connections().len(), 2);
        assert_eq!(reloaded.labels(), vec!["lab".to_string()]);
        assert_eq!(reloaded.filtered("off", None).len(), 1);
        assert_eq!(reloaded.filtered("", Some("lab")).len(), 1);
    }

    #[test]
    fn corrupt_book_is_quarantined() {
        let guard = tempfile_dir::Guard::new();
        let paths = StorePaths::in_dir(guard.path().to_path_buf());
        paths.ensure().unwrap();
        std::fs::write(&paths.address_book, b"{ not json").unwrap();
        assert!(AddressBook::load(paths.clone()).is_err());

        let (book, warnings) = AddressBook::load_or_quarantine(paths.clone()).unwrap();
        assert!(book.connections().is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(!paths.address_book.exists());
        assert!(paths.root.join("addressbook.json.broken").exists());
    }

    #[test]
    fn recents_order() {
        let (mut book, _g) = tmp_book();
        let mut a = Connection::new("a", "a.local", 5900);
        let mut b = Connection::new("b", "b.local", 5900);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        a.last_connected = Some(now - 10);
        b.last_connected = Some(now);
        book.upsert(a);
        book.upsert(b);
        let recents = book.recents(1);
        assert_eq!(recents[0].name, "b");
    }

    #[test]
    fn remembered_password_roundtrip_uses_authenticated_encryption() {
        let (book, _guard) = tmp_book();
        let id = ConnectionId::new();
        let other = ConnectionId::new();
        let password = "a secret 密码";
        assert_eq!(book.paths.load_password(id).unwrap(), None);

        book.paths.save_password(id, password).unwrap();
        let path = book.paths.password_path(id);
        let first = fs::read(&path).unwrap();
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            !first
                .windows(password.len())
                .any(|part| part == password.as_bytes())
        );
        assert_eq!(
            book.paths.load_password(id).unwrap().as_deref(),
            Some(password)
        );

        book.paths.save_password(id, password).unwrap();
        let second = fs::read(&path).unwrap();
        assert_ne!(first, second);
        fs::write(book.paths.password_path(other), &second).unwrap();
        assert!(book.paths.load_password(other).is_err());

        let mut tampered = second;
        *tampered.last_mut().unwrap() ^= 1;
        fs::write(&path, tampered).unwrap();
        assert!(book.paths.load_password(id).is_err());
        book.paths.delete_credentials(id).unwrap();
        assert_eq!(book.paths.load_password(id).unwrap(), None);
    }

    #[test]
    fn connection_and_unlock_passwords_share_one_file_and_clear_independently() {
        let (mut book, _guard) = tmp_book();
        let connection = Connection::new("phone", "phone-a", 5900);
        let id = connection.id;
        let mut req = ConnectRequest::from_connection(&connection, None);
        book.upsert(connection);
        let code = UnlockCode::new("001234".into()).unwrap();
        book.paths.save_password(id, "vnc-password").unwrap();
        book.paths.save_unlock_code(&req, &code).unwrap();
        assert_eq!(
            fs::read_dir(book.paths.root.join("passwords"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            book.paths.load_password(id).unwrap().as_deref(),
            Some("vnc-password")
        );
        assert_eq!(
            book.paths.load_unlock_code(&req).unwrap().unwrap().digits(),
            b"001234"
        );
        req.host = "phone-b".into();
        assert!(book.paths.load_unlock_code(&req).unwrap().is_none());
        req.host = "phone-a".into();
        book.paths.delete_password(id).unwrap();
        assert!(book.paths.load_password(id).unwrap().is_none());
        assert!(book.paths.load_unlock_code(&req).unwrap().is_some());
        book.paths.save_password(id, "replacement").unwrap();
        book.paths.delete_unlock_code(&req).unwrap();
        assert!(book.paths.load_unlock_code(&req).unwrap().is_none());
        assert_eq!(
            book.paths.load_password(id).unwrap().as_deref(),
            Some("replacement")
        );
        book.paths.save_unlock_code(&req, &code).unwrap();
        book.remove(id).unwrap();
        assert!(!book.paths.password_path(id).exists());
    }

    #[test]
    fn legacy_password_file_survives_adding_unlock_password() {
        let (book, _guard) = tmp_book();
        let conn = Connection::new("phone", "localhost", 5900);
        let req = ConnectRequest::from_connection(&conn, None);
        let nonce = [1; 12];
        let mut encrypted = b"legacy-vnc".to_vec();
        password_cipher()
            .unwrap()
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(conn.id.to_string().as_bytes()),
                &mut encrypted,
            )
            .unwrap();
        let data = [LEGACY_PASSWORD_FILE_VERSION.as_slice(), &nonce, &encrypted].concat();
        fs::create_dir_all(book.paths.root.join("passwords")).unwrap();
        fs::write(book.paths.password_path(conn.id), data).unwrap();
        assert_eq!(
            book.paths.load_password(conn.id).unwrap().as_deref(),
            Some("legacy-vnc")
        );
        book.paths
            .save_unlock_code(&req, &UnlockCode::new("001234".into()).unwrap())
            .unwrap();
        assert_eq!(
            book.paths.load_password(conn.id).unwrap().as_deref(),
            Some("legacy-vnc")
        );
        assert_eq!(
            &fs::read(book.paths.password_path(conn.id)).unwrap()[..4],
            PASSWORD_FILE_VERSION
        );
        book.paths.delete_unlock_code(&req).unwrap();
        assert_eq!(
            book.paths.load_password(conn.id).unwrap().as_deref(),
            Some("legacy-vnc")
        );
    }

    #[test]
    fn ad_hoc_unlock_uses_same_local_store_and_isolates_endpoints() {
        let (book, _guard) = tmp_book();
        let conn = Connection::new("phone", "localhost", 5900);
        let mut req = ConnectRequest::from_connection(&conn, None);
        req.connection_id = None;
        book.paths
            .save_unlock_code(&req, &UnlockCode::new("001234".into()).unwrap())
            .unwrap();
        assert!(book.paths.load_unlock_code(&req).unwrap().is_some());
        req.port = 5901;
        assert!(book.paths.load_unlock_code(&req).unwrap().is_none());
        req.port = 5900;
        book.paths.delete_unlock_code(&req).unwrap();
        assert!(book.paths.load_unlock_code(&req).unwrap().is_none());
    }

    #[test]
    fn removing_connection_deletes_saved_password() {
        let (mut book, _guard) = tmp_book();
        let connection = Connection::new("office", "localhost", 5900);
        let id = connection.id;
        book.upsert(connection);
        book.paths.save_password(id, "secret").unwrap();
        book.remove(id).unwrap();
        assert_eq!(book.paths.load_password(id).unwrap(), None);
    }
}
