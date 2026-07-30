use std::fmt;

use crate::SyncError;

pub struct ApiKey(String);

impl ApiKey {
    pub fn new(secret: String) -> Result<Self, SyncError> {
        if secret.trim().is_empty() {
            return Err(SyncError::InvalidIdentifier { kind: "API key" });
        }
        Ok(Self(secret))
    }

    pub(crate) fn expose_for_header(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKey([REDACTED])")
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeviceId(String);

impl DeviceId {
    pub fn parse(candidate: String) -> Result<Self, SyncError> {
        let compact = candidate.replace('-', "");
        if compact.len() != 56 || !compact.chars().all(is_device_id_character) {
            return Err(SyncError::InvalidIdentifier { kind: "device ID" });
        }

        let canonical = compact
            .as_bytes()
            .chunks(7)
            .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
            .collect::<Vec<_>>()
            .join("-");

        if candidate.contains('-') && candidate.split('-').any(|segment| segment.len() != 7) {
            return Err(SyncError::InvalidIdentifier { kind: "device ID" });
        }

        Ok(Self(canonical))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_device_id_character(character: char) -> bool {
    character.is_ascii_uppercase() || matches!(character, '2'..='7')
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FolderId(String);

impl FolderId {
    pub fn new(candidate: String) -> Result<Self, SyncError> {
        if candidate.is_empty()
            || candidate == "."
            || candidate == ".."
            || candidate.contains(['/', '\\'])
            || candidate.chars().any(char::is_control)
        {
            return Err(SyncError::InvalidIdentifier { kind: "folder" });
        }
        Ok(Self(candidate))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiKey, DeviceId, FolderId};

    const VALID_DEVICE_ID: &str = "ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG";

    #[test]
    fn rejects_empty_api_keys_and_redacts_debug_output() {
        assert!(ApiKey::new(String::new()).is_err());
        assert!(ApiKey::new("   ".to_owned()).is_err());

        let secret = "super-secret-api-key";
        let key = ApiKey::new(secret.to_owned()).expect("non-empty API key should parse");
        let debug = format!("{key:?}");
        assert!(!debug.contains(secret));
        assert_eq!(debug, "ApiKey([REDACTED])");
    }

    #[test]
    fn accepts_canonical_device_id_and_rejects_invalid_values() {
        let device_id = DeviceId::parse(VALID_DEVICE_ID.to_owned())
            .expect("canonical Syncthing device ID should parse");
        assert_eq!(device_id.as_str(), VALID_DEVICE_ID);

        for candidate in ["", "not-a-device-id", "ABCDEFG-ABCDEFG", "abc"] {
            assert!(DeviceId::parse(candidate.to_owned()).is_err(), "accepted {candidate}");
        }
    }

    #[test]
    fn folder_ids_are_non_empty_single_path_components() {
        for candidate in ["", "/", "folder/name", "folder\\name", ".", ".."] {
            assert!(FolderId::new(candidate.to_owned()).is_err(), "accepted {candidate}");
        }

        let folder_id = FolderId::new("notes".to_owned()).expect("simple folder ID should parse");
        assert_eq!(folder_id.as_str(), "notes");
    }
}
