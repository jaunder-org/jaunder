# Plan — #400: `InviteCode` / `ProfferedInviteCode` newtypes

Spec:
[`docs/superpowers/specs/2026-07-13-issue-400-invite-code-newtype.md`](../specs/2026-07-13-issue-400-invite-code-newtype.md).
Read it for the _what/why_; this plan is the _how_.

## Review header

**Goal.** Make the invite code a type, with **directional secrecy** encoded
structurally: `ProfferedInviteCode` (inbound, in `common`, wasm-visible, serde)
that a client submits, and `InviteCode` (validated domain type, in the
server-only `host` crate, no serde) that never reaches a client. A shared
`validate_shape` is the single invariant. One coherent goal under #400 — it adds
a small macro variant, an ADR amendment, and an xtask gate, all in service of
it.

**Scope — in:**

- `macros` — a `#[str_newtype(secret, serde)]` variant.
- ADR-0063 amendment (draft) sanctioning a serde-inbound secret.
- `common::invite` — `validate_shape`, `InvalidInviteCode`,
  `ProfferedInviteCode`.
- `host::invite` — `InviteCode` + `TryFrom<ProfferedInviteCode>`.
- `storage`/`web`/`server` threading, incl. **removing server→client code
  display** (`create_invite` returns `()`, `InviteInfo` drops `code`).
- An xtask gate pinning `ProfferedInviteCode` to `#[server]` parameters.

**Scope — out:** emailing the code (#433, the web delivery follow-on; #400 is
CLI-only interim); #17 tokens; schema/generator changes.

**Tasks:**

1. `#[str_newtype(secret, serde)]` macro variant (+ macro tests).
2. ADR-0063 amendment draft (`jaunder-adr`).
3. `common::invite` — shared validator + `ProfferedInviteCode`.
4. `host::invite` — `InviteCode` + `TryFrom<ProfferedInviteCode>`.
5. Thread `InviteCode` through `storage`/`web`/`server` + remove server→client
   code display (one atomic cross-crate commit).
6. xtask `ProfferedInviteCode`-placement gate.

**Key risks / decisions:**

- **Directional secrecy is structural, not serde.** `InviteCode` in `host`
  (server-only crate) can't be named in wasm; the gate keeps
  `ProfferedInviteCode` off return positions. Together: a code is never sent
  server→client.
- **Task 5 is atomic across crates.** Changing `InviteStorage`/`AtomicOps`
  signatures breaks `web`/`server` until their call sites move; the
  whole-workspace gate compiles all crates, so storage threading + web changes +
  server changes + test updates land in one green commit.
- **Interim UX (accepted).** The web invite UI goes metadata-only; the CLI
  (`jaunder user invite` → invitation URL) is the reveal channel until #433.
- **Trusted-DB re-parse.** `build_invite_record` becomes fallible (no infallible
  `InviteCode` constructor; `FromStr` is the chokepoint) → `sqlx::Error::Decode`
  / `NotFound` on the impossible corrupt-code case.

**For agentic workers:** execute with `jaunder-iterate`, delegating a task to a
subagent via `jaunder-dispatch` when useful. Tick checkboxes in real time.

## Global constraints

- No `Co-Authored-By` trailer. Run `cargo xtask check` clean before each commit
  (`jaunder-commit`); serialize edit → gate → commit.
- Storage tests follow the dual-backend template (`#[apply(backends)]`).
- Import discipline: import `InviteCode` / `ProfferedInviteCode` /
  `validate_shape` at module tops; no fully-qualified paths at call sites.
- `xtask/*.rs` is not coverage-measured; run its tests via
  `--manifest-path xtask/Cargo.toml` (not `-p xtask`).

---

## Task 1 — `#[str_newtype(secret, serde)]` macro variant

**Files:** `macros/src/str_newtype.rs`, `macros/tests/str_newtype.rs`.

**How.** Today `is_secret` returns `bool` and errors on any option other than
`secret`. Generalize it to parse a **set** of flags (`secret`, `serde`) and
return them (e.g. a small `struct Opts { secret: bool, serde: bool }`), erroring
on unknown options and on `serde` _without_ `secret` (plain already has serde).
Factor the two serde impls the non-secret path emits into a
`fn serde_impls(name) -> TokenStream`. Dispatch:

- neither → existing full non-secret trailer (unchanged).
- `secret` only → existing `secret_trailer` (unchanged).
- `secret, serde` → `secret_trailer(name)` **+** `serde_impls(name)`.

**Test (TDD, `macros/tests/str_newtype.rs`).** Add a fixture
`#[str_newtype(secret, serde)] struct SecretWire(String);` with a hand `FromStr`
(charset-any for the fixture). Assert: serde round-trips
(`to_string`/`from_str`), `Deserialize` rejects an input its `FromStr` rejects,
`Debug` is redacted (`SecretWire([redacted])`), `AsRef<str>` works. (Absence of
`Display`/`Deref` is a compile property, not asserted at runtime.)

**Run:** `cargo nextest run --manifest-path macros/Cargo.toml str_newtype` →
FAIL then PASS. `cargo xtask check` → commit
(`feat(macros): add #[str_newtype(secret, serde)] variant (#400)`).

## Task 2 — ADR-0063 amendment (draft)

**Files:** edit `docs/adr/0063-domain-value-newtype-convention.md` (its "secret
exception" paragraph) to carve out the serde-inbound case; the amendment is part
of the existing ADR, so no new draft file is needed — note the change inline and
let `cargo xtask adr` keep the README table in sync at ship. Follow
`jaunder-adr`.

**Content.** A secret that must cross the serialization boundary as _inbound_
client→server data (a capability token a client submits) may opt back into the
serde bridge via `#[str_newtype(secret, serde)]`, keeping every other secret
restriction; its inbound-only role is enforced by an xtask gate (Task 6).
`ProfferedInviteCode` is the first user; `InviteCode` remains a plain `secret`.

**Run:** `prettier -w` the ADR before staging (`jaunder-commit`);
`cargo xtask check` → commit
(`docs: amend ADR-0063 for serde-inbound secrets (#400)`).

## Task 3 — `common::invite`: shared validator + `ProfferedInviteCode`

**Files:** create `common/src/invite.rs`; `pub mod invite;` in
`common/src/lib.rs`.

```rust
use std::str::FromStr;
use macros::StrNewtype;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("invite code must be non-empty and use only base64url characters ([A-Za-z0-9_-])")]
pub struct InvalidInviteCode;

/// Single source of truth for invite-code shape (non-empty, base64url-no-pad
/// charset; not length-pinned). Both invite newtypes' `FromStr` delegate here.
pub fn validate_shape(s: &str) -> Result<(), InvalidInviteCode> {
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(InvalidInviteCode);
    }
    Ok(())
}

/// A raw invite code as submitted by a client. Inbound-only (Task 6 gate); its
/// only jobs are to validate, travel client→server, and become an `InviteCode`.
#[derive(Clone, PartialEq, Eq, Hash, StrNewtype)]
#[str_newtype(secret, serde)]
pub struct ProfferedInviteCode(String);

impl FromStr for ProfferedInviteCode {
    type Err = InvalidInviteCode;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_shape(s)?;
        Ok(ProfferedInviteCode(s.to_owned()))
    }
}
```

**Tests (in-file):** `validate_shape` accept (`"abcABC012-_"`) / reject (`""`,
`"has space"`, `"plus+code"`, `"slash/code"`); `ProfferedInviteCode` FromStr
valid/invalid; serde round-trip + deserialize rejects `"\"a b\""`; `Debug`
redacted.

**Run:** `cargo nextest run -p common invite` → PASS. `cargo xtask check` →
commit (`feat(common): add validate_shape + ProfferedInviteCode (#400)`).

## Task 4 — `host::invite`: `InviteCode` + conversion

**Files:** create `host/src/invite.rs`; `pub mod invite;` in `host/src/lib.rs`.

```rust
use std::str::FromStr;
use common::invite::{validate_shape, InvalidInviteCode, ProfferedInviteCode};
use macros::StrNewtype;

/// A validated invite code. Server-only (this crate is not built for wasm), so it
/// can never be named client-side; secret-bearing and serde-free, so it can never
/// be serialized to a client.
#[derive(Clone, PartialEq, Eq, Hash, StrNewtype)]
#[str_newtype(secret)]
pub struct InviteCode(String);

impl FromStr for InviteCode {
    type Err = InvalidInviteCode;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_shape(s)?;
        Ok(InviteCode(s.to_owned()))
    }
}

impl TryFrom<ProfferedInviteCode> for InviteCode {
    type Error = InvalidInviteCode;
    fn try_from(p: ProfferedInviteCode) -> Result<Self, Self::Error> {
        p.as_ref().parse() // shared validator ⇒ always Ok, but we don't rely on it
    }
}
```

Confirm `host` depends on `common` (it does: `host/Cargo.toml`).

**Tests (in-file):** `InviteCode` FromStr valid/invalid;
`TryFrom< ProfferedInviteCode>` round-trip
(`"code123".parse::<ProfferedInviteCode>()` → `InviteCode::try_from`); `Debug`
redacted; `AsRef` round-trip.

**Run:** `cargo nextest run -p host invite` → PASS. `cargo xtask check` → commit
(`feat(host): add InviteCode domain type (#400)`).

## Task 5 — Thread `InviteCode` through `storage`/`web`/`server` (atomic)

One green cross-crate commit. Make every edit, then run the invite tests as the
safety net. Includes the deliberate **server→client code-display removal**.

### 5a. `storage` (uses `host::InviteCode`)

- `storage/src/invites.rs` (`use host::invite::InviteCode;`):
  `InviteRecord.code: InviteCode`; `use_invite(code: &InviteCode, …)` (bind
  `code.as_ref()` at UPDATE + detail SELECT);
  `create_invite(…) -> sqlx::Result<InviteCode>` (INSERT binds the generated
  `String` via `.as_str()`, then
  `InviteCode::try_from(code).map_err(|e| sqlx::Error::Decode(Box::new(e)))`);
  `list_invites` collects the fallible mapper into `Result<Vec<_>, _>` → map to
  `sqlx::Error::Decode`. Closed-pool unit tests:
  `use_invite(&"code".parse().unwrap(), 1)`.
- `storage/src/helpers.rs`: `build_invite_record` / `invite_record_from_row`
  parse `code` → `InviteCode` and return
  `Result<InviteRecord, InvalidInviteCode>`; update the two helper unit tests
  (`.unwrap()`, valid-charset `"code"`).
- `storage/src/atomic.rs`:
  `AtomicOps::create_user_with_invite(…, invite_code: &InviteCode)`; update
  `create_user_with_invite_*` unit tests to pass
  `&"<code>".parse::<InviteCode>().unwrap()`.
- `storage/src/sqlite/mod.rs`, `storage/src/postgres/mod.rs`: impl signature +
  `.bind(invite_code.as_ref())` at each bind (postgres has two). (No change to
  the per-backend `invites.rs` — they are `pub type` aliases.)

### 5b. `web` (inbound `ProfferedInviteCode`; drop server→client code)

- `web/src/auth/mod.rs` `register`: arg
  `invite_code: Option<ProfferedInviteCode>`
  (`use common::invite::ProfferedInviteCode;`). Replace the
  `and_then( non_empty_owned)` block:
  `Some(p) => { let code = InviteCode::try_from(p) .map_err(|_| InternalError::validation("invalid invite code"))?; atomic .create_user_with_invite(&username, &password, None, false, &code) … }`,
  `None => Err(InternalError::validation("invite code required"))`.
  (`use host::invite::InviteCode;` under the server cfg.)
- `web/src/pages/auth.rs` (register form): the invite field sends `None` when
  blank, `Some(ProfferedInviteCode)` when filled (client-side validate per
  ADR-0065). Ensure a blank field is not submitted as `Some("")`.
- `web/src/invites/mod.rs`: `create_invite` → `WebResult<()>` (create, keep the
  `Created` metric, return `Ok(())`); remove `code` from `InviteInfo`;
  `list_invites` maps records to the code-less `InviteInfo`.
- `web/src/pages/invites.rs`: drop the `"Code: " {i.code…}` line; render status
  (created/expires/used) only.

### 5c. `server` (CLI reveal)

- `server/src/commands.rs` `cmd_user_invite`: obtain `base_url` from site config
  and `println!("{}/register?invite_code={}", base_url, code.as_ref())`.

### 5d. Tests

- `server/tests/storage/storage.rs` invite tests: `create_invite` yields
  `InviteCode`; `use_invite(&code, …)`; `create_user_with_invite(…, &code)`;
  literals via `&"…".parse::<InviteCode>().unwrap()`; compare via
  `record.code.as_ref()`.
- `web`/`server` web tests (`server/tests/web/web_auth.rs`, …): register submits
  a `ProfferedInviteCode`; `create_invite` asserts `()`; `list_invites` asserts
  no code field.

**Run:** `cargo nextest run -p storage invite`, `-p storage atomic`, `-p server`
invite/web tests, `-p web` → PASS. `cargo xtask check` → commit
(`refactor(storage,web,server): thread InviteCode; stop returning codes to clients (#400)`).

## Task 6 — xtask `ProfferedInviteCode`-placement gate

**Files:** `xtask/src/steps/proffered_invite_code_check.rs` (new), register it
in `xtask/src/lib.rs`. Model on `xtask/src/steps/server_fn_registrar_check.rs`
and `test_pattern_check.rs`.

**How.** Walk the source tree; for every occurrence of `ProfferedInviteCode` in
a type position, assert it is a parameter of a `#[server]`-attributed `fn` —
fail (naming file:line) on any return-type, struct-field, `let`, or other
position. Use the same parsing approach the sibling guards use (syn-based).

**Test:** an xtask unit test with two in-memory fixtures — a `#[server]`-param
use passes; a return-type use fails. Run via
`cargo nextest run --manifest-path xtask/Cargo.toml proffered`.

**Run:** wire the step into the `check`/`validate` static-check set;
`cargo xtask check` (the gate now passes — Task 5 left `ProfferedInviteCode`
only on `register`) → commit
(`feat(xtask): gate ProfferedInviteCode to #[server] params (#400)`).

---

## Self-review checklist

- [ ] Acceptance met: storage/host speak `InviteCode`, `register` speaks
      `ProfferedInviteCode`; no raw code serialized server→client (gate +
      host-only + serde-free); both types secret-bearing, validated via
      `validate_shape`.
- [ ] `cargo xtask validate --no-e2e` clean.
- [ ] Web invite UI is metadata-only; CLI prints the invitation URL.
- [ ] No `.expect()`/`.unwrap()` on production paths; no schema/generator
      change.
