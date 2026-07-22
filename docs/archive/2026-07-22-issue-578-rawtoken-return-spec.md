# Spec — #578: `login` / `register` return `RawToken`, not `String`

- Issue: [#578](https://github.com/jaunder-org/jaunder/issues/578)
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md),
  [ADR-0065](../../adr/0065-typed-wire-args.md),
  [ADR-0011](../../adr/0011-unified-observability.md) (credential redaction)
- Related: [#458](https://github.com/jaunder-org/jaunder/issues/458)
  (`RawToken`/`TokenHash`),
  [#500](https://github.com/jaunder-org/jaunder/issues/500) (typed reset/verify
  wire args)
- **Security-adjacent** (session credential): full review regardless of diff
  size.
- Date: 2026-07-22

## Problem

`login` and `register` mint a `RawToken` (the freshly created session
credential) and then erase it to `String` for the return trip:

- `web/src/auth/api.rs` — `login(…) -> WebResult<String>`;
  `Ok(raw_token.to_string())`.
- `web/src/registration/api.rs` — `register(…) -> WebResult<String>`;
  `Ok(raw_token.to_string())`.

`RawToken` (`common::token`) is already the wire _argument_ type for
password-reset / email-verification and carries the full wire-capable trailer
(`Display`, `Deref`, serde) plus a **hand-written redacting `Debug`** (ADR-0011
— it's a credential). The documented auth carve-out covers the inbound
_password_ args staying `String`; it does **not** cover the token return, which
currently flattens a redaction-protected credential back to a bare `String` at
the boundary.

## Decision

Return `WebResult<RawToken>` from both. In both fns `raw_token` is _already_ a
`RawToken` (from `SessionStorage::create_session(…) -> sqlx::Result<RawToken>`,
and passed to `set_session_cookie(&RawToken)`), so the body change is only
`Ok(raw_token.to_string())` → `Ok(raw_token)`.

- `web/src/auth/api.rs`: `login(…) -> WebResult<RawToken>`; add an **ungated**
  `use common::token::RawToken;` (the type is now named in the `#[server]`
  return signature, referenced on both the client and server builds);
  `Ok(raw_token)`; update the doc comment ("Returns the raw session token" → the
  typed `RawToken`).
- `web/src/registration/api.rs`: the same three edits for `register`.

### Client consumers — type annotation only

Both client pages already _discard_ the returned token value (the real auth is
the server-set cookie; the client only branches on `Ok`/`Err` for UI state). Two
sites carry an **explicit** `Result<String, WebError>` annotation that must
track the new type; the value stays discarded:

- `web/src/auth/component.rs` (LoginPage):
  `.map(|r: Result<String, WebError>| …)` → `Result<RawToken, WebError>` (the
  `Ok(_)` arm is unchanged). Add `use common::token::RawToken;`.
- `web/src/registration/component.rs` (RegisterPage):
  `.and_then(|r: Result<String, WebError>| r.err())` →
  `Result<RawToken, WebError>`. Add the import.

The unannotated `if let Some(Ok(_)) = …value().get()` effects (both pages) infer
the type and need no edit.

## Security invariants (preserved / improved)

- **Wire bytes unchanged.** `RawToken`'s serde bridge serializes as the same
  plain base64url string a `String` did, so the response body is byte-identical
  — the existing auth/session integration tests
  (`extract_token(body) -> RawToken`) and e2e are unaffected.
- **No new exposure.** The token still reaches the client (as before — the
  client needs the login to succeed; the cookie is server-set). Typing it does
  not add a transmission path.
- **Redaction gained on the return path.** The value now travels as a `RawToken`
  server-side up to serialization, so an accidental `{:?}` in a span/error on
  the return path redacts to `RawToken([redacted])` instead of leaking the raw
  credential — the point of ADR-0011. This is a net safety improvement.

## Out of scope

- The inbound `password` args stay `String`/`ProfferedPassword` (the documented
  auth carve-out; #500 covers reset/verify args).
- No change to `create_session`, `set_session_cookie`, cookie attributes, or the
  token generator.

## Tests

- The existing web auth/session integration tests already parse the response
  through `common::token::RawToken` (`extract_token`), so they exercise the
  typed return with no change; the auth e2e exercises the
  login→cookie→authenticated-session persist path.
- No new unit test is warranted (the type carries its own `common::token` tests;
  the change is a boundary retype with unchanged behavior). If the gate surfaces
  a web test asserting the return as `String`, update it to `RawToken`.

## Acceptance

- `login` / `register` return `WebResult<RawToken>`; no `raw_token.to_string()`
  at the return boundary.
- Both client consumers compile against the typed value (annotations updated);
  the persist path is exercised by the existing auth e2e.
- `cargo xtask validate --no-e2e` clean.
