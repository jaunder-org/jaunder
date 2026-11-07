# Issue #1895 Qualification Workflows Implementation Outline

> Execute with dev-cycle-iterate. Outline needed because the approved design
> changes the durable Just/Git, Nix, and host-xtask ownership split for
> qualification workflows.

## Execution summary

First prove the ordinary discover/accept admission boundary can move to Just/Git
while binding its upstream-verified harness HEAD to the host's admitted clean
`SourcePin`. Then route the focused host operations through Just without their
workflow-specific generic report path, preserving unrelated xtask behavior.
Finally migrate the documented callers and prove the distinct ordinary-baseline,
styling, and smoke outcomes through the existing qualification harness. See the
approved spec for full scope and acceptance.

## Task outline

- [x] Task 1: Establish Just/Git admission and the normalized host input
      contract.
  - Contract: discover/accept pass full resolved product SHAs plus expected
    current harness HEAD; one canonical-origin upstream check covers that
    complete set. The host rejects a mismatch against its clean `SourcePin`
    before Nix admission or lifecycle mutation. Styling Just captures its clean
    expected `HEAD`; the host must compare it with `SourcePin` before Nix
    admission or lifecycle mutation, with no origin/network/upstream check.
    Smoke remains mutable local/no-evidence.
  - Verification: focused Just/Git and host admission tests prove distinct/equal
    SHA handling, upstream refusal, baseline source-pin mismatch refusal, no
    duplicated fetch, and styling expected-HEAD/`SourcePin` mismatch refusal
    before Nix or lifecycle work, without an origin/network dependency.
- [x] Task 2: Replace both public xtask routes and the workflow-specific report
      path with a focused internal host execution leaf.
  - Contract: Just owns the four public entrypoints and ordinary ordered
    invocation. Retire the discover/accept/styling `ProductionBaselineCommand` /
    `Command::ProductionBaseline` route and remove only the smoke
    `NixCommand::ProductionBaselineSmoke` subcommand from the `Command::Nix`
    family; preserve the `Command::Nix` family and unrelated Nix subcommands. A
    narrow internal host execution leaf serves the Just recipes. Remove only
    these workflows' `CommandResult`/`StepResult` constructions and bypass the
    generic `main.rs` JSON/report, last-result sidecar, and `xtask-done:` path.
    Preserve shared result types and generic reporting for unrelated xtask
    commands; retain host-domain lifecycle/evidence/failure handling.
  - Verification: update and run focused CLI/plain-process/Just workflow tests;
    prove the three old `ProductionBaselineCommand` routes and smoke subcommand
    are retired, the other `Command::Nix` subcommands remain, the internal leaf
    returns ordinary status, and unrelated commands retain generic reporting.
- [ ] Task 3: Migrate callers and certify workflow-specific outcome/evidence
      contracts.
  - Contract: update Just recipes, production-baseline/styling runbooks, and the
    projected architecture; browser tests remain consumers. Ordinary
    discover/accept may publish a valid sanitized failed product/protocol
    outcome and still exit nonzero; publication-inadmissibility failures publish
    nothing. Styling retains pass-only publication and its separate validation;
    smoke publishes neither evidence form.
  - Verification: run the existing focused tests and established qualification
    VM/harness proof for ordinary, styling, and smoke distinctions. Confirm
    source/harness provenance, required lease/cleanup, failure attribution, and
    report/no-report behavior. Qualification must use Nix-selected evidence; a
    native build is not a qualification result.

## Risk checks

- A locally committed but unpushed harness HEAD must be rejected for
  discover/accept before lifecycle mutation. For styling, Just's captured clean
  HEAD must equal the host `SourcePin` before Nix admission/lifecycle, with
  mismatch refusal and no origin/upstream request; styling must not acquire a
  network dependency.
- No stale-worktree/source drift may be attributed to the expected SHA; no
  duplicate upstream check may diverge from Just's preflight.
- Both public routes are retired: three operations from
  `ProductionBaselineCommand`/`Command::ProductionBaseline` and smoke from
  `NixCommand::ProductionBaselineSmoke`/`Command::Nix`; the focused internal
  host leaf is not a replacement public catalog.
- Generic report/sentinel retirement is workflow-scoped; shared result types,
  unrelated commands, and their tests remain intact.
- A failed product/protocol observation is reportable only under the ordinary
  workflow's existing validation/sanitization contract; evidence-inadmissibility
  failures remain non-passing and unpublished. Styling and smoke retain their
  separate behavior.
