use std::fmt;

use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::PasswordPolicyError;

pub(crate) const DATA_KEY_LENGTH: usize = 32;
pub(crate) const KEY_NONCE_LENGTH: usize = 24;
pub(crate) const SALT_LENGTH: usize = 16;
pub(crate) const WRAPPED_KEY_LENGTH: usize = 48;

const MINIMUM_PASSWORD_CHARACTERS: usize = 8;

/// A non-serializable password that clears its allocation when dropped.
pub struct EncryptionPassword {
    value: Zeroizing<String>,
}

impl EncryptionPassword {
    pub fn new(value: String) -> Result<Self, PasswordPolicyError> {
        let character_count = value.chars().count();
        if character_count == 0 {
            return Err(PasswordPolicyError::Empty);
        }
        if character_count < MINIMUM_PASSWORD_CHARACTERS {
            return Err(PasswordPolicyError::TooShort {
                minimum_characters: MINIMUM_PASSWORD_CHARACTERS,
            });
        }

        Ok(Self { value: Zeroizing::new(value) })
    }

    pub(crate) fn expose(&self) -> &str {
        self.value.as_str()
    }
}

impl fmt::Debug for EncryptionPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

impl Drop for EncryptionPassword {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

/// Per-tab key material for an unlocked encrypted Markdown document.
pub struct UnlockedNoteSession {
    document_id: Uuid,
    data_key: Zeroizing<[u8; DATA_KEY_LENGTH]>,
    salt: [u8; SALT_LENGTH],
    key_nonce: [u8; KEY_NONCE_LENGTH],
    wrapped_key: [u8; WRAPPED_KEY_LENGTH],
}

impl UnlockedNoteSession {
    pub fn document_id(&self) -> Uuid {
        self.document_id
    }

    pub(crate) fn new(
        document_id: Uuid,
        data_key: [u8; DATA_KEY_LENGTH],
        salt: [u8; SALT_LENGTH],
        key_nonce: [u8; KEY_NONCE_LENGTH],
        wrapped_key: [u8; WRAPPED_KEY_LENGTH],
    ) -> Self {
        Self { document_id, data_key: Zeroizing::new(data_key), salt, key_nonce, wrapped_key }
    }

    pub(crate) fn data_key(&self) -> &[u8; DATA_KEY_LENGTH] {
        &self.data_key
    }

    pub(crate) fn envelope_fields(
        &self,
    ) -> (&[u8; SALT_LENGTH], &[u8; KEY_NONCE_LENGTH], &[u8; WRAPPED_KEY_LENGTH]) {
        (&self.salt, &self.key_nonce, &self.wrapped_key)
    }
}

impl fmt::Debug for UnlockedNoteSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnlockedNoteSession")
            .field("document_id", &self.document_id)
            .field("data_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        DATA_KEY_LENGTH, EncryptionPassword, KEY_NONCE_LENGTH, SALT_LENGTH, UnlockedNoteSession,
        WRAPPED_KEY_LENGTH,
    };
    use crate::PasswordPolicyError;

    #[test]
    fn password_debug_output_is_redacted() {
        let password = EncryptionPassword::new("秘密password".to_owned())
            .expect("test password should satisfy the policy");

        assert_eq!(format!("{password:?}"), "<redacted>");
    }

    #[test]
    fn password_policy_counts_unicode_characters() {
        assert!(EncryptionPassword::new("八个字符密码成立".to_owned()).is_ok());
        assert_eq!(
            EncryptionPassword::new("短密码".to_owned()).expect_err("short password must fail"),
            PasswordPolicyError::TooShort { minimum_characters: 8 }
        );
        assert_eq!(
            EncryptionPassword::new(String::new()).expect_err("empty password must fail"),
            PasswordPolicyError::Empty
        );
    }

    #[test]
    fn session_debug_output_never_contains_key_material() {
        let session = UnlockedNoteSession::new(
            Uuid::nil(),
            [0x41; DATA_KEY_LENGTH],
            [0x42; SALT_LENGTH],
            [0x43; KEY_NONCE_LENGTH],
            [0x44; WRAPPED_KEY_LENGTH],
        );
        let rendered = format!("{session:?}");

        assert!(rendered.contains("<redacted>"));
        for secret_byte in ["41", "42", "43", "44"] {
            assert!(!rendered.contains(secret_byte));
        }
    }
}
