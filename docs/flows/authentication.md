# Authentication

Matrix: `matrix:docs/coverage/csr-e2e-matrix.md#authentication`

## Routes

- `route:/login`
- `route:/register`
- `route:/logout`
- `route:/app`

## Endpoints

- `endpoint:/api/auth/login`
- `endpoint:/api/auth/logout`
- `endpoint:/api/registration/get_policy`
- `endpoint:/api/registration/register`

`/login` submits one typed username/password request. Success creates an
`HttpOnly` session cookie, returns only the operator bit needed for immediate
chrome, seeds the shared session marker, and navigates directly to Home at
`/app`.

`/register` first reads the site's registration policy. Open sites render the
form directly. Invite-only sites reuse the same route but suppress the submit
form when the URL carries no invite code. Closed sites reject in the server fn.
A successful registration follows the same cookie-only session-establishment
rule as login, seeds the shared client session with `is_operator: false`, and
navigates directly to Home.

`/logout` is a mount-only action page. It revokes the current session when one
exists, clears the cookie either way, clears the shared client marker on
success, and returns the browser to Local at `/` through router-managed
same-document navigation.

The authenticated cockpit at `/app` is documented separately. Authentication
makes Home directly reachable without relying on the document-level prepaint
script to run again.

## Login to authenticated shell

```mermaid
sequenceDiagram
    participant Browser
    participant Auth as auth/login
    participant Users as UserStorage
    participant Sessions as SessionStorage

    Browser->>Auth: submit typed username/password
    Auth->>Users: authenticate user
    Users-->>Auth: user record + operator flag
    Auth->>Sessions: create cookie-backed session
    Sessions-->>Auth: raw session token
    Auth-->>Browser: Set-Cookie + redirect("/app")
    Browser->>Browser: write shared session marker
    Browser->>Browser: authenticated sidebar renders in place
    Browser->>Browser: render Home at /app
```
