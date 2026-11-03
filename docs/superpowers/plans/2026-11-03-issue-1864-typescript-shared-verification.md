# TypeScript shared-verification implementation outline

> Execute with `jaunder-iterate` only after approval. Outline needed because
> this change adds a durable Nix verification boundary and coordinates a check
> catalog, Nix leaf, and host consumer.

## Execution summary

Keep the existing `devtool check tsc` behavior, move it into a `typescript`
catalog group without changing named-check behavior or the ordered `--all`
inventory. Then build the smallest end-to-end `static-typescript` leaf/xtask
consumer slice and resolve the risky timing attribution immediately: either
positively correlate tsc timing with the current Nix builder request, or mark
execution duration unavailable. Only after this vertical slice is understood,
expand to compiler-population, failure, invalidation and reuse proofs. Preserve
the accepted `e2ePackage` whole-`end2end` dependency coupling. The approved spec
and ADR define the full contract.

## Scope

In: the existing tsc population, `typescript` group/catalog ordering,
`static-typescript` leaf and host verdict consumer, current-request timing
event/consumer, focused contract tests, and approved documentation/ADR
projection.

Out: other verification families, TypeScript formatting, `e2eCensusContract.ts`,
Playwright execution, dependency pruning, generic timing/cache infrastructure,
hook/workflow/required-check changes, commits or implementation before outline
approval.

## Task outline

- [ ] Task 1: Preserve and expose the named TypeScript check.
  - Contract: `devtool check tsc` remains named and runs the existing
    command/project. Add a `typescript` group containing only `tsc`, remove only
    `tsc` from `code`, and preserve the existing ordered, duplicate-free `--all`
    identities exactly (tsc remains after `byte-compile`, before `cargo-deny`).
    Measure only the child with a monotonic clock and emit one bounded stderr
    timing event with check identity, elapsed milliseconds, and outcome.
  - Verification: focused devtool test with controlled child commands proves
    event duration/outcome for success and nonzero exit; catalog tests prove
    direct selector, group membership, and full `--all` order/population.
- [ ] Task 2: Wire the minimum TS leaf/consumer and resolve current-builder
      timing attribution.
  - Contract: add the smallest `static-typescript` leaf invoking the existing
    `devtool check tsc` definition, plus its distinct xtask validation consumer.
    Retain the existing `e2ePackage`/Playwright dependency path and a narrow
    direct project filter; do not yet claim full population or invalidation
    coverage. The host xtask parses the per-tsc timing event only from the
    current devtool child output and projects it as an execution `PhaseRecord`;
    whole-step duration remains separately labeled. The Nix consumer accepts the
    event only from the live output of this `nix build` request when positively
    attributable to the builder. Saved logs/`nix log` never establish current
    execution. Ambiguous/missing evidence is `GateExecution: unavailable`, not
    inferred time. No force-build or new tracing framework.
  - Verification: first establish that a passing `static-typescript` request
    reaches the existing xtask consumer. A focused host test proves a current
    tsc event is projected, while missing/malformed event data remains
    unavailable without changing the underlying check verdict. Focused Nix tests
    provide a positively attributed current-builder timing event versus
    identical content supplied only from retained historical output; only the
    current event may count. For a real builder that runs, either demonstrate
    current-request event attribution and a distinct execution phase, or
    demonstrate the agreed unavailable fallback when the live stream cannot
    positively correlate execution. Confirm a subsequent reused-output request
    reports request/reuse evidence but no current tsc execution time. Do not
    expand the proof matrix until this boundary is resolved.
- [ ] Task 3: Prove the complete TS population and verdict/input contract.
  - Contract: preserve host `tsc --noEmit -p end2end/tsconfig.json`, pinned
    tools, provisioning, direct source inputs and existing `e2ePackage`
    transitive coupling. `static-code` retains its other checks. Host and Nix
    failure semantics match; TypeScript edits may still invalidate
    `static-code`, and unrelated non-project `end2end` edits may invalidate the
    TS leaf through `e2ePackage`.
  - Verification: retain compiler-discovered file list and show ignored
    `end2end/node_modules` is provisioned rather than scanned as checkout
    source. A deliberate included TS error fails host and Nix; restoring it
    passes both. Compare leaf derivation identity for a relevant project TS
    input and an unrelated `server/src/lib.rs` input outside the
    direct/transitive closure; document that another `end2end` change may
    invalidate it. Demonstrate valid local-store reuse without treating reuse as
    execution. Keep the current timing rule: reused output has no current
    execution duration absent positively correlated builder evidence.
- [ ] Task 4: Reconcile aggregate validation and maintained documentation.
  - Contract: keep `static-docs`, `static-typescript`, and `static-code` as
    separate validation verdicts; retain TypeScript formatting and
    `e2eCensusContract.ts` in `static-code`. Preserve full validation authority
    and the narrow ADR-0178 supersession. No other family, hook, workflow, or
    GitHub required-check behavior changes.
  - Verification: focused affected unit tests first; host `cargo xtask check`
    confirms the local catalog/steps;
    `traced-run -- cargo xtask validate --no-e2e` verifies the hermetic static
    leaves and existing non-E2E aggregate. Use existing `precommit`/`prepush`
    hooks at normal commit/push boundaries. Do not run full E2E validation
    unless a later boundary requires it.

## Parallel ownership

One integration writer owns overlapping `tools/devtool/src/check.rs`,
`xtask/src/sh.rs`, `xtask/src/result.rs`, `xtask/src/steps/nix.rs`, and
`nix/checks.nix` edits, plus final docs/ADR integration. No separate
implementation checkout is justified yet: the catalog, execution event, leaf,
and consumer tests depend on these same contracts/files. The read-only
population census, proof-fixture design, and timing-stream review in
`.xtask/issue-1864-next-slices.md` can be delegated independently before
implementation. A test/helper-only checkout may be proposed later only if an
actual test file and stable contract are demonstrably disjoint; do not create
speculative worktrees. ERT and any other family need separate approval and
serialize only on actual shared-file conflicts.

## Approval and delivery boundary

The spec and ADR direction are approved; this outline is not. Obtain outline
approval before implementation. After that approval, use `jaunder-commit` to
commit the approved spec, ADR draft, architecture projection and outline before
code work. Implement one task at a time with focused proof; Lead reviews the
exact intended tree. Normal hook-backed commits and required validation follow
existing policy. No hook/required-CI/merge approval is conferred by this
outline.
