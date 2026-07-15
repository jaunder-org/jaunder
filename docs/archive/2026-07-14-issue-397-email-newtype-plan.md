# Plan — issue #397: `Email` newtype for validated email addresses

- Spec:
  [2026-07-14-issue-397-email-newtype.md](../specs/2026-07-14-issue-397-email-newtype.md)
  (the "what/why"; this plan is the "how")
- Issue: [#397](https://github.com/jaunder-org/jaunder/issues/397)
- For agentic workers: drive with **`jaunder-iterate`**; delegate an individual
  task via **`jaunder-dispatch`** when useful. Tick checkboxes in real time.

---

## Review header

**Goal.** Add a `common::email::Email` str-newtype (validated via
`email_address`, domain-lowercased) and replace the raw
`email_address::EmailAddress` throughout the user record, email-verification
store, `set_email`, and the mailer recipient; type the web write-boundary arg
and the profile read DTO as `Email`, with client-side pre-validation on the
email form (ADR-0065).

**Scope — in.** `common` (new type + mailer field types); `storage` (user
record, helpers parse, verification store, `set_email` + tests); `web` (email
server fn arg + form validation, profile DTO, password-reset construction);
`server` (CLI test-email, mailer fixtures, integration tests).

**Scope — out.** General DTO/`#[server]` newtype sweep (#14/#91); any
SMTP/mailer behavior change; new ADRs. No separable concerns to file — the
spec's non-goals map onto existing issues.

**Tasks.**

1. `common::email::Email` newtype + unit tests.
2. Type-flip `EmailAddress` → `Email` across storage + mailer + all consumers +
   tests (one atomic compile unit; arg stays `String`, parsed inline to
   `Email`).
3. Type the `request_email_verification` arg as `Email` (delete inline parse) +
   client-side validation on `EmailPage` + `Field<Email>` host test.
4. e2e test: the validated email form (disable-until-valid, inline error,
   happy-path send).

**Key risks / decisions.**

- Task 2 is one commit by necessity — a cross-crate type flip has no
  intermediate compiling state. Mitigated by a per-file checklist; it is
  mechanical + behavior-preserving (existing tests updated, must stay green).
- `Email::from_str` lowercases the **domain only** (preserve local-part case)
  and **rejects** display-name forms (`without_display_text()`) — do not reuse
  the `Username` form's `transform=str::to_lowercase`.
- Send path (`server/src/mailer/smtp.rs`) is **unchanged** — it already goes
  through `Display` → `parse::<Mailbox>()`.

---

## Global constraints

- **No `Co-Authored-By` trailer** on any commit.
- Clippy denies `unwrap_used`/`expect_used` at boundaries — introduce none. Map
  errors instead.
- Before each commit: run `cargo xtask check` clean (the pre-commit hook runs
  the full gate: fmt + clippy + Nix coverage/tests). Follow
  **`jaunder-commit`**; request review, do not commit without approval.
- Storage tests follow the **dual-backend** template (ADR-0053 /
  `CONTRIBUTING.md` "backend parity"); a bare `#[tokio::test]` that should be
  dual-backend fails the `test-backend-pattern` guard. Here we **update
  existing** email tests to `Email`, not add new bare ones.
- Coverage policy (ADR-0050): the `Email` constructor + error path are fully
  covered by the `common` unit tests in Task 1.

---

## Task 1 — `common::email::Email` newtype + unit tests

**Goal.** A validated, domain-normalized `Email` str-newtype with the ADR-0063
trailer (via `StrNewtype`) and a hand-written `FromStr`. Standalone, additive.

**Files.**

- `common/src/email.rs` (new) — the type, per spec "The type" section.
- `common/src/lib.rs` — add `pub mod email;` (alongside `pub mod username;`
  etc.; no flattening `pub use`).
- Tests: in-file `#[cfg(test)] mod tests` (mirrors
  `common/src/username.rs:39-86`).

**Interfaces.**

```rust
// common/src/email.rs
use std::str::FromStr;
use email_address::{EmailAddress, Options};
use macros::StrNewtype;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct Email(String);

#[derive(Debug, Error)]
#[error("invalid email address")]
pub struct InvalidEmail;

impl FromStr for Email {
    type Err = InvalidEmail;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let addr = EmailAddress::parse_with_options(s, Options::default().without_display_text())
            .map_err(|_| InvalidEmail)?;
        let domain = addr.domain();
        let canonical = if domain.starts_with('[') {
            format!("{}@{}", addr.local_part(), domain)          // literal: leave verbatim
        } else {
            format!("{}@{}", addr.local_part(), domain.to_lowercase())
        };
        Ok(Email(canonical))
    }
}
```

(`parse_with_options` + `Options::default().without_display_text()` verified
present in `email_address` 0.2.9.)

**Test (write first — RED, then implement — GREEN).** In-file, mirroring
`username.rs`:

- `parses_valid` — `"user@example.com".parse::<Email>()` is `Ok`.
- `lowercases_domain_preserves_local` — `"Foo.Bar@Example.COM"` → `Display` ==
  `"Foo.Bar@example.com"`.
- `rejects_display_name_form` — `"Foo <a@b.com>".parse::<Email>()` is `Err` (not
  silently stripped).
- `preserves_domain_literal` — `"u@[127.0.0.1]"` → unchanged (round-trips
  verbatim).
- `rejects_malformed` — `"not-an-email"`, `"a@"`, `"@b"` all `Err`.
- `display_round_trip` — `Display` yields the canonical string.
- `serde_bridge` — `to_string(&e)` is a plain `"…"`;
  `from_str::<Email>("\"USER@EXAMPLE.COM\"")` lowercases the domain;
  `from_str::<Email>("\"bad\"")` is `Err`.
- `idempotent` — `let c = e.to_string(); c.parse::<Email>().unwrap() == e`
  (canonical re-parses to itself).

**Run.**

- `cargo nextest run -p common email` — expect **FAIL** before impl (module
  absent), **PASS** after.
- `cargo xtask check` clean → commit (`jaunder-commit`): _"types(common): add
  Email newtype validated via email_address (#397)"_.

---

## Task 2 — type-flip `EmailAddress` → `Email` (storage + mailer + consumers + tests)

**Goal.** Replace `email_address::EmailAddress` with `common::email::Email` at
every site the spec's threading map names. Behavior-preserving; one commit (no
intermediate cross-crate state compiles). The web write-boundary arg **stays
`String`** here, parsed inline to `Email` (Task 3 flips the arg).

**Files / changes (checklist — the implementing agent works this file-by-file,
compiling with `cargo check` between files where possible):**

- **`common/src/mailer.rs`** — `EmailMessage.to: Vec<EmailAddress>` →
  `Vec<Email>`; `from: Option<EmailAddress>` → `Option<Email>` (lines ~24,26).
  Update the in-file test helper `parse_email` (line 137) and fixtures to build
  `Email` (`s.parse::<Email>().expect(...)` is a **test** helper — `expect` is
  allowed in `#[cfg(test)]` per clippy.toml). Import `common::email::Email`
  (same crate → `crate::email::Email`).
- **`storage/src/users.rs`** — user record field `email: Option<EmailAddress>` →
  `Option<Email>` (line 36); `set_email` trait + impl arg
  `Option<&EmailAddress>` → `Option<&Email>` (lines 176-181, 419-432); the bind
  `email.map(EmailAddress::as_str)` → `email.map(|e| &**e)` /
  `email.map(AsRef::as_ref)` (Email: `AsRef<str>`). Swap
  `use email_address::EmailAddress;` (line 5) for `use common::email::Email;`.
- **`storage/src/helpers.rs`** — `build_user_record` parse (lines 48-50):
  `.parse()` target is now `Email` (type-inferred from the field); still
  `.map_err(|e| sqlx::Error::Decode(Box::new(e)))`
  (`InvalidEmail: Error + Send + Sync + 'static` satisfies the `Decode` bound).
  Import `Email` if needed.
- **`storage/src/email.rs`** — `create_email_verification(email: &EmailAddress)`
  → `&Email` (lines 40-45); bind `email.as_str()` → `&**email`/`email.as_ref()`
  (line 119). `use_email_verification` return `(i64, EmailAddress)` →
  `(i64, Email)` (lines 55-58); the stored-string re-parse
  `.parse::<EmailAddress>()` → `.parse::<Email>()` (lines 159-161), mapping the
  error into `UseEmailVerificationError` as today. Swap the `email_address`
  import for `Email`.
- **`web/src/email/mod.rs`** — the inline parse (lines 26-29) target changes
  `email.parse::<email_address::EmailAddress>()` → `email.parse::<Email>()`
  (**arg still `String`** in this task); `email_addr: Email` then flows
  unchanged to `create_email_verification(auth.user_id, &email_addr, …)`,
  `EmailMessage { to: vec![email_addr], … }`, and
  `set_email(user_id, Some(&email_addr), true)` (line 71) — no temporary
  conversions. Update imports.
- **`web/src/profile/mod.rs`** — DTO field `email: Option<String>` →
  `Option<Email>` (line 26); builder `user.email.map(|e| e.to_string())` →
  `user.email` (line 44). Import `Email`.
- **`web/src/password_reset/mod.rs`** —
  `EmailMessage { to: vec![verified_email], … }` (lines 52-54): `verified_email`
  comes from `use_email_verification` → now already `Email`, so it drops
  straight into `to`. Adjust types/imports.
- **`server/src/commands.rs`** — CLI test-email (lines 300-306):
  `let to_addr: email_address::EmailAddress = to.parse()…` →
  `let to_addr: Email = to.parse()…` (propagate the parse error via `?`/the
  command's error type — **no `expect`**);
  `EmailMessage { to: vec![to_addr], … }`.
- **`server/src/mailer/mod.rs`, `server/src/mailer/file.rs`,
  `server/src/main.rs`** — fixture/test builders of `EmailMessage` updated to
  `Email`. **`server/src/mailer/smtp.rs` send path is UNCHANGED** (already
  `to_addr.to_string().parse::<Mailbox>()`); confirm its `divergent_address`
  tests (lines 206-270) still build `Email` recipients that stay
  lettre-divergent (`"user@[127.0.0.1]"` parses as `Email`, `Mailbox` rejects →
  `MailError::Send`).
- **`server/tests/storage/mod.rs`** (and any other integration tests) — every
  `set_email(…, Some(&addr), …)` / verification-store call passing
  `&EmailAddress` updated to `&Email` (e.g. lines ~860, 1597, 1627). These are
  **updated existing** dual-backend tests; keep their structure.

- **Dependency cleanup** — after the flip, `storage`/`web`/`server` may no
  longer reference `email_address::EmailAddress` directly (only `common` does,
  inside `Email`). If a crate's last `email_address` use is gone, drop it from
  that crate's `Cargo.toml` so an unused-dependency check (`cargo machete` /
  clippy) stays green; `common/Cargo.toml` keeps it.

**Tests.** No new behavior — the existing storage/mailer/web tests are the
safety net. Keep `build_user_record_rejects_invalid_email`
(`helpers.rs:616-630`) asserting `sqlx::Error::Decode` for
`Some("not-an-email")`, now through `Email`.

**Run.**

- `cargo check --workspace` — iterate until the cascade compiles.
- `cargo nextest run -p storage` and `-p server` and `-p web` (host) — expect
  **PASS** (behavior preserved).
- `cargo xtask check` clean → commit: _"types: thread Email through storage,
  mailer, and web consumers (#397)"_.

---

## Task 3 — typed `#[server]` arg + client-side validation on the email form

**Goal.** Type `request_email_verification`'s arg as `Email` (delete the inline
parse) and pre-validate on `EmailPage` so a legitimate submit never hits the
generic decode-error path (ADR-0065). Template: `ForgotPasswordPage`
(`web/src/pages/password_reset.rs:13-59`) →
`request_password_reset(username: Username)`.

**Files.**

- **`web/src/email/mod.rs`** — signature
  `request_email_verification(email: String)` → `(email: Email)` (line 18);
  **delete** the inline parse (lines 26-29); `email` is already the
  validated/normalized `Email` — pass `&email` to `create_email_verification`,
  `EmailMessage { to: vec![email], … }`,
  `set_email(user_id, Some(&email), true)`.
- **`web/src/pages/email.rs`** — adopt the validated-input pattern:
  - Import `crate::forms::{Field, ValidatedInput}` and `common::email::Email`.
  - `let email = Field::<Email>::new();`
  - Replace the raw `<input type="email" name="email"/>` (lines 35-40) with
    `<ValidatedInput<Email> label="New email address" name="email" field=email input_type="email"/>`
    — **`name` must stay `"email"`** (the generated `RequestEmailVerification`
    struct field). **Do not** pass `transform=str::to_lowercase` (would corrupt
    the case-sensitive local-part; see spec).
  - Submit button: add `prop:disabled=move || !email.is_valid()`.
  - Keep `ActionForm action=request_action` and the `Result<(), WebError>`
    result display.

**Test (host — write first, RED, then GREEN).** A `#[cfg(test)]` host test
(under an `Owner`, per ADR-0065's coverage boundary — mirror an existing
`Field<Username>` test if present, else `web/src/forms.rs`'s own tests)
asserting `field_error::<Email>("user@example.com").is_none()` and
`field_error::<Email>("bad").is_some()`; and that a `Field::<Email>::new()` is
`!is_valid()` while one holding a valid address `is_valid()`. (The
`Email::from_str` rule itself is already covered in Task 1; this only pins the
`web` wiring.)

**Run.**

- `cargo nextest run -p web forms` (or the new test's filter) — **FAIL** before,
  **PASS** after.
- `cargo check -p web` (wasm target too, via the gate) — ensure the `#[server]`
  arg + form compile for both targets.
- `cargo xtask check` clean → commit: _"web: type email verification arg as
  Email with client-side validation (#397)"_.

---

## Task 4 — e2e: validated email form

**Goal.** Exercise the `<ValidatedInput<Email>>` rendering + disable-until-valid
gating that the host build cannot (the component is `#[component]`,
dead-but-exempt on host per ADR-0050). Mirror the existing `Username`-form e2e.

**Files.**

- `end2end/tests/…` — locate the existing email-verification / settings e2e (or
  the `Username` validated-form e2e as the template) and add: (a) submit button
  disabled while the field is empty/invalid; (b) an invalid entry shows the
  inline client-local error after blur; (c) a valid entry enables submit and the
  happy path fires `request_email_verification` (verification mail recorded via
  the file mailer / existing e2e assertion pattern).

**Run.**

- `cargo xtask e2e sqlite chromium` (single combo for the iterate loop); full
  `cargo xtask validate` runs all four `{sqlite,postgres}×{chromium,firefox}`
  combos at ship.
- `cargo xtask check` clean → commit: _"e2e: validated email verification form
  (#397)"_.

---

## Definition of done

- `cargo xtask validate --no-e2e` clean (acceptance) + the Task 4 e2e green
  under full `cargo xtask validate`.
- Email validation lives in exactly one tested place (`Email::from_str`,
  `common/src/email.rs`).
- User record, verification store, `set_email`, mailer recipient, the web write
  arg, and the profile read DTO are all typed `Email`.
- No new `unwrap()`/`expect()` at boundaries.
- Ship via **`jaunder-ship`** (final review, archive spec+plan, PR referencing
  #397).
