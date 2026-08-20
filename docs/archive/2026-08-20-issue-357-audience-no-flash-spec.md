# #357 — prove Audience refetches preserve mounted state

Issue: [#357](https://github.com/jaunder-org/jaunder/issues/357). Milestone:
Test infrastructure & E2E.

## Problem

The Audience E2E suite currently asserts `Loading members…` is absent and
Audience rows remain visible only after an add/remove or list mutation's refetch
has completed. Those end-state assertions cannot detect the regression they
name: a `Resource` can clear to loading, or a list can unmount, during the
refetch and recover before the assertion runs.

The affected contracts are intentionally distinct:

- `MemberChecklist` uses `client::reactive::sticky` for its
  `list_members(audience_id)` resource. Its prior member state must remain
  rendered while a successful add/remove triggers the next read.
- `AudienceList` uses `client::reactive::patched` for `list_mine`. Existing
  Audience cards must remain mounted while create, rename, or delete triggers
  the next read.

The existing named-Audience publishing scenario exercises a real cross-vertical
path (Audience membership followed by post composition), but it currently makes
no in-flight assertion around the membership mutation.

## Decisions

- **D1 — Stall the read refetch, not its mutation.** `client::reactive::action`
  notifies its invalidator only after a successful mutation. Holding
  `add_subscriber` or `remove_subscriber` observes pending mutation UI, not the
  resource transition #357 must prove. Tests therefore hold the subsequent
  `list_members` or `list_mine` request.
- **D2 — Use local, selective Playwright routes.** The current shared
  `stallServerFn` intentionally holds every request to an endpoint. These tests
  register a local `page.route` after initial data has settled, filter
  `list_members` by the target `audience_id`, observe that the intended refetch
  reached the route, and release only that route after the in-flight assertions.
  A shared abstraction has no second established caller.
- **D3 — Cover both resource contracts.** `audiences.spec.ts` covers membership
  refetches and list refetches separately. The existing named-Audience publish
  scenario in `visibility.spec.ts` adds the membership-refetch guard at its
  Add-member step; it remains a real product chain, not a duplicate
  list-resource test.
- **D4 — No timing sleeps or document reloads.** Tests use request-arrival and
  element assertions as synchronization. They keep the established
  one-boot-per-Page discipline and use the traced E2E helpers for navigation,
  clicks, and element waits.

No ADR or `CONTEXT.md` change is warranted: this is a test adequacy correction
that applies existing reactive and E2E conventions.

## Acceptance criteria

- **AC1 — Membership add observes the in-flight state.** In `audiences.spec.ts`,
  after initial lists settle, an Add-member action stalls only the target
  Audience's next `/api/audiences/list_members` request. While held, the
  target's pre-refetch member row/checklist and the unrelated Audience's
  checklist remain connected, the previous Add state remains rendered, and no
  `Loading members…` state appears. Releasing the request produces the expected
  Remove state.
- **AC2 — Membership remove observes the in-flight state.** The corresponding
  Remove-member transition stalls only its target refetch and proves the
  pre-refetch Remove state and mounted checklists persist until release; release
  produces Add.
- **AC3 — Audience-list mutations observe the in-flight state.** Successful
  create, rename, and delete mutations each stall the triggered
  `/api/audiences/list_mine` refetch after the pre-existing list has settled.
  While held, unaffected existing Audience cards remain connected and visible;
  neither the empty nor loading branch replaces them. Release produces each
  mutation's existing expected final state.
- **AC4 — The named-Audience publishing flow observes membership stability.** In
  `visibility.spec.ts`'s existing named-Audience scenario, holding the post-add
  `list_members` refetch proves the existing roster stays mounted and does not
  render `Loading members…`; after release, its existing publish flow remains
  unchanged.
- **AC5 — Routes are selective and deterministic.** The test records arrival of
  the intended refetch before making in-flight assertions, never stalls initial
  or unrelated requests, releases every held request, and adds no sleep,
  `networkidle`, retry, or navigation exemption.
- **AC6 — Test-only scope stays intact.** Production reactive code and endpoint
  behavior remain unchanged. The focused Audience and visibility E2E specs pass
  under `cargo xtask e2e-local`; the full verification path remains
  `cargo xtask validate`.
