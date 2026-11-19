# Issue #2003: server-function coverage verdict flips between runs of one commit

## Outcome

For one commit, `server-fn-coverage-verify` gives the same verdict on every run.
Sign-out sends exactly one `logout` request. The committed snapshot
(`docs/coverage/server-fns.json`, `orphans: {}`) stays current, and the verifier
is not changed or relaxed.

## Root cause (recorded on the issue)

The verifier is deterministic. Running it locally against the two CI runs'
downloaded `sqlite-chromium-unsplit` captures reproduces both verdicts on one
tree. The merge-group capture passes. The push capture adds
`"orphans": {"auth::get_session": ["no-parent"]}`. That is one
`POST /api/auth/get_session` carrying no `traceparent` at all, the only
parentless request in the run.

It reached the server 14 ms after `persistent-session.spec.ts` ended. That test
signs out on a second page (`nextPage`) and closes it as soon as the URL becomes
`/`. The app's sign-out tail is still running at that moment:

| Step | Request                   | Page URL when sent |
| ---- | ------------------------- | ------------------ |
| 1    | `logout`                  | `/logout`          |
| 2    | `get_session` (refetch)   | `/logout`          |
| 3    | `get_session` (route `/`) | `/`                |
| 4    | `logout` **again**        | `/`                |

A local `dev e2e-local` probe confirmed this sequence. Every sign-out in the CI
capture (11 tests) sends exactly two `logout` requests. A request still being
sent while Playwright closes the page goes out without the context's extra
headers, so the server sees no parent. Header loss during close is inferred from
timing, not reproduced. The rest is observed.

The double `logout` comes from `LogoutPage` dispatching again once the location
is `/`. `/logout` renders in the private app shell and `/` in the public one.
Switching between them rebuilds `<Outlet/>` (`web/src/app/component.rs`, the
`is_public_presentation_path` closure), which briefly runs `LogoutPage` a second
time.

## Approach and tradeoffs

1. **Test:** before clicking Sign out, `persistent-session.spec.ts` starts
   tracking every `/api/` request on `nextPage`. It records each one when it
   starts and clears it on `requestfinished` or `requestfailed`. After the URL
   reaches `/`, the test waits until the anonymous home page has rendered and no
   tracked request is still pending. Only then does it close the page. This
   covers `get_session`, `list_local_timeline` and any other request `/` sends,
   in whatever order they run. After that drain, the test asserts that sign-out
   sent exactly one `/api/auth/logout`, which is the regression proof for
   step 4. The tracking lives in this test only; it is not a harness change.
2. **App:** `LogoutPage` dispatches `logout` only while the current location is
   `/logout`, read untracked. A copy mounted after the redirect does nothing.
   Loading `/logout` directly (SSR, then hydration at `/logout`) and navigating
   there from a private page still sign out. A comment notes that the guard is
   coupled to the route declared in `app_routes!`.

Alternatives rejected:

- **Draining in-flight requests in the harness before every close.** You chose
  the test-level fix. One source has been observed, and a drain cannot catch a
  request that starts during close.
- **Restructuring the app shell so `<Outlet/>` survives a public/private
  switch.** This is the general fix for any route mounted on the wrong side of
  that switch. It is a presentation refactor that needs visual proof, so it goes
  to a follow-up issue.

Accepted limitation: another test that closes a page mid-request could still
produce an orphan. ADR-0081 treats trailing traffic after a test's last
assertion as normal and attributable. The new fact is that a request sent while
the page is closing loses its `traceparent`. If it happens again, the gate fails
loudly, as designed, and names the fn.

## Acceptance

- `dev e2e-local persistent-session.spec.ts` passes, including the single-logout
  assertion. The assertion fails on the current `LogoutPage`.
- Smoke check only: the local capture from that run has no parentless `/api/`
  request spans. The deterministic proofs are the single-logout assertion and
  the verify step in CI.
- Snapshot and verifier code are unchanged in the diff.
- CI on the PR, then `main` after merge, shows `server-fn-coverage-verify`
  `[ ok ] … snapshot current`.
- A root-cause comment is posted on #2003. A follow-up issue is filed for the
  shell remount.

## Boundaries

- No change to `host-proof` coverage extraction, snapshot format or comparison.
- No harness-wide request-drain machinery.
- No app-shell restructuring (follow-up only).
- The other push-to-main failures (Playwright timeout, duration pressure) are
  out of scope.
