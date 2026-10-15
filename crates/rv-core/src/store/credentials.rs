//! Independent encrypted credential repository. Updates are transactional across processes.
use crate::{ConnectRequest, ConnectionId, StoreError, StorePaths, UnlockCode};
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

const VERSION: &[u8; 4] = b"RVS1";
// Retain the application's existing local cipher; no keychain or legacy-file access.
const KEY: [u8; 32] = [
    0x7a, 0xf7, 0x4d, 0x61, 0x24, 0xa6, 0x38, 0x77, 0xc4, 0x15, 0x3b, 0x9e, 0x1a, 0x05, 0xb0, 0xb2,
    0x68, 0x22, 0x16, 0x61, 0x3b, 0x8c, 0x19, 0xe5, 0x30, 0xe8, 0x34, 0x79, 0xfa, 0x72, 0x44, 0x06,
];
#[derive(Default, Serialize, Deserialize)]
struct Credentials {
    password: Option<String>,
    unlock: Option<SavedUnlock>,
}
#[derive(Serialize, Deserialize)]
struct SavedUnlock {
    host: String,
    port: u16,
    code: String,
}

fn cipher() -> Result<aead::LessSafeKey, StoreError> {
    aead::UnboundKey::new(&aead::AES_256_GCM, &KEY)
        .map(aead::LessSafeKey::new)
        .map_err(|_| StoreError::Credential("invalid encryption key".into()))
}
pub(crate) fn seal(scope: &str, plain: &[u8]) -> Result<Vec<u8>, StoreError> {
    let mut nonce = [0; 12];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| StoreError::Credential("cannot generate nonce".into()))?;
    let mut encrypted = plain.to_vec();
    cipher()?
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(scope.as_bytes()),
            &mut encrypted,
        )
        .map_err(|_| StoreError::Credential("cannot encrypt credentials".into()))?;
    Ok([VERSION.as_slice(), &nonce, &encrypted].concat())
}
pub(crate) fn unseal(scope: &str, data: Vec<u8>) -> Result<Vec<u8>, StoreError> {
    if data.len() < 16 + aead::AES_256_GCM.tag_len() || &data[..4] != VERSION {
        return Err(StoreError::Credential(
            "invalid encrypted credentials".into(),
        ));
    }
    let nonce = data[4..16].try_into().expect("validated nonce");
    let mut encrypted = data[16..].to_vec();
    let plain = cipher()?
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(scope.as_bytes()),
            &mut encrypted,
        )
        .map_err(|_| StoreError::Credential("credential authentication failed".into()))?;
    Ok(plain.to_vec())
}
fn encode(id: ConnectionId, credentials: &Credentials) -> Result<Vec<u8>, StoreError> {
    seal(&id.to_string(), &serde_json::to_vec(credentials)?)
}
fn decode(id: ConnectionId, data: Vec<u8>) -> Result<Credentials, StoreError> {
    Ok(serde_json::from_slice(&unseal(&id.to_string(), data)?)?)
}
fn read(db: &rusqlite::Connection, id: ConnectionId) -> Result<Credentials, StoreError> {
    let data: Option<Vec<u8>> = db
        .query_row(
            "SELECT encrypted FROM credentials WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    data.map(|v| decode(id, v))
        .transpose()
        .map(|v| v.unwrap_or_default())
}
impl StorePaths {
    fn update_credentials(
        &self,
        id: ConnectionId,
        update: impl FnOnce(&mut Credentials),
    ) -> Result<(), StoreError> {
        let mut db = crate::database::open(&self.database)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut credentials = read(&tx, id)?;
        update(&mut credentials);
        if credentials.password.is_none() && credentials.unlock.is_none() {
            tx.execute("DELETE FROM credentials WHERE id=?1", [id.to_string()])?;
        } else {
            tx.execute("INSERT INTO credentials VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET encrypted=excluded.encrypted",
                params![id.to_string(), encode(id, &credentials)?])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn save_password(&self, id: ConnectionId, password: &str) -> Result<(), StoreError> {
        self.update_credentials(id, |c| c.password = Some(password.to_owned()))
    }
    pub fn load_password(&self, id: ConnectionId) -> Result<Option<String>, StoreError> {
        Ok(read(&crate::database::open(&self.database)?, id)?.password)
    }
    pub fn delete_password(&self, id: ConnectionId) -> Result<(), StoreError> {
        self.update_credentials(id, |c| c.password = None)
    }
    pub fn load_unlock_code(&self, req: &ConnectRequest) -> Result<Option<UnlockCode>, StoreError> {
        match read(&crate::database::open(&self.database)?, credential_id(req))?.unlock {
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
        self.update_credentials(credential_id(req), |c| {
            c.unlock = Some(SavedUnlock {
                host: req.host.clone(),
                port: req.port,
                code: std::str::from_utf8(code.digits())
                    .expect("validated digits")
                    .to_owned(),
            })
        })
    }
    pub fn delete_unlock_code(&self, req: &ConnectRequest) -> Result<(), StoreError> {
        self.update_credentials(credential_id(req), |c| c.unlock = None)
    }
    pub(super) fn delete_credentials(&self, id: ConnectionId) -> Result<(), StoreError> {
        crate::database::open(&self.database)?
            .execute("DELETE FROM credentials WHERE id=?1", [id.to_string()])?;
        Ok(())
    }
}
fn credential_id(req: &ConnectRequest) -> ConnectionId {
    req.connection_id.unwrap_or_else(|| {
        let endpoint = format!("rv-ad-hoc|{}:{}", req.host, req.port);
        let digest = ring::digest::digest(&ring::digest::SHA256, endpoint.as_bytes());
        ConnectionId(uuid::Uuid::from_bytes(
            digest.as_ref()[..16].try_into().unwrap(),
        ))
    })
}
