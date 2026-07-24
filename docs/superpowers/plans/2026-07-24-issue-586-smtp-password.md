# Plan — #586: secret `SmtpPassword` newtype for the SMTP relay credential

Spec:
[`2026-07-24-issue-586-smtp-password.md`](../specs/2026-07-24-issue-586-smtp-password.md).
The plan is "how"; the spec is "what/why" (and the AC list).
**Security-adjacent: full review at ship.**

## Review header

**Goal.** Close the last plaintext-`String` credential: give the SMTP relay
password the ADR-0063 §2 secret newtype (`SmtpPassword`), thread it through
`SmtpConfig`, and expose the plaintext at exactly one audited site.

**Scope.**

- _In:_ `common::smtp_password::SmtpPassword` (`#[str_newtype(secret)]` + shared
  non-empty validator); `parse_smtp_password` test helper;
  `SmtpConfig.password: Option<SmtpPassword>`; `load_smtp_config` empty→`Err`;
  the single `.as_ref()` expose at `Credentials::new`; redaction regression
  test.
- _Out:_ `ProfferedSmtpPassword` twin / serde / `#[server]` setter / web UI
  (deferred to **#638**, blocked-by this); `username` stays `Option<String>`;
  SMTP schema/TLS/sender/transport.

**Tasks.**

- [x] 1. `common`: `SmtpPassword` secret newtype (module, shared
     `validate_smtp_password_shape`, `FromStr`, tests) + `lib.rs` registration.
     (`parse_smtp_password` helper deferred to task 2 — its first user — so it
     is not an uncovered region at this commit.)
- [ ] 2. `storage` + `server`: `SmtpConfig.password: Option<SmtpPassword>`,
     `load_smtp_config` empty→`InvalidPassword`, the `Credentials::new` expose,
     the `parse_smtp_password` test helper (first used here), and all
     `SmtpConfig` test fixtures/asserts (redaction regression, load cases).
- [ ] 3. Full gate: `cargo xtask validate --no-e2e` clean (incl.
     `proffered-secret`).

**Key risks / decisions.**

- `Option<SmtpPassword>` has **no `PartialEq`** (secret trailer omits it): the
  two existing `assert_eq!` on `config.password` are rewritten by case — present
  → `.as_ref().map(SmtpPassword::as_ref)`, absent → `.is_none()`. `SmtpConfig`
  itself derives only `Clone, Debug` (no `PartialEq`), so the field type change
  breaks no derive.
- `SmtpConfigError::InvalidPassword` is **valueless** (never embed the
  credential) — deliberately asymmetric with the value-echoing
  `InvalidPort`/`InvalidTlsMode`/`InvalidSender` siblings.
- Storage struct change ripples to the server consumer + fixtures → tasks 2's
  storage and server edits are **one commit** (tree won't compile split).
- Follow-up #638 already filed (no plan task needed); the shared validator is
  the only forward-accommodation.

**For agentic workers.** Execute with **jaunder-iterate**, delegating a task to
a subagent via **jaunder-dispatch** where useful. Tick checkboxes in real time.

## Global constraints

- **Secret hygiene:** no `Display`/serde/`Deref`/owned-`String`/`PartialEq` on
  `SmtpPassword` (the `secret` trailer guarantees this); the only plaintext read
  is one `.as_ref()`. The error carries no value.
- **Newtype test helpers:** build `SmtpPassword` in tests via
  `common::test_support::parse_smtp_password(...)`, never `.parse().unwrap()` at
  the call site.
- **Coverage:** `common` + macros are coverage-measured — cover the empty-reject
  `FromStr` path and the `InvalidPassword` load path.
- **Gate before commit:** `cargo xtask check` clean first (the pre-commit hook
  runs the full `cargo xtask check`); see **jaunder-commit**. **No
  `Co-Authored-By` trailer.**

---

## Task 1 — `common`: the `SmtpPassword` secret newtype

**Files.** New `common/src/smtp_password.rs`; `common/src/lib.rs` (module reg);
`common/src/test_support.rs` (helper).

**Test (RED first).** In `common/src/smtp_password.rs` `#[cfg(test)]` (mirrors
`common/src/password.rs` tests):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_non_empty() {
        assert!("s3cr3t".parse::<SmtpPassword>().is_ok());
    }

    #[test]
    fn rejects_empty() {
        assert!("".parse::<SmtpPassword>().is_err());
    }

    #[test]
    fn debug_is_redacted() {
        let raw = "s3cr3t-relay-pw";
        let p: SmtpPassword = raw.parse().unwrap();
        let out = format!("{p:?}");
        assert!(!out.contains(raw));
        assert_eq!(out, "SmtpPassword([redacted])");
    }

    #[test]
    fn as_ref_returns_original_value() {
        let raw = "correct horse relay";
        let p: SmtpPassword = raw.parse().unwrap();
        assert_eq!(p.as_ref(), raw);
    }
}
```

`cargo nextest run -p common smtp_password` → **FAIL** (module absent).

**Implement.**

```rust
// common/src/smtp_password.rs
use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// A validated, non-empty SMTP relay password.
///
/// Adopts the [`StrNewtype`] `secret` surface (ADR-0063 §2): a redacting `Debug`
/// and borrowed `AsRef<str>` access for the lettre `Credentials`, with no
/// `Display`, serde, `Deref`, owned-`String`, or `PartialEq` — so the relay
/// credential cannot be rendered, serialised, logged, or value-compared. The
/// `macros` crate is the authoritative list of what `secret` emits.
///
/// SMTP config is server-side only today (CLI / config-KV → storage → mailer), so
/// there is no inbound `ProfferedSmtpPassword` twin. Making it web-settable (#638)
/// will add that twin and share [`validate_smtp_password_shape`].
#[derive(Clone, StrNewtype)]
#[str_newtype(secret)]
pub struct SmtpPassword(String);

/// Error returned when an SMTP password fails its shape invariant (empty).
#[derive(Debug, Error)]
#[error("SMTP password must not be empty")]
pub struct InvalidSmtpPassword;

impl FromStr for SmtpPassword {
    type Err = InvalidSmtpPassword;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_smtp_password_shape(s)?;
        Ok(SmtpPassword(s.to_owned()))
    }
}

/// The shared shape invariant for an SMTP relay password: non-empty. Kept as a
/// free fn so the future `ProfferedSmtpPassword` inbound twin (#638) delegates to
/// the same invariant and cannot drift (mirrors
/// `common::password::validate_password_shape`).
fn validate_smtp_password_shape(s: &str) -> Result<(), InvalidSmtpPassword> {
    if s.is_empty() {
        return Err(InvalidSmtpPassword);
    }
    Ok(())
}
```

- `common/src/lib.rs`: add `pub mod smtp_password;` (alphabetical among the
  sibling `pub mod` lines).
- `common/src/test_support.rs`: add `use crate::smtp_password::SmtpPassword;`
  and, mirroring `parse_password` (line 348):

  ```rust
  /// Parse `s` into a valid [`SmtpPassword`] for tests — the single place a test
  /// SMTP-password literal is parsed.
  ///
  /// # Panics
  ///
  /// Panics if `s` is empty.
  #[must_use]
  pub fn parse_smtp_password(s: &str) -> SmtpPassword {
      s.parse().expect("valid test SMTP password")
  }
  ```

**Verify.** `cargo nextest run -p common smtp_password` → **PASS**.
`cargo clippy -p common --all-targets -- -D warnings` clean.

**Commit** (jaunder-commit): `feat(common): secret SmtpPassword newtype (#586)`.

---

## Task 2 — `storage` + `server`: thread `SmtpPassword` through `SmtpConfig`

**Files.** `storage/src/smtp.rs` (struct, load, error, tests);
`server/src/mailer/smtp.rs` (consumer + test fixtures).

**Test (RED first).** In `storage/src/smtp.rs` tests:

1. Rewrite the two existing asserts:
   - line 229 →
     `assert_eq!(config.password.as_ref().map(SmtpPassword::as_ref), Some("s3cr3t"));`
   - line 250 → `assert!(config.password.is_none());`
2. Add the empty-reject load case:
   ```rust
   // guard:no-backend — reads SMTP config from an injected mock SiteConfigStorage; no live database backend
   #[tokio::test]
   async fn load_smtp_config_returns_err_for_empty_password() {
       let store = InMemorySiteConfig::from_pairs([
           ("smtp.host", "mail.example.com"),
           ("smtp.password", ""),
       ]);
       let err = load_smtp_config(&store).await.unwrap_err();
       assert!(matches!(err, SmtpConfigError::InvalidPassword));
   }
   ```
3. Add the redaction regression (AC2):
   ```rust
   #[test]
   fn smtp_config_debug_redacts_password() {
       use common::test_support::parse_smtp_password;
       let config = SmtpConfig {
           host: "mail.example.com".to_owned(),
           port: 587,
           tls_mode: SmtpTlsMode::StartTls,
           username: Some("user@example.com".to_owned()),
           password: Some(parse_smtp_password("s3cr3t")),
           sender: "Jaunder <noreply@example.com>".parse::<Mailbox>().unwrap(),
       };
       let out = format!("{config:?}");
       assert!(out.contains("SmtpPassword([redacted])"));
       assert!(!out.contains("s3cr3t"));
       // A cloned config also redacts — and this exercises the mandatory
       // `SmtpPassword::clone` derive so it isn't an uncovered llvm-cov region.
       let cloned = format!("{:?}", config.clone());
       assert!(cloned.contains("SmtpPassword([redacted])"));
       assert!(!cloned.contains("s3cr3t"));
   }
   ```

`cargo nextest run -p jaunder --lib smtp` (storage lib tests run under the
server package? no — storage tests: `cargo nextest run -p storage smtp`) →
**FAIL** (field still `String`; `SmtpConfigError::InvalidPassword` absent).

**Implement — storage (`storage/src/smtp.rs`).**

- `use common::smtp_password::SmtpPassword;` (top).
- Struct field (line 67): `pub password: Option<SmtpPassword>,`.
- Add error variant to `SmtpConfigError`:
  ```rust
  /// `smtp.password` is present but empty. Valueless — the credential is never
  /// embedded in an error (unlike the sibling variants).
  #[error("smtp.password must not be empty")]
  InvalidPassword,
  ```
- Load (line 132): replace the bare `.ok().flatten()` with a parsing match
  (mirrors `InvalidPort`/`InvalidTlsMode`):
  ```rust
  let password = match store.get("smtp.password").await.ok().flatten() {
      None => None,
      Some(v) => Some(v.parse::<SmtpPassword>().map_err(|_| SmtpConfigError::InvalidPassword)?),
  };
  ```
  The `test_support::parse_smtp_password` import goes in the test module only;
  the production `.parse()` uses `FromStr` directly.

**Implement — server (`server/src/mailer/smtp.rs`).**

- Consumer (line 66):
  `Credentials::new(username.clone(), password.as_ref().to_owned())` (`password`
  is `&SmtpPassword` here; `.as_ref()` → `&str`, `.to_owned()` → `String` for
  lettre). This is the sole plaintext read (AC4).
- Test fixtures: the `SmtpConfig { … password: Some("s3cr3t".to_owned()) … }` at
  line 158 (`from_config_with_credentials_succeeds`) →
  `password: Some(parse_smtp_password("s3cr3t"))`. It is the **only** `Some`
  fixture; the rest are `None`/absent (base_config:132, only-username:169,
  transport-error:185, and the sender-reject fixture at :222 sets no password
  via `..base_config`) and need no change. Add
  `use common::test_support::parse_smtp_password;` to the test module.

**Verify.**

- `cargo nextest run -p storage smtp` → **PASS** (incl. empty-reject +
  redaction).
- `cargo nextest run -p jaunder mailer::smtp` → **PASS** (authenticated-builder
  path intact).
- `cargo check -p storage -p jaunder --all-targets` compiles.
- `cargo clippy -p storage -p jaunder --all-targets -- -D warnings` clean.

**Commit** (jaunder-commit):
`refactor(storage): type SmtpConfig.password as SmtpPassword (#586)`.

---

## Task 3 — Full local gate

Run `cargo xtask validate --no-e2e` (foreground, `timeout: 600000`). Confirm the
`proffered-secret` gate stays green (no `Proffered*` added). Resolve any clippy
/ coverage findings; `cargo fmt` if the hook reflowed anything
(`git status --porcelain` after green). No web/e2e surface touched, so
`--no-e2e` is the gate; **jaunder-ship** runs the full validate.

**Done when:** `validate --no-e2e` green and the tree clean (AC6).
