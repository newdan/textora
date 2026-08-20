use uuid::Uuid;

use crate::EncryptionError;
use crate::crypto::{
    EncryptionRandomness, create_encrypted_markdown_with_randomness,
    encrypt_markdown_with_content_nonce,
};
use crate::format::{
    EncryptedEnvelope, inspect_encrypted_markdown, parse_envelope, serialize_envelope,
};
use crate::secret::{KEY_NONCE_LENGTH, SALT_LENGTH, WRAPPED_KEY_LENGTH};
use crate::{
    EncryptionPassword, create_encrypted_markdown, decrypt_markdown_with_session,
    unlock_encrypted_markdown,
};

const TEST_DOCUMENT_ID: &str = "c9b375e6-ccfe-40f2-ab74-7b705c10145d";
const FIXED_ENCRYPTED_VECTOR: &str = r#"<!-- textora-encrypted-markdown:v1
document-id=21212121-2121-4121-a121-212121212121
kdf-profile=argon2id-64m-t3-p1
salt=MTExMTExMTExMTExMTExMQ
key-nonce=UVFRUVFRUVFRUVFRUVFRUVFRUVFRUVFR
wrapped-key=z3tIYWKnhzFtbPlddnmuopWLMOVUMTu38F3bPpgxn1KIUz8YPHI8MrXaGhjNeXbA
content-nonce=YWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFh
-->

```textora-encrypted
1ZVSQET4JQH65hPcuoIbASY_F_ebD0wAB8UZuHGXzTd1n5o2kKbEB3HiQH7IAIYlbA
```
"#;

#[test]
fn canonical_envelope_round_trips() {
    let serialized = serialize_envelope(&sample_envelope());
    let parsed = parse_envelope(&serialized).expect("canonical envelope should parse");

    assert_eq!(serialize_envelope(&parsed), serialized);
    assert_eq!(
        inspect_encrypted_markdown(&serialized)
            .expect("canonical envelope should be inspectable")
            .document_id
            .to_string(),
        TEST_DOCUMENT_ID
    );
}

#[test]
fn ordinary_markdown_is_not_guessed_to_be_encrypted() {
    assert_eq!(
        inspect_encrypted_markdown(b"# ordinary\n\n```broken")
            .expect_err("ordinary Markdown must not parse as encrypted"),
        EncryptionError::NotEncryptedDocument
    );
}

#[test]
fn unsupported_version_has_a_stable_error() {
    let serialized = String::from_utf8(serialize_envelope(&sample_envelope()))
        .expect("test envelope should be UTF-8")
        .replacen(":v1\n", ":v2\n", 1);

    assert_eq!(
        inspect_encrypted_markdown(serialized.as_bytes()).expect_err("unknown version must fail"),
        EncryptionError::UnsupportedVersion
    );
}

#[test]
fn malformed_fields_and_noncanonical_base64_are_rejected() {
    let canonical = String::from_utf8(serialize_envelope(&sample_envelope()))
        .expect("test envelope should be UTF-8");
    let mutations = [
        canonical.replacen("document-id=", "unknown-field=", 1),
        canonical.replacen("salt=", "salt=AA", 1),
        canonical.replacen("key-nonce=", "salt=", 1),
        canonical.replacen("\n```\n", "\n```\ntrailing", 1),
        canonical.replacen("\n-->\n", "\nduplicate=value\n-->\n", 1),
    ];

    for mutation in mutations {
        assert_eq!(
            inspect_encrypted_markdown(mutation.as_bytes())
                .expect_err("malformed envelope must fail"),
            EncryptionError::MalformedEnvelope
        );
    }
}

#[test]
fn truncated_and_noncanonical_payload_wrapping_are_rejected() {
    let canonical = String::from_utf8(serialize_envelope(&sample_envelope()))
        .expect("test envelope should be UTF-8");
    let truncated = canonical.trim_end_matches("```\n");
    let split_payload = canonical.replacen(
        "ERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERER",
        "ERERERERER\nERERERERERERERERERERERERERERERERERERERERERERERERERERERER",
        1,
    );

    for mutation in [truncated.as_bytes(), split_payload.as_bytes()] {
        assert_eq!(
            inspect_encrypted_markdown(mutation).expect_err("invalid payload must fail"),
            EncryptionError::MalformedEnvelope
        );
    }
}

fn sample_envelope() -> EncryptedEnvelope {
    EncryptedEnvelope {
        document_id: Uuid::parse_str(TEST_DOCUMENT_ID)
            .expect("test document id should be a UUID v4"),
        salt: [0x0a; SALT_LENGTH],
        key_nonce: [0x0b; KEY_NONCE_LENGTH],
        wrapped_key: [0x0c; WRAPPED_KEY_LENGTH],
        content_nonce: [0x0d; 24],
        ciphertext: vec![0x11; 64],
    }
}

#[test]
fn fixed_crypto_vector_is_deterministic_and_unlocks() {
    let password = test_password();
    let plaintext = "# 私密标题\r\n\r\nemoji: 🔐\0end";
    let first = create_encrypted_markdown_with_randomness(
        &password,
        plaintext.as_bytes(),
        fixed_randomness(),
    )
    .expect("fixed encryption vector should be created");
    let second = create_encrypted_markdown_with_randomness(
        &password,
        plaintext.as_bytes(),
        fixed_randomness(),
    )
    .expect("same fixed encryption vector should be created");

    assert_eq!(first.serialized(), second.serialized());
    assert_eq!(first.serialized(), FIXED_ENCRYPTED_VECTOR.as_bytes());
    assert!(
        !first
            .serialized()
            .windows("私密标题".len())
            .any(|window| { window == "私密标题".as_bytes() })
    );
    let unlocked = unlock_encrypted_markdown(first.serialized(), &password)
        .expect("fixed vector should unlock");
    assert_eq!(unlocked.plaintext(), plaintext);
}

#[test]
fn wrong_password_is_rejected_without_sensitive_output() {
    let created = create_encrypted_markdown_with_randomness(
        &test_password(),
        b"private body",
        fixed_randomness(),
    )
    .expect("encrypted document should be created");
    let wrong_password = EncryptionPassword::new("wrong-password".to_owned())
        .expect("wrong test password should satisfy policy");
    let error = unlock_encrypted_markdown(created.serialized(), &wrong_password)
        .expect_err("wrong password must fail");

    assert_eq!(error, EncryptionError::PasswordRejected);
    assert!(!format!("{error:?}: {error}").contains("wrong-password"));
}

#[test]
fn every_authenticated_envelope_component_rejects_single_byte_tampering() {
    let password = test_password();
    let created = create_encrypted_markdown_with_randomness(
        &password,
        b"authenticated body",
        fixed_randomness(),
    )
    .expect("encrypted document should be created");
    let envelope = parse_envelope(created.serialized()).expect("created envelope should parse");

    for mutation in 0..6 {
        let mut tampered = copy_envelope(&envelope);
        let expected = match mutation {
            0 => {
                let mut document_id = *tampered.document_id.as_bytes();
                document_id[15] ^= 1;
                tampered.document_id = Uuid::from_bytes(document_id);
                EncryptionError::PasswordRejected
            }
            1 => {
                tampered.salt[0] ^= 1;
                EncryptionError::PasswordRejected
            }
            2 => {
                tampered.key_nonce[0] ^= 1;
                EncryptionError::PasswordRejected
            }
            3 => {
                tampered.wrapped_key[0] ^= 1;
                EncryptionError::PasswordRejected
            }
            4 => {
                tampered.content_nonce[0] ^= 1;
                EncryptionError::AuthenticationFailed
            }
            5 => {
                tampered.ciphertext[0] ^= 1;
                EncryptionError::AuthenticationFailed
            }
            _ => unreachable!("mutation range is fixed to authenticated fields"),
        };

        assert_eq!(
            unlock_encrypted_markdown(&serialize_envelope(&tampered), &password)
                .expect_err("tampered envelope must fail authentication"),
            expected
        );
    }
}

#[test]
fn session_save_uses_a_fresh_content_nonce_and_preserves_document_identity() {
    let password = test_password();
    let created =
        create_encrypted_markdown_with_randomness(&password, b"initial", fixed_randomness())
            .expect("encrypted document should be created");
    let (_, session) = created.into_parts();
    let first = encrypt_markdown_with_content_nonce(&session, b"changed", [0x71; 24])
        .expect("first session save should encrypt");
    let second = encrypt_markdown_with_content_nonce(&session, b"changed", [0x72; 24])
        .expect("second session save should encrypt");
    let first_envelope = parse_envelope(&first).expect("first saved envelope should parse");
    let second_envelope = parse_envelope(&second).expect("second saved envelope should parse");

    assert_ne!(first, second);
    assert_eq!(first_envelope.document_id, second_envelope.document_id);
    assert_eq!(first_envelope.wrapped_key, second_envelope.wrapped_key);
    assert_eq!(
        unlock_encrypted_markdown(&first, &password)
            .expect("first saved envelope should unlock")
            .plaintext(),
        "changed"
    );
    assert_eq!(
        unlock_encrypted_markdown(&second, &password)
            .expect("second saved envelope should unlock")
            .plaintext(),
        "changed"
    );
}

#[test]
fn a_different_encrypted_document_is_classified_as_a_session_mismatch() {
    let password = EncryptionPassword::new("session-mismatch-password".to_owned())
        .expect("test password should satisfy policy");
    let first = create_encrypted_markdown(&password, b"first")
        .expect("first encrypted fixture should be created");
    let second = create_encrypted_markdown(&password, b"second")
        .expect("second encrypted fixture should be created");
    let (first_serialized, first_session) = first.into_parts();
    let (second_serialized, _) = second.into_parts();

    assert_eq!(
        decrypt_markdown_with_session(&second_serialized, &first_session),
        Err(EncryptionError::SessionMismatch)
    );
    assert_eq!(
        decrypt_markdown_with_session(&first_serialized, &first_session)
            .expect("matching session should still decrypt"),
        "first"
    );
}

#[test]
fn invalid_utf8_plaintext_is_rejected_before_encryption() {
    assert_eq!(
        create_encrypted_markdown_with_randomness(
            &test_password(),
            &[0xff, 0xfe],
            fixed_randomness(),
        )
        .expect_err("invalid UTF-8 must fail"),
        EncryptionError::InvalidUtf8Payload
    );
}

fn test_password() -> EncryptionPassword {
    EncryptionPassword::new("correct horse battery staple".to_owned())
        .expect("test password should satisfy policy")
}

fn fixed_randomness() -> EncryptionRandomness {
    EncryptionRandomness {
        document_id: [0x21; 16],
        salt: [0x31; SALT_LENGTH],
        data_key: [0x41; 32],
        key_nonce: [0x51; KEY_NONCE_LENGTH],
        content_nonce: [0x61; 24],
    }
}

fn copy_envelope(envelope: &EncryptedEnvelope) -> EncryptedEnvelope {
    EncryptedEnvelope {
        document_id: envelope.document_id,
        salt: envelope.salt,
        key_nonce: envelope.key_nonce,
        wrapped_key: envelope.wrapped_key,
        content_nonce: envelope.content_nonce,
        ciphertext: envelope.ciphertext.clone(),
    }
}
