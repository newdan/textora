use semver::Version;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("invalid loopback endpoint: {reason}")]
    InvalidEndpoint { reason: String },
    #[error("Syncthing connection was refused")]
    ConnectionRefused,
    #[error("Syncthing request timed out during {operation}")]
    RequestTimeout { operation: &'static str },
    #[error("Syncthing authentication failed")]
    Authentication,
    #[error("incompatible Syncthing version: {found}")]
    IncompatibleVersion { found: Version },
    #[error("invalid Syncthing response during {operation}: {message}")]
    InvalidResponse { operation: &'static str, message: String },
    #[error("Syncthing operation {operation} failed with HTTP status {status}")]
    Remote { operation: &'static str, status: u16 },
    #[error("invalid {kind} identifier")]
    InvalidIdentifier { kind: &'static str },
    #[error("Syncthing configuration verification failed during {operation}")]
    ConfigurationMismatch { operation: &'static str },
}
