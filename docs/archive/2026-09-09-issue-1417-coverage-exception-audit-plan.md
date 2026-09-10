# Coverage Exception Audit Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for independent path
> slices. This outline exists because the reason-bearing marker grammar changes
> a durable coverage-gate contract and the audit needs stable multi-agent
> ownership.

## Scope

In:

- Disposition and remediation of all 95 baseline line exclusions, 63 baseline
  exclusion blocks, and 8 baseline CRAP overrides.
- A closed census of workspace membership, coverage source filtering, host
  target gating, and literal message-bearing `unreachable!` exemptions.
- A clean cutover to mandatory reasons on line exclusions and block starts.
- Proposed ADR, architecture projection, contributor guidance, and complete
  review evidence for the pull request and issue.

Out:

- Coverage/CRAP threshold changes, backend or source-population changes,
  synthetic host coverage for browser-only code, permanent audit snapshots, and
  unrelated complexity work.

## Shared contracts

- Path ownership is exclusive while audit slices run: `server/**`, `storage/**`,
  `host/**` + `macros/**` + `test-support/**`, and `web/**` are independent.
- Slice work may remove or narrow exceptions immediately, but retained
  `cov:ignore` markers keep legacy spelling until the final grammar cutover.
- Every baseline item returns one disposition record: baseline kind and
  location; exempted behavior/complexity; host executability; removal, refactor,
  narrowing, structural replacement, or retention; verification evidence; final
  location; and retained rationale category when applicable.
- Retained rationale categories are closed: `non-host`, `genuinely-unreachable`,
  `fault-injection-only`, `generated-build-script`, or `compiler-bookkeeping`. A
  site that cannot meet one category returns to remediation rather than gaining
  a new category silently.
- Consumer-observable behavior, not helper wiring or source text, is the test
  contract. Dual-backend storage behavior uses the established backend harness.
- Final marker grammar is exactly `// cov:ignore: <specific reason>` and
  `// cov:ignore-start: <specific reason>` … `// cov:ignore-stop`.

## Task outline

- [x] Task 1: Remediate server coverage and build-script exceptions
  - Contract: Own `server/**`, including its five CRAP overrides. Preserve
    public behavior and record every baseline disposition using the shared
    schema.
  - Verification: Focused server and build-staging behavior runs prove each
    removed exception; retained spans are minimal and category-valid.
- [x] Task 2: Remediate storage coverage exceptions
  - Contract: Own `storage/**`; preserve SQLite/PostgreSQL parity and use
    `#[apply(backends)]` where behavior crosses adapters. Record every baseline
    disposition using the shared schema.
  - Verification: Focused dual-backend behavior runs prove removals and
    unchanged storage semantics; retained fault/compiler spans are minimal.
- [x] Task 3: Remediate host, macro, and test-support exceptions
  - Contract: Own `host/**`, `macros/**`, and `test-support/**`. Prefer
    structural `unreachable!("reason")` only for proven domain invariants.
    Record every baseline disposition using the shared schema.
  - Verification: Focused host-native behavior and macro expansion runs prove
    removals; retained non-runtime/compiler spans are minimal.
- [x] Task 4: Remediate web coverage and browser-only CRAP exceptions
  - Contract: Own `web/**`, including three CRAP overrides. Extract
    host-testable pure behavior only where it simplifies the browser handler; do
    not build host surrogates for DOM primitives. Record every baseline
    disposition.
  - Verification: Host-testable extracted behavior and the relevant actual web
    surface prove changes; retained WASM-only spans are minimal and
    category-valid.
- [x] Task 5: Cut over the coverage exception contract
  - Depends on: Tasks 1–4 disposition records and code changes.
  - Contract: Update the marker parser and regression checks to require
    non-empty reasons on line markers and block starts, preserve anchored
    real-comment and balanced-block semantics, migrate every remaining marker,
    and reject all legacy bare forms. Record the decision in a tracked proposed
    ADR, project it into `docs/ARCHITECTURE.md`, update `CONTRIBUTING.md`, and
    leave `CONTEXT.md` unchanged because no domain vocabulary changes.
  - Verification: Focused xtask coverage-parser tests prove accepted/rejected
    grammar and fail-closed behavior; a repository scan finds no active legacy
    marker and no unexplained retained exception.
- [x] Task 6: Reconcile the complete audit and prove the repository
  - Depends on: Tasks 1–5.
  - Contract: Re-run the active-marker census; enumerate all four structural
    classes from their owning manifests/configuration/source; reconcile every
    baseline disposition exactly once; prepare the complete per-item table,
    before/after counts by kind, and remaining counts by rationale category for
    the PR and issue record. Commit no generated audit snapshot.
  - Verification: Run the focused checks missed by no slice, then the repository
    gate selected by `jaunder-commit`; final hermetic coverage proves unchanged
    line/CRAP thresholds and the established SQLite/PostgreSQL union.

## Risk checks

- No baseline item disappears through line drift: reconciliation keys each item
  by baseline path plus enclosing symbol/span, not final line number alone.
- Parser migration and retained-marker rewrite land as one clean cutover; no
  compatibility grammar survives.
- No slice introduces a lint suppression without explicit approval.
- Existing integration coverage invalidates a contradictory retained rationale.
- Broadness is reviewed semantically; no arbitrary formatted-line cap
  substitutes for examining the exempted behavior.
- `docs/README.md` and ADR promotion remain automation-owned.
- Each independently accepted slice reaches `jaunder-commit`; no commit receives
  a `Co-Authored-By` trailer.
