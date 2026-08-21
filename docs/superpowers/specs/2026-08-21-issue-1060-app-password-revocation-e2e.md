# Cover App Password revocation in e2e

- Issue: [#1060](https://github.com/jaunder-org/jaunder/issues/1060)

## Problem

`/sessions` already lets a user mint and revoke an **App Password**, and the
server-side revocation path is covered. The CSR browser-flow matrix still has a
specific gap: no Playwright test revokes an App Password from the mounted
Sessions page and verifies the resulting user-visible state.

This matters because App Passwords are individually revocable credentials by
design. ADR-0014 defines an App Password as a labelled session token carried
over HTTP Basic for AtomPub, and `docs/ARCHITECTURE.md` records revocation as
deleting that labelled session in the Sessions UI. Existing e2e coverage proves
that an App Password can be minted and can authenticate AtomPub requests, while
`end2end/tests/sessions.spec.ts` proves browser-session revocation. The missing
piece is the App Password-specific revoke control and its credential-death
effect.

## Decision

Add a focused Playwright test for App Password revocation through `/sessions`.
The test should live with the existing App Password and AtomPub browser flows,
so `end2end/tests/atompub.spec.ts` is the natural home unless implementation
evidence shows a better local fit.

The test mints an App Password through the Sessions UI, verifies the labelled
row appears, revokes that row through the Sessions UI, verifies the row is no
longer visible, and then proves the revoked token no longer works for AtomPub
HTTP Basic auth.

The work is coverage-only. It must not add product behavior, change the Sessions
UI contract, or introduce a separate App Password credential model. If the test
exposes a product bug, fix the bug in the smallest owning surface and keep this
issue scoped to the revocation behavior.

## Boundaries

- No new server endpoint, storage schema, or App Password type marker.
- No change to browser-session revocation coverage in
  `end2end/tests/sessions.spec.ts`.
- No direct `/api/sessions/revoke` shortcut; the browser flow must use the
  mounted Sessions page.
- No extra document load on an already-booted Playwright `Page` unless it is
  justified under ADR-0111's one-boot-per-page rule.
- No credential or token value may be logged or added to docs/coverage snapshots
  except through the existing redacted trace evidence machinery.

## Acceptance criteria

- A Playwright test mints an App Password from `/sessions` using the mounted CSR
  UI.
- The test identifies the App Password by its unique label and verifies that its
  row appears in the Sessions list before revocation.
- The test revokes that App Password through the Sessions UI's revoke control,
  not by calling the server function endpoint directly.
- The test verifies the App Password row disappears after revocation while the
  current browser session remains usable.
- The test attempts an AtomPub HTTP Basic request with the revoked App Password
  from a request context that does not carry the browser session cookie, and
  verifies authentication fails with the existing unauthorized response.
- The test follows the e2e helper/navigation discipline, including the
  one-boot-per-page rule.
- Coverage documentation that tracks CSR App Password management is updated only
  if the existing matrix text needs to name the new evidence.
- The targeted e2e command for the relevant spec passes.
- `cargo xtask check` passes.
