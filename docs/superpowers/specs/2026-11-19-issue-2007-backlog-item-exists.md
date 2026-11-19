# Issue #2007 — `issue-create` survives the Backlog auto-add race

## Outcome

`dev issue-create` finishes normally — Priority set, blockers linked, readback
verified — when the new issue lands in Jaunder Backlog (#1) through project
automation at the same moment `gh project item-add` tries to add it.

## What happens today

After `POST /issues`, `_finish` runs `gh project item-add 1 --url <issue>`. On
2026-11-18 this failed for #2003 with
`GraphQL: Content already exists in this project (addProjectV2ItemById)` and the
front door stopped before setting Priority or blockers.

`item-add` is idempotent for an issue that is already a project item: run on
2026-11-19 against #2007 (already in the Backlog), it returned the existing item
id `PVTI_lADOECw7os4BblPPzhKKIYM` without error. So the error is a race, not
"already there". Another insert (presumably the project's auto-add rule) was in
flight at the same moment. `issue-edit` always targets an existing issue, so it
gets the idempotent answer and is **not affected**. It needs no change, though
it shares the fix because both front doors go through `_finish`.

## Approach

When `item-add` fails, `_finish` reads the issue's Backlog items by issue
number, using the same per-issue GraphQL query as the final readback
(`_backlog_items`):

| After `item-add` fails (not by signal), the Backlog holds | Result                                 |
| --------------------------------------------------------- | -------------------------------------- |
| exactly one item for the issue                            | use its id; continue Priority/blockers |
| no item                                                   | fail with `item-add`'s own exit status |
| two or more items                                         | fail 5 (`_backlog`'s exactly-one rule) |
| an incomplete or malformed listing                        | fail 5, as the readback does today     |
| (the listing's own `gh` call fails)                       | fail with that call's status           |

An `item-add` ended by a forwarded termination signal never falls back: the
failure propagates unchanged, so a user's interrupt cannot be followed by
further writes.

After that, the Priority edit, blocker links and the full readback (which
already requires exactly one Backlog item, the same id, with the requested
Priority) run unchanged.

Tradeoff: the fallback runs on _any_ `item-add` failure. It does not match the
"Content already exists" text. `dev`'s step runner passes a child's stderr
through to the user rather than capturing it, so matching the message would need
new stderr-capture machinery in `steps`. That is not worth it here: whatever the
error said, the fallback continues only when the tracker shows exactly one item
for this issue. That one item is the state a successful `item-add` would have
produced, and every later write is still verified. Every failure that leaves no
item still fails closed with gh's status, as now. Alternative considered:
retrying `item-add` once. It is equally small, but it adds a second mutation and
depends on the racing insert having committed. Reading the item directly is the
evidence the readback trusts anyway.

## Acceptance

- New tests in `tools/dev/tests/test_issue_frontdoors.py`, against the fake gh:
  - `item-add` exits nonzero printing the real
    `GraphQL: Content already exists in this project (addProjectV2ItemById)`
    text, and the next call (the per-issue Backlog listing) has one item.
    `issue-create` succeeds: Priority is edited on that item id and the report
    carries the readback.
  - Same failure, listing empty: fails with `item-add`'s status, no stdout, no
    `item-edit` call. Listing with two items: fails 5.
  - `item-add` killed by SIGTERM: fails signalled, with no listing or
    `item-edit` call.
- Existing front-door tests pass unchanged, including the `item-add` reply
  without an id (exit 4).
- Proof: the `tools/dev` pytest lane for that file, then the normal commit
  hooks.

## Boundaries

- No change to `issue-edit`'s path beyond the shared `_finish`, no rollback, and
  no change to the `steps` runner.
- The project auto-add rule is not investigated or changed.
