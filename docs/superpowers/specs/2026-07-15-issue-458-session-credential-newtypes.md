# Spec — Session-credential newtypes (`RawToken`/`TokenHash`) + auth hardening

**Issues:** #458 (session credentials → `RawToken` vs `TokenHash`) bundled with #344
(auth/credential hardening + `From` asymmetry). One branch, one PR, closes both.

**Governing decision:** ADR-0063 (domain-value newtype convention) — this is an
_application_ of it, including its **secret-bearing exception** (redacting `Debug`,
ADR-0011). No new ADR.

**Split lineage:** #458 is track 2 of the former #17 (split into #457 IDs / #458
session credentials / #459 content hash).

---

## 1. Problem

Session credentials are stringly-typed and security-relevant. Nothing in the type
system distinguishes the **raw bearer token** from its **stored SHA-256 hash**:
`create_session -> String` (raw), `revoke_session(token_hash: &str)`,
`SessionRecord.token_hash: String`, `authenticate(raw_token: &str)`. Every
raw-vs-hash transposition compiles today — `revoke_session(raw_token)` typechecks,
storing the raw token in the hash column typechecks, `raw == stored_hash`
typechecks. Same footgun family as the numeric-ID transposition (#457).

Riding along (all pre-existing, surfaced by #334's cold review — #344): an empty
`session=` cookie short-circuit, an unsanitized `Set-Cookie` interpolation, a stale
doc comment about username comparison, and a missing `From` impl that masks
password-reset error classification as a 500.

## 2. Design decisions (resolved)

### Types — `common::token`

- **`RawToken(String)`** — the secret bearer token. `#[derive(Clone, StrNewtype)]`
  `#[str_newtype(secret, serde)]`: redacting `Debug` (`RawToken([redacted])`),
  `AsRef<str>`, `TryFrom<String>`, and the validating serde bridge (it **crosses the
  wire** in the app-password response, so serde is required). Hand-written `FromStr`
  delegates to `common::token::validate_shape` (non-empty, base64url charset), with
  `Err = common::token::InvalidTokenShape`. Note: `secret, serde` is used here for an
  **outbound** value (the server returns it), the opposite of `ProfferedInviteCode`'s
  inbound intent — deliberate, and the `ProfferedInviteCode` return-guard gate is
  type-specific so it does not apply (see Enforcement).
- **`TokenHash(String)`** — the stored/compared hash.
  `#[derive(Clone, PartialEq, Eq, Hash, StrNewtype)]` — the **std** `PartialEq`/`Eq`
  supply `TokenHash == TokenHash` (the macro's default trailer emits only
  `PartialEq<str>`/`PartialEq<&str>`, not `PartialEq<Self>`). The default
  `StrNewtype` trailer adds `Display`, `AsRef`, `Deref`, `Borrow`, `TryFrom`, serde;
  hand-written `FromStr` via `validate_shape` (`Err = InvalidTokenShape`). **Not**
  secret: it is compared (`is_current`, revoke ownership check), bound into SQL,
  rendered in the session-management UI, and crosses the wire in `SessionInfo`. A
  SHA-256 hash, not a bearer credential.
- **One generic pair**, shared by the session / password-reset / email-verification
  families. Not per-family types: the target bug is raw-vs-hash transposition
  _within_ a family; cross-family misuse already fails safe via separate tables.
  `InviteCode` remains its own domain newtype, minted from a `RawToken`.

### Machinery — `host` (server-only; keeps RNG/SHA-256 out of the wasm bundle)

- `generate() -> RawToken`, `hash(&RawToken) -> Result<TokenHash, _>` (fallible:
  base64 decode can fail), `generate_hashed() -> (RawToken, TokenHash)` (infallible —
  a freshly minted token always decodes; a documented `.expect` on that invariant,
  which also removes a spurious error path from every create site).
- **`hash` is the sole `RawToken -> TokenHash` bridge.** No `From<RawToken> for
  TokenHash`, no reverse conversion, no cross-type `PartialEq`.
- `storage::auth::{generate_token, hash_token}` **and** the
  `storage::helpers::generate_hashed_token` wrapper are **removed** (all three are the
  old stringly doors); `storage` becomes a pure consumer of `host`'s token machinery
  (`storage -> host` already exists; `host` does not depend on `storage`, so no
  cycle).

### Module-home rationale (why types and machinery are split)

Both tokens cross the server-fn wire (`AppPassword.token` = raw; `SessionInfo.token_hash`
= hash), so the **types** must live in the only wasm-compiled shared crate (`common`).
The **machinery** needs `rand`/`getrandom`/`sha2` — server-only work the client never
runs — so it must **not** be co-located into `common` (that would ship server crypto
into every client bundle). Hence: contract in `common`, producer in `host`. Mirrors
the `ProfferedInviteCode` (common) / `generate_token` (server) precedent.

### Enforcement

No xtask gate. The type system makes the transposition **uncompilable** (distinct
types, private field, no cross-conversion/equality); redacting `Debug` makes log
leaks harmless. "Proves it bites" is discharged by a **`compile_fail` test**, not a
runtime gate — the invite-code egress gate does not map here (our raw token is
server-minted and _deliberately_ egressed at two sanctioned points).

## 3. Acceptance criteria

Each is stated so ship's conformance review can tell delivered from not.

### #458 — the newtypes

- **AC1.** `common::token::RawToken` exists as specified. `"".parse::<RawToken>()`
  and a non-base64url string both `Err`; `format!("{raw:?}")` contains `[redacted]`
  and never the token body. _(unit tests in `common`)_
- **AC2.** `common::token::TokenHash` exists as specified; serde round-trips
  (`serde_json` of a `TokenHash` deserializes back equal); two equal hashes compare
  equal. _(unit tests in `common`)_
- **AC3.** `host` exposes `generate`/`hash`/`generate_hashed`; `hash` of a known raw
  token equals the pre-refactor `hash_token` output for the same input (golden
  vector, so behavior is byte-identical). There is no path from `TokenHash` back to
  `RawToken` and no `RawToken -> TokenHash` conversion other than `hash`. _(unit test
  + `compile_fail`)_
- **AC4.** The `SessionStorage` trait and both backends speak the newtypes:
  `create_session(...) -> Result<RawToken>`, `authenticate(&RawToken)`,
  `revoke_session(&TokenHash)`, `SessionRecord.token_hash: TokenHash`. Existing
  session tests pass unchanged in behavior against **both** sqlite and postgres
  (backend parity). _(existing storage session suite, both backends)_
- **AC5.** Password-reset and email-verification paths consume the typed helpers:
  `confirm_password_reset(&RawToken, ...)`, `create_password_reset`/
  `create_email_verification` return `RawToken` / store `TokenHash` via
  `generate_hashed()`; their `use`/confirm lookups hash via `host::hash`. Existing
  reset/email suites pass on both backends. _(existing suites)_
- **AC6.** Invite generation mints a `RawToken` (`host::generate`) and wraps it into
  `InviteCode`; no `storage::auth::generate_token` reference remains. _(compile +
  existing invite suite)_
- **AC7.** Web boundary is typed: `SessionInfo.token_hash: TokenHash`,
  `AppPassword.token: RawToken`, `revoke_session(token_hash: TokenHash)` server fn;
  the session-list render and hidden-form-field revoke round-trip still work. _(web
  unit + e2e session/app-password tests)_
- **AC8.** A `compile_fail` test proves the distinction bites: passing a `RawToken`
  to `revoke_session`, `raw == some_hash`, and any `RawToken`<->`TokenHash`
  conversion other than `host::hash` each fail to compile.

### #344 — auth hardening

- **AC9 (item 1).** `Username` is confirmed case-normalizing (`FromStr` lowercases);
  the stale comment at `web/src/auth/server.rs:141` ("The pure comparison lives in
  `common::auth`") is corrected/removed. A test asserts Basic-auth username matching
  is case-insensitive: a token issued for `Alice` authenticates a request whose
  Basic-auth username is `alice`. _(web auth test)_
- **AC10 (item 2).** `Credential.token` becomes `RawToken` across **all three**
  sources (cookie, `Authorization: Bearer`, `Basic` password). Each source parses its
  string into a `RawToken` and contributes a credential **only** on a successful
  parse; a source whose value fails to parse (empty/non-base64url) is skipped rather
  than short-circuiting. Concretely: a request with an empty `session=` cookie **plus**
  a valid `Authorization: Bearer`/`Basic` header authenticates via the header (not
  rejected on the empty cookie token). _(host auth test)_
- **AC11 (item 3).** `session_cookie_header` takes `&RawToken`; the base64url-charset
  invariant (no header-special characters) is now type-guaranteed, with a comment
  recording the invariant at the interpolation site. _(covered by AC1 charset
  validation; documented)_
- **AC12 (item 4).** `From<ConfirmPasswordResetError> for host::error::InternalError`
  exists, mapping `NotFound`/`Expired`/`AlreadyUsed` -> `validation` and `Internal`
  -> `storage` (mirroring `RegisterWithInviteError`); `web::password_reset::
  confirm_password_reset` uses `?` rather than `map_err(InternalError::storage)`, so
  confirming an expired/used/unknown reset token yields a **validation-class** error,
  not a masked storage 500. _(a test asserts the error class/kind for an expired
  token is validation, not storage)_

## 4. Non-goals

- **Not** replacing the client-exposed `token_hash` revoke handle with an opaque
  session id. Sending the hash to the client and revoking-by-hash is pre-existing;
  it fails safe (server re-checks ownership, `web/src/sessions/mod.rs:80`). Candidate
  follow-up, out of scope here.
- **Not** per-family token types (`SessionToken`/`ResetToken`/…). Generic
  `RawToken`/`TokenHash` per the issue framing.
- **Not** newtyping the numeric IDs (#457) or content hash (#459) — separate tracks.
- **Not** strengthening `RawToken`'s invariant to "decodes as base64" (charset-only,
  matching `validate_shape`); decode failure surfaces at `host::hash` as an auth
  failure, as today.

## 5. Blast radius

Newtype threads through ~8 `storage` files (`sessions`, `password`, `email`,
`helpers`, `auth`, `atomic` — the `From<ConfirmPasswordResetError>` impl lives here
by the orphan rule, beside its `RegisterWithInviteError` mirror — and both backend
`sessions`/`mod`), ~5 `web` files (`sessions`, `pages/sessions`, `auth/server`,
`auth/mod`, `password_reset`), `host` (`auth`, new `token` module), `common` (new
`token` types). ~19 test occurrences in `server/tests`. Backend parity (sqlite +
postgres) and coverage policy apply.
