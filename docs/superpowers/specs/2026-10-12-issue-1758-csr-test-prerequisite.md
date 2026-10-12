# Complete CSR prerequisite for host application tests (#1758)

- Issue: https://github.com/jaunder-org/jaunder/issues/1758
- Status: spec and outline HUMAN-APPROVED; independent final review has no
  findings.
- Authorization: hook-backed planning commit only; implementation not yet
  assigned.
- Base: current main `28fb72fed89fdd074cb7cf0493bdbe4408bf203b`.
- Branch: `herdsman/work-e01e49cc` (explicitly approved naming exception).

## Outcome

The shared host-native product test lane resolves a complete source-matched CSR
bundle before compiling or launching startup-capable fixtures and behavioral
tests. Missing provisioning cannot reach startup as a misleading behavioral
failure; declared invalid inputs remain visible prerequisite failures.

## Load-bearing decisions

- HUMAN-APPROVED: absent explicit `JAUNDER_CSR_BUNDLE_DIR`, use the supported
  checked-out local producer before startup-capable fixture/test compilation.
- HUMAN-APPROVED: explicit input receives early complete validation with no
  fallback: CSR manifest/representations, rendered shell and system inventory.
- HUMAN-APPROVED: one resolved absolute root reaches every startup-capable
  fixture and test compiler; preserve supplied nextest arguments/populations.
- Manifest integrity is not source provenance. An explicit root must be supplied
  intentionally from matching resolved producer source inputs, toolchain and
  profile, or independently verified equivalent inputs. Validators alone do not
  establish equivalence. Arbitrary cross-source or merely same-HEAD reuse is
  out.
- #1701 rejected peer reuse after `server/assets/jaunder.css` disagreed with
  embedded `application.css`; this is evidence against arbitrary bundle reuse,
  not a reason to weaken verification or transfer peer proof ownership.
- Bare/dependency compilation may retain its existing non-production empty
  embed; runnable test preparation must not treat that permission as
  deployability.
- Prior supported production took about 620 seconds; the roughly ten-minute
  producer cost is disclosed and HUMAN-APPROVED. No speculative provenance cache
  or acceptance based only on existing `target/site` presence is introduced.
- Shared orchestration belongs at `xtask/src/steps/test_local.rs`, reaching
  check and prepush. Manager authorized the future
  `xtask/src/steps/build_csr.rs` wasm Cargo `--locked` addition with small
  debug/release argument-helper tests; all other arguments, outputs and
  lockfiles remain unchanged. Offline is not locked.
- Manager ACCEPTED the exact future mechanical writer boundary; execution
  remains prohibited by this documents-only assignment: new
  `tools/csr_bundle/src/shell.rs`, `tools/csr_bundle/src/lib.rs`,
  `server/src/build_staging.rs`, `xtask/src/steps/test_local.rs`, new
  `xtask/src/steps/csr_prerequisite.rs`, `xtask/src/lib.rs`,
  `xtask/src/steps/host_server.rs`, and `CONTRIBUTING.md`. Register the
  prerequisite in the actual inline `mod steps` in `xtask/src/lib.rs`. The
  outline defines exact validator/caller/test ownership, not an open extraction.
- Accepted shared shell validation is mechanical internal reuse in the existing
  unpublished (`publish = false`) `csr_bundle` build-contract crate. Rust `pub`
  access across existing dependencies is not a new product API/protocol.
  Preserve rules/diagnostics; add no dependencies, manifests or versions.
  Accepted lock reuse changes only `host_artifact_build_lock` visibility to
  `pub(crate)` and retains ownership from prerequisite resolution through
  consumer compilation.
- Current main has no #1702 application provider. Do not copy its unmerged
  `application_fixture.rs` or native corrections. A future provider must consume
  the same resolved root only through an explicitly coordinated integration.
- Preserve security validators, populations, SQLite/PostgreSQL parity, coverage
  authority, command/test deadlines, clean-tree hooks and prepush's no-Nix rule.

## Acceptance

- Regression tests exercise the actual shared orchestration: prerequisite
  success precedes current test compilation; producer/validation failure blocks
  compilation and the runner; explicit absent/corrupt input never invokes
  fallback production.
- Assertions cover absolute-root propagation to current test compilation and
  unchanged nextest argument/default-workspace behavior, including failure
  results. Future fixture-provider compilation must consume that same root
  through an explicitly coordinated integration; adding/copying that provider is
  not scope.
- Controlled subprocess collaborators prove orchestration, not real bundle or
  startup validity; pair them with a real existing startup test through
  test-local.
- Accepted extraction moves shell-only tests to the shared validator; server
  keeps staging tests and proves rejection of a mismatched application
  stylesheet URL. Preserve canonical manifest/system validation and all existing
  diagnostics.
- Bounded later startup proof selects
  `commands::lifecycle::tests::prepare_server_auto_initializes_in_dev_mode` with
  no fail-fast and immediate-final failure output. Retain exact command, source,
  bundle identity, exit/signal and selected/executed evidence; no coverage
  claim.
- If accepted proof expands to all six #1701 lifecycle tests, preserve the sixth
  test's discarded-task-error limitation; no observability source changes here.
- Future tests use offline/locked Cargo, one job, own or consented compatible
  target and existing cache policy, at most 1800 seconds per invocation. Stop on
  dependency, timeout, signal or capacity failure; no retries or cache cleanup.
- Real producer/startup proof requires coordination with #1701/#1702. Their
  independently provisioned proof is supporting evidence, not our certification.

## Boundaries

No edits to peer `application_workflow.rs`, `application_workflow_owner.rs`,
`commands.rs`, `supervision.rs` or server-main diagnostics; no verifier changes,
synthetic inventories, excludes/ignores, deadline increases or resource
mutation. No start-blocking dependency on peer shipping. Preparation performed
no workloads. Planning commit is now separately authorized after independent
review, human outline approval and Manager capacity release; only approved
planning documents are staged. Normal launcher/tool realization and the enforced
hook are allowed for that commit. No implementation, producer, startup proof,
push, PR or configuration changes in the planning-commit assignment.
Implementation requires the next bounded assignment; merge needs approval.
