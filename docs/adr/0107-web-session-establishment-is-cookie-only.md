# ADR-0107: Web session establishment is cookie-only

- Status: accepted
- Date: 2026-08-09
- Issue: [#533](https://github.com/jaunder-org/jaunder/issues/533)

## Context

`login` and `register` both mint a session token, set it as an `HttpOnly`
`session` cookie, and — until #533 — also returned the raw token as their `Ok`
payload.

An `HttpOnly` cookie exists precisely so page JavaScript cannot read the session
credential. Returning that same credential in the response body hands it
straight back to JS and undoes the protection: an XSS present at login or
registration time could exfiltrate a long-lived session token instead of
nothing. The two mechanisms were working against each other.

Nothing in the browser code ever used the payload. `LoginPage` matched `Ok(_)`
and discarded it; `RegisterPage` only inspected the `Err` arm. The only consumer
was a handful of server tests, which could reach the session through the cookie
instead. The leak was inertia, not a feature.

## Decision

**Session establishment for the web client is cookie-only.** A `#[server]` fn on
the auth path sets the `HttpOnly` `session` cookie and returns **no
session-token material** in its response body.

Concretely: `register` returns `()`, and `login` returns the complete advisory
`SessionUser` identity: the authenticated record's canonical `username` and
`is_operator`. It contains no credential; the `HttpOnly` cookie remains the
session credential.

### The deliberate exception

`create_app_password` (`web/src/sessions/api.rs`) returns `AppPassword`, which
carries a `token: RawToken` field. That is **not** a violation: showing the
secret exactly once, at creation, is the entire purpose of an app password, and
it is a credential for out-of-tree clients rather than a browser session. The
rule above is about the browser's session, and this endpoint does not establish
one.

### Why no machine gate

A structural "no `#[server]` fn returns a `RawToken`" check was considered and
declined. The app-password case is a token nested in a returned struct, not a
bare token return, so the check would have to walk field types — and it would
still need an allowlist for that one endpoint. The allowlist is the part that
rots: it grows entries nobody re-justifies, and a rule with exceptions recorded
only in a lint config teaches a future reader nothing.

If this invariant is ever worth enforcing structurally, the right shape is a
distinct secret type for the app password, so "a session token in a response
body" becomes unambiguously wrong everywhere and needs no exception list.

## Consequences

- Enforced today by assertions in `server/tests/web/web_auth.rs` that the
  `login` and `register` success bodies do not contain the token recovered from
  the `Set-Cookie` header. Asserting the token _value_, not the absence of a
  `"token"` field name, is what makes it meaningful — `register`'s body is now a
  bare `null`, so a field-name check would be vacuous.
- Server tests that need a session read it from the `Set-Cookie` header via
  `helpers::token_from_set_cookie`, the inverse of `helpers::session_cookie`.
  This exercises the delivery path a browser actually uses: a test that queried
  storage by `user_id` instead would still pass if the cookie stopped being set
  at all.
- There is deliberately **no bearer-token API for the web client**. If one is
  ever wanted, it is a deliberate design with its own threat model — not a leak
  preserved because removing it felt like a breaking change.
- Adding a new auth-path `#[server]` fn means honoring this rule by hand; the
  gate will not catch a violation. That is the accepted cost of declining the
  allowlist.
