# Plan — issue #451: `Mailbox` newtype (Email + display name) for the SMTP sender

- Spec:
  [2026-07-15-issue-451-mailbox-newtype.md](../specs/2026-07-15-issue-451-mailbox-newtype.md)
  (what/why; this plan is how)
- Issue: [#451](https://github.com/jaunder-org/jaunder/issues/451)
- For agentic workers: drive with **`jaunder-iterate`**; delegate a task via
  **`jaunder-dispatch`** if useful. Tick checkboxes in real time.

---

## Review header

**Goal.** Add a `common::mailbox::Mailbox` newtype composing `Email` (address) +
`Option<DisplayName>` (name), and type `SmtpConfig::sender` with it — retiring
the raw `email_address::EmailAddress` sender and dropping `email_address` from
`storage`.

**Scope — in.** `common` (new `Mailbox` type); `storage` (`SmtpConfig::sender`
field

- `load_smtp_config` parse + tests + `Cargo.toml`); `server` (SMTP mailer test
  fixtures that build a sender).

**Scope — out.** `EmailMessage.from`/`to` stay `Email` (spec decision). No serde
/ no `Hash` on `Mailbox`. No RFC-quoting of exotic display names. Send path
unchanged.

**Tasks.**

1. `common::mailbox::Mailbox` newtype + unit tests.
2. Type `SmtpConfig::sender: Mailbox`, route the config parse + tests through
   it, and drop `email_address` from `storage/Cargo.toml`.

**Key risks / decisions.**

- `Mailbox` is a **compound** newtype (two fields), not a `StrNewtype` —
  hand-written `FromStr`/`Display`; validation composes `Email` + `DisplayName`,
  so it stays in one place per half.
- `FromStr` splits the angle-addr **manually** (no direct `email_address` use in
  `Mailbox`); adequate for the operator-config sender, exotic RFC-quoted names
  are the documented limitation.
- Send path is **unchanged** — `server/src/mailer/smtp.rs` already does
  `config.sender.to_string().parse::<lettre::Mailbox>()`, and `Mailbox`'s
  `Display` round-trips identically (same shape as #397's mailer flip).

---

## Global constraints

- **No `Co-Authored-By` trailer.**
- Clippy denies `unwrap_used`/`expect_used` outside `#[cfg(test)]` — introduce
  none in production paths; map errors.
- Before each commit: `cargo xtask check` clean (pre-commit hook runs the full
  gate). Follow **`jaunder-commit`**; request review, don't commit without
  approval.
- The `load_smtp_config` / SMTP-mailer tests are **config-parse** tests, not DB
  backend-parity tests — keep their existing structure (do not add a
  dual-backend `#[case]` where none exists; `test-backend-pattern` only flags DB
  tests).

---

## Task 1 — `common::mailbox::Mailbox` newtype + unit tests

**Goal.** A `Mailbox` composing `Email` + `Option<DisplayName>`, validated in
one place. Additive; compiles standalone.

**Files.**

- `common/src/mailbox.rs` (new).
- `common/src/lib.rs` — add `pub mod mailbox;` (alphabetical: `mailbox` sorts
  **before** `mailer` — `mailb…` < `maile…` — so `invite`, `mailbox`, `mailer`,
  `media`).
- Tests: in-file `#[cfg(test)] mod tests` (mirrors
  `email.rs`/`display_name.rs`).

**Interfaces.**

```rust
use std::fmt;
use std::str::FromStr;

use thiserror::Error;

use crate::display_name::{DisplayName, InvalidDisplayName};
use crate::email::{Email, InvalidEmail};

/// An email address with an optional display name (RFC 5322 mailbox), e.g.
/// `Jaunder <noreply@localhost>` or a bare `noreply@localhost`. Composes the
/// normalized address (`Email`) and the bounded name (`DisplayName`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mailbox {
    display_name: Option<DisplayName>,
    address: Email,
}

#[derive(Debug, Error)]
pub enum InvalidMailbox {
    #[error("invalid mailbox address: {0}")]
    Address(#[from] InvalidEmail),
    #[error("invalid mailbox display name: {0}")]
    DisplayName(#[from] InvalidDisplayName),
}

impl Mailbox {
    #[must_use]
    pub fn new(address: Email, display_name: Option<DisplayName>) -> Self {
        Self { display_name, address }
    }
    #[must_use]
    pub fn address(&self) -> &Email {
        &self.address
    }
    #[must_use]
    pub fn display_name(&self) -> Option<&DisplayName> {
        self.display_name.as_ref()
    }
}

impl FromStr for Mailbox {
    type Err = InvalidMailbox;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Split a `Name <addr>` angle-addr from a bare `addr`; delegate each half to
        // the newtype that owns its rule (address → Email, name → DisplayName).
        let s = s.trim();
        let (name, addr) = match s.strip_suffix('>').and_then(|rest| rest.rsplit_once('<')) {
            Some((name, addr)) => (Some(name.trim()), addr),
            None => (None, s),
        };
        let address: Email = addr.parse()?; // InvalidEmail via #[from]
        let display_name = match name {
            Some(n) if !n.is_empty() => Some(n.parse::<DisplayName>()?), // InvalidDisplayName via #[from]
            _ => None,
        };
        Ok(Self { display_name, address })
    }
}

impl fmt::Display for Mailbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.display_name {
            Some(name) => write!(f, "{name} <{}>", self.address),
            None => write!(f, "{}", self.address),
        }
    }
}
```

**Test (write first — RED, then implement — GREEN).** In-file:

- `parses_named` — `"Jaunder <noreply@localhost>"` → `display_name()` ==
  `"Jaunder"`, `address()` == `"noreply@localhost"`.
- `parses_bare` — `"noreply@localhost"` → `display_name()` is `None`,
  `address()` == `"noreply@localhost"`.
- `normalizes_address_preserves_name` — `"Foo <A.B@EXAMPLE.COM>"` → `address()`
  == `"A.B@example.com"` (domain lowered via `Email`), `display_name()` ==
  `"Foo"`.
- `rejects_malformed_address` — `"Jaunder <not-an-email>"`, `"<@x>"` →
  `Err(InvalidMailbox::Address(_))`.
- `rejects_bad_display_name` — an over-long name (`> MAX_DISPLAY_NAME_CHARS`) in
  `"<name> <a@b.com>"` → `Err(InvalidMailbox::DisplayName(_))`; a
  whitespace-only name (`"   <a@b.com>"`) parses as **no name** (`None`), not an
  error.
- `display_round_trips_both_forms` — named renders
  `"Jaunder <noreply@localhost>"`, bare renders `"noreply@localhost"`; each
  re-parses equal (idempotency).
- `new_and_accessors` — `Mailbox::new(email, Some(name))` exposes both via
  accessors.

**Run.**

- `cargo nextest run -p common mailbox` — **FAIL** before impl (module absent),
  **PASS** after.
- `cargo xtask check` clean → commit (`jaunder-commit`): _"types(common): add
  Mailbox newtype composing Email + DisplayName (#451)"_.

---

## Task 2 — type `SmtpConfig::sender: Mailbox` + drop `email_address` from storage

**Goal.** Flip the sender field and its config parse to `Mailbox`; the send path
is unchanged. Behavior-preserving (the default `"Jaunder <noreply@localhost>"`
and any `"Name <addr>"` config still parse and keep the name).

**Files / changes.**

- **`storage/src/smtp.rs`:**
  - Field: `pub sender: email_address::EmailAddress` (line ~68) →
    `pub sender: Mailbox`.
  - `load_smtp_config` parse (line ~140):
    `sender_str.parse::<email_address::EmailAddress>()` →
    `sender_str.parse::<Mailbox>()`, keeping
    `.map_err(|_| SmtpConfigError::InvalidSender(sender_str))` (`InvalidSender`
    already captures the offending string; behavior unchanged).
  - Import `use common::mailbox::Mailbox;`; drop the `email_address` references.
  - Tests (lines ~253, 276): the expected sender built as
    `"...".parse::<email_address::EmailAddress>()` → `"...".parse::<Mailbox>()`.
    The `not-a-valid-email` invalid-sender test (line ~288) still errors (the
    bare `"not-a-valid-email"` fails `Email`, hence `Mailbox`). Keep the tests'
    existing (non-dual-backend) shape.
- **`storage/Cargo.toml`:** remove `email_address.workspace = true` (line ~16).
  After this flip `smtp.rs` is storage's last `email_address` user (`common`
  keeps it, backing `Email`).
- **`server/src/mailer/smtp.rs`:** the send path
  (`config.sender.to_string().parse::<lettre::Mailbox>()`, `from_config`) is
  **unchanged**. The `#[cfg(test)]` fixtures that build a
  `SmtpConfig { sender: … }` (e.g. `base_config`'s
  `"Jaunder <noreply@example.com>".parse()…`, and the domain-literal sender
  `"user@[127.0.0.1]".parse()…`) now infer `Mailbox` from the field type —
  confirm both parse as `Mailbox` (named form ✓; bare domain-literal ✓, `Email`
  accepts `[127.0.0.1]`). No code change unless a turbofish named `EmailAddress`
  is present (swap it to `Mailbox` / let inference handle it).
- **Grep guard:** `rg 'email_address' storage/ server/` after the change —
  expect only comments (and none in `storage/`); no live `email_address::` in
  `storage`/`server`.

**Tests.** No new behavior — the existing `load_smtp_config` tests
(`…returns_some_with_all_keys_present`,
`…uses_defaults_for_missing_optional_fields`, `…returns_err_for_invalid_sender`)
are the safety net, now asserting through `Mailbox`; the default
`"Jaunder <noreply@localhost>"` parses and the name survives.

**Run.**

- `cargo check --workspace` — iterate until the flip compiles.
- `cargo nextest run -p storage smtp` and `cargo nextest run -p jaunder mailer`
  (server crate package is `jaunder`) — **PASS** (behavior preserved).
- `cargo xtask check` clean → commit: _"types(storage): type SmtpConfig::sender
  as Mailbox; drop email_address (#451)"_.

---

## Definition of done

- `common::mailbox::Mailbox` composes `Email` + `DisplayName`, validation in one
  tested place (`Mailbox::from_str`).
- `SmtpConfig::sender: Mailbox`; default + `"Name <addr>"` configs parse and
  preserve the display name in the `From:` header.
- `email_address` removed from `storage/Cargo.toml`; no live `email_address::`
  in `storage`/`server`.
- No new `unwrap()`/`expect()` in production paths;
  `cargo xtask validate --no-e2e` clean.
- Ship via **`jaunder-ship`** (review, archive, PR referencing #451).
