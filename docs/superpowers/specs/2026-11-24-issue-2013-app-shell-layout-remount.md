# App shell keeps one route outlet across public and private layouts

## Outcome

Navigating between a public path (`/`, `/feeds`, `/tags/…`, `/~…`) and a private
path mounts the destination route once and never mounts the route being left a
second time. Navigations that stay within one layout no longer rebuild the
shell. A successful sign-out sends one `logout` request and, after its response,
one `get_session` request.

## Approach and tradeoffs

Today `AppShell` (`web/src/app/component.rs`) chooses between two complete view
trees inside a closure that reads `location.pathname`. Each tree has its own
Sidebar and `<Outlet/>`, so every navigation rebuilds the shell, and a
navigation that changes layout builds a new Outlet that briefly renders the old
route again.

`AppShell` will render one tree whose Sidebar, `main` element and Outlet survive
navigation (the existing presentation-failure `Show` around the shell still
rebuilds them when that failure toggles; unchanged here). A memoized `is_public`
signal (from `common::theme::is_public_presentation_path`) drives only what
differs:

| Element                   | Public                                                      | Private                                    |
| ------------------------- | ----------------------------------------------------------- | ------------------------------------------ |
| Wrapper around `.j-shell` | `class="j-theme-clip"` + `data-jaunder-theme-clip`          | `class="j-contents"` (`display: contents`) |
| `.j-shell`                | `data-jaunder-theme-surface`, `data-jaunder-style-contract` | neither                                    |
| `main.j-main`             | `data-jaunder-part="main"`                                  | none                                       |
| Warning banners           | inside `#j-trusted-chrome` after the shell                  | first in `.j-main-region`, before `main`   |
| `#j-trusted-chrome`       | present, after `#j-trusted-post-actions`                    | absent (as today)                          |

`#j-trusted-post-actions` stays in both layouts and becomes one stable node.

`display: contents` removes the private wrapper from layout, so private pages
keep today's box tree. The public markup keeps the order and hooks the SSR
projector (`web/src/app/render.rs`) emits, so it is unchanged. Each banner
placement sits behind a `Show` on `is_public`, so the banners (not routes)
remount when the layout changes, as they do today. Rejected: memoizing the
layout choice alone stops same-layout rebuilds but still creates a new Outlet,
and so a remount, on every layout change.

**Logout guard.** #2003 made `LogoutPage` dispatch only when the path is
`/logout` because the remount re-ran its effect at `/`. With one Outlet that
remount is gone, so the guard and `route_policy::is_logout_route` (only caller)
are removed. The existing e2e assertion "one sign-out sends one `logout`" then
proves the shell fix directly.

**Session reconcile on sign-out (ADR-0044).** Leptos fills the action value
first and performs the server redirect to `/` one animation frame later. Both
successful outcomes (`Confirmed`, `CommitIndeterminate`) carry that redirect,
and the shared session `Resource` refetches on the path change. So on success
`LogoutPage` only clears the session (`clear_session`) and drops its explicit
refetch. On `Err` there is no redirect; the server may still have revoked the
session (e.g. response lost), so `LogoutPage` calls
`use_session().reconcile.refetch()` to re-learn the real session. Accepted cost:
if a future server change returned success without a redirect, the client stays
cleared but unconfirmed until the next navigation. The `get_session` sent on
arriving at `/logout` may resolve after `clear_session` and briefly restore the
user; the redirect's refetch corrects it a frame later. `LogoutPage`'s comment
claiming the redirect lands "on the same resolution" as the action value is
corrected.

## Acceptance

Proof to execute (focused e2e via `dev e2e-local`):

- Sign-out from a private page sends exactly one `/api/auth/logout` and, after
  its response, exactly one `get_session` request, with the `/logout` guard
  removed.
- In-app navigation from public `/feeds` to a private page: a `MutationObserver`
  installed before the click records no insertion of an element unique to the
  Feeds page after the click. This check is shown to fail on the unfixed shell
  before the fix lands.
- The Sidebar element is the same DOM node before and after a same-layout
  navigation and after a cross-layout navigation.
- A sign-out whose `logout` request fails shows the error and sends a
  `get_session` request after the failed response.
- Existing theme-presentation, responsive-shell and post-actions e2e and the
  `render.rs` projector tests stay green.

Visual proof (`visual-proof`, before/after pairs, desktop and mobile widths):
public `/` and `/~<user>`, private `/app` and `/sessions`, and one private page
with a warning banner showing.

## Boundaries

- No change to the SSR projector, CSS, route table, or `PrivateRoute`.
- Banner placement per layout is unchanged; unifying it is out of scope.
- `LoginPage`'s refetch-plus-navigate is not changed here.
