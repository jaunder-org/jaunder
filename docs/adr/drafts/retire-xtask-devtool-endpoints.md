# ADR-DRAFT: Simplify Residual Tooling Capabilities and Retire Obsolete Endpoints

- Status: proposed
- Issue: [#1898](https://github.com/jaunder-org/jaunder/issues/1898)

## Context

Prior family migrations moved public workflow composition to Just, producer
selection to Nix and distributed scheduling/privileged identity to native CI.
Residual xtask/devtool callers still include substantive project policy,
evidence validation, focused proof and qualification lifecycle algorithms.
Deleting executable names alone would either lose these guarantees or relocate
the same omnibus behind a new dispatcher. Rewritten Milestone33 requires a
finite useful system and lower total maintenance, not universal sharing, receipt
adoption or a language-wide rewrite.

This is a PROPOSED amendment to the preserved design-only endpoint decision, not
implementation or a claim the endpoints are gone. Old qualified approval and
exact bytes remain historical evidence. All command-level material-loss and
implementation/outline/merge approval gates remain binding.

## Decision

Just owns common local workflow composition; Nix owns pinned tools and admitted
immutable input/output packages; native CI owns job dependencies, artifacts and
required aggregates. Keep each static pinned invocation/argument set in one
named Just leaf called by both host composition and its hermetic Nix leaf.
Preserve different modes/populations/platforms and host/offline Cargo/cargo-deny
policy. Sharing definition does not establish equivalent host/hermetic verdicts.

Retain only project algorithms with real consumers and demonstrated requirements
not met by the selected standard tools. Reuse existing syntax/schema/domain
libraries; do not implement parser/SDK/Git engines or a general workflow/report
framework. Existing narrow Leptosfmt scope/UTF8/false-green-error validation
remains until supported equivalent behavior is demonstrated. Python is allowed
for focused maintainable infrastructure under the delivered policy, not
required.

Choose these concern homes for the three residual clusters:

- **Repository policies:** one auxiliary tools/repository-policy package,
  private shared parsing/library code, and repository-docs/source-contracts
  executables exposing fixed typed policy operations. No execution commands,
  arbitrary check-name registry, gate population/order ownership, `--all`
  workflow or generic report envelope. Just retains independent leaf verdicts.
  Existing Markdown gate recognition is preserved rather than silently changed
  to the promoter parser's separately approved eligibility behavior. The package
  also owns separate git-index-safety (whole single-process precommit/docs
  admission), scoped-leptosfmt and Org-timezone extraction targets.
  Checkout/source admission and best-effort relative hook setup share one
  library owner and explicit prepare callsites; pure/artifact consumers do not
  acquire Git-config mutation.
- **Qualification:** one auxiliary tools/qualification package and a single
  qualification-host executable retaining the existing bounded harness/source/
  target argument contract behind four public Just recipes. Reuse host/common
  inventory/domain code, in-process jsonschema, native Git/Nix and the current
  safe-open lease/lifecycle/publication algorithms. Nix's harness binds the
  selected executable to its compiled output and retains helper/browser/VM
  admission. Keep ordinary report parity/sanitization, failed-product evidence
  semantics, styling pass-only policy and no-evidence smoke distinct. Ordinary
  post-publication private cleanup failure does not revoke already durable
  evidence. A new small concern package avoids feature/source-filter coupling in
  performance's product/WASM-admitted package; performance remains unchanged.
- **Focused host proof:** one auxiliary tools/host-proof library with direct
  test-local/e2e-local executable targets and named Just frontdoors, not an
  umbrella host runner. Keep current ProcessKit/Tokio application adapters,
  source admission, test-local artifact-lock/CSR/worker environment, and e2e
  immutable-artifact/root/readiness/capture/cleanup status contracts. Standalone
  sandbox remains a separate owner; no new supervisor/shared runtime is created.

The complete finite ignored surviving-owner delta names remaining interfaces,
not “their concern owners” placeholders. Existing coverage/doctests/performance
packages own named coverage/status/policy, WASM, Elisp, mutation,
diagnostic-build, doctest and performance interfaces. Host-proof's fixed
e2e-evidence and trace- analysis/source-diagnostics interfaces serve their real
evidence/measurement consumers; Just sequences finite trace/benchmark workflows.
Existing test-support owns canonical seed-e2e. PG's scoped ephemeral
closure/custody moves into coverage's nondefault ephemeral-postgres library
module, shared by its producer and host-proof pg-run. No coverage-to-host-proof
dependency, product-host source/ feature expansion, second PG implementation or
new ProcessKit dependency is added. Qualification's separate system-artifacts
target is a Nix-executed producer, not host qualification execution. No package
becomes a catch-all dispatcher. Source/disposition evidence remains ignored;
maintained docs explain interfaces and invariants, not a permanent command
catalog.

Disposition manual commands individually. Preserve known operator/safety
capabilities, measurement windows and failure/diagnostic interpretation;
material capability/safety loss returns for explicit human confirmation. “None
known” external users and bounded search misses are not absence proof. No
forwarding facade is retained without evidence of a real consumer.

Retire launchers, aliases, generic result/catalog/report envelopes and dedicated
Cargo/Nix workspace/source/vendor/build paths with their last real consumers.
Compensating auxiliary tests move deliberately. Experiment-only coverage
emit/worker/aggregate endpoints are explicitly DEFERRED, retained with required
devtool build/test paths under #1898 Lead until fresh caller/operator readback
and an approved preserve/retire/material-loss disposition complete. Full devtool
retirement is not claimed or permitted while that residual endpoint remains.
Bootstrap/last-consumer delivery is required before any final retirement claim.
Compare combined production, test, manifest, executable, recipe, Nix and
documentation maintenance; one new interface cannot justify a larger hidden
framework. Explicitly deferred items remain named residual work, never falsely
reported as removed.

## Narrow supersessions and preserved contracts

- ADR0028 owner/interface clauses no longer require xtask/devtool omnibus
  executables. Keep host-versus-sandbox execution litmus, no host orchestration
  inside Nix derivations, and standalone traced-run.
- ADR0052/0146/0161/0178 static command-definition owner becomes shared named
  Just leaves. Keep ast-grep rules/fixtures, scope/failure checks, independent
  hermetic leaves, order/populations and host/sandbox policy differences.
- ADR0141 permits retirement of the standalone xtask workspace only after all
  consumers/test obligations are rehomed. Root product and tools auxiliary
  workspaces remain distinct; new concern packages are auxiliary, not product
  membership by implication.
- ADR0239 changes only executor/package/provenance path from selected xtask to
  qualification-host; source/admission/lease/smoke/ordinary/styling contracts
  remain. ADR0241's performance-only producer/host ownership is not expanded.
- ADR0087's legacy xtask PR interface is already superseded by the separately
  accepted ordinary observer/action migration; preserve exact-head required
  evidence, passive observation, approval-bearing land and managed protection.
- ADR0180 and promoted standalone-sandbox ADR0244 retain current independent
  sandbox ownership/lifecycle. This endpoint ADR neither chooses a new sandbox
  supervisor nor absorbs that family. Focused E2E ownership stays distinct.

ADR0029 Git/index safety is NOT weakened; ADR0237's later fail-fast composition
and mandatory failure-path reconciliation are current. Preserve selected
request/verdict identity, input-correct reuse/invalidation, malformed/missing
producer evidence failure and CI aggregate authority transferred from1865.
Backend/browser/test/coverage/provenance/authentication populations remain.
Promoter ADR0242/0152 privileges/serialization are unchanged.

#1864 owns its finite docs-only share/retain/defer disposition; no universal
verdict-sharing prerequisite is added. #1933 owns selected surviving passive
observations and local/native-CI retention/export contracts. Preserve those
interfaces when retiring legacy callers; add no recorder to retiring xtask,
PR1932 receipt protocol, Just patch, collector or competing supervisor.

## Consequences

Purpose-specific packages add explicit interfaces and manifests while removing
unrelated dispatch/build/report coupling. They preserve useful capabilities at
real consumers but do not promise cold bootstrap, cache hits, speedups or a
measured line reduction before implementation. Existing documented policy gaps
justify preserving algorithms initially; they do not prove every COTS option
inadequate. Meaningful consumer/failure/source-admission tests and a final total
maintenance comparison must support each implemented retirement.

No implementation, outline, deletion, commit, merge, tracker transition, cleanup
or artifact disposal is authorized by this ignored proposed text.
