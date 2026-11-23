# Issue #2071 — order `issue-discover` candidates by Backlog priority

## Outcome

`dev issue-discover <milestone>` reports each issue's Jaunder Backlog Priority
and lists candidates in the order they should be worked: an agent that takes the
first candidate takes the highest-priority unblocked, unclaimed issue.

## Approach and tradeoffs

- **One read per issue, two fields.** Discovery already reads each issue's
  Backlog item with one GraphQL query for Status. That query will request Status
  and Priority together, so there is no second round trip. The single-field
  query builder becomes one fixed Backlog-item query selecting both fields, used
  by every caller; `issue-create`/`issue-edit` readback ignores `status`.
- **Validation mirrors Status.** On the one Backlog item: an absent field key is
  "missing priority evidence"; `null` means no priority; a name not in the
  existing `PRIORITY_OPTIONS` (`P1`–`P4`) is "invalid priority". Each refusal
  exits 5 like the existing Status checks, and applies to every gathered issue,
  candidate or skipped, as Status does. An issue with no Backlog item has
  `priority: null`.
- **Output.** Every candidate and skipped row gains
  `"priority": "P1" | "P2" | "P3" | "P4" | null`. Candidates sort by (priority
  rank in `PRIORITY_OPTIONS` order, P1 first, null last; then issue number).
  `skipped` stays sorted by number.
- **Docs.** CONTRIBUTING.md's "Discovering milestone issue candidates" states
  the `priority` field and candidate order.

Alternative rejected: leaving order alone and having the skill sort. The order
is project policy every consumer needs; putting it in the one producer keeps a
single definition.

Accepted cost: discovery gets stricter — malformed Priority on any issue in the
milestone fails the run, as malformed Status already does.

## Acceptance

- `tools/dev/tests` prove: `priority` on every candidate and skipped row
  (including `null` for no Backlog item and for an unset priority); candidates
  ordered by mixed priorities, missing priority last, ties by number; skipped
  still by number; invalid priority name and missing priority evidence exit 5,
  including on a skipped (claimed) issue.
- Existing discovery tests, with fixtures and the expected query updated for the
  combined read, still prove full pagination, ambiguous Backlog refusal,
  read-only and no claims; create/edit tests unchanged in behaviour.
- CONTRIBUTING.md describes the ordering and the field.
- Proof: pytest on `tools/dev/tests/test_issue_frontdoors.py` while iterating;
  the `dev-python` check (ruff, `mypy --strict`, 100% branch coverage) before
  ship.

## Boundaries

- No change to skip reasons, claim semantics, or resume evidence.
- The `jaunder-develop` skill's "Milestone selection" update (take the first
  candidate unless the user names one) lives outside this repo; the PR notes it
  as required follow-up.
