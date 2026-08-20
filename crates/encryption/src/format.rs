use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use uuid::{Uuid, Variant, Version};

use crate::EncryptionError;
use crate::secret::{KEY_NONCE_LENGTH, SALT_LENGTH, WRAPPED_KEY_LENGTH};

pub const KDF_PROFILE: &str = "argon2id-64m-t3-p1";

const MAGIC_PREFIX: &str = "<!-- textora-encrypted-markdown:";
const MAGIC_LINE: &str = "<!-- textora-encrypted-markdown:v1";
const HEADER_END: &str = "-->";
const PAYLOAD_FENCE_OPEN: &str = "```textora-encrypted";
const PAYLOAD_FENCE_CLOSE: &str = "```";
const DOCUMENT_ID_FIELD: &str = "document-id=";
const KDF_PROFILE_FIELD: &str = "kdf-profile=";
const SALT_FIELD: &str = "salt=";
const KEY_NONCE_FIELD: &str = "key-nonce=";
const WRAPPED_KEY_FIELD: &str = "wrapped-key=";
const CONTENT_NONCE_FIELD: &str = "content-nonce=";
const CONTENT_NONCE_LENGTH: usize = 24;
const AUTHENTICATION_TAG_LENGTH: usize = 16;
const PAYLOAD_LINE_WIDTH: usize = 80;
const MINIMUM_LINE_COUNT: usize = 13;
const MAXIMUM_ENVELOPE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncryptedMarkdownHeader {
    pub document_id: Uuid,
    pub kdf_profile: &'static str,
}

pub(crate) struct EncryptedEnvelope {
    pub document_id: Uuid,
    pub salt: [u8; SALT_LENGTH],
    pub key_nonce: [u8; KEY_NONCE_LENGTH],
    pub wrapped_key: [u8; WRAPPED_KEY_LENGTH],
    pub content_nonce: [u8; CONTENT_NONCE_LENGTH],
    pub ciphertext: Vec<u8>,
}

pub fn inspect_encrypted_markdown(
    serialized: &[u8],
) -> Result<EncryptedMarkdownHeader, EncryptionError> {
    let envelope = parse_envelope(serialized)?;
    Ok(EncryptedMarkdownHeader { document_id: envelope.document_id, kdf_profile: KDF_PROFILE })
}

pub(crate) fn parse_envelope(serialized: &[u8]) -> Result<EncryptedEnvelope, EncryptionError> {
    if serialized.len() > MAXIMUM_ENVELOPE_BYTES {
        return Err(EncryptionError::MalformedEnvelope);
    }

    let text = std::str::from_utf8(serialized).map_err(|_| {
        if serialized.starts_with(MAGIC_PREFIX.as_bytes()) {
            EncryptionError::MalformedEnvelope
        } else {
            EncryptionError::NotEncryptedDocument
        }
    })?;
    if !text.starts_with(MAGIC_PREFIX) {
        return Err(EncryptionError::NotEncryptedDocument);
    }
    if !text.starts_with(MAGIC_LINE) {
        return Err(EncryptionError::UnsupportedVersion);
    }

    let lines: Vec<&str> = text.split('\n').collect();
    validate_structure(&lines)?;

    let document_id_text = parse_field(lines[1], DOCUMENT_ID_FIELD)?;
    let document_id =
        Uuid::parse_str(document_id_text).map_err(|_| EncryptionError::MalformedEnvelope)?;
    if document_id.to_string() != document_id_text
        || document_id.get_version() != Some(Version::Random)
        || document_id.get_variant() != Variant::RFC4122
    {
        return Err(EncryptionError::MalformedEnvelope);
    }

    if parse_field(lines[2], KDF_PROFILE_FIELD)? != KDF_PROFILE {
        return Err(EncryptionError::UnsupportedKdfProfile);
    }

    let salt = decode_array(parse_field(lines[3], SALT_FIELD)?)?;
    let key_nonce = decode_array(parse_field(lines[4], KEY_NONCE_FIELD)?)?;
    let wrapped_key = decode_array(parse_field(lines[5], WRAPPED_KEY_FIELD)?)?;
    let content_nonce = decode_array(parse_field(lines[6], CONTENT_NONCE_FIELD)?)?;
    let ciphertext = decode_payload(&lines[10..lines.len() - 2])?;

    if ciphertext.len() < AUTHENTICATION_TAG_LENGTH {
        return Err(EncryptionError::MalformedEnvelope);
    }

    Ok(EncryptedEnvelope { document_id, salt, key_nonce, wrapped_key, content_nonce, ciphertext })
}

pub(crate) fn serialize_envelope(envelope: &EncryptedEnvelope) -> Vec<u8> {
    let salt = URL_SAFE_NO_PAD.encode(envelope.salt);
    let key_nonce = URL_SAFE_NO_PAD.encode(envelope.key_nonce);
    let wrapped_key = URL_SAFE_NO_PAD.encode(envelope.wrapped_key);
    let content_nonce = URL_SAFE_NO_PAD.encode(envelope.content_nonce);
    let ciphertext = URL_SAFE_NO_PAD.encode(&envelope.ciphertext);
    let mut serialized = String::with_capacity(ciphertext.len() + 384);

    serialized.push_str(MAGIC_LINE);
    serialized.push('\n');
    push_field(&mut serialized, DOCUMENT_ID_FIELD, &envelope.document_id.to_string());
    push_field(&mut serialized, KDF_PROFILE_FIELD, KDF_PROFILE);
    push_field(&mut serialized, SALT_FIELD, &salt);
    push_field(&mut serialized, KEY_NONCE_FIELD, &key_nonce);
    push_field(&mut serialized, WRAPPED_KEY_FIELD, &wrapped_key);
    push_field(&mut serialized, CONTENT_NONCE_FIELD, &content_nonce);
    serialized.push_str(HEADER_END);
    serialized.push_str("\n\n");
    serialized.push_str(PAYLOAD_FENCE_OPEN);
    serialized.push('\n');
    for chunk in ciphertext.as_bytes().chunks(PAYLOAD_LINE_WIDTH) {
        serialized.push_str(
            std::str::from_utf8(chunk)
                .expect("base64url encoder output must contain only valid ASCII bytes"),
        );
        serialized.push('\n');
    }
    serialized.push_str(PAYLOAD_FENCE_CLOSE);
    serialized.push('\n');

    serialized.into_bytes()
}

fn validate_structure(lines: &[&str]) -> Result<(), EncryptionError> {
    if lines.len() < MINIMUM_LINE_COUNT
        || lines[0] != MAGIC_LINE
        || lines[7] != HEADER_END
        || !lines[8].is_empty()
        || lines[9] != PAYLOAD_FENCE_OPEN
        || lines[lines.len() - 2] != PAYLOAD_FENCE_CLOSE
        || !lines[lines.len() - 1].is_empty()
    {
        return Err(EncryptionError::MalformedEnvelope);
    }

    Ok(())
}

fn parse_field<'a>(line: &'a str, prefix: &str) -> Result<&'a str, EncryptionError> {
    line.strip_prefix(prefix)
        .filter(|value| !value.is_empty())
        .ok_or(EncryptionError::MalformedEnvelope)
}

fn decode_array<const LENGTH: usize>(encoded: &str) -> Result<[u8; LENGTH], EncryptionError> {
    let decoded = decode_canonical(encoded)?;
    decoded.try_into().map_err(|_| EncryptionError::MalformedEnvelope)
}

fn decode_payload(lines: &[&str]) -> Result<Vec<u8>, EncryptionError> {
    if lines.is_empty() {
        return Err(EncryptionError::MalformedEnvelope);
    }
    for (index, line) in lines.iter().enumerate() {
        let is_last = index + 1 == lines.len();
        let valid_length = if is_last {
            !line.is_empty() && line.len() <= PAYLOAD_LINE_WIDTH
        } else {
            line.len() == PAYLOAD_LINE_WIDTH
        };
        if !valid_length || !line.is_ascii() {
            return Err(EncryptionError::MalformedEnvelope);
        }
    }

    decode_canonical(&lines.concat())
}

fn decode_canonical(encoded: &str) -> Result<Vec<u8>, EncryptionError> {
    let decoded =
        URL_SAFE_NO_PAD.decode(encoded).map_err(|_| EncryptionError::MalformedEnvelope)?;
    if URL_SAFE_NO_PAD.encode(&decoded) != encoded {
        return Err(EncryptionError::MalformedEnvelope);
    }

    Ok(decoded)
}

fn push_field(serialized: &mut String, field: &str, value: &str) {
    serialized.push_str(field);
    serialized.push_str(value);
    serialized.push('\n');
}
