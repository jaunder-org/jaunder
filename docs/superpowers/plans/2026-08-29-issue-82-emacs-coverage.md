# Emacs Protocol Client Coverage Implementation Outline

> Execute with `jaunder-iterate`, delegating an individual task through
> `jaunder-dispatch` when useful. This outline exists because issue #82 changes
> the durable Nix/xtask verification boundary and its producer-consumer
> contract.

Authoritative contract:
[approved issue #82 specification](../specs/2026-08-29-issue-82-emacs-coverage.md).

## Scope

In:

- Pinned, source-loaded Undercover instrumentation for the production Emacs
  Protocol Client.
- A module/form census, LCOV reconciliation, strict line-marker gate, and
  controlled producer status.
- One combined pure/live NixOS producer in `validate --no-e2e` and CI, replacing
  the standalone live-integration check.
- Tests and current contributor/client/architecture documentation for the new
  verification contract.

Out:

- Emacs Protocol Client or AtomPub behavior changes.
- Rust LLVM/CRAP, Wasm, and Playwright coverage-policy changes.
- External coverage services, committed baselines, thresholds, or block markers.

## Task outline

- [x] Task 1: Implement and prove the stateless Elisp coverage consumer.
  - Contract: a new `xtask` Elisp-coverage module consumes production
    `elisp/*.el` source plus
    `elisp-coverage/{lcov.info,summary.txt,status.json}`; it remains separate
    from the Rust LLVM coverage module.
  - Contract: `status.json` uses schema `elisp-coverage-v1`, exactly the
    controlled outcomes `success`, `ert-failure`, `instrumentation-failure`, and
    `invalid-report`, and the producer-owned pre-test census. The census records
    module paths and forms as `{start_line, kind, points}`, where each point is
    `{line, kind: ordinary|synthetic}`; an unknown or malformed schema fails
    closed.
  - Contract: validate that the handed-off modules/forms match current
    production source, reconcile every ordinary point with exactly one LCOV
    record, treat each synthetic point as uncovered, and accept only trailing
    same-line `;; cov:ignore: <reason>` with a non-empty trimmed reason.
  - Verification: focused xtask tests over a shared coverage-fixture corpus
    prove all status outcomes; missing module, form, or LCOV point; duplicate
    point; uncovered and covered points; zero-stop and production-macro forms;
    valid, empty, malformed, covered, non-executable, and non-trailing markers.
    Use the xtask workspace's `test-local` lane with
    `--manifest-path xtask/Cargo.toml`.

- [x] Task 2: Produce authoritative combined coverage hermetically.
  - Depends on Task 1's artifact/status schema and population rules.
  - Contract: pin Undercover in `emacsForCi`; no runtime package resolution.
    Before tests or `(require 'jaunder)`, enumerate the flat production modules
    and top-level forms, instrument them, and write the authoritative census
    defined by Task 1 into `status.json`. The umbrella load then eagerly loads
    every measured production module.
  - Contract: one batch coverage runner loads production once, runs all pure
    ERT, then runs all live ERT through the existing shared self-booting server
    lifecycle, preserving teardown and report finalization after controlled ERT
    failure. Test files, helpers, runners, vendor sources, generated files, and
    bytecode never enter the census.
  - Contract: the NixOS producer realizes
    `$out/elisp-coverage/{lcov.info,summary.txt,status.json}` for every
    controlled outcome and exits successfully for the consumer to decide;
    uncontrolled Nix or VM failures remain derivation failures.
  - Verification: focused producer tests use the same fixture corpus as the
    consumer to prove instrumentation precedes module loading, the census
    handoff is identical, both ERT populations contribute, and every excluded
    file class stays absent beside an included production module. They also
    prove every controlled failure retains all three artifacts; a forced
    uncontrolled VM/wrapper failure makes the derivation fail without a
    controlled artifact set; `jaunder--with-blog` and representative
    `cl-defstruct`/declaration forms remain visible; and server teardown always
    runs. Build the producer through
    `devtool run -- nix build -L --accept-flake-config .#checks.x86_64-linux.elisp-coverage-producer`.

- [ ] Task 3: Cut over the verification ladder and remove the obsolete path.
  - Depends on Tasks 1 and 2.
  - Contract: `steps::nix::test_checks` builds the producer and invokes the
    consumer once for both `validate --no-e2e` and full `validate`; the e2e path
    does not rerun live ERT.
  - Contract: remove the `e2e-elisp-integration` flake check, xtask
    command/step, `e2e-checks` membership, standalone CI job, and e2e-gate
    dependency. Keep the fast pure `ert` static check and host-side live runner
    available for development.
  - Contract: update `CONTRIBUTING.md`'s validation/CI/Elisp sections,
    `elisp/README.md`'s runner and coverage section, and
    `docs/ARCHITECTURE.md`'s e2e, Elisp-testing, and verify-ladder projections.
    Remove interim-exemption and standalone-live language; state no-e2e
    ownership, fixed artifacts, census reconciliation, and the strict marker.
  - Verification: xtask orchestration tests prove no-e2e ownership and no full
    validate duplicate; CI-shape checks prove the static job owns the verdict
    and e2e-gate depends only on the browser matrix; documentation checks prove
    obsolete standalone-live/interim-exemption claims are absent and the new
    ownership/marker policy is present; `cargo xtask validate --no-e2e` proves
    the complete authoritative path.

## Key contracts

- Producer artifacts are an all-or-nothing controlled-outcome set at the three
  approved `$out/elisp-coverage/` paths. Missing artifacts are infrastructure or
  invalid-report failures, never an empty successful report.
- Producer status owns execution/instrumentation state and the pre-test
  module/form/point census; the consumer validates that handoff against current
  source, owns LCOV/marker reconciliation, and decides the final xtask verdict.
- Production population is discovered from the flat `elisp/*.el` module set, not
  a hand-maintained allowlist. Every discovered module and top-level form is
  accounted for before tests execute.
- Pure and live observations form one union. Neither suite independently owes
  complete coverage.
- The clean cutover leaves no alias, deprecated command, duplicate VM run, or
  external uploader.

## Risk checks

- `require` caching must not let any production module load before
  instrumentation.
- ERT failure must not skip live-server teardown or report/status finalization.
- Undercover/Edebug omissions, especially `defmacro`, `cl-defstruct`,
  declaration, and `provide` forms, must become census failures or synthetic
  points rather than disappear.
- A controlled producer failure must preserve artifacts; an uncontrolled Nix/VM
  failure must not be mislabeled as a coverage gap.
- The staged tree reaches `jaunder-commit` after each independently verified
  task; no lint suppression or `Co-Authored-By` trailer is introduced.
