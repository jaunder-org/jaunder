# Application shell and boot state

Matrix:
`matrix:docs/coverage/csr-e2e-matrix.md#application-shell-and-boot-state`

## Routes

- `route:<shell>`
- `route:/`
- `route:/app`

## Endpoint

- `endpoint:/api/auth/get_session`

The shell is the single `ParentRoute` frame: sidebar, backup banner,
site-base-url banner, and the main outlet all mount inside the same `j-root` /
`j-shell` layout. The server projector and the CSR app share that frame so the
browser can adopt the painted shell instead of swapping to a different layout at
boot.

Boot starts with the inline prepaint script. It reads the advisory
`jaunder_auth` marker from `localStorage`. At `/`, a structurally valid marker
synchronously replaces the URL with `/app` before Local can paint; it carries
only recognized `order=oldest` state and otherwise uses canonical `/app`. The
marker remains advisory: `/app` confirms the real session before loading Home.

`AppShell` provides the shared session context inside the router. That context
seeds from the advisory marker, re-runs the session reconcile on every pathname
change, writes the confirmed result back to the marker, and keeps the sidebar
chrome in sync with the cookie-backed session. When it confirms a live session
missing a marker at `/`, it restores the marker and immediately replaces Local
with `/app`; a stale marker instead reaches `/app`'s unauthenticated `/login`
redirect. Login and registration seed the same client state and navigate
directly to `/app`; logout clears it and returns to Local.

The shell also owns the theme signal and applies it directly on the `data-theme`
attribute of `j-root`, so the mounted tree, the projector output, and the
post-login chrome all stay on one attribute-driven theme path.
