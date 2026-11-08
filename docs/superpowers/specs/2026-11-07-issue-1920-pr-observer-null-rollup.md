# Fix spec: explicitly null PR check rollup

**Status:** approved spec; human approval recorded below. **Issue:**
[#1920](https://github.com/jaunder-org/jaunder/issues/1920), Bug, P1, milestone
#33 “Streamline testing without reducing protection.” **Checkout:**
`/home/mdorman/src/jaunder/fix-pr-observer-null-rollup`, assigned branch
`fix/pr-observer-null-rollup` (preserved per assignment exception).
**Approval:** Manager request `bc2d7ae3-305d-47ae-aa5c-28c3ac98a45c`; human
answer: “Approve fix and tests.” Approved spec identity:
`observer-null-rollup-spec-v1`.

## Outcome

A PR response whose `statusCheckRollup` is explicitly JSON `null` represents an
empty check page, not a malformed pagination response. For an otherwise eligible
open, unarmed, unqueued PR, absent required contexts yield `AwaitingChecks`,
never readiness; existing outcome and phase precedence remains unchanged.
Missing or malformed pagination on a non-null rollup remains an error.

## Approach and tradeoffs

Make the smallest parser-only change in canonical
`tools/pr-observer/src/pr/snapshot.rs`, in `rollup_page_info`: recognize only a
present `statusCheckRollup` field whose value is JSON null and return
`(false, None)` (no next page, no cursor). An omitted `statusCheckRollup` must
still error. For a present non-null rollup, keep requiring valid nested
`contexts.pageInfo.hasNextPage`; do not relax missing, null, or non-boolean
`hasNextPage`. The existing next-page-without-usable-cursor error also remains.
The adjacent `parse_subject_snapshot` already maps absent/null rollup checks to
an empty list. This fixes the inconsistency at its boundary without changing the
GraphQL query or public CLI/schema.

No separate outline is proposed: this is a localized parser correction with
existing pure unit-test seams, not a new architecture, public interface,
protocol, concurrency, or storage decision.

## Acceptance

- Add a parser regression using a PR payload with `"statusCheckRollup": null`;
  snapshot/page parsing succeeds with an empty check list and a terminal empty
  page.
- Assert that omitting `statusCheckRollup` still errors. Also retain rejection
  of a present non-null rollup with missing/malformed `contexts.pageInfo`,
  including missing, null, or non-boolean `hasNextPage`.
- Preserve rejection when `hasNextPage` is true but `endCursor` is missing or
  unusable.
- At the decision seam, demonstrate that when a nonempty required-context set is
  absent from the snapshot, an otherwise eligible open, unarmed, unqueued PR
  classifies as `Phase::AwaitingChecks` (not green/ready). Preserve existing
  outcome and phase precedence; other eligible higher-priority conditions are
  not reclassified.
- After approval, run the focused `tools/pr-observer` tests covering these
  cases; broader gates are outside this intake and should follow the approved
  implementation cycle's applicable verification ladder.

## Boundaries

Canonical parser/test code under `tools/pr-observer` only. No historical
`xtask/` repair; no public flag/schema, permission, action, readiness, or
required-check policy change. No unrelated Actions/event investigation or
PR-conflict investigation, no live PR/CI operation, and no merge or push/PR in
this assignment.

## Evidence and limitations

- Supplied diagnosis `evidence:issue-1916-watcher-diagnosis#2` records an actual
  GraphQL response with explicit null rollup and the resulting
  `status-check response omitted pageInfo` failure. Its PR/CI observations are
  historical, not current-state claims.
- Supplied canonical map `result:canonical-observer-map#1` independently found
  the same parser defect at canonical `snapshot.rs` and confirms `decide.rs`
  remains fail-closed for absent requirements.
- The cold review `result:observer-spec-review#1` findings were folded into the
  corrected spec; `result:observer-spec-review#2` reports soundness approved
  with no remaining findings. Review is not human approval; approval was
  separately granted as recorded above.
- Relevant source seams: `tools/pr-observer/src/pr/snapshot.rs:1025-1060`
  (pagination parser/caller), `:259` (rollup check extraction), `:1074-1076`
  (next-page cursor requirement);
  `tools/pr-observer/src/pr/decide.rs:140-150, 187-195, 318-325` (required-state
  and decision behavior). ADR-0087 and ADR-0135 establish read-only
  observer/capability split and fail-closed empty-gate behavior; ADR-0004 covers
  cursor pagination generally. No conflict was identified.
- `origin/main` was fetched at implementation start and resolved to the same
  commit as `HEAD`, `9cc10e048be02b26c345055cb1e4d2bd52fc20ed`; no rebase is
  needed. This is local Git freshness, not a statement about subsequent remote
  movement.
- Initial checkout inventory was clean. No tests have yet been run for this
  implementation.
