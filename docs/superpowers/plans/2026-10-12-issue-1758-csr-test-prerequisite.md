# CSR test prerequisite implementation outline (#1758)

> Execute with jaunder-iterate; use jaunder-dispatch for a bounded task only
> when useful. Trigger: artifact concurrency and the future multi-agent provider
> contract. Spec is authoritative; final independent review has no findings and
> HUMAN outline approval is recorded. Planning commit `dbbb24be` is complete.
> Current assignment: routine shipping after full landed-main integration.
> Independent source and positive-proof conformance accepted without findings;
> normal commits/hooks/push/PR observation authorized. Merge remains unapproved.
> Earlier planning/preparation restrictions below are historical.

## Scope

In: shared test-local prerequisite resolution, canonical validation reuse,
compilation-lifetime ownership, current compiler root propagation, regressions,
and CONTRIBUTING provenance/cost documentation.

Out: copying #1702's application provider/native changes, verifier relaxation,
provenance caching, changed populations/parity/deadlines, launcher/config
repairs, resource cleanup, additional producer/startup proof, broad gates,
source commit, push or PR in this preparation assignment. Reviewed source stays
unchanged.

## Task outline

- [x] Shared prerequisite and ownership deliverable.
  - Implemented the exact accepted source paths. Focused offline/locked
    regressions pass: 11 xtask orchestration/prerequisite/producer tests, two
    migrated shell tests and 15 server staging/caller tests. Independent source
    conformance accepted with no findings; these remain focused regression
    proofs.
  - Acquire checkout-local artifact ownership before automatic production or
    explicit-root validation; retain it through all current consumer
    compilation. Coordinate with HostArtifacts' existing
    `.xtask/host-artifacts.lock` contract: `build_csr` deletes previous output,
    so releasing after production is unsafe. Advisory ownership is not
    protection against an uncooperative external writer; explicit input requires
    owner-consented stability through compilation. No foreign-root deletion or
    claim of locking arbitrary cross-checkout producers.
  - Automatic: invoke the checked-out supported producer, then validate its
    complete result; explicit: canonicalize the intentional root, validate
    early, no fallback. Manager-authorized `build_csr.rs` wasm Cargo `--locked`
    addition preserves all other arguments, outputs and lockfiles; small
    argument-helper regressions cover debug/release and preserve the existing
    locked postprocessor launch test. Offline environment alone does not prevent
    lockfile mutation.
  - Compose canonical CSR manifest/representation validation, the
    system-inventory loader and mechanically extracted rendered-shell
    validation. The exact accepted writer/test boundary below replaces any
    open-ended extraction permission.
  - Pass one absolute root to current test compilation while preserving
    PostgreSQL wrapping, compile-cache environment and explicit/default nextest
    arguments. Future fixture-provider compilation uses the same contract only
    after a separate coordinated integration; it is not added by this issue.
  - Verification: actual orchestration assertions cover ownership duration,
    ordering, early failure/no fallback and identical-root propagation;
    controlled collaborators stand in for subprocesses only, not canonical
    validators or startup correctness.
- [x] Smallest complete consumer proof and documentation deliverable.
  - CONTRIBUTING provisioning/provenance/stability/cost documentation is
    implemented. One coordinated automatic producer/complete validation/real
    startup invocation passed: exit 0, no signal, 1043 seconds within the
    1800-second ceiling. Independent positive-proof conformance accepted with no
    findings. Evidence is retained in ignored
    `.xtask/issue1758-positive-proof/`; successful output suppression prevents a
    fresh executed census, coverage or backend-matrix claim. Source/index
    identities and bounded ownership settlement were preserved.
  - Before harness expansion, run one coordinated real existing startup test via
    the corrected shared lane, using its automatic producer once (no duplicate
    build). Keep actual source/bundle identity and selected/executed evidence,
    terminal outcome, and errors; report nextest output-retention limits
    honestly.
  - Document explicit source-match trust versus integrity, ownership
    expectations, automatic supported path, approved roughly 620-second
    historical producer cost, failure behavior and no-Nix product-test work in
    CONTRIBUTING.
  - Verification: paired orchestration proof and real startup proof,
    documentation conformance, unchanged args/populations/backend
    parity/security/deadlines. Peer receipts support the route but do not
    certify our revision or coverage.

## Manager-accepted mechanical writer/test boundary

These are accepted future paths, NOT permission to implement in the
planning-commit assignment. No new dependencies, Cargo manifest/lock/version
edits or product API/protocol change; all validation rules, diagnostics and
behavior stay unchanged.

- `tools/csr_bundle/src/shell.rs` (new): move `validate_shell` unchanged as
  `validate_rendered_shell`; narrowly scoped `ShellValidationError` preserves
  display text. Own shell-validator unit tests here. Existing `publish = false`
  build-contract crate uses Rust `pub` for internal cross-crate reuse, not a
  product API.
- `tools/csr_bundle/src/lib.rs`: private module declaration and explicit
  validator/ error re-exports only; no dependency additions.
- `server/src/build_staging.rs`: remove private validator;
  `verify_staged_bundle` calls the shared validator and maps its error into
  existing `BundleStageError`. Keep manifest validation/staging unchanged. Move
  `shell_requires_unique_urls_and_required_order` to shell.rs; split
  `staging_handles_missing_public_tree_and_missing_shell_positions`, keeping the
  missing-public-tree staging proof in server and moving shell-only
  missing-marker assertions. Keep other staging tests; add `stage_bundle`
  rejection of a mismatched application stylesheet URL to prove shared-validator
  caller wiring.
- `xtask/src/steps/test_local.rs`: shared prerequisite orchestration and
  actual-seam
  lock-lifetime/ordering/blocked-consumer/explicit-no-fallback/root/argument
  tests.
- `xtask/src/steps/csr_prerequisite.rs` (new): compose existing manifest, system
  and shared shell validators; own focused prerequisite/error-path tests. No
  grammar copy.
- `xtask/src/lib.rs`: register the sibling in the existing inline `mod steps`
  (verified against the actual tree); no assumed steps/mod.rs registration.
- `xtask/src/steps/host_server.rs`: only make `host_artifact_build_lock`
  `pub(crate)`; retain existing callers and reuse ownership through consumer
  compilation.
- `xtask/src/steps/build_csr.rs`: exact wasm `--locked`
  addition/helper/regressions above; output paths and remaining launch arguments
  unchanged.
- `CONTRIBUTING.md`: approved provisioning, source-match/stability trust and
  cost.

Untouched: `server/build.rs`, manifest/system validator behavior, production
empty embed permissiveness, Cargo manifests/locks/versions and all #1702
provider/native paths. Existing server/xtask csr_bundle dependencies and xtask
host dependency already support this reuse; no dependency edge or cycle is
introduced.

## Bounded diagnostic plan — completed under separate assignments

Every invocation has an explicit 1800-second runtime ceiling; stop on
dependency, capacity, timeout or signal failure without retries. Use the normal
existing cache policy and an owned or explicitly consented compatible target; no
cache resets. Native Cargo below is auxiliary-test diagnostics, never an xtask
launcher fallback.

- Auxiliary orchestration regression filter (extend filter only to the named new
  prerequisite module if its tests are not under test_local):
  `devtool run -- env CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=1 RUSTC_WRAPPER=sccache CARGO_INCREMENTAL=0 cargo test --offline --locked -j 1 --manifest-path xtask/Cargo.toml --target-dir /home/mdorman/src/jaunder/herdsman-work-e01e49cc/target/issue-1758-xtask test_local`
- Real startup proof shape (completed separately with explicit ambient-input
  scrub, `--json`, normal owned targets and exact identity/settlement capture;
  no rerun):
  `devtool run -- env CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=1 cargo xtask test-local -- -p jaunder --offline --locked -E 'test(/^commands::lifecycle::tests::prepare_server_auto_initializes_in_dev_mode$/)' --no-fail-fast --failure-output immediate-final`
  Use this checkout's owned default `target` with producer/consumer
  serialization; obtain consent before any shared compatible target instead. The
  supported producer's `target/site`/wasm path remains its contract. No
  cross-source peer reuse.

## Risk checks and later boundaries

- Source provenance is not inferred from digests, HEAD or bundle existence;
  retain #1701's application-CSS mismatch rejection and explicit-input trust
  requirements.
- Test-local lock lifetime must cover consumer compilation; consent/shared-lock
  participation must account for direct producers that otherwise remove output.
- Current main has no application fixture provider. Preserve its future
  interface without editing peer workflow/owner/commands/supervision/server-main
  paths.
- Human approved spec/outline and cost; Manager accepted exact mechanical paths/
  test migration and locked producer inputs. Final reviewer recheck has no
  findings. Planning commit is authorized; implementation still needs its
  bounded assignment.
- Manager clarified `cargo xtask -> shell cargo-xtask -> nix run .#xtask` is
  supported: Nix builds the standalone executable, which executes on the HOST,
  not in a derivation. No policy rewrite, alias, native fallback or environment
  repair.
- Manager released capacity for our planning-hook/focused-regression window.
  Stage approved spec/outline alone; normal git commit invokes precommit with an
  explicit 3600-second ceiling. Cold source-matched standalone realization is
  allowed through the normal launcher; do not pre-run broad gates or use a
  native fallback. Stop on timeout, signal, dependency or capacity failure
  without retry/cleanup. No producer/startup proof or source implementation in
  this planning-commit task.
