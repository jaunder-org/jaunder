# Shared CI failure signatures implementation outline

> Execute with `jaunder-iterate`; delegate bounded tasks with
> `jaunder-dispatch`. This outline exists because the versioned report schema,
> deadline-enforced `gh` transport, and landed #1498 interface are cross-task
> contracts.

## Scope

In:

- Exact failed-job signature extraction and deterministic candidate selection.
- Bounded, read-only GitHub evidence acquisition for current open PR heads and
  current `main`.
- Optional version-1 `PrReport.shared_failure` enrichment after `checks-failed`.

Out:

- Changes to #1498's direct/transitive requirement classification.
- Fuzzy signatures, incident ownership, mutations, durable storage, and
  cross-repository searches.
- Human-output or exit-semantics changes.

## Task outline

- [x] Task 1: Establish typed failure identity and pure policy
  - Files: create `xtask/src/pr/shared_failure.rs`; modify
    `xtask/src/pr/mod.rs`, `xtask/src/pr/types.rs`, `xtask/src/pr/decide.rs`,
    `xtask/src/pr/watch.rs`, and every `PrReport` constructor/test in
    `xtask/src/pr/execute.rs`, `xtask/src/pr/land.rs`, and
    `xtask/src/result.rs`.
  - Contract: the selected failed Actions check flows from the already-observed
    snapshot through `Step`/watch termination as an internal
    `SubjectFailure { workflow_run_id, check_run_id, name }`. `PrReport` retains
    this typed identity with `#[serde(skip)]`; it is never reconstructed from
    the human job URL and never requires a mutable second PR snapshot. A
    non-Actions status-context failure has no subject-job identity.
  - Contract: this identity propagation does not change ADR-0195's precedence,
    direct/transitive classification, detail, pointer, optional-failure events,
    or fallible-evidence behavior.
  - Contract: `PrReport` also gains optional, omitted-when-absent
    `shared_failure: Option<SharedFailure>` using the specification's exact v1
    field names and discriminated `source` representation. All construction
    paths initialize both new fields so this task remains buildable.
  - Contract: pure functions normalize and extract one bounded
    `github-actions-error-block-v1` block, hash its exact UTF-8 bytes, select
    newest eligible completed runs, compare normalized text rather than hashes,
    and deterministically cap/order matches. No IO or clock reads occur in this
    policy layer.
  - Verification: focused tests prove direct and transitive Actions failures
    retain the exact workflow/check IDs, status contexts retain none, and the
    internal field never serializes. Signature tests use both cited `jiff` logs;
    cover ANSI/CRLF/prefix normalization, last-anchor selection, runner- marker
    exclusion, continuation and stopping grammar, absent anchors, independent
    eight-line and 4 KiB rejection, and preserved-value non-match. Selection
    tests prove the 24-hour/50-run ordering and pre-filter cap, eligible
    open-head/current-main restriction, subject and superseded-head exclusion,
    latest-completed precedence, green supersession, in-progress treatment, and
    ten-match ordering/cardinality.

- [x] Task 2: Add deadline-bound GitHub evidence acquisition
  - Files: modify `xtask/src/pr/gh.rs`, `xtask/src/pr/shared_failure.rs`, and
    `xtask/src/pr/snapshot.rs`; extend `xtask/src/pr/test_support.rs` only for
    reusable test doubles/fixtures.
  - Depends on: Task 1's typed subject failure, evidence inputs, and annotation
    output.
  - Contract: a dedicated shared-failure observation capability resolves the
    subject's check-run identity to its REST job ID, then supplies open PR
    number/URL/head SHA records, current `main` SHA, at most 50 newest
    `.github/workflows/ci.yml` runs within 24 hours, failed-job metadata, and
    raw failed-job logs. Repository identity continues to come from `Subject`;
    only `gh` crosses the network boundary.
  - Contract: raw-log transport is distinct from JSON parsing. Every subprocess
    receives the remaining portion of one monotonic ten-second deadline; expiry
    terminates and reaps the child. No query, parse, UTF-8, rate-limit,
    authentication, missing/expired-log, or timeout error escapes the enrichment
    boundary.
  - Verification: request/parser tests prove repository and primary-workflow
    scoping, open-PR and current-`main` head acquisition, the 24-hour/50-run
    request bound, exact check-run-to-job correlation, and exclusion of
    successful jobs, non-primary workflows, closed/superseded/non-eligible
    heads, and outside-repository evidence. Additional cases cover non-JSON log
    bytes, malformed responses, and query failure. A supervised blocking-child
    test proves deadline expiry returns promptly and leaves no child running.

- [ ] Task 3: Enrich only established watch failures
  - Files: modify `xtask/src/pr/execute.rs` and its tests.
  - Depends on: Tasks 1 and 2.
  - Contract: after `watch` returns `Outcome::ChecksFailed`, and before
    `into_result` serializes it, run best-effort enrichment only when the report
    carries Task 1's typed subject-failure identity. `land`, status-context
    failures without a failed Actions job, pending/successful watch outcomes,
    and all other adverse outcomes never perform the lookup.
  - Contract: enrichment may only change `shared_failure`; outcome, detail,
    pointer, events, command step, `ok`, exit code, and human rendering remain
    byte-for-byte equivalent to the unannotated path. Any enrichment failure or
    no-match returns the original report with the field omitted.
  - Verification: fake-backed execution tests prove the lookup gate, exact
    annotated JSON shape and links, unchanged authoritative fields, unique/no-
    anchor/query/log/parse/timeout degradation, and unchanged land/queue/
    ADR-0195 behavior. Run focused `xtask` PR tests, then
    `devtool run -- cargo xtask check` as the integration gate.

## Risk checks

- Build on, but do not duplicate or weaken, ADR-0195's landed current-head
  Actions identity and requirement classifier.
- Keep `decide.rs` pure and `gh.rs` the sole subprocess/network boundary under
  ADR-0087.
- A ten-second budget must bound process lifetime, not merely stop issuing new
  requests after an already-blocked `gh` call.
- Compare full normalized text; SHA-256 is output identity, never the equality
  authority.
- Apply the 50-run cap before eligible-head filtering exactly as specified, and
  select the latest completed run—not merely the latest listed run—for each
  eligible SHA.
- Update every `PrReport` construction site in `execute.rs`, `watch.rs`,
  `land.rs`, and `result.rs` tests; absence must serialize by omission, never
  `null`.
- Preserve clean degradation: enrichment failures never consume watcher strike
  budget, append watcher events, or become `watcher-error`.
- No ADR or `CONTEXT.md` update is expected: this refines the existing
  observation boundary and introduces no product-domain term.
