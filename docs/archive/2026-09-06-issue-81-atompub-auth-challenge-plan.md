# AtomPub Basic Authentication Challenge Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` when delegation is
> useful. This outline exists because issue #81 changes public AtomPub
> authentication semantics.

Authoritative spec:
`docs/superpowers/specs/2026-09-06-issue-81-atompub-auth-challenge.md`

## Scope

In:

- Project the fixed HTTP Basic challenge onto authentication-originated 401
  responses from authenticated AtomPub routes.
- Cover each authentication rejection class at the running router boundary.
- Extend the revoked-App-Password e2e assertion and the MarsEdit acceptance
  checklist.

Out:

- Shared `auth::User` rejection behavior and non-AtomPub routes.
- New credentials, error bodies, authorization rules, or live MarsEdit
  automation.

## Task outline

- [x] Task 1: Make the authenticated AtomPub router emit the Basic challenge and
      prove the complete server contract.
  - Contract: authenticated routes return exactly
    `WWW-Authenticate: Basic realm="Jaunder AtomPub"` on
    authentication-originated 401 responses; the public RSD route and shared
    `auth::User` response mapping remain outside this projection.
  - Coverage: missing credentials; malformed and unsupported explicit
    authorization with no cookie fallback; unknown or revoked credential; Basic
    username mismatch; successful Basic, Bearer, and cookie requests; unchanged
    empty 401 body; and unchanged headers/representations at each distinct
    excluded boundary: public RSD, raw `/media/proxy`, Leptos server-function
    auth, and cookie-only client telemetry.
  - Verification: focused dual-backend AtomPub integration tests plus the narrow
    host checks selected by `jaunder-iterate`.

- [x] Task 2: Carry the challenge contract through running-application and
      human-client acceptance surfaces.
  - Contract: the revoked-App-Password Playwright flow asserts the exact
    challenge; the MarsEdit checklist instructs a human to observe the
    challenge-driven credential prompt and successful App Password
    authentication.
  - Verification: focused AtomPub Playwright scenario against the running
    application and documentation formatting/link checks selected by
    `jaunder-iterate`.

## Ordering and handoff

- Task 1 establishes the server behavior consumed by Task 2.
- Each completed task updates its checkbox before its `jaunder-commit` gate.
- No shared cross-task API is introduced; Task 2 consumes only the public HTTP
  response contract from the approved spec.

## Risk checks

- Scope the response projection structurally to authenticated `/atompub/*`
  routes so `/~{username}/rsd.xml`, `/media/proxy`, Leptos server functions, and
  client telemetry cannot inherit it.
- Preserve all response properties except the challenge header: status, empty
  body, existing headers, and cookie-retirement behavior.
- Do not challenge internal authentication failures reported as 500 or
  non-authentication failures from AtomPub handlers.
- Preserve explicit `Authorization` precedence and rejection without ambient
  cookie fallback.
- Keep the challenge value static and exact; do not derive the realm from
  usernames, hosts, configuration, or request data.
