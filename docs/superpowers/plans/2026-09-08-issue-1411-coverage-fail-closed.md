# Fail-Closed Rust Coverage Evidence Implementation Outline

> Execute with `jaunder-iterate`, delegating bounded slices with
> `jaunder-dispatch`. This outline exists because the producer status protocol,
> Nix source closure, and host consumer form a multi-agent contract.

## Scope

In:

- Make the existing Nix Rust coverage producer prove an unfiltered,
  root-workspace test population and fail closed at every required stage.
- Make the coverage source closure include required Cargo path dependencies
  while retaining bounded invalidation and the root-workspace instrumentation
  boundary.
- Validate producer evidence independently in the host gate and document the
  resulting authority.

Out:

- Application behavior, test semantics, unrelated verify gates, and the separate
  `xtask`/`tools`/doctest test authorities.
- A permanent duplicate root-workspace test lane, retries, output-text success
  classification, or instrumentation of build-time tooling.
- Root-workspace membership changes for `tools/csr_bundle`.

## Task outline

- [x] Task 1: Make coverage producer evidence fail closed
  - Files: `tools/coverage/src/status.rs`, `tools/devtool/src/coverage/emit.rs`,
    `.config/nextest.toml`, the tools manifests/lockfile if structured parsers
    add dependencies, and focused tests beside the changed Rust modules.
  - Contract: a versioned `CoverageStatus` owns the required-stage catalog,
    checked process outcomes, population evidence, and category invariants.
    Workspace resolution is the checked
    `cargo metadata --manifest-path Cargo.toml --format-version 1 --no-deps`
    stage. The remaining required stages are profile cleanup, unfiltered
    `cargo nextest list --workspace --message-format json`, instrumented
    `cargo llvm-cov --no-report nextest --workspace --profile coverage`,
    population reconciliation, text report, LCOV report, and CRAP report. Disk
    usage and copied prose remain best-effort.
  - Contract: the dedicated `coverage` nextest profile writes JUnit to a fixed,
    producer-private path with `report-skipped = "ignored"`. Stable nextest list
    JSON supplies expected identities; JUnit supplies executed, failed, and
    statically ignored identities. `tests-ok` requires every stage successful, a
    nonempty population, and exact identity reconciliation. Unknown
    nonzero/spawn failures are infrastructure failures; structured test failures
    remain `test-failure`. Sanitized stage/outcome/detail crosses `status.json`;
    raw output stays under `diagnostics/`.
  - Verification: focused tools-workspace tests prove exact command/profile
    selection, list-JSON and JUnit parsing,
    missing/duplicate/unexpected/malformed identity rejection, ignored-test
    accounting, category contradictions, and nonzero outcomes at every required
    stage—including the PR #1401 metadata-101/no-`FAIL [` shape.

- [ ] Task 2: Make the coverage source closure Cargo-complete and bounded
  - Files: `nix/packages.nix`, `nix/checks.nix`, `xtask/src/coverage/probe.rs`,
    and its focused tests.
  - Contract: export the existing `workspaceMembers` and `cargoTargetSource`
    package internals to `checks.nix`. One Nix member binding derives the
    coverage Cargo source closure from `workspaceMembers` plus the existing
    external path dependency `tools/csr_bundle`; replace the blanket `tools/`
    exclusion with `cargoTargetSource` admission so that dependency's manifest,
    build script, and source enter without admitting the rest of `tools/`.
    Root-workspace selection—not source presence—continues to define the
    instrumented/test population.
  - Contract: expose a probe-only derivation identity whose only changing input
    is the filtered coverage source; do not infer filter behavior from
    `coverage.drvPath`, which is also coupled to `devtoolBin`. Extend the typed
    source-probe matrix so manifest, build-script, and source changes under the
    required `csr_bundle` package change that filtered-source identity; root
    instrumented source also changes it; staged `xtask` source does not. Retain
    one end-to-end `coverage.drvPath` assertion for the required dependency.
  - Verification: focused xtask probe-verdict tests cover every matrix arm and
    failure precedence. The agent-runnable probe observes the filtered-source
    and end-to-end derivation relationships, realizes the producer, consumes its
    actual text or LCOV report, and fails if `tools/csr_bundle` contributes any
    executable line. Synthetic report fixtures prove parser behavior only.

- [ ] Task 3: Make every consumer reject incomplete coverage evidence
  - Depends on: Task 1's final `CoverageStatus` wire schema and invariant
    validator, and Task 2's completed `nix/checks.nix` source/probe changes.
  - Files: `tools/devtool/src/main.rs`, its coverage command module,
    `nix/checks.nix`, `xtask/src/steps/nix.rs`, `xtask/src/coverage/run.rs`,
    focused tests in those modules, and the testing and coverage-source sections
    of `CONTRIBUTING.md` and `docs/ARCHITECTURE.md`.
  - Contract: add sandbox-available
    `devtool coverage validate-status --status <path>`, backed by the shared
    pure `CoverageStatus::validate`. The Nix gate invokes it and accepts only a
    valid completed producer result; it does not duplicate status invariants in
    jq. The host path independently reads and validates `status.json` even when
    the Nix gate succeeded; it does not reuse the intentionally lossy
    failed-gate renderer. Before stateless line/CRAP evaluation, the host
    separately rejects a missing or empty text report and a parsed report with
    zero executable lines. Error details identify the failed stage or evidence
    invariant without exposing raw command output.
  - Verification: focused xtask/devtool fixtures reject
    missing/malformed/version-unknown statuses, duplicate/missing/failed stages,
    category-field contradictions, incomplete/empty populations, empty text, and
    zero executable lines; a complete producer fixture reaches the existing line
    and CRAP verdict. Documentation explicitly states the unfiltered root
    workspace, expected/executed/ignored reconciliation, checked subprocess
    exits, empty/zero-line rejection, and SQLite/PostgreSQL backend-parity
    contract. Review both resulting documentation sections against those five
    assertions. `cargo xtask validate --no-e2e` is the final integration proof
    that the repaired producer and both consumers remain the single required CI
    path.

## Ordering and ownership

- Tasks 1 and 2 may execute concurrently; they share no implementation file.
  Task 3 starts only after both are complete, serializing its `nix/checks.nix`
  consumer edit after Task 2's source-closure edit.
- `tools/coverage` owns pure status parsing and invariant validation;
  `tools/devtool` owns sandbox execution/artifact production; `xtask` owns host
  orchestration and post-processing; Nix owns source closure and derivation
  wiring. Do not duplicate pure status rules in prose parsers.
- Each task reaches `jaunder-commit` after its iteration evidence. Commits carry
  no `Co-Authored-By` trailer.

## Risk checks

- The census command and instrumented run both contain explicit `--workspace`
  and no package, test, partition, expression, or runtime narrowing filter.
- Structured execution evidence is stable under the pinned nextest version;
  ignored tests are represented explicitly rather than inferred from absence.
- A command cannot be classified successful from stdout/stderr text; every
  required process outcome is checked before `tests-ok` is constructed.
- `tools/csr_bundle` remains outside the root workspace and coverage
  denominator; only its required build input bytes affect the coverage
  derivation.
- The exact missing-manifest failure from PR #1401, partial artifacts, malformed
  status, and zero-line reports all produce red `Validate (no e2e)` results.
- Update comments that currently claim the producer always succeeds or reports
  are best-effort so the documented control flow matches the repaired protocol.
