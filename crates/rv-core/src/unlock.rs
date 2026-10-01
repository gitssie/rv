use crate::{ConnectRequest, StoreError};
use zeroize::Zeroizing;

/// Only numeric passcodes are supported by the guarded HID input lease.
/// Keep the secret out of Debug output and erase owned buffers on drop.
pub struct UnlockCode(Zeroizing<String>);

impl UnlockCode {
    pub fn new(value: String) -> Result<Self, StoreError> {
        let value = Zeroizing::new(value);
        if value.len() != 6 || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(StoreError::Credential("请输入 6 位数字锁屏密码".into()));
        }
        Ok(Self(value))
    }
    pub fn digits(&self) -> &[u8] {
        self.0.as_bytes()
    }
}
impl std::fmt::Debug for UnlockCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UnlockCode([REDACTED])")
    }
}

pub fn load_unlock_code(req: &ConnectRequest) -> Result<Option<UnlockCode>, StoreError> {
    crate::StorePaths::default_dir()?.load_unlock_code(req)
}

pub fn save_unlock_code(req: &ConnectRequest, code: &UnlockCode) -> Result<(), StoreError> {
    crate::StorePaths::default_dir()?.save_unlock_code(req, code)
}

pub fn delete_unlock_code(req: &ConnectRequest) -> Result<(), StoreError> {
    crate::StorePaths::default_dir()?.delete_unlock_code(req)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passcode_validation_and_debug_do_not_expose_secret() {
        let code = UnlockCode::new("001234".into()).unwrap();
        assert_eq!(code.digits(), b"001234");
        assert_eq!(format!("{code:?}"), "UnlockCode([REDACTED])");
        for invalid in [
            "123",
            "1234",
            "12345",
            "12345678",
            "1234567890123",
            "abcd",
            "１２３４",
            "12 34",
            "1234\n",
        ] {
            assert!(UnlockCode::new(invalid.into()).is_err());
        }
    }
}
