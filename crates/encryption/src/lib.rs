//! Authenticated encrypted Markdown containers for Textora.

#![forbid(unsafe_code)]

mod crypto;
mod error;
mod format;
mod secret;

#[cfg(test)]
mod test_vectors;

pub use crypto::{
    CreatedEncryptedMarkdown, UnlockedEncryptedMarkdown, create_encrypted_markdown,
    decrypt_markdown_with_session, encrypt_markdown_with_session, unlock_encrypted_markdown,
};
pub use error::{EncryptionError, PasswordPolicyError};
pub use format::{EncryptedMarkdownHeader, inspect_encrypted_markdown};
pub use secret::{EncryptionPassword, UnlockedNoteSession};
