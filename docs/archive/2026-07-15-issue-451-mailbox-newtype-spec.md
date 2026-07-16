# Spec — issue #451: `Mailbox` newtype (Email + display name) for the SMTP sender

- Issue: [#451](https://github.com/jaunder-org/jaunder/issues/451) — _types:
  Mailbox newtype (Email + display name) for the SMTP sender_
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
- Composes: `common::email::Email` (#397) + `common::display_name::DisplayName`
  (#401, merged)

## Goal

Give `SmtpConfig::sender` a proper typed home. Today it is
`email_address::EmailAddress` — a thin validate-on-load wrapper over a `String`
that must accept the display-name form (`"Jaunder <noreply@localhost>"`), is
parsed once to reject bad config, then immediately `to_string()`'d and re-parsed
into `lettre::Mailbox` at the send site (`server/src/mailer/smtp.rs`). It's the
same stringly-typed gap `Email` closed — but a **named mailbox**, not a bare
address.

`Email` can't represent it (it rejects the display-name form by design), and
extending `Email` to carry a name would break its str-newtype trailer. So
introduce a `Mailbox` type that **composes** the two existing newtypes — the
address (`Email`) and the optional display name (`DisplayName`).

## The type — `common/src/mailbox.rs`

A **compound** newtype (not a `StrNewtype` — it has two logical fields),
modeling RFC 5322's `[display-name] angle-addr`:

```rust
use std::str::FromStr;
use email_address::EmailAddress;
use thiserror::Error;

use crate::display_name::{DisplayName, InvalidDisplayName};
use crate::email::{Email, InvalidEmail};

/// An email address with an optional display name (RFC 5322 mailbox), e.g.
/// `Jaunder <noreply@localhost>` or a bare `noreply@localhost`. Composes the
/// address (`Email`, normalized) and the name (`DisplayName`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mailbox {
    display_name: Option<DisplayName>,
    address: Email,
}

#[derive(Debug, Error)]
pub enum InvalidMailbox {
    #[error("invalid mailbox: {0}")]
    Address(email_address::Error),
    #[error("invalid mailbox display name: {0}")]
    DisplayName(InvalidDisplayName),
    #[error("invalid mailbox address: {0}")]
    Address2(InvalidEmail), // see note below — likely folded into one variant
}

impl Mailbox {
    #[must_use]
    pub fn new(address: Email, display_name: Option<DisplayName>) -> Self { … }
    #[must_use]
    pub fn address(&self) -> &Email { &self.address }
    #[must_use]
    pub fn display_name(&self) -> Option<&DisplayName> { self.display_name.as_ref() }
}

impl FromStr for Mailbox {
    type Err = InvalidMailbox;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // `email_address` (default options *allow* display text) parses both
        // "Name <addr>" and a bare "addr", and splits the two halves for us.
        let parsed: EmailAddress = s.parse().map_err(InvalidMailbox::Address)?;
        // Re-validate + domain-normalize the bare addr-spec through `Email`
        // (`parsed.email()` is `local@domain`, no display name).
        let address: Email = parsed.email().parse().map_err(InvalidMailbox::Address2)?;
        let display = parsed.display_part(); // "" when there is no name
        let display_name = if display.is_empty() {
            None
        } else {
            Some(display.parse::<DisplayName>().map_err(InvalidMailbox::DisplayName)?)
        };
        Ok(Self { display_name, address })
    }
}

impl std::fmt::Display for Mailbox {
    // "Name <addr>" when named, else "addr".
}
```

Notes / decisions:

- **Parse via `email_address`, delegate the halves.** Reusing `email_address`'s
  parser (which `Email` already depends on) handles the `"Name <addr>"` grammar
  robustly; the address half is then re-validated + domain-normalized through
  `Email`, and the name half through `DisplayName`. The address re-parse through
  `Email` is effectively infallible (`parsed.email()` is already a valid bare
  addr-spec), but we **map** its error rather than `expect()` (clippy). The
  error enum's exact variant set will be tidied in the plan (likely two
  variants: address / display-name) — the sketch above is illustrative.
- **`DisplayName`'s rule fits a mail name.** #401's `DisplayName` trims,
  requires non-empty, caps at 255 scalars, preserves casing/Unicode, and imposes
  **no charset restriction** — so `"Jaunder"`, `"Dr. Ada Lovelace"`, etc. all
  parse. No looser rule is needed; `Mailbox` reuses `DisplayName` as-is.
- **No serde, no `Hash`.** Nothing (de)serializes `SmtpConfig`/`EmailMessage`
  (both are built in-process, not over the wire), and a mailbox is never a map
  key. Add a serde bridge later if a DTO ever needs one — keeping the type
  minimal and fully covered now.
- **Idempotency + a known edge.** `Display` → re-parse round-trips for normal
  names. A display name containing RFC-special characters (a comma, `<`) could
  break the `"Name <addr>"` re-parse; the SMTP sender is simple operator config,
  so we accept that as a documented limitation rather than gold-plate RFC
  quoting now. The send path re-parses through `lettre::Mailbox`, which is the
  ultimate gate.

## Threading

| Site                     | today                                                                                      | change                                                                                                                    |
| ------------------------ | ------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------- |
| `SmtpConfig::sender`     | `email_address::EmailAddress` (`storage/src/smtp.rs:68`)                                   | → `Mailbox`                                                                                                               |
| `load_smtp_config` parse | `sender_str.parse::<EmailAddress>()` (`smtp.rs:140`)                                       | → `.parse::<Mailbox>()`; keep `SmtpConfigError::InvalidSender`                                                            |
| SMTP config tests        | build sender via `EmailAddress` (`smtp.rs:253,276` etc.)                                   | build via `Mailbox`                                                                                                       |
| Send path                | `config.sender.to_string().parse::<lettre::Mailbox>()` (`server/src/mailer/smtp.rs:37-44`) | **unchanged** — `Mailbox`'s `Display` round-trips identically                                                             |
| `storage/Cargo.toml`     | `email_address.workspace = true`                                                           | **drop it** — after this, `SmtpConfig::sender` is storage's last `email_address` use (`common` keeps it, backing `Email`) |

## Decision: scope stays the SMTP sender (`EmailMessage.from` unchanged)

`EmailMessage.from` stays `Option<Email>`. Rationale: production **never** sets
`from` (every prod `EmailMessage` uses `from: None` and falls back to
`SmtpConfig::sender`); a message-level `from` override, if ever used, is a bare
address. Flipping it to `Option<Mailbox>` is consistency-only churn touching the
mailer + every `EmailMessage` constructor with no functional gain.

_Alternative (flag for approval):_ if you'd rather both sender-slots be
`Mailbox` for RFC-correctness, we type `EmailMessage.from: Option<Mailbox>` too
— say so and I'll fold it in; the send path already `to_string()`s `from`, so
it's mechanical.

## Testing

- **`common/src/mailbox.rs`** unit tests: parse `"Jaunder <noreply@localhost>"`
  (name + address), parse a bare `"noreply@localhost"` (no name), domain is
  normalized (`Foo <A@EXAMPLE.COM>` → address `a@example.com`), reject a
  malformed address, reject an empty/over-long display name, `Display`
  round-trip for both forms, `new`/accessor round-trip. Validation lives in
  exactly one place (`Mailbox::from_str`).
- **Storage:** the existing `load_smtp_config` tests
  (`load_smtp_config_returns_some_with_all_keys_present`, `…uses_defaults…`,
  `…returns_err_for_invalid_sender`) keep asserting the same behavior, now
  through `Mailbox` — the default `"Jaunder <noreply@localhost>"` still parses
  and the name survives.
- **Gate:** `cargo xtask validate --no-e2e` clean.

## Acceptance

- A `common::mailbox::Mailbox` newtype composing `Email` + `DisplayName`, with
  its validation in exactly one tested place.
- `SmtpConfig::sender` is typed `Mailbox`; the default and any `"Name <addr>"`
  config still parse and preserve the name in the `From:` header.
- `email_address` dropped from `storage/Cargo.toml` (`common` retains it).
- No `unwrap()`/`expect()` at boundaries; `cargo xtask validate --no-e2e` clean.

## Non-goals

- Typing `EmailMessage.from`/`to` as `Mailbox` (see decision above).
- RFC-quoting exotic display names on `Display` (documented limitation).
- A serde bridge for `Mailbox` (add when a DTO needs it).
