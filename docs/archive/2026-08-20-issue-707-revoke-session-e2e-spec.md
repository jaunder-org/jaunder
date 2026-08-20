# Issue #707: revoke_session browser-flow coverage

## Problem

`web::sessions::revoke` is server-tested but not browser-flow covered. The
server-fn flow-coverage gate currently allowlists `sessions::revoke` because a
real `sqlite × chromium` e2e trace shows zero hits by either signal for that
endpoint.

This is a browser-flow gap, not a storage or endpoint-correctness gap:

- `server/tests/web/web_sessions.rs` covers revoking an authenticated caller's
  own session, rejecting another user's session, and requiring authentication.
- `server/tests/web/web_account.rs` covers that re-auth with a revoked token
  fails, plus unknown-hash and other-user-hash errors.
- `web/src/sessions/component.rs` already renders `/sessions` with per-row
  `Revoke` buttons wired to `ServerAction<Revoke>`.

Per ADR-0081, the coverage claim must come from trace evidence, not prose.
Closing #707 therefore means adding a real browser flow that hits
`/api/sessions/revoke`, regenerating the server-fn coverage snapshot from the
authoritative `sqlite × chromium` capture, and deleting the allowlist entry.

## Constraints

- Use `tracedContext` for the second browser context. Raw `browser.newContext()`
  is forbidden and would under-report coverage because its traffic is not
  attributed to the test span (ADR-0081, ADR-0096).
- Use seeded auth helpers (`signInAsNewUser`, `signInAs`) rather than UI
  login/register setup. ADR-0098 keeps real auth flows only at named holdout
  sites; this test's subject is session revocation, not login.
- Drive the UI, not the server fn directly. The target evidence is browser-flow
  coverage.
- Do not hand-edit `docs/coverage/server-fns.json` or
  `docs/coverage/server-fns-evidence.json`; regenerate them after an e2e capture
  (`CONTRIBUTING.md`).
- Remove `sessions::revoke` from `docs/coverage/server-fns-allowlist.json` only
  once the regenerated snapshot covers it.

## Intended browser flow

Add an e2e spec under `end2end/tests/` that:

1. Seeds and applies a fresh authenticated session to the default `page` with
   `signInAsNewUser(page)`.
2. Creates a second traced browser context with `tracedContext()`, opens a page,
   and calls `signInAs(otherPage, username)` to create a second session for the
   same user.
3. Boots the second page to an authenticated route and waits for authenticated
   UI, proving the second context is live before revocation.
4. Boots the default page at `/sessions`, identifies the non-current session
   row, and clicks that row's `Revoke` button.
5. Asserts the non-current row disappears while the current row remains.
6. Drives the second page through an in-app authenticated navigation (not a
   second `goto`) and asserts it is redirected/logged out, proving the revoked
   session token is dead.
7. Closes the second context in a `finally` block.

The two seeded sessions may share the default label (`E2E seed`). If the
implementation keeps that default, select the target row by excluding
`(current)`. A small helper extension to pass a distinct session label is
allowed only if it makes the test materially clearer and stays on the existing
Rust-owned seed path.

## Coverage update

After the browser flow passes:

1. Run the authoritative capture-producing combo:

   ```bash
   devtool run -- cargo xtask e2e sqlite chromium
   ```

2. Regenerate server-fn coverage from that capture:

   ```bash
   devtool run -- cargo xtask server-fn-coverage regenerate
   ```

3. Expected coverage state:
   - `docs/coverage/server-fns.json` includes `sessions::revoke` in `covered`.
   - `docs/coverage/server-fns-evidence.json` includes `sessions::revoke` with
     the new e2e title.
   - `docs/coverage/server-fns-allowlist.json` no longer contains
     `sessions::revoke` and may become `[]`.

## Acceptance criteria

- A Playwright browser test drives `web::sessions::revoke` through `/sessions`.
- The test uses `tracedContext` for the second browser context.
- The test proves both visible UI removal and behavioral logout/death of the
  revoked second session.
- `docs/coverage/server-fns-allowlist.json` no longer allowlists
  `sessions::revoke`.
- `docs/coverage/server-fns.json` and `docs/coverage/server-fns-evidence.json`
  are regenerated from a fresh `sqlite × chromium` e2e capture, not hand-edited.
- `cargo xtask check` is green after the coverage update.
