use std::fmt;

use textora_sync::ApiKey;

use crate::sync_connection_store::SYNC_KEYCHAIN_SERVICE;

pub(crate) trait SyncSecretStore: Send + Sync {
    fn load_api_key(&self, account: &str) -> Result<Option<ApiKey>, SyncSecretStoreError>;
    fn save_api_key(&self, account: &str, new_secret: &str) -> Result<(), SyncSecretStoreError>;
    fn delete_api_key(&self, account: &str) -> Result<(), SyncSecretStoreError>;
}

#[derive(Debug)]
pub(crate) enum SyncSecretStoreError {
    InvalidAccount,
    InvalidSecret,
    InvalidStoredSecret,
    Backend { operation: &'static str, message: String },
    UnsupportedPlatform,
}

impl fmt::Display for SyncSecretStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAccount => formatter.write_str("invalid keychain account"),
            Self::InvalidSecret => formatter.write_str("invalid API key"),
            Self::InvalidStoredSecret => formatter.write_str("keychain value is not valid UTF-8"),
            Self::Backend { operation, message } => {
                write!(formatter, "keychain {operation} failed: {message}")
            }
            Self::UnsupportedPlatform => {
                formatter.write_str("Syncthing API key storage is unavailable on this platform")
            }
        }
    }
}

impl std::error::Error for SyncSecretStoreError {}

pub(crate) struct MacKeychainSecretStore;

impl MacKeychainSecretStore {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[cfg(target_os = "macos")]
impl SyncSecretStore for MacKeychainSecretStore {
    fn load_api_key(&self, account: &str) -> Result<Option<ApiKey>, SyncSecretStoreError> {
        validate_account(account)?;
        match security_framework::passwords::get_generic_password(SYNC_KEYCHAIN_SERVICE, account) {
            Ok(bytes) => {
                let secret = String::from_utf8(bytes)
                    .map_err(|_| SyncSecretStoreError::InvalidStoredSecret)?;
                ApiKey::new(secret).map(Some).map_err(|_| SyncSecretStoreError::InvalidStoredSecret)
            }
            Err(error) if error.code() == security_framework_sys::base::errSecItemNotFound => {
                Ok(None)
            }
            Err(error) => Err(backend_error("load", error.code())),
        }
    }

    fn save_api_key(&self, account: &str, new_secret: &str) -> Result<(), SyncSecretStoreError> {
        validate_account(account)?;
        ApiKey::new(new_secret.to_owned()).map_err(|_| SyncSecretStoreError::InvalidSecret)?;
        security_framework::passwords::set_generic_password(
            SYNC_KEYCHAIN_SERVICE,
            account,
            new_secret.as_bytes(),
        )
        .map_err(|error| backend_error("save", error.code()))
    }

    fn delete_api_key(&self, account: &str) -> Result<(), SyncSecretStoreError> {
        validate_account(account)?;
        match security_framework::passwords::delete_generic_password(SYNC_KEYCHAIN_SERVICE, account)
        {
            Ok(()) => Ok(()),
            Err(error) if error.code() == security_framework_sys::base::errSecItemNotFound => {
                Ok(())
            }
            Err(error) => Err(backend_error("delete", error.code())),
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl SyncSecretStore for MacKeychainSecretStore {
    fn load_api_key(&self, _account: &str) -> Result<Option<ApiKey>, SyncSecretStoreError> {
        Err(SyncSecretStoreError::UnsupportedPlatform)
    }

    fn save_api_key(&self, _account: &str, _new_secret: &str) -> Result<(), SyncSecretStoreError> {
        Err(SyncSecretStoreError::UnsupportedPlatform)
    }

    fn delete_api_key(&self, _account: &str) -> Result<(), SyncSecretStoreError> {
        Err(SyncSecretStoreError::UnsupportedPlatform)
    }
}

fn validate_account(account: &str) -> Result<(), SyncSecretStoreError> {
    if account.trim().is_empty()
        || account.contains(['/', '\\'])
        || account.chars().any(char::is_control)
    {
        return Err(SyncSecretStoreError::InvalidAccount);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn backend_error(operation: &'static str, status_code: i32) -> SyncSecretStoreError {
    SyncSecretStoreError::Backend {
        operation,
        message: format!("Security Framework status {status_code}"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use textora_sync::ApiKey;

    use super::{SyncSecretStore, SyncSecretStoreError};

    #[derive(Default)]
    struct FakeSecretStore {
        values: Mutex<HashMap<String, String>>,
        failure: Option<String>,
    }

    impl SyncSecretStore for FakeSecretStore {
        fn load_api_key(&self, account: &str) -> Result<Option<ApiKey>, SyncSecretStoreError> {
            if let Some(message) = &self.failure {
                return Err(SyncSecretStoreError::Backend {
                    operation: "load",
                    message: message.clone(),
                });
            }
            let value = self
                .values
                .lock()
                .map_err(|_| SyncSecretStoreError::Backend {
                    operation: "load",
                    message: "secret store lock poisoned".to_owned(),
                })?
                .get(account)
                .cloned();
            value
                .map(|secret| ApiKey::new(secret).map_err(|_| SyncSecretStoreError::InvalidSecret))
                .transpose()
        }

        fn save_api_key(
            &self,
            account: &str,
            new_secret: &str,
        ) -> Result<(), SyncSecretStoreError> {
            if let Some(message) = &self.failure {
                return Err(SyncSecretStoreError::Backend {
                    operation: "save",
                    message: message.clone(),
                });
            }
            ApiKey::new(new_secret.to_owned()).map_err(|_| SyncSecretStoreError::InvalidSecret)?;
            self.values
                .lock()
                .map_err(|_| SyncSecretStoreError::Backend {
                    operation: "save",
                    message: "secret store lock poisoned".to_owned(),
                })?
                .insert(account.to_owned(), new_secret.to_owned());
            Ok(())
        }

        fn delete_api_key(&self, account: &str) -> Result<(), SyncSecretStoreError> {
            if let Some(message) = &self.failure {
                return Err(SyncSecretStoreError::Backend {
                    operation: "delete",
                    message: message.clone(),
                });
            }
            self.values
                .lock()
                .map_err(|_| SyncSecretStoreError::Backend {
                    operation: "delete",
                    message: "secret store lock poisoned".to_owned(),
                })?
                .remove(account);
            Ok(())
        }
    }

    #[test]
    fn fake_secret_store_round_trips_and_deletes_api_key() {
        let store = FakeSecretStore::default();
        assert!(store.load_api_key("account").expect("missing secret should load").is_none());
        store.save_api_key("account", "secret-value").expect("secret should save");
        assert!(store.load_api_key("account").expect("saved secret should load").is_some());
        store.delete_api_key("account").expect("secret should delete");
        assert!(store.load_api_key("account").expect("deleted secret should load").is_none());
    }

    #[test]
    fn backend_errors_never_contain_the_secret() {
        let store = FakeSecretStore {
            values: Mutex::new(HashMap::new()),
            failure: Some("keychain unavailable".to_owned()),
        };
        let error =
            store.save_api_key("account", "secret-value").expect_err("fake backend should fail");
        assert!(!error.to_string().contains("secret-value"));
    }

    #[test]
    fn blank_secret_is_rejected_before_backend_write() {
        let store = FakeSecretStore::default();
        assert!(store.save_api_key("account", "   ").is_err());
        assert!(store.load_api_key("account").expect("missing secret should load").is_none());
    }
}
