# Private-route login return

Issue: [#1563](https://github.com/jaunder-org/jaunder/issues/1563)

## Outcome

A signed-out visitor entering an authentication-required browser route is sent
to Login without seeing partial private content. Successful password or Passkey
authentication returns them to the exact requested internal destination.

## Load-bearing decisions

- The policy applies uniformly to this authoritative private-route inventory:
  `/app`, `/profile`, `/profile/email`, `/sessions`, `/passkeys`, `/audiences`,
  `/invites`, `/admin/backups`, `/admin/site`, `/admin/smtp`, `/admin/websub`,
  `/posts/new`, `/drafts`, `/scheduled`, `/media`, `/themes`, `/history`,
  `/posts/:post_id/history`, `/posts/:post_id/history/:revision_id`, and
  `/posts/:post_id/edit`. Parameterized entries match only their SPA route
  shape. This list is the issue's acceptance snapshot. Runtime ownership is one
  exhaustive `Public`/`Private` classification beside the route declarations;
  the gate and return validator share it, and conformance testing rejects an
  unclassified route or policy/router divergence.
- Anonymous and unauthorized remain distinct states. A confirmed missing Session
  redirects to Login; an authenticated User without sufficient authority
  receives the existing permission-denied behavior.
- Private content and controls do not render while authoritative Session
  reconciliation is pending.
- A Session-reconciliation failure is an operational error. It is presented as
  such and never converted into anonymous state or a login redirect.
- The automatic private-route-to-Login navigation replaces the current browser
  history entry, preventing Back from immediately repeating the redirect.
- The return destination preserves the requested internal path, query string,
  and fragment.
- A valid return target starts with exactly one `/`, contains no authority or
  backslash path separator, parses as a relative URL against Jaunder's own
  origin, and has a parsed pathname matching the private-route inventory above.
  Its query and fragment may be empty or arbitrary URL data and are preserved.
  Public routes (including `/login` and `/logout`), unknown SPA routes,
  server-owned paths, absolute URLs, protocol-relative URLs, and malformed
  values are invalid and fall back to Home (`/app`).
- Confirmed password and Passkey authentication obey the same return policy. An
  indeterminate authentication outcome retains its existing cautionary state and
  does not navigate as if success were confirmed.
- A direct visit to `/login`, with no valid private return destination,
  continues to Home after confirmed authentication.
- Navigation remains client-side in accordance with ADR-0076; this work does not
  introduce a full document load.

## Acceptance

- Entering `/sessions` without a live Session withholds Sessions controls,
  confirms anonymous state, and replaces the current route with Login.
- Confirmed password and Passkey login each preserve a private destination's
  path, query, and fragment; direct Login still navigates to `/app`.
- Return-target tests cover a valid parameterized private route plus rejected
  absolute, protocol-relative, backslash-path, public, server-owned, unknown,
  and malformed targets; every rejection resolves to `/app`.
- Route conformance accounts for every declared route and every private
  acceptance-snapshot member. Representative cold-entry browser tests cover a
  member route, an operator-only route, and a parameterized route.
- A live Session reaches private content directly; an authenticated non-operator
  on an operator route retains the existing authorization rejection.
- A pending Session check shows a non-sensitive loading state without mounting
  private controls. A failed check shows an error with a visible Retry control;
  Retry reruns authoritative Session confirmation without redirecting first.
- Browser tests cover cold entry and in-app navigation without adding a second
  document load, and exercise both password and Passkey return behavior.
- Visual proof compares signed-out `/sessions` before and after the change at a
  desktop viewport, showing the prior partial private page/error state beside
  the resulting Login presentation.
- The architecture view describes the private-route authentication and return
  policy beside the shared Session context.

## Boundaries

- This does not change credentials, Session storage, cookie policy,
  Registration, recovery, machine authentication, or operator authority.
- This does not make public routes private or redirect anonymous Local visitors.
- This does not add server-side rendering, a new HTTP endpoint, or a browser
  bearer token.
- This does not redesign Login, private-page presentation, or navigation beyond
  the authentication gate and validated return behavior.
