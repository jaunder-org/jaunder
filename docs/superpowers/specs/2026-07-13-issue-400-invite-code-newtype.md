# Spec — #400: `InviteCode` / `ProfferedInviteCode` newtypes

- Issue: [#400](https://github.com/jaunder-org/jaunder/issues/400)
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  (amended by this issue)
- Related: [#433](https://github.com/jaunder-org/jaunder/issues/433) (email
  delivery, follow-on)
- Date: 2026-07-13

## Problem

An invite code is a capability token — the thing that authorizes registration —
but it crosses as a bare `&str`/`String` through `storage`, `web`, and the CLI.
Any string can be passed where an invite code is expected, and at a call site a
code is indistinguishable from any other string. Per ADR-0063 it qualifies on
the **transposition** axis (a capability token that must not be a stray string)
and the **invariant** axis (a fixed base64url shape worth validating at the
boundary).

It is also a **256-bit secret** (`common::auth::generate_token`, same generator
as the session token; `storage/src/sqlite/mod.rs` calls it "a high-entropy
secret"). Two facts shape the design:

1. **Directional secrecy.** A client legitimately _submits_ a code
   (registration), but the server should _never send one back to a client_.
   "Submittable, never returned" is not expressible via serde traits
   (`Serialize`/`Deserialize` are encode/decode _operations_, and every leptos
   `#[server]` payload — arg or return — needs both). It has to be encoded
   **structurally**.
2. **Deliberate egress only.** The code must eventually be printed (a CLI
   invitation URL now; an email in #433), but should never leak via implicit
   `Display`/`Debug`/serialization.

## Decision

Two newtypes — a raw _inbound_ type and a validated _domain_ type — split across
crates so the directional-secrecy rule becomes a compile-time fact, plus one
shared validator so the invariant has a single source of truth.

### The shared invariant — `common::token`

Every opaque token (invite code, session `RawToken`, reset token) is a
base64url-no-pad string from the same generator, so the shape check is
token-general and lives in a token-neutral module (not `invite`):

```rust
/// Error when a string is not a syntactically valid opaque token.
#[derive(Debug, thiserror::Error)]
#[error("token must be non-empty and use only base64url characters ([A-Za-z0-9_-])")]
pub struct InvalidTokenShape;

/// The single source of truth for opaque-token shape: non-empty and base64url-no-pad
/// charset. Not length-pinned (so it is not coupled to any particular token size).
pub fn validate_shape(s: &str) -> Result<(), InvalidTokenShape>;
```

Both invite newtypes' hand-written `FromStr` delegate to
`common::token::validate_shape`. `FromStr` remains each type's sole constructor
(ADR-0063 chokepoint); it merely delegates the _rule_ so the types cannot drift.

### `ProfferedInviteCode` — the inbound wire type, in `common`

```rust
#[derive(Clone, StrNewtype)]
#[str_newtype(secret, serde)]                       // NEW macro variant (see below)
pub struct ProfferedInviteCode(String);
```

- Lives in **`common`** because the **client (wasm)** constructs it from the
  registration form and serializes it as the `register` argument.
- Surface (new `secret, serde` trailer): redacting `Debug`, `AsRef<str>`,
  `TryFrom<String>`, **and** the validating serde bridge — but \*\*no `Display`,
  `Deref`, `Borrow`, `From<Self> for String`, or `PartialEq<str>`.
- Its only jobs: be validated (on the server at `Deserialize`, routed through
  `validate_shape`), travel client→server, and convert to `InviteCode`. It
  cannot render or hand out an owned plaintext `String`. (Full ADR-0065 live
  client-side validation via `ValidatedInput<ProfferedInviteCode>` is a possible
  follow-up; see the `register` form note below.)

### `InviteCode` — the validated domain type, in `host`

```rust
#[derive(Clone, StrNewtype)]
#[str_newtype(secret)]                               // the EXISTING secret variant
pub struct InviteCode(String);

impl TryFrom<ProfferedInviteCode> for InviteCode {   // p.as_ref().parse(); can't actually fail
    type Error = common::token::InvalidTokenShape;
    /* ... */
}
```

- Lives in **`host`** — a **server-only** crate (web enables it only under its
  `server` feature). So `InviteCode` **cannot be named by client wasm code at
  all**: "never appears client-side" is structural, not conventional. `storage`
  depends on `host`, so storage records and traits can speak `InviteCode`.
- Surface (existing `secret` trailer): redacting `Debug`, `AsRef<str>`,
  `TryFrom<String>` — no serde, no `Display`. The
  `ProfferedInviteCode → InviteCode` conversion is `p.as_ref().parse()`
  (fallible, but the shared validator makes it always succeed; we do not rely on
  infallibility).

### The new macro variant — `#[str_newtype(secret, serde)]`

`macros/src/str_newtype.rs` gains a `secret, serde` option: the existing
`secret_trailer` **plus** the serde bridge (the same borrowing `Serialize` and
`FromStr`-routing `Deserialize` the non-secret path already emits). It is the
only variant that pairs redacting `Debug` with (de)serialization — for a secret
that must cross the wire _inbound_ but is otherwise sealed.

### ADR-0063 amendment

The secret exception currently states a secret "omits the serde bridge … so a
secret cannot (de)serialize." Amend it: a secret that must cross the
serialization boundary as _inbound_ client→server data (a capability token
submitted by a client) may opt back into serde via
`#[str_newtype(secret, serde)]`, keeping every other secret restriction. Its
inbound-only role is enforced by a gate (below). `ProfferedInviteCode` is the
first user. Recorded via `jaunder-adr` (numberless draft in `docs/adr/drafts/`,
numbered at ship).

### The enforcing gate

An `xtask` static check (sibling of the ADR-0066 server-fn registrar guard and
the `test-backend-pattern` guard): `ProfferedInviteCode` may appear **only** as
a `#[server]` function parameter type — never as a return type, struct field, or
other signature position. This is what prevents the raw code from being _sent_
server→client (the leak is the transmission itself, independent of client
usability). Failure names the offending site.

## Propagation

### `storage` (`InviteCode`, from `host`)

- `InviteRecord.code`: `String` → `host::InviteCode`.
- `InviteStorage::use_invite(code: &InviteCode, …)`; binds `code.as_ref()`.
- `InviteStorage::create_invite(…) -> sqlx::Result<InviteCode>`: the generated
  `String` token is `INSERT`ed (`.bind(code.as_str())`), then wrapped via
  `InviteCode::try_from` (mapping the impossible failure to
  `sqlx::Error::Decode`, keeping `expect_used` clean).
- `AtomicOps::create_user_with_invite(…, invite_code: &InviteCode)` (trait +
  `sqlite`/`postgres` impls; binds `.as_ref()`).
- `helpers::build_invite_record` parses the trusted DB `code` column into
  `InviteCode` and so becomes **fallible**; a parse failure (data-integrity, the
  column is written only by `create_invite`) surfaces as `sqlx::Error::Decode`
  (`list_invites`) or `UseInviteError::NotFound` (`use_invite` detail path).

### `web` — inbound typed, **server→client code display removed**

- `register(invite_code: Option<ProfferedInviteCode>)` — the wire arg becomes
  the proffered type. The invite field renders only in invite-only mode (so
  open-mode submits omit the key → `None`) and carries `required` (blocking an
  empty submit client-side); a filled field arrives as `Some`, validated on the
  wire at `Deserialize`. Server-side: `Some(p)` → `InviteCode::try_from(p)`
  (mapping failure to a validation error) → `create_user_with_invite(&code)`.
  Replaces today's `Option<String>` + `non_empty_owned` handling. (A malformed
  non-empty code now fails deserialization rather than reaching the `not-found`
  path — an earlier, ADR-0022-aligned rejection.)
- **`create_invite` no longer returns the code.** `WebResult<String>` →
  `WebResult<()>` (still creates the invite, still emits the `Created` metric).
- **`InviteInfo` drops its `code` field**; `list_invites` returns metadata only
  (`created_at`/`expires_at`/`used_at`/`used_by`). `pages/invites.rs` drops the
  "Code: …" line and shows status instead.

### `server` (CLI — the interim reveal channel)

- `cmd_user_invite` composes and prints the **invitation URL** by deliberate
  `AsRef`: `println!("{}/register?invite_code={}", base_url, code.as_ref())`
  (`base_url` from site config). No `Display`/serde. This is how an operator
  obtains a shareable invite until #433 automates the same string into an email.

## Interim UX consequence (accepted)

Removing server→client code display degrades the **web** invite UI to
**metadata-only**: the web "create invite" action creates a valid invite but
reveals no code, and the list shows no codes. The operator's channel for a
_shareable_ invite is the **CLI** (`jaunder user invite` prints the URL). #433
restores a web-first flow by emailing the code directly. This is the deliberate
"command-line only until #433" interim.

## Tests

- `common::token`: `validate_shape` accept/reject.
- `common::invite`: `ProfferedInviteCode` FromStr valid/invalid, serde
  round-trip (deserialize validates, rejects bad input), `Debug` redacted.
- `host::invite`: `InviteCode` FromStr via the shared validator;
  `TryFrom< ProfferedInviteCode>` round-trip; `Debug` redacted; `AsRef`
  round-trip.
- `macros/tests/str_newtype.rs`: a `secret, serde` fixture — serde round-trip +
  redacting `Debug` + `AsRef`.
- `xtask`: unit test for the `ProfferedInviteCode`-placement gate (a fixture
  with a return-position use fails; parameter-position passes).
- Existing `storage`/`server` invite tests updated for the new signatures
  (behavior unchanged); `web` `create_invite`/`list_invites`/register tests
  updated for the `()` return, dropped `code` field, and typed arg.

## Acceptance

- `use_invite`, `create_invite`, `InviteRecord`, and `create_user_with_invite`
  speak `InviteCode`; `register` speaks `ProfferedInviteCode`.
- A raw invite code is never serialized **server→client** (enforced by the
  gate + `InviteCode` being `host`-only and serde-free).
- `InviteCode`/`ProfferedInviteCode` are secret-bearing (redacting `Debug`, no
  `Display`), `FromStr`-validated through the shared `validate_shape`.
- `cargo xtask validate --no-e2e` clean.

## Non-goals

- Emailing the code (#433) — the web delivery story; #400 is CLI-only interim.
- Sibling session/reset tokens (#17); the token generator; storage schema. The
  `invites.code` column stays plaintext (unchanged) and the wire _shapes_ are
  unchanged except the deliberate `create_invite`/`InviteInfo` code removal.
