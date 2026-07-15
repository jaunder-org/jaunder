# Spec — issue #397: `Email` newtype for validated email addresses

- Issue: [#397](https://github.com/jaunder-org/jaunder/issues/397) — _types:
  Email newtype for validated email addresses_
- Milestone: Domain-value type safety (newtypes)
- Governing ADRs: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  (domain-value newtype convention + standard trailer) and
  [ADR-0065](../../adr/0065-client-side-domain-validation.md) (typed `#[server]`
  wire args with client-side pre-validation)
- Blocker: #403 (StrNewtype/IdNewtype derive macros) — **CLOSED/COMPLETED**, so
  this is unblocked.

## Goal

Introduce an `Email` newtype in `common`, backed by the `email_address` crate
for the actual parse, and thread it through every place a user's email address
lives so that a boundary parse is the **only** place an address can be invalid.
This is a straight application of ADR-0063 (which already names `Email` as one
of its target newtypes) — no new architectural decision, so **no new ADR**.

## Premise correction (verified against the worktree)

The issue narrative describes email as a bare `String` "everywhere." That is no
longer accurate: the storage and mailer layers already hold the **raw external
type** `email_address::EmailAddress`. So the substance of #397 is to replace
that _raw_ external type with a first-class `common::Email` newtype (giving it
the ADR-0063 trailer, a single normalizing chokepoint, and the serde bridge),
and to type the two remaining bare-`String` email surfaces at the web boundary.

## Design decisions (confirmed with the user)

1. **Normalization = lowercase the domain only.** RFC-correct: the domain is
   case-insensitive; the local-part technically is not. Matches the issue's
   exact wording ("lowercases the domain"). `email_address` v0.2.9 is
   case-preserving on `parse`, so normalization is explicit in our hand-written
   `FromStr`.
2. **Full boundary threading — typed `#[server]` arg + client pre-validation
   (ADR-0065).** Beyond the four internal sites the issue's Direction names,
   type the web write boundary's _argument_ as `Email`
   (`request_email_verification(email: Email)`) and the profile **read** DTO as
   `Option<Email>`. This is the ADR-0063 §4 ideal parsed at the outermost
   boundary, and it follows ADR-0065's blessed shape exactly (already proven for
   `Username` on `request_password_reset`, `login`, `register`).

   The cold review's concern — that a typed arg's deserialize failure surfaces
   as the generic `WebError::ServerFunction`, losing the curated message and
   `boundary!` telemetry — is **the precise trade ADR-0065 addresses and
   accepts**: the calling form pre-validates with the _same_ `Email::from_str`
   and gates submit **disable-until-valid**, so a decode failure is only
   reachable by a non-browser/malicious client (defense-in-depth, not the user
   path). So the earlier "keep the arg `String`" refinement is **reversed**: the
   arg is `Email`, and the email form adopts client-side validation (see
   "Client-side pre-validation" below). This reaches into #414/#404 territory
   **for the email form only** — not a general form sweep.

## The type — `common/src/email.rs`

Mirror `common/src/username.rs` exactly (the ADR-0063 exemplar template):

```rust
use std::str::FromStr;
use email_address::{EmailAddress, Options};
use macros::StrNewtype;
use thiserror::Error;

/// A validated, domain-normalized email address (RFC 5321/6531 addr-spec).
///
/// The domain is lowercased on construction; the local-part is preserved
/// (it is case-sensitive per RFC). Display-name / angle-bracket forms are
/// rejected — this is a bare address, not a mailbox. `FromStr` is the single
/// validating, normalizing chokepoint.
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct Email(String);

#[derive(Debug, Error)]
#[error("invalid email address")]
pub struct InvalidEmail;

impl FromStr for Email {
    type Err = InvalidEmail;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Reject the display-name form ("Foo <a@b>") that email_address accepts
        // by default — for a bare-address primitive it is a typo, not input to
        // silently strip.
        let opts = Options::default().without_display_text();
        let addr = EmailAddress::parse_with_options(s, opts).map_err(|_| InvalidEmail)?;
        // email_address is case-preserving. Canonicalize the DNS domain only;
        // leave a domain-literal ([1.2.3.4] / [IPv6:…]) untouched so its casing
        // survives (the crate stores it verbatim and lowercasing an IPv6 tag,
        // while re-parseable, changes RFC-5321 meaning).
        let domain = addr.domain();
        let canonical = if domain.starts_with('[') {
            format!("{}@{}", addr.local_part(), domain)
        } else {
            format!("{}@{}", addr.local_part(), domain.to_lowercase())
        };
        Ok(Email(canonical))
    }
}
```

_(Verify the exact `Options` builder name — `without_display_text()` — against
email_address 0.2.9 when writing the code; if the builder differs, assert
`addr.display_part().is_empty()` post-parse instead.)_

- The `StrNewtype` derive (from #403) generates the entire ADR-0063 trailer:
  `Display`, `AsRef<str>`, `Borrow<str>`, `Deref<Target = str>`,
  `TryFrom<String>` (routed through `FromStr`), `From<Email> for String`,
  `PartialEq<str>`, `PartialEq<&str>`, and the direct `Serialize`/`Deserialize`
  impls (serialize borrows; deserialize routes through `FromStr`, rejecting
  invalid input on the wire). No inherent `as_str()` — the `str` traits replace
  it.
- Register `pub mod email;` in `common/src/lib.rs` (no flattening `pub use`, per
  the crate's established style). Consumers path in as `common::email::Email`.
- `common` is wasm-compiled; `email_address` is pure-Rust and already a `common`
  dependency (`common/Cargo.toml`), so `Email` belongs here with no new dep.

## Threading map — `email_address::EmailAddress` → `Email`

**Internal sites (the four the issue names + the storage parse chokepoint):**

| Site                              | file:line (today)                      | Change                                                                                                 |
| --------------------------------- | -------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| User record field                 | `storage/src/users.rs:36`              | `email: Option<EmailAddress>` → `Option<Email>`                                                        |
| `build_user_record` parse         | `storage/src/helpers.rs:48-50`         | `.parse()` target `EmailAddress` → `Email`; still maps parse error to `sqlx::Error::Decode`            |
| Verification store — create       | `storage/src/email.rs:40-45`           | `email: &EmailAddress` → `&Email`; bind `email.as_str()` → `&*email` / `email.as_ref()`                |
| Verification store — use (return) | `storage/src/email.rs:55-58,159-161`   | returns `(i64, EmailAddress)` → `(i64, Email)`; the stored-string re-parse targets `Email`             |
| `set_email`                       | `storage/src/users.rs:176-181,419-432` | `email: Option<&EmailAddress>` → `Option<&Email>`; bind via `Email` deref                              |
| Mailer `to` (and `from`)          | `common/src/mailer.rs:24,26`           | `Vec<EmailAddress>` → `Vec<Email>`; `from: Option<EmailAddress>` → `Option<Email>` (same field family) |

**Boundary sites (full-boundary scope):**

| Site                             | file:line (today)               | Change                                                                                                                                                                                                                                            |
| -------------------------------- | ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --- | ---------------- |
| `request_email_verification` arg | `web/src/email/mod.rs:18,26-29` | `email: String` → `email: Email`; the inline `.parse::<EmailAddress>()` at 26-29 is **deleted** (arg arrives already parsed/normalized); downstream `create_email_verification`/`EmailMessage.to` already take `Email` per the internal threading |
| Email form (the caller)          | `web/src/pages/email.rs:35-40`  | raw `<input type="email" name="email">` → a validated `Field<Email>` + `<ValidatedInput<Email>>` with `prop:disabled` submit gating (see "Client-side pre-validation")                                                                            |
| Profile read DTO field           | `web/src/profile/mod.rs:26`     | `email: Option<String>` → `Option<Email>`; builder at line 44 becomes `user.email` directly (drop `.map(                                                                                                                                          | e   | e.to_string())`) |

**Downstream call sites to update** (compiler-forced, mechanical):
`web/src/email/mod.rs` (build `EmailMessage`, call `set_email`),
`web/src/password_reset/mod.rs:52-54`, `server/src/commands.rs:300-306` (CLI
test-email — parse `to` into `Email`), `server/src/mailer/*`, and the
storage/server test suites that pass `&EmailAddress` (e.g.
`server/tests/storage/mod.rs`). The `request_email_verification` caller — the
`EmailPage` form — is changed deliberately (see "Client-side pre-validation").
The profile-DTO email consumers now receive `Email` but read it via
`Display`/`Deref<str>`, so those sites are unchanged.

## Client-side pre-validation (ADR-0065) — the email form

Because `request_email_verification`'s arg becomes `Email`, the form that
submits it must pre-validate so a legitimate submission never hits the generic
decode-error path. The infra already exists in `web/src/forms.rs`
(`field_error<T>`, `Field<T>`, `<ValidatedInput<T>>`, all bounded
`T: FromStr + 'static, T::Err: Display` — which `Email` satisfies via its
`StrNewtype` + `InvalidEmail: Display`). **Template to mirror:**
`ForgotPasswordPage` (`web/src/pages/password_reset.rs:13-59`) validating
`Username` → `request_password_reset(username: Username)`.

Change `EmailPage` (`web/src/pages/email.rs:9-56`):

- Import `crate::forms::{Field, ValidatedInput}` and `common::email::Email`.
- `let email = Field::<Email>::new();`
- Replace the raw `<input type="email" name="email">` with
  `<ValidatedInput<Email> label="New email address" name="email" field=email input_type="email"/>`.
  The `name` **must** stay `"email"` — it is the generated
  `RequestEmailVerification` struct field name the `ActionForm` deserializes
  through `Email`'s serde bridge.
- Gate submit: `<button … prop:disabled=move || !email.is_valid()>`.
- Keep the existing `ActionForm action=request_action` and the
  `Result<(), WebError>` result display unchanged.

**Subtlety — do _not_ pass `transform=str::to_lowercase`** (as the `Username`
form does). Email lowercases the **domain only**; a whole-input `to_lowercase`
would corrupt the case-sensitive local-part. Omit `transform`, let the user's
raw input ride the wire, and let `Email::from_str` (via the server-side serde
deserialize) produce the domain-normalized canonical form. Client validation
only checks parseability; canonicalization happens once, at the server
boundary's parse.

## The send path — no change needed (corrected after cold review)

An earlier draft claimed a "tension" at the SMTP boundary. It was wrong:
`storage/src/smtp.rs` is **config loading only** (no lettre, no send). The real
send path is `server/src/mailer/smtp.rs:80-116` (`LettreMailSender::send_email`)
and `:37-44` (`from_config`), and it already converts every address by going
through `Display`:

```rust
let mailbox: Mailbox = to_addr.to_string().parse()          // smtp.rs:92-95
    .map_err(|e: lettre::address::AddressError| MailError::Send(Box::new(e)))?;
```

`StrNewtype` gives `Email` a `Display` identical to `EmailAddress`'s (the inner
string), so flipping `EmailMessage.to`/`from` from `EmailAddress` to `Email`
requires **no change to the send path at all** — it still does
`to_string().parse::<Mailbox>()` and still maps the error to `MailError::Send`.
The `map_err` arm is genuinely reachable (an `Email` may hold a domain-literal
like `user@[127.0.0.1]` that lettre's `Mailbox` rejects — exercised by the
existing `divergent_address` tests at `server/src/mailer/smtp.rs:206-270`), and
it already maps rather than unwraps. No new `unwrap()`/`expect()` is introduced
anywhere.

## Testing

- **In `common/src/email.rs`** (mirroring `username.rs` tests): parses a valid
  address; **lowercases the domain, preserves local-part case** (e.g.
  `Foo.Bar@Example.COM` → `Foo.Bar@example.com`); **rejects the display-name
  form** (`Foo <a@b.com>` → `Err`, not silently stripped); **preserves a
  domain-literal verbatim** (`u@[127.0.0.1]` unchanged; no lossy lowercasing);
  rejects malformed input; `Display` round-trip; serde round-trip (serializes as
  a plain string; **rejects invalid input on deserialize**); **idempotency**
  (re-parsing a canonical value yields itself). This is the "exactly one tested
  place" for validation.
- **Storage:** the existing `build_user_record_rejects_invalid_email` test
  (`helpers.rs:616-630`) keeps asserting `sqlx::Error::Decode` for bad email,
  now via `Email`. Verification-store round-trip tests updated to `Email`.
- **Client validation:** `field_error::<Email>` / `Field::<Email>` are
  host-testable under an `Owner` (ADR-0065 coverage boundary) — a small host
  test asserting a valid email is `is_valid()` and a malformed one is not; the
  `Email::from_str` rule itself is already covered in `common`. The
  `<ValidatedInput<Email>>` rendering + the disable-until-valid submit gating
  are exercised by an **e2e** test on the email form (mirroring the existing
  `Username` form e2e), since the component is a `#[component]` (dead-but-exempt
  on host per ADR-0050).
- **Gate:** `cargo xtask validate --no-e2e` clean (acceptance) for the non-e2e
  work; the form's e2e assertion runs under `cargo xtask validate` (full).
  Coverage: the `Email` constructor + error path fully covered by the `common`
  unit tests.

## Acceptance mapping

- _Email validation exists in exactly one tested place_ → `Email::from_str` in
  `common/src/email.rs`, with the unit tests above.
- _User email, verification store, set_email, mailer recipient typed `Email`_ →
  the threading map (internal sites), plus the boundary sites for full coverage.
- _No `unwrap()`/`expect()` at boundaries; `cargo xtask validate --no-e2e`
  clean_ → no new `unwrap`/`expect`; the send path already maps its parse error
  and is unchanged; the typed arg is validated by serde on deserialize. Gate is
  the exit criterion.

## Non-goals

- A general DTO/`#[server]` newtype sweep (that is #14/#91). We touch only the
  email path's boundary sites.
- Changing wire formats: the serde bridge keeps `Email` on the wire as a plain
  string, so no serialized shape changes.
- Any change to SMTP semantics or the mailer's send behavior beyond the type
  swap.

## Risks / watch-items

- **email_address v0.2.9 reconstruction fidelity** (verified against crate
  source): `local_part()` **preserves quotes** (split is `rsplit_once('@')`), so
  a quoted local-part reconstructs faithfully; `domain()` returns the literal
  with brackets, which the `starts_with('[')` guard leaves untouched. The only
  normalization is `to_lowercase()` on a DNS domain. Covered by tests.
- **Existing stored mixed-case data:** re-parsing a stored value that had an
  uppercase local-part stays stable (we only lowercase the domain), so no data
  migration is needed. Stored uppercase _domains_ would re-normalize on read —
  still a valid, deterministic value.
- **Profile read DTO coupling:** typing the DTO field `Option<Email>` means a
  stored value that fails `Email::from_str` would fail DTO deserialization on
  the wasm client and brick the whole profile fetch, not just the email field.
  Low risk — storage already validated the value on write (`helpers.rs:48`) with
  the _same_ `Email::from_str`, so it is idempotent by construction — but called
  out as a deliberate failure-coupling of the full-boundary choice. The single
  consumer (`web/src/pages/email.rs`) reads it via `Display`/`clone`, so the
  happy path is unaffected.
- **email_address default `Options`:** `FromStr` uses defaults that accept
  display-name text; we deliberately parse with `without_display_text()` to
  reject it. Confirm the exact builder name at implementation time (watch-item
  in the type section).
