# Plan — #648: `SmtpUsername` newtype for the SMTP relay auth identifier

Spec:
[`2026-07-24-issue-648-smtp-username.md`](../specs/2026-07-24-issue-648-smtp-username.md).
The plan is "how"; the spec is "what/why" (and the AC list).

## Review header

**Goal.** Type the SMTP auth **username** as a non-secret `SmtpUsername` newtype
(the pair #586 left half-typed), read symmetrically via the sqlx bridge, and
generalize the load error to a shared valueless `InvalidCredential`.

**Scope.**

- _In:_ `common::smtp_username::SmtpUsername` (non-secret default `StrNewtype`,
  non-empty); `parse_smtp_username` helper;
  `SmtpConfig`/`SmtpCredentials.username: Option<SmtpUsername>`;
  `get_smtp_credentials` decodes the username via the bridge; `InvalidPassword`
  → valueless `InvalidCredential`; the mailer expose.
- _Out:_ `Proffered*`/serde twin, `#[server]` setter, web UI (→ #638); other
  `SmtpConfig` fields.

**Tasks.**

- [x] 1. `common`: `SmtpUsername` newtype (module + non-empty `FromStr` +
     trailer tests) + `lib.rs` registration. (`parse_smtp_username` helper
     deferred to task 2, its first user — so it is not an uncovered region at
     this commit.)
- [x] 2. `storage` + `server`: thread `SmtpUsername` through `SmtpConfig` /
     `SmtpCredentials`; `get_smtp_credentials` decodes the username via the
     bridge; `InvalidPassword` → `InvalidCredential`; `InMemorySiteConfig`
     double; the mailer expose; `parse_smtp_username` helper; all test updates +
     empty-username test.
- [x] 3. Full gate: `cargo xtask validate --no-e2e` clean.

**Key risks / decisions.**

- A **default** `StrNewtype` emits the full trailer (Display/Deref/serde/sqlx/
  `PartialEq`); task 1's module tests exercise parse,
  `Display`+`PartialEq<&str>`, and a serde round-trip (mirroring
  `common::site::SiteTitle`) so no generated concrete impl is an uncovered
  region. (Generic sqlx `Encode`/`Type` stay uninstantiated → unmeasured, as
  with `SmtpPassword`.)
- `SmtpUsername` does **not** trim (matches the `SmtpPassword` sibling: exact
  value, reject only empty).
- Storage struct change ripples to the server consumer + fixtures → task 2's
  storage and server edits are **one commit**.
- The `InvalidPassword`→`InvalidCredential` rename is self-contained to
  `storage/src/smtp.rs` (variant def/doc/message, the `map_err`, its comment,
  and the `..._returns_err_for_empty_password` test).

**For agentic workers.** Execute with **jaunder-iterate**, delegating a task to
a subagent via **jaunder-dispatch** where useful. Tick checkboxes in real time.

## Global constraints

- **Newtype test helpers:** build `SmtpUsername` in tests via
  `common::test_support::parse_smtp_username(...)`.
- **Backend parity (ADR-0019):** the new empty-username decode test is
  dual-backend (`#[apply(backends)]`), like
  `get_smtp_credentials_rejects_empty_password`.
- **Coverage:** `common` is coverage-measured — task 1 covers the trailer
  surface; the username bridge `Decode` is exercised by the dual-backend getter
  tests.
- **Gate before commit:** `cargo xtask check` clean first (the pre-commit hook
  runs it); see **jaunder-commit**. **No `Co-Authored-By` trailer.**

---

## Task 1 — `common`: the `SmtpUsername` newtype

**Files.** New `common/src/smtp_username.rs`; `common/src/lib.rs` (module reg).

**Test (RED first).** In `common/src/smtp_username.rs` `#[cfg(test)]` (mirrors
`common/src/site.rs` `SiteTitle`):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_non_empty() {
        assert_eq!("user@example.com".parse::<SmtpUsername>().unwrap(), "user@example.com");
    }

    #[test]
    fn rejects_empty() {
        assert!("".parse::<SmtpUsername>().is_err());
        assert_eq!(
            "".parse::<SmtpUsername>().unwrap_err().to_string(),
            "SMTP username must not be empty"
        );
    }

    #[test]
    fn display_and_partial_eq_str() {
        let u: SmtpUsername = "relay-user".parse().unwrap();
        assert_eq!(u.to_string(), "relay-user");
        assert_eq!(u, "relay-user");
    }

    #[test]
    fn serde_round_trips_as_plain_string_and_validates() {
        let u: SmtpUsername = "relay-user".parse().unwrap();
        assert_eq!(serde_json::to_string(&u).unwrap(), "\"relay-user\"");
        assert_eq!(
            serde_json::from_str::<SmtpUsername>("\"relay-user\"").unwrap(),
            "relay-user".parse::<SmtpUsername>().unwrap()
        );
        assert!(serde_json::from_str::<SmtpUsername>("\"\"").is_err());
    }
}
```

`cargo nextest run -p common smtp_username` → **FAIL** (module absent).

**Implement.**

```rust
// common/src/smtp_username.rs
use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// A validated, non-empty SMTP relay auth username.
///
/// An **identifier, not a secret**, so it adopts the full default [`StrNewtype`]
/// trailer (`Display`, `AsRef<str>`, `Deref<str>`, serde, the validating #438 sqlx
/// bridge, `PartialEq`, owned-`String` conversions). Its sole invariant is
/// non-emptiness — an empty username paired with a set password is a
/// misconfiguration. The paired secret is `SmtpPassword`; making both typed keeps
/// `SmtpCredentials` a fully-typed pair (no same-typed transposition at the lettre
/// `Credentials` boundary). No trim — the stored value is used verbatim, matching
/// `SmtpPassword`. Web-settable wiring (a typed wire arg, non-secret) is #638.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct SmtpUsername(String);

/// Error returned when an SMTP username fails its shape invariant (empty).
#[derive(Debug, Error)]
#[error("SMTP username must not be empty")]
pub struct InvalidSmtpUsername;

impl FromStr for SmtpUsername {
    type Err = InvalidSmtpUsername;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(InvalidSmtpUsername);
        }
        Ok(SmtpUsername(s.to_owned()))
    }
}
```

- `common/src/lib.rs`: add `pub mod smtp_username;` (alphabetical — after
  `pub mod smtp_password;`).

**Verify.** `cargo nextest run -p common smtp_username` → **PASS**.
`cargo clippy -p common --all-targets -- -D warnings` clean.

**Commit** (jaunder-commit): `feat(common): SmtpUsername newtype (#648)`.

---

## Task 2 — `storage` + `server`: thread `SmtpUsername` + shared error

**Files.** `storage/src/smtp.rs` (structs, error, load);
`storage/src/site_config.rs` (getter + bound + tests);
`storage/src/test_support.rs` (double); `server/src/mailer/smtp.rs` (expose +
fixtures); `common/src/test_support.rs` (helper).

**Test (RED first).**

1. `common/src/test_support.rs`: add the helper (mirrors `parse_smtp_password`):
   ```rust
   #[must_use]
   pub fn parse_smtp_username(s: &str) -> SmtpUsername {
       s.parse().expect("valid test SMTP username")
   }
   ```
   with `use crate::smtp_username::SmtpUsername;`.
2. `storage/src/site_config.rs`: add a dual-backend empty-username reject test
   (sibling of `get_smtp_credentials_rejects_empty_password`):
   ```rust
   #[apply(backends)]
   #[tokio::test]
   async fn get_smtp_credentials_rejects_empty_username(#[case] backend: Backend) {
       let env = backend.setup().await;
       let storage = &*env.state.site_config;
       storage.set("smtp.username", "").await.unwrap();
       let err = storage.get_smtp_credentials().await.unwrap_err();
       assert!(matches!(err, sqlx::Error::ColumnDecode { .. }), "got: {err:?}");
   }
   ```
   and retype the username assert in `get_smtp_credentials_reads_typed_pair`
   (`site_config.rs:498`) to `creds.username.map(...)`/`.expect(...).as_ref()`.
3. `storage/src/smtp.rs`: rename the empty-password test's expected variant to
   `SmtpConfigError::InvalidCredential`, and add an empty-username load case
   asserting the same. Retype the `smtp.rs` username asserts — both the
   `load_smtp_config_returns_some_with_all_keys_present` assert and the
   `smtp_config_debug_redacts_password` fixture's `username: Some(...)` (→
   `parse_smtp_username(...)`; both are compiler-forced by the field type
   change).

`cargo nextest run -p storage smtp` / `-p common smtp_username` → **FAIL**
(fields still `String`; `InvalidCredential`/helper absent).

**Implement — `common`.** (Helper added in the RED step above.)

**Implement — `storage/src/smtp.rs`.**

- `use common::smtp_username::SmtpUsername;`.
- `SmtpConfig.username` and `SmtpCredentials.username`: `Option<SmtpUsername>`;
  update the `SmtpCredentials` doc (username now typed).
- Rename `SmtpConfigError::InvalidPassword` → `InvalidCredential`; message →
  `#[error("smtp.username or smtp.password holds an invalid value")]`; update
  the variant doc + the `load_smtp_config` rationale comment (drop
  "password"-specific wording). `load_smtp_config`'s
  `.map_err(|_| SmtpConfigError::InvalidCredential)`.

**Implement — `storage/src/site_config.rs`.**

- `use common::smtp_username::SmtpUsername;`.
- Add `(SmtpUsername,): for<'r> sqlx::FromRow<'r, DB::Row>` to the generic impl
  where-clause (mirrors the `(SmtpPassword,)` bound).
- In `get_smtp_credentials`, decode the username via the bridge (replacing
  `self.get("smtp.username")`):
  ```rust
  let username = sqlx::query_as::<_, (SmtpUsername,)>(
      "SELECT value FROM site_config WHERE key = $1",
  )
  .bind("smtp.username")
  .fetch_optional(&self.pool)
  .await?
  .map(|(username,)| username);
  ```

**Implement — `storage/src/test_support.rs`.**
`InMemorySiteConfig::get_smtp_credentials` parses the username too, mapping a
reject to a decode error (mirroring the password):

```rust
let username = self
    .get("smtp.username")
    .await?
    .map(|v| v.parse::<common::smtp_username::SmtpUsername>())
    .transpose()
    .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
```

**Implement — `server/src/mailer/smtp.rs`.**

- Expose (line ~68):
  `Credentials::new(username.as_ref().to_owned(), password.as_ref().to_owned())`.
- Test fixtures building `SmtpConfig` with a username move to
  `parse_smtp_username(...)`; add
  `use common::test_support::parse_smtp_username;`.

**Verify.**

- `cargo nextest run -p common smtp_username` → **PASS**.
- `cargo nextest run -p storage smtp` → **PASS** (both backends;
  empty-username + empty-password reject, typed-pair read).
- `cargo nextest run -p jaunder mailer::smtp` → **PASS**.
- `cargo check -p storage -p jaunder --all-targets`; `cargo clippy` clean.

**Commit** (jaunder-commit):
`refactor(storage): type SMTP username as SmtpUsername (#648)`.

---

## Task 3 — Full local gate

Run `cargo xtask validate --no-e2e` (foreground, `timeout: 600000`). Resolve any
clippy/coverage findings; `cargo fmt` if the hook reflowed anything. No web/e2e
surface touched, so `--no-e2e` is the gate; **jaunder-ship** runs full validate.

**Done when:** `validate --no-e2e` green and the tree clean.
