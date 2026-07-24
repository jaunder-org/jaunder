# Spec — #586: secret `SmtpPassword` newtype for the SMTP relay credential

Milestone #13 (Domain-value type safety). Applies ADR-0063 §2 (the secret
string-newtype exception) and the established `#410` `Password` template. **No
new ADR** — mechanical application of the existing secret flavor.

**Security-adjacent (credential handling): full review regardless of diff
size.**

## Problem

The SMTP relay password is a plaintext `Option<String>` on a struct that derives
plain `Debug`:

- `storage/src/smtp.rs:57-70` —
  `#[derive(Clone, Debug)] pub struct SmtpConfig { … pub password: Option<String>, … }`,
  loaded from config KV at `storage/src/smtp.rs:132`.
- `server/src/mailer/smtp.rs:66` — consumed via
  `Credentials::new(username, password)`.

Nothing `{:?}`-formats `SmtpConfig` today, but the derived `Debug` means the
relay credential is one `debug!(?config)` from a log line — the exposure the
`#[str_newtype(secret)]` flavor exists to close. It is the one remaining
plaintext credential held as a bare `String`.

> **Post-review update (design revised).** Code review reshaped Decisions 1 & 4:
> `SmtpPassword` is a **stored secret** (`#[str_newtype(secret, sqlx)]`, like
> `InviteCode`), and the credentials are read through a typed
> `SiteConfigStorage::get_smtp_credentials()` that decodes `smtp.password` via the
> sqlx bridge — so an empty/garbage value is rejected as a `ColumnDecode` error at
> the query boundary rather than by a hand-rolled parse. The sections below reflect
> the final design.

## Decisions

1. **`SmtpPassword` secret *stored-secret* newtype in `common`.** New module
   `common/src/smtp_password.rs` (mirrors `common/src/password.rs`):

   ```rust
   #[derive(Clone, StrNewtype)]
   #[str_newtype(secret, sqlx)]
   pub struct SmtpPassword(String);
   ```

   Hand-written validating `FromStr` rejecting the empty string (→
   `InvalidSmtpPassword`) via a shared `validate_smtp_password_shape(&str)` free
   fn (mirrors `common::password::validate_password_shape`), so the future #638
   `ProfferedSmtpPassword` twin can delegate to the same invariant without
   drift. The `secret` trailer supplies the redacting `Debug`
   (`SmtpPassword([redacted])`), `AsRef<str>` (the sole read-out), and
   `TryFrom<String>`; deliberately **no** `Display`, `Deref`, `Borrow`, serde,
   owned-`String` conversion, or `PartialEq`. The `sqlx` opt-in adds **only** the
   validating sqlx bridge (like `InviteCode`): the `site_config` value column
   decodes straight into `SmtpPassword` through `FromStr` (#438).

2. **No `Proffered*`/serde twin — yet.** SMTP config is set only via CLI /
   config-KV today (config KV → storage → mailer) and never crosses a
   `#[server]` wire surface, so the flavor is `secret` (not `secret, serde`).
   Adding an inbound twin now would be speculative — a `Proffered*` with no wire
   to travel on, and the `proffered-secret` gate (which pins `Proffered*` to
   `#[server]` params) would have nothing to pin. A twin-less secret does not
   trip that gate, so it stays satisfied. **When SMTP config becomes
   web-settable (#638), that work adds `ProfferedSmtpPassword`
   (`#[str_newtype(secret, serde)]`) + a shared non-empty validator +
   `TryFrom<ProfferedSmtpPassword> for SmtpPassword` + the `#[server]` setter
   and client validation** — the additive `Password`/`ProfferedPassword`
   pattern. To keep that future split drift-free, this issue puts the non-empty
   check in a small `validate_smtp_password_shape` free fn (mirroring
   `common::password::validate_password_shape`), which the future twin will
   share.

3. **`SmtpConfig.password: Option<SmtpPassword>`.** The derived struct `Debug`
   then redacts the password automatically. `username` stays `Option<String>`
   (an identifier, not a secret; audited config-KV carve-out).

4. **Typed credential retrieval; empty stored password is rejected at the bridge.**
   A new `SiteConfigStorage::get_smtp_credentials() -> sqlx::Result<SmtpCredentials>`
   reads `smtp.username` (plain `String`) and `smtp.password` **together** as a
   typed `SmtpCredentials { username, password }`. The password is decoded via
   `query_as::<_, (SmtpPassword,)>` on the value column, so a present-but-empty (or
   garbage) value fails `FromStr` and surfaces as a `sqlx::Error::ColumnDecode` at
   the query boundary — the #438 way, no hand-rolled parse. `load_smtp_config`
   destructures the pair and maps a decode failure to a **valueless**
   `SmtpConfigError::InvalidPassword` (never embeds the credential, unlike the
   value-echoing sibling variants). Absent key → `None` (unchanged).

5. **Single expose site.** `server/src/mailer/smtp.rs:66` becomes
   `Credentials::new(username.clone(), password.as_ref().to_owned())` — the one
   `.as_ref()` plaintext read, at the mailer credentials boundary. No other
   `Display`/serde/owned-`String` extraction anywhere.

## Acceptance criteria

- **AC1** `common::smtp_password::SmtpPassword` exists with the `secret` trailer
  and a validating `FromStr` that rejects the empty string; it has no
  `Display`/`Deref`/serde/`PartialEq`/owned-`String` surface (the `secret`
  flavor guarantees this — a regression test asserts `Debug` redacts).
- **AC2** `SmtpConfig.password` is `Option<SmtpPassword>`;
  `format!("{config:?}")` on a config carrying a password provably contains
  `SmtpPassword([redacted])` and **not** the secret value (regression test).
- **AC3** `load_smtp_config` returns `Ok` with `Some(SmtpPassword)` for a valid
  present password, `None` for an absent key, and
  `Err(SmtpConfigError::InvalidPassword)` for a present-but-empty value; the
  error carries no password value.
- **AC4** Exactly one plaintext read of `SmtpPassword` (`.as_ref()`), at the
  `Credentials::new` site. The mailer still authenticates only when both
  username and password are present (behavior unchanged for the valid case).
- **AC5** Validation/error paths covered (macros crate + `common` are
  coverage-measured): the empty-reject `FromStr` path and the `InvalidPassword`
  load path are exercised. Test `SmtpPassword` values build via
  `common::test_support::parse_smtp_password(...)` (newtype test-helper
  convention; the helper must be added to `common/src/test_support.rs`).
  `Option<SmtpPassword>` has no `PartialEq`, so the existing `assert_eq!`
  assertions are rewritten by case: the present case reads through `.as_ref()`
  (`config.password.as_ref().map(SmtpPassword::as_ref) == Some("s3cr3t")` at
  `storage/src/smtp.rs:229`), the absent case uses `.is_none()`
  (`storage/src/smtp.rs:250`).
- **AC6** `cargo xtask validate --no-e2e` clean (incl. the `proffered-secret`
  gate still green).

## Out of scope

- `username` stays `Option<String>` (identifier, not a secret).
- Any `#[server]`/wire exposure of `SmtpPassword`, the `ProfferedSmtpPassword`
  inbound twin, and a web setter UI — **deferred to #638** (blocked-by this
  issue).
- SMTP config schema/keys, TLS mode, sender, or mailer transport behavior.

## Testing

- `common`: `SmtpPassword` `FromStr` accepts non-empty / rejects empty; `Debug`
  redacts; `as_ref` round-trips. (`common/src/smtp_password.rs` `#[cfg(test)]`.)
- `storage`: `load_smtp_config` present/absent/empty cases (update the existing
  `smtp.rs` tests to build/assert via `SmtpPassword`); the `SmtpConfig` `Debug`
  redaction regression test.
- `server`: the `mailer/smtp.rs` test fixtures constructing `SmtpConfig` with a
  password move to `parse_smtp_password`; the authenticated-builder path stays
  green.
