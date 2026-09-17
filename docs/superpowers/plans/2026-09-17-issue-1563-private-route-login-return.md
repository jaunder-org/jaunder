# Private-route Login Return Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for a bounded task
> when useful. This outline exists because issue #1563 changes authentication
> and Session-routing semantics.

## Scope

In:

- One explicit, exhaustive `Public`/`Private` access classification at the SPA
  route declarations.
- Authoritative Session gating, safe return-destination handling, password and
  Passkey return navigation, focused browser evidence, visual proof, and
  architecture projection.

Out:

- Credential, cookie, Session-storage, Registration, recovery, server-rendering,
  and machine-authentication changes.
- Changes to operator authorization or to the public-route population.

## Task outline

- [x] Task 1: Make route access and return destinations one maintainable policy.
  - Contract: introduce a host-testable app-route policy with `Public` and
    `Private` classifications and a typed validated private destination
    preserving path, query, and fragment. Route declarations must pass through
    one local declaration/catalog mechanism that supplies both router
    construction and the matcher used by destination validation; adding an
    unclassified route or drifting the router, matcher, and approved private
    inventory must fail conformance tests.
  - Verification: `devtool run -- cargo xtask test-local -- -p web` proves the
    complete inventory, parameterized-route matching, round trips, and rejection
    of absolute, protocol-relative, backslash-path, public, server-owned,
    unknown, and malformed targets.

- [ ] Task 2: Gate private route presentation on authoritative Session
      reconciliation.
  - Contract: a shared private-route gate consumes `SessionContext.reconcile`;
    pending state mounts no private page, confirmed anonymous state captures the
    current validated destination and replaces navigation with Login, failure
    presents Retry without redirect, and authenticated state mounts the
    requested page. Authenticated-but-unauthorized behavior remains owned by the
    destination page/server boundary.
  - Verification: wasm/browser-focused tests prove pending, retry, anonymous,
    authenticated, operator-only, parameterized-route, cold-entry, and
    in-app-navigation behavior without a second document load.

- [ ] Task 3: Return confirmed password and Passkey authentication safely.
  - Contract: Login consumes only the typed private destination produced by
    Task 1. Confirmed password and Passkey outcomes navigate client-side to it,
    or to `/app` when absent/invalid; indeterminate outcomes do not navigate.
    Remove the password endpoint's unconditional `/app` redirect only as needed
    to leave confirmed navigation under the client Login flow, preserving its
    cookie-only `SessionUser` response and updating server integration
    assertions.
  - Verification: focused web/server tests pin redirect-response behavior and
    both outcome classes; focused `auth.spec.ts` and the existing Chromium
    virtual-authenticator Passkey flow prove exact path/query/fragment return
    plus direct-Login fallback.

- [ ] Task 4: Complete conformance, documentation, and visual evidence.
  - Contract: project the private-route gate and route-policy ownership into
    `docs/ARCHITECTURE.md`; keep the issue spec as the acceptance snapshot
    rather than a second runtime registry. Capture signed-out `/sessions`
    before/after desktop evidence through `visual-proof`.
  - Verification: run focused
    `devtool run -- cargo xtask e2e-local auth.spec.ts` and the applicable
    Passkey positional test, then use the repository gate required by
    `jaunder-iterate` before each focused commit.

## Risk checks

- The local-storage auth marker remains advisory; only successful Session
  reconciliation may admit private content or trigger anonymous redirect.
- Return validation cannot become an open redirect and cannot admit Login
  recursion, public routes, unknown SPA routes, or server-owned resources.
- Every current and future SPA route requires an explicit access classification
  at its declaration; no prose or parallel runtime list is the operational
  authority.
- ADR-0076 client-side navigation and ADR-0111 one-boot test discipline remain
  intact.
- Password and Passkey login converge on identical confirmed navigation while
  preserving indeterminate-outcome warnings.
- Removing the server Login redirect does not change cookie establishment,
  credential-free response bodies, metrics, or non-browser auth transports.
- Private components remain unmounted during pending/error/anonymous states, and
  retry cannot navigate before a new authoritative result.
- The implementation stays within thin-component limits by keeping matching,
  validation, and state projection in host-testable pure logic.
