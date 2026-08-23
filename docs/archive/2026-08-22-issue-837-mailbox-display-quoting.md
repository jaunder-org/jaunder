# Issue #837 — Quote Mailbox display names at render time

## Outcome

`common::Mailbox` stores the human display label, not an RFC wire fragment, and
renders a valid RFC 5322 mailbox for display names that need quoting. SMTP
sender values such as `Acme, Inc <noreply@example.com>` continue to parse and
become usable by lettre instead of failing later at mailer construction.

## Load-bearing decisions

- Scope the fix to `common::Mailbox`; do not change `DisplayName` validation
  because the same type is the user profile display name.
- `Mailbox::from_str` stores a decoded human label when the input uses quoted
  display-name syntax, so `display_name()` never exposes surrounding quotes or
  backslash escapes.
- `Mailbox::Display` owns mailbox wire rendering: quote/escape display names
  when RFC phrase syntax requires it, and leave simple names unquoted.
- `Mailbox::new(address, Some(display_name))` and parsed `Mailbox` values render
  through the same path; caller-built names containing comma, colon,
  parentheses, quotes, or backslashes must round-trip.
- The SMTP lettre seam should stop compensating for `common::Mailbox` quoting
  behavior once `Mailbox::Display` is correct; remaining sender failures should
  represent genuinely invalid sender values, not ordinary names needing quotes.

## Acceptance

- `Mailbox::from_str` for quoted display names stores the decoded label without
  surrounding quotes or wire escapes.
- `Mailbox::Display` round-trips simple display names unquoted and
  quoting-requiring display names quoted/escaped.
- `Mailbox::new` with a quoting-requiring `DisplayName` renders a mailbox that
  parses back to the same `Mailbox`.
- The SMTP sender config cases with comma, colon, parentheses, and
  already-quoted comma display names build a `LettreMailSender` successfully
  rather than returning `InvalidSender` or double-quoting.
- Existing address normalization and bare-address behavior are unchanged.

## Boundaries

- No `DisplayName` rule changes and no profile/display-name UI behavior changes.
- No SMTP configuration schema or storage changes.
- No changes to recipient address conversion or lettre dependency handling.
