# Spec — #648: `SmtpUsername` newtype for the SMTP relay auth identifier

Milestone #13 (Domain-value type safety). Follow-up from #586, which typed the
SMTP **password** (`SmtpPassword`) but left the paired **username** a raw
`Option<String>`. Applies the ADR-0063 `StrNewtype` trailer, the #438 sqlx
bridge, and the `SmtpCredentials` structure #586 introduced. **No new ADR.**

## Problem

`storage/src/smtp.rs` carries `SmtpConfig.username: Option<String>` and
`SmtpCredentials.username: Option<String>`; `get_smtp_credentials`
(`storage/src/site_config.rs`) reads `smtp.username` via the generic
`get(key) -> Option<String>` while the password decodes typed through its
bridge. The username is an SMTP auth identifier with at least a non-empty
invariant (an empty username paired with a set password is a misconfiguration),
and typing it makes `SmtpCredentials` a fully-typed pair instead of half-typed.

## Decisions

1. **`SmtpUsername` newtype in `common`.** New module
   `common/src/smtp_username.rs`:

   ```rust
   #[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
   pub struct SmtpUsername(String);
   ```

   A **non-secret** default `StrNewtype` (an identifier, not a credential — so
   the full default trailer: `Display`, `AsRef<str>`, `Deref<str>`, serde, the
   validating #438 sqlx bridge, `PartialEq`, owned-`String` conversions). A
   hand-written `FromStr` rejects the empty string (→ `InvalidSmtpUsername`);
   the non-empty check is inline (a single caller, unlike the password's shared
   validator, so no free fn).

2. **`SmtpConfig.username` / `SmtpCredentials.username: Option<SmtpUsername>`.**
   Both fields typed; the `SmtpCredentials` doc updated (username is now typed,
   no longer "stays a plain `String`").

3. **Symmetric bridge decode.** `get_smtp_credentials` decodes **both** columns
   via the sqlx bridge — `query_as::<_, (SmtpUsername,)>` for `smtp.username`
   alongside the existing `query_as::<_, (SmtpPassword,)>` for `smtp.password`.
   An empty/garbage stored **username** is rejected as a `ColumnDecode` error at
   the query boundary, exactly like the password. (Adds the
   `(SmtpUsername,): FromRow` bound to the generic `SiteConfigStore` impl.) The
   `InMemorySiteConfig` double parses the username too, mapping a reject to a
   decode error.

4. **Shared valueless error.** `SmtpConfigError::InvalidPassword` is renamed to
   `SmtpConfigError::InvalidCredential` — a **valueless** variant covering an
   invalid username **or** password (never embeds either value). Its
   `#[error(...)]` message becomes credential-neutral (e.g.
   `"smtp.username or smtp.password holds an invalid value"`), and the variant
   doc + the `load_smtp_config` rationale comment (which today reference
   "password") update to name the shared credential. `load_smtp_config` maps a
   `get_smtp_credentials` decode failure to it. This also sharpens #586's prior
   lossy label (it no longer claims "password" for a username failure). Rename
   scope is self-contained to `storage/src/smtp.rs` (variant def/doc/message,
   the `map_err`, and the `..._returns_err_for_empty_password` test).

5. **Single expose, symmetric.** `server/src/mailer/smtp.rs` builds
   `Credentials::new(username.as_ref().to_owned(), password.as_ref().to_owned())`
   — both borrowed via `AsRef<str>` and owned for lettre. `username` is
   non-secret, so this is a plain conversion (not a guarded expose), now
   symmetric with the password.

## Acceptance criteria

- **AC1** `common::smtp_username::SmtpUsername` exists as a default (non-secret)
  `StrNewtype` with a validating `FromStr` rejecting the empty string; the
  reject/accept paths are covered (`common` is coverage-measured).
- **AC2** `SmtpConfig.username` and `SmtpCredentials.username` are
  `Option<SmtpUsername>`. No `String`-typed SMTP username field remains.
- **AC3** `get_smtp_credentials` decodes the `smtp.username` column into
  `SmtpUsername` via the bridge; a present-but-empty stored username reads as a
  `sqlx::Error::ColumnDecode` (dual-backend test), symmetric with the password.
- **AC4** `SmtpConfigError::InvalidCredential` (valueless) replaces
  `InvalidPassword`; `load_smtp_config` returns it for an empty username **or**
  password (updating #586's `..._returns_err_for_empty_password` test and adding
  an empty-username case). No credential value appears in the error.
- **AC5** The mailer authenticates only when both are present; the single
  `username.as_ref()` / `password.as_ref()` reads are the only plaintext
  exposes. Behavior unchanged for the valid case.
- **AC6** Test `SmtpUsername` values build via
  `common::test_support::parse_smtp_username(...)` (newtype test-helper
  convention). `cargo xtask validate --no-e2e` clean.

## Out of scope

- Any `Proffered*`/serde twin, `#[server]` setter, or web UI — the web-settable
  path (#638) adds a typed wire arg for the username per ADR-0065 (non-secret,
  so no `Proffered` twin needed).
- Other `SmtpConfig` fields (host/port/tls/sender) and mailer transport.

## Testing

- `common`: `SmtpUsername` `FromStr` accepts non-empty / rejects empty;
  `parse_smtp_username` helper. (`common/src/smtp_username.rs` `#[cfg(test)]`.)
- `storage`: `get_smtp_credentials` reads the typed pair (dual-backend);
  empty-username and empty-password both `ColumnDecode` (dual-backend);
  `load_smtp_config` maps each to `InvalidCredential` (update the #586 tests).
- `server`: the `mailer/smtp.rs` fixtures build `SmtpUsername` via the helper;
  the authenticated-builder path stays green.
