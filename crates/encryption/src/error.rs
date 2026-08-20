use std::fmt;

/// Stable failure categories exposed by the encrypted Markdown engine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncryptionError {
    NotEncryptedDocument,
    UnsupportedVersion,
    MalformedEnvelope,
    UnsupportedKdfProfile,
    PasswordRejected,
    SessionMismatch,
    AuthenticationFailed,
    InvalidUtf8Payload,
    RandomSourceUnavailable,
    EncryptionFailed,
}

impl fmt::Display for EncryptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::NotEncryptedDocument => "not an encrypted Textora document",
            Self::UnsupportedVersion => "unsupported encrypted document version",
            Self::MalformedEnvelope => "malformed encrypted document envelope",
            Self::UnsupportedKdfProfile => "unsupported encrypted document KDF profile",
            Self::PasswordRejected => "password rejected",
            Self::SessionMismatch => "encrypted document does not match the unlocked session",
            Self::AuthenticationFailed => "encrypted document authentication failed",
            Self::InvalidUtf8Payload => "encrypted Markdown payload is not valid UTF-8",
            Self::RandomSourceUnavailable => "secure random source unavailable",
            Self::EncryptionFailed => "encrypted document operation failed",
        };

        formatter.write_str(message)
    }
}

impl std::error::Error for EncryptionError {}

/// Password policy errors never retain or render the rejected password.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PasswordPolicyError {
    Empty,
    TooShort { minimum_characters: usize },
}

impl fmt::Display for PasswordPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("password must not be empty"),
            Self::TooShort { minimum_characters } => {
                write!(formatter, "password must contain at least {minimum_characters} characters")
            }
        }
    }
}

impl std::error::Error for PasswordPolicyError {}

#[cfg(test)]
mod tests {
    use super::{EncryptionError, PasswordPolicyError};

    #[test]
    fn errors_do_not_render_sensitive_context() {
        for error in [
            EncryptionError::PasswordRejected,
            EncryptionError::SessionMismatch,
            EncryptionError::AuthenticationFailed,
            EncryptionError::EncryptionFailed,
        ] {
            let rendered = format!("{error:?}: {error}");

            assert!(!rendered.contains("secret"));
            assert!(!rendered.contains("plaintext"));
        }

        let policy_error = PasswordPolicyError::TooShort { minimum_characters: 8 };
        assert_eq!(policy_error.to_string(), "password must contain at least 8 characters");
    }
}
