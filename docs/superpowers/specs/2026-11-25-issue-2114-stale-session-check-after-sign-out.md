# A session check sent before sign-out must not undo it

## Outcome

After a successful sign-out, the browser always lands on Local (`/`) as an
anonymous viewer. Today it occasionally ends on `/login?return_to=%2Fapp`
instead. The masthead wait in `persistent-session.spec.ts:15` then times out
(#2114).

## Root cause (from the retained trace, run 45613739403)

The second page's frame URLs go
`/account → /logout → / → /app → /login?return_to=%2Fapp`. `/` → `/app` comes
from the session context's missing-marker recovery (ADR-0193). Its only trigger
is a `get_session` result that is authenticated while the client session is
empty and the path is `/`.

1. Going to `/logout` starts the per-navigation session check (`get_session`).
   It is sent with the still-valid cookie, so it answers with the User.
2. Logout succeeds. `clear_session` empties the client session and removes the
   marker, and the redirect moves the app to `/`.
3. The session check is a Leptos `Resource`. Its reload loop awaits one check at
   a time. A path change only queues the next check; it does not discard the one
   in flight. So the step-1 answer is applied at `/`. That restores the session
   and rewrites the marker, and recovery replaces `/` with `/app`.
4. `/app`'s own check finds no cookie and redirects to `/login`.

Trace evidence:

- Only the step-1 response carried the User; it was sent with the cookie.
- Every later `get_session` was sent without a cookie and returned `null`.
- `/` rendered its chrome and called `list_local_timeline` at 17.524–17.540, but
  made no `get_session` call until 17.594. That fits a loop still blocked on
  step 1.
- One caveat: the step-1 network response finished about 7 ms _before_ logout's.
  The order in which the client then handled the two is inferred, not observed.
  The fix below holds for either order.

## Approach and tradeoffs

This is an app fix. In `clear_session` (`web/src/auth/session.rs`), also write
the anonymous result `Some(Ok(None))` into the session-check Resource.

A write to a Leptos async Resource increments its version. The reload loop then
discards any check that started earlier when it completes (reactive_graph
`ArcAsyncDerived::try_write`). This works in both possible orders:

- **Step-1 check still in flight:** it is dropped when it completes.
- **Step-1 check already applied:** the write overwrites its value.

The next navigation's check runs normally. Logout's failure path calls
`refetch`, which starts a new check that still applies. The write only ever
denies access, so it cannot make a gate admit anyone. Every consumer that awaits
the Resource benefits too, including `PrivateRoute` and the pages that await the
session directly. The reconcile Effect is the only place this bug surfaced, but
they all read the same Resource.

Rejected alternatives:

- **Guard in the Effect (a counter, or "ignore if the path changed"):** the
  counter re-creates the version guard Leptos already has, and leaves the stale
  value in place for every other awaiter.
- **Writing `Some(user)` on sign-in:** this would put a server-unconfirmed user
  into the cookie-checked gate. Sign-in paths already `refetch` after setting
  the session, and nothing has been observed failing there. Out of scope.

## Acceptance

- A new e2e regression test in `end2end/tests/auth.spec.ts`:
  - It seeds a session and `site.title` itself, then enters a private page.
  - Once that page's own check has settled, it holds exactly one `get_session`
    call, the one sent for `/logout`. It uses `route.fetch()`, which replays the
    request's cookie to the server, then a delayed `route.fulfill`. This is
    added as an option on `stallServerFn`, not as a second helper.
  - It holds the logout request until that fetch has been answered, and asserts
    the answer names the User, so the test cannot pass vacuously.
  - It delivers the stale answer only after the page reaches `/` and the Local
    masthead shows. Then it awaits the next `get_session` response, registered
    before release. After that, the page is still on `/`, the masthead shows the
    seeded title, and there is no account control.
  - Proof: `dev e2e-local auth.spec.ts:<line>`, red without the fix and green
    with it.
- `persistent-session.spec.ts` and `auth.spec.ts` still pass under
  `dev e2e-local`.
- No sleeps, longer timeouts or added retries (jaunder-e2e wait discipline).

## Boundaries

- Sign-out only. There is no change to:
  - ADR-0193's recovery or the marker;
  - `PrivateRoute`;
  - the sign-in paths;
  - server code.
- The only change to the session-check Resource is that logout writes an
  anonymous value into it.
- `persistent-session.spec.ts` is unchanged. Its readiness barrier is correct;
  the flake came from the app bug.
