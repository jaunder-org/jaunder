# Repository Census Implementation Outline

> Execute with `jaunder-iterate`, delegating individual tasks with
> `jaunder-dispatch`. This outline exists because independently implemented
> collectors must share one stable report and failure contract.

## Scope

In:

- The approved issue #1222 census command, report model, minimum collector
  matrix, focused tests, and user documentation.
- Host-only implementation under `xtask`, using the repository's existing result
  envelope and declared analyzers.

Out:

- Census findings, ranking, remediation, generated baselines, production
  refactors, and verification-gate integration.
- Claims of behavioral parity between storage adapters.

## Task outline

- [x] Task 1: Establish the census command and report contract
  - Contract: `cargo xtask census [--json]`; one census report owns
    deterministically ordered signal sections and language/signal cells with
    `clean`, `candidates`, `unavailable`, or `failed` state, collector metadata,
    evidence method, limitations, and repository-relative candidates.
  - Contract: source inputs are Git-tracked working-tree files in the approved
    roots; failed cells fail the command without discarding completed cells.
  - Contract: Task 1 owns CLI dispatch, rendering, snapshot construction,
    collector orchestration, and the shared `CollectorContext` input,
    `CellReport` output, and `CollectorSpec` registration interface. A collector
    is invoked as `fn(&CollectorContext) -> CellReport`; collector modules own
    no output or exit policy.
  - Verification: focused xtask tests prove CLI parsing, snapshot
    inclusion/exclusion, deterministic human/JSON rendering, state aggregation,
    unavailable tooling, failed-cell exit behavior, and partial-result
    preservation.

- [x] Task 2: Implement dependency, semantic, and structural collectors
  - Contract: collectors consume the Task 1 `CollectorContext`/`CellReport`
    interface and own no output policy.
  - Contract: Rust and TypeScript reference evidence is semantic only;
    structural collectors cover the approved clone, repeated-test, conversion,
    and error-mapping cells; Elisp support covers the approved dependency and
    structural cells. Missing analyzers report `unavailable`, never heuristic
    reference results.
  - Verification: focused xtask collector tests exercise positive and negative
    fixtures for the dependency, semantic-reference, unreferenced-symbol, clone,
    repeated-test, conversion, and error-mapping cells and assert evidence
    method, collector identity, and material limitations.

- [x] Task 3: Implement history and adapter correspondence, then document the
      complete command
  - Contract: history considers full non-merge history reachable from `HEAD`,
    applies Git rename detection, and reports deterministic churn/co-change
    candidates for every tracked source tree.
  - Contract: adapter analysis reports paired and unmatched SQLite/PostgreSQL
    paths as heuristic correspondence candidates without equivalence claims.
  - Contract: Task 3 is the serialized integration owner: after Task 2 lands, it
    extends the collector registry only for history and adapter collectors, then
    owns the final complete-command smoke proof.
  - Contract: `docs/codebase-audits.md` documents invocation, report semantics,
    collector coverage and limitations, ephemerality, and non-gating status.
  - Verification: positive and negative fixture repositories prove history,
    rename, merge exclusion, and adapter correspondence behavior; a command
    smoke run produces both compact human output and equivalent JSON sections;
    focused xtask tests and the applicable static-check lane pass.

## Risk checks

- No collector may turn missing tools, malformed output, excluded paths, or
  unsupported cells into a clean result.
- Semantic, structural, and heuristic evidence remain distinguishable through
  rendering and serialization.
- Report ordering and repository-relative identities remain stable across
  unchanged runs.
- Tasks execute in order. Task 1 exclusively owns CLI, snapshot, report, and
  orchestration modules; Task 2 owns its collector modules and fixtures; Task 3
  owns history/adapter modules and fixtures plus the serialized
  collector-registry integration. No sibling agents edit the same files
  concurrently.
- External commands use the existing scrubbed process and result conventions;
  tool versions and failures remain visible.
- Existing uncommitted user documentation and ADR work remains untouched except
  for the issue-authorized `docs/codebase-audits.md` update in Task 3.
- Each task reaches `jaunder-commit`; no commit receives a `Co-Authored-By`
  trailer.
