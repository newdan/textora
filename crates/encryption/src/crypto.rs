use std::fmt;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rand_core::{OsRng, RngCore};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::format::{EncryptedEnvelope, KDF_PROFILE, parse_envelope, serialize_envelope};
use crate::secret::{DATA_KEY_LENGTH, KEY_NONCE_LENGTH, SALT_LENGTH, WRAPPED_KEY_LENGTH};
use crate::{EncryptionError, EncryptionPassword, UnlockedNoteSession};

const ARGON2_MEMORY_COST_KIB: u32 = 64 * 1024;
const ARGON2_ITERATIONS: u32 = 3;
const ARGON2_PARALLELISM: u32 = 1;
const CONTENT_NONCE_LENGTH: usize = 24;
const DOCUMENT_ID_RANDOM_BYTES: usize = 16;
const PROTOCOL_AAD: &[u8] = b"textora-encrypted-markdown:v1";
const WRAPPED_KEY_AAD_LABEL: &[u8] = b"wrapped-key";
const CONTENT_AAD_LABEL: &[u8] = b"content";

pub struct CreatedEncryptedMarkdown {
    serialized: Vec<u8>,
    session: UnlockedNoteSession,
}

impl CreatedEncryptedMarkdown {
    pub fn serialized(&self) -> &[u8] {
        &self.serialized
    }

    pub fn into_parts(self) -> (Vec<u8>, UnlockedNoteSession) {
        (self.serialized, self.session)
    }
}

impl fmt::Debug for CreatedEncryptedMarkdown {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreatedEncryptedMarkdown")
            .field("serialized_length", &self.serialized.len())
            .field("session", &self.session)
            .finish()
    }
}

pub struct UnlockedEncryptedMarkdown {
    plaintext: String,
    session: UnlockedNoteSession,
}

impl UnlockedEncryptedMarkdown {
    pub fn plaintext(&self) -> &str {
        &self.plaintext
    }

    pub fn into_parts(self) -> (String, UnlockedNoteSession) {
        (self.plaintext, self.session)
    }
}

impl fmt::Debug for UnlockedEncryptedMarkdown {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnlockedEncryptedMarkdown")
            .field("plaintext", &"<redacted>")
            .field("session", &self.session)
            .finish()
    }
}

pub fn create_encrypted_markdown(
    password: &EncryptionPassword,
    plaintext: &[u8],
) -> Result<CreatedEncryptedMarkdown, EncryptionError> {
    let randomness = EncryptionRandomness::from_operating_system()?;
    create_encrypted_markdown_with_randomness(password, plaintext, randomness)
}

pub fn unlock_encrypted_markdown(
    serialized: &[u8],
    password: &EncryptionPassword,
) -> Result<UnlockedEncryptedMarkdown, EncryptionError> {
    let envelope = parse_envelope(serialized)?;
    let key_encryption_key = derive_key_encryption_key(password, &envelope.salt)?;
    let wrapped_key_aad = wrapped_key_associated_data(envelope.document_id, &envelope.salt);
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key_encryption_key[..]));
    let data_key = Zeroizing::new(
        cipher
            .decrypt(
                XNonce::from_slice(&envelope.key_nonce),
                Payload { msg: &envelope.wrapped_key, aad: &wrapped_key_aad },
            )
            .map_err(|_| EncryptionError::PasswordRejected)?,
    );
    let data_key: [u8; DATA_KEY_LENGTH] =
        data_key.as_slice().try_into().map_err(|_| EncryptionError::AuthenticationFailed)?;
    let plaintext = decrypt_content(
        envelope.document_id,
        &data_key,
        &envelope.content_nonce,
        &envelope.ciphertext,
    )?;
    let plaintext =
        String::from_utf8(plaintext).map_err(|_| EncryptionError::InvalidUtf8Payload)?;
    let session = UnlockedNoteSession::new(
        envelope.document_id,
        data_key,
        envelope.salt,
        envelope.key_nonce,
        envelope.wrapped_key,
    );

    Ok(UnlockedEncryptedMarkdown { plaintext, session })
}

pub fn decrypt_markdown_with_session(
    serialized: &[u8],
    session: &UnlockedNoteSession,
) -> Result<String, EncryptionError> {
    let envelope = parse_envelope(serialized)?;
    let (salt, key_nonce, wrapped_key) = session.envelope_fields();
    if envelope.document_id != session.document_id()
        || &envelope.salt != salt
        || &envelope.key_nonce != key_nonce
        || &envelope.wrapped_key != wrapped_key
    {
        return Err(EncryptionError::SessionMismatch);
    }
    let plaintext = decrypt_content(
        envelope.document_id,
        session.data_key(),
        &envelope.content_nonce,
        &envelope.ciphertext,
    )?;
    String::from_utf8(plaintext).map_err(|_| EncryptionError::InvalidUtf8Payload)
}

pub fn encrypt_markdown_with_session(
    session: &UnlockedNoteSession,
    plaintext: &[u8],
) -> Result<Vec<u8>, EncryptionError> {
    validate_plaintext(plaintext)?;
    let content_nonce = random_array()?;
    encrypt_markdown_with_content_nonce(session, plaintext, content_nonce)
}

pub(crate) fn encrypt_markdown_with_content_nonce(
    session: &UnlockedNoteSession,
    plaintext: &[u8],
    content_nonce: [u8; CONTENT_NONCE_LENGTH],
) -> Result<Vec<u8>, EncryptionError> {
    validate_plaintext(plaintext)?;
    let ciphertext =
        encrypt_content(session.document_id(), session.data_key(), &content_nonce, plaintext)?;
    let (salt, key_nonce, wrapped_key) = session.envelope_fields();
    let envelope = EncryptedEnvelope {
        document_id: session.document_id(),
        salt: *salt,
        key_nonce: *key_nonce,
        wrapped_key: *wrapped_key,
        content_nonce,
        ciphertext,
    };

    Ok(serialize_envelope(&envelope))
}

pub(crate) struct EncryptionRandomness {
    pub document_id: [u8; DOCUMENT_ID_RANDOM_BYTES],
    pub salt: [u8; SALT_LENGTH],
    pub data_key: [u8; DATA_KEY_LENGTH],
    pub key_nonce: [u8; KEY_NONCE_LENGTH],
    pub content_nonce: [u8; CONTENT_NONCE_LENGTH],
}

impl EncryptionRandomness {
    fn from_operating_system() -> Result<Self, EncryptionError> {
        Ok(Self {
            document_id: random_array()?,
            salt: random_array()?,
            data_key: random_array()?,
            key_nonce: random_array()?,
            content_nonce: random_array()?,
        })
    }
}

pub(crate) fn create_encrypted_markdown_with_randomness(
    password: &EncryptionPassword,
    plaintext: &[u8],
    mut randomness: EncryptionRandomness,
) -> Result<CreatedEncryptedMarkdown, EncryptionError> {
    validate_plaintext(plaintext)?;
    make_uuid_v4(&mut randomness.document_id);
    let document_id = Uuid::from_bytes(randomness.document_id);
    let key_encryption_key = derive_key_encryption_key(password, &randomness.salt)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key_encryption_key[..]));
    let wrapped_key_aad = wrapped_key_associated_data(document_id, &randomness.salt);
    let wrapped_key = cipher
        .encrypt(
            XNonce::from_slice(&randomness.key_nonce),
            Payload { msg: &randomness.data_key, aad: &wrapped_key_aad },
        )
        .map_err(|_| EncryptionError::EncryptionFailed)?;
    let wrapped_key: [u8; WRAPPED_KEY_LENGTH] =
        wrapped_key.try_into().map_err(|_| EncryptionError::EncryptionFailed)?;
    let ciphertext =
        encrypt_content(document_id, &randomness.data_key, &randomness.content_nonce, plaintext)?;
    let session = UnlockedNoteSession::new(
        document_id,
        randomness.data_key,
        randomness.salt,
        randomness.key_nonce,
        wrapped_key,
    );
    let envelope = EncryptedEnvelope {
        document_id,
        salt: randomness.salt,
        key_nonce: randomness.key_nonce,
        wrapped_key,
        content_nonce: randomness.content_nonce,
        ciphertext,
    };

    Ok(CreatedEncryptedMarkdown { serialized: serialize_envelope(&envelope), session })
}

fn derive_key_encryption_key(
    password: &EncryptionPassword,
    salt: &[u8; SALT_LENGTH],
) -> Result<Zeroizing<[u8; DATA_KEY_LENGTH]>, EncryptionError> {
    let parameters = Params::new(
        ARGON2_MEMORY_COST_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        Some(DATA_KEY_LENGTH),
    )
    .map_err(|_| EncryptionError::EncryptionFailed)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, parameters);
    let mut key = Zeroizing::new([0_u8; DATA_KEY_LENGTH]);
    argon2
        .hash_password_into(password.expose().as_bytes(), salt, key.as_mut())
        .map_err(|_| EncryptionError::EncryptionFailed)?;

    Ok(key)
}

fn encrypt_content(
    document_id: Uuid,
    data_key: &[u8; DATA_KEY_LENGTH],
    content_nonce: &[u8; CONTENT_NONCE_LENGTH],
    plaintext: &[u8],
) -> Result<Vec<u8>, EncryptionError> {
    let content_aad = content_associated_data(document_id);
    XChaCha20Poly1305::new(Key::from_slice(data_key))
        .encrypt(XNonce::from_slice(content_nonce), Payload { msg: plaintext, aad: &content_aad })
        .map_err(|_| EncryptionError::EncryptionFailed)
}

fn decrypt_content(
    document_id: Uuid,
    data_key: &[u8; DATA_KEY_LENGTH],
    content_nonce: &[u8; CONTENT_NONCE_LENGTH],
    ciphertext: &[u8],
) -> Result<Vec<u8>, EncryptionError> {
    let content_aad = content_associated_data(document_id);
    XChaCha20Poly1305::new(Key::from_slice(data_key))
        .decrypt(XNonce::from_slice(content_nonce), Payload { msg: ciphertext, aad: &content_aad })
        .map_err(|_| EncryptionError::AuthenticationFailed)
}

fn wrapped_key_associated_data(document_id: Uuid, salt: &[u8; SALT_LENGTH]) -> Vec<u8> {
    canonical_associated_data(WRAPPED_KEY_AAD_LABEL, document_id, &[KDF_PROFILE.as_bytes(), salt])
}

fn content_associated_data(document_id: Uuid) -> Vec<u8> {
    canonical_associated_data(CONTENT_AAD_LABEL, document_id, &[])
}

fn canonical_associated_data(label: &[u8], document_id: Uuid, fields: &[&[u8]]) -> Vec<u8> {
    let mut associated_data = Vec::with_capacity(128);
    push_length_prefixed(&mut associated_data, PROTOCOL_AAD);
    push_length_prefixed(&mut associated_data, label);
    push_length_prefixed(&mut associated_data, document_id.as_bytes());
    for field in fields {
        push_length_prefixed(&mut associated_data, field);
    }
    associated_data
}

fn push_length_prefixed(destination: &mut Vec<u8>, value: &[u8]) {
    destination.extend_from_slice(&(value.len() as u64).to_be_bytes());
    destination.extend_from_slice(value);
}

fn validate_plaintext(plaintext: &[u8]) -> Result<(), EncryptionError> {
    std::str::from_utf8(plaintext).map(|_| ()).map_err(|_| EncryptionError::InvalidUtf8Payload)
}

fn random_array<const LENGTH: usize>() -> Result<[u8; LENGTH], EncryptionError> {
    let mut bytes = [0_u8; LENGTH];
    OsRng.try_fill_bytes(&mut bytes).map_err(|_| EncryptionError::RandomSourceUnavailable)?;
    Ok(bytes)
}

fn make_uuid_v4(bytes: &mut [u8; DOCUMENT_ID_RANDOM_BYTES]) {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
}
