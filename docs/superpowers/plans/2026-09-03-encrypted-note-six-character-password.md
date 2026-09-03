# Encrypted Note Six-Character Password Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow encrypted-note passwords containing at least six Unicode characters.

**Architecture:** `EncryptionPassword` remains the policy owner and exposes its minimum as an associated constant. Notora reuses that constant for localized pre-validation, while the pure UI dialog only updates its user-facing placeholder.

**Tech Stack:** Rust, Cargo unit tests, `textora-encryption`, `notora-app`, `textora-ui`.

## Global Constraints

- Count Unicode characters, not UTF-8 bytes.
- Accept six or more characters and reject one through five characters.
- Preserve encrypted-file compatibility and all cryptographic parameters.
- Keep `ui` independent from application state and the encryption crate.

---

### Task 1: Establish failing six-character policy tests

**Files:**
- Modify/Test: `crates/encryption/src/secret.rs`
- Modify/Test: `crates/notora-app/src/state.rs`

**Interfaces:**
- Consumes: `EncryptionPassword::new(String) -> Result<EncryptionPassword, PasswordPolicyError>` and `NotoraState::reduce(NotoraAction) -> Vec<NotoraEffect>`.
- Produces: regression coverage for the six-character boundary in the encryption and Notora layers.

- [x] **Step 1: Change the encryption boundary test before production code**

Use `"六位密码可用"` as the six-character accepted value and `"五位密码短"` as the five-character rejected value. Assert the rejection is `PasswordPolicyError::TooShort { minimum_characters: 6 }`.

- [x] **Step 2: Change existing Notora workflow tests before production code**

Use `"abc123"` in the successful encrypted-note creation, conflict-copy, and unlock workflow tests so each independent validation path is covered.

- [x] **Step 3: Verify RED**

Run:

```bash
cargo test -p textora-encryption password_policy_counts_unicode_characters
cargo test -p notora-app encrypted_note_creation_prevents_duplicate_submission
```

Expected: both fail because the current minimum is eight characters.

### Task 2: Centralize and apply the six-character policy

**Files:**
- Modify: `crates/encryption/src/secret.rs`
- Modify: `crates/notora-app/src/state.rs`
- Modify: `crates/ui/src/widgets/encrypted_note_dialog.rs`

**Interfaces:**
- Consumes: `SensitiveText::expose() -> &str`.
- Produces: `EncryptionPassword::MINIMUM_CHARACTERS: usize` with value `6`, plus consistent localized Notora validation.

- [x] **Step 1: Implement the core policy**

Add `pub const MINIMUM_CHARACTERS: usize = 6` to `EncryptionPassword` and make `new` use `Self::MINIMUM_CHARACTERS`.

- [x] **Step 2: Remove application-layer password-length magic values**

Add a focused helper that compares `password.expose().chars().count()` with `EncryptionPassword::MINIMUM_CHARACTERS` and formats `密码至少需要 6 个字符`. Reuse it from creation, conflict-copy confirmation, and unlock validation.

- [x] **Step 3: Update the dialog copy**

Change the password placeholder from `至少 8 个字符` to `至少 6 个字符`.

- [x] **Step 4: Verify GREEN and formatting**

Run:

```bash
cargo test -p textora-encryption
cargo test -p notora-app --lib encrypted_
cargo test -p textora-ui encrypted_note_dialog
cargo fmt --check
```

Expected: all commands exit successfully with zero failed tests and no formatting diff.

- [x] **Step 5: Review the final diff**

Run `git diff --check` and `git diff --stat`; confirm only the specification, plan, and three intended Rust files changed.
