# #1780 — E2E lane concurrency implementation outline

> Execute with `jaunder-iterate`; use `jaunder-dispatch` for bounded tasks when
> useful. Trigger: concurrency admission/refill and settlement invariants plus
> an explicit CLI contract. The approved #1780 spec remains authoritative.

## Scope and contracts

In: full `validate --e2e-jobs N`, truthful readback, invocation-local bounded
build dispatch, preserved evidence/verdicts, focused host proof and guidance.
Out: global scheduling/resource estimation, #1753 admission, CI/topology
changes, guest/browser changes, VM performance campaigns, or protection waivers.

Keep one implementation writer. The three slices below form one integrated
review deliverable; no placeholder interfaces or independently shipped partial
CLI behavior. Readiness for implementation requires separate outline approval
and normal approved-contract commit prerequisites, not merely spec approval.

## Task outline

- [x] 1. Expose the approved caller contract and resolved readback.
  - Contract: `cli.rs` parses optional positive N only for `Validate` and
    rejects invalid/overflow values and explicit `--no-e2e` conflict before work
    starts. `dispatch.rs` passes the resolved typed input to full E2E
    orchestration. Readback distinguishes omitted/requested N, enabled count and
    effective min(N, count), using the ordinary structured result and human
    output before dispatch; omission keeps existing fan-out. Other command
    defaults stay put.
  - Verification: focused CLI parsing and result-rendering/serialization tests,
    including omission, valid/above-count N, every specified rejection, and
    readback against the actual enabled catalog. No validation or VM launch.
- [x] 2. Bound actual lane-build work without reducing completed protection.
  - Contract: adapt `steps/nix.rs::build_candidate_lanes`' injected build seam
    and full `e2e` callsite. Refill a freed slot without waiting for an
    unrelated active build; count through existing build-completion handling.
    Return every enabled lane exactly once in catalog order after all work
    settles, including reported build/spawn/wait failures. Compatibility callers
    retain their current all-selected-lanes behavior. Keep
    `build_installable`/`Process` supervision, roots and recovery rather than
    introducing another runtime.
  - Verification: synchronized fake build collaborators exercise this same
    production dispatcher at one/intermediate/at-or-above-population bounds.
    Prove bounded peak, permitted overlap, prompt refill, exactly-once enabled
    population, disabled exclusion, out-of-order completion, early failure with
    pending work continuing, complete settlement and deterministic results.
- [x] 3. Close integrated evidence, timing and documentation acceptance.
  - Contract: preserve `finish_e2e_combo`'s unconditional lift and successful
    duration-then-boot checks; reconcile both Firefox backends only after all
    builds settle; preserve `E2eOutcome` and authoritative flow-coverage gating.
    Keep lane evidence paths isolated. Preserve phase sidecars, but do not use
    `record_parallel_phases`' maximum as elapsed time for sequentially admitted
    lanes: report unobservable aggregate timing as unavailable with a reason,
    rather than inventing measurements or adding timing infrastructure.
  - Verification: compose fake build outcomes with existing real evidence
    consumers/maintained fixtures to prove red propagation, complete expected
    evidence, artifact independence and ordering. Add focused timing/readback
    regression checks. Update CONTRIBUTING's full-local-validation guidance with
    the option/default, caller CPU/RAM responsibility, Nix scheduling
    distinction and separate #1753 scope. Retain proof output only in ignored
    run storage/PR.

## Risk checks and verification boundary

- Existing all-start-before-wait test must not deadlock bounded execution; use
  controlled completions, not an all-lanes barrier or wall-clock speed claim.
- Preserve ADR-0203's catalog/parity/reconciliation and ADR-0037 diagnostics;
  fake only the build collaborator, never browser/database/evidence semantics.
- Use focused auxiliary-workspace diagnostics, not product `test-local`:
  `devtool run -- cargo test --manifest-path xtask/Cargo.toml <test-filter>`.
  Applicable filters cover CLI, actual dispatcher, results and evidence
  consumers; then the full xtask host suite closes the integrated deliverable.
  These are future proof commands, not permission to execute during outline
  preparation.
- Normal hook-backed contract/implementation commits and later push/ship gates
  retain their documented authority. No gate bypass, coverage exception or full
  VM matrix is justified merely to demonstrate the host dispatch bound.
