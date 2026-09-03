# Issue #1023 — centralize admin settings re-entry

## Outcome

The admin-site and backup Playwright specs use one shared, target-typed helper
for leaving and re-entering an admin settings page. The refactor preserves each
flow's browser context, route sequence, readiness barriers, and observable
behavior.

## Load-bearing decisions

- Own the helper in a focused `end2end/tests/admin-settings.ts` module rather
  than expanding the generic navigation helper with admin-specific knowledge.
- The helper accepts the caller's existing `Page` and a typed admin settings
  target. A central target registry keeps each target's route, operator-sidebar
  link, readiness selector, and re-entry intermediate target together.
- The current `site` and `backups` targets use each other as their intermediate
  destination. Adding another admin settings page requires adding one typed
  registry entry, not branching the helper or duplicating lifecycle code in a
  spec.
- Both hops remain in-app navigation through `navigateInApp`, preserving
  ADR-0111's one-document-load-per-`Page` lifecycle and meaningful readiness
  barriers.
- Migrate only the three audited admin-site call sites and two audited backup
  call sites. Each invokes the shared helper with its respective typed target;
  all other test interfaces, fixture pages, authentication setup, mutations, and
  assertions remain unchanged.
- Keep the admin-site boot-warning flow's explicit `allowSecondBoot` and `goto`
  sequence local: fresh document loads are semantically required to observe
  boot-time configuration and are not settings re-entry.
- Do not change per-test browser-context isolation, the serial admin-site
  project quarantine, timeout policy, routes, or production code.
- This test-only consolidation introduces no domain terminology or durable
  architectural choice; `CONTEXT.md`, ADRs, and `docs/ARCHITECTURE.md` remain
  unchanged.

## Acceptance

- The three admin-site call sites invoke the shared helper with the `site`
  target, the two backup call sites invoke it with the `backups` target, and
  both duplicated local helper definitions are removed.
- Re-entry still traverses the configured intermediate admin settings page
  before returning, with the same URL and destination-specific readiness checks
  on each hop.
- A future admin settings target can be added through one typed registry entry
  without changing the helper's control flow.
- The boot-warning test retains its explicit fresh-load lifecycle.
- The affected focused Playwright tests pass.
- `cargo xtask check` passes.

## Boundaries

- No redesign of generic navigation helpers, admin routes, or settings pages.
- No consolidation of unrelated direct entries, extra-context tests, or other
  admin test fixtures.
- No public, production, or unrelated test interface changes.
