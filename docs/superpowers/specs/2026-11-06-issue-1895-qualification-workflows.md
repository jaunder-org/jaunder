# Issue #1895: Make qualification workflows single-command Just workflows

## Outcome

Developers can invoke each currently exposed production-baseline and
qualification workflow through one documented Just command. The migration
removes redundant host command catalog, scheduling, and generic report-envelope
layers without weakening the actual workflow: Nix remains authoritative for
pinned executables, derivations, and VM/harness provenance; focused host code
retains only qualification-specific work that ordinary tools cannot replace with
demonstrated equivalent guarantees.

## Approach and tradeoffs

Use root Just as the documented human front door, with four public recipes:
`production-baseline-discover <revision>`,
`production-baseline-accept <source> <target>`,
`production-baseline-qualify-styling`, and `production-baseline-smoke`. Just
owns workflow names, positional argument forwarding, ordinary input
normalization, dependency ordering, and the top-level fail-fast invocation. For
discover and accept, Just requires a clean checkout and records its current full
`HEAD` as the expected harness commit; it resolves the requested product
source/target expressions with `git rev-parse --verify`, rejects equal resolved
commits, validates canonical `origin`, and performs one
`git fetch --quiet --no-write-fetch-head --refetch origin <commits>` for the
distinct set of expected harness HEAD plus source and optional target. Ordinary
fetch can accept locally complete objects without asking the server for them;
`--refetch` requests the selected objects afresh. This accepts larger transfers
and normal Git maintenance to establish that origin serves every requested
object, not that it is reachable from a particular named branch. All three
commits remain upstream-verified even when the harness differs from product
revisions. The host re-admits the clean source with `SourcePin` and requires its
pinned SHA to equal the Just-supplied expected harness SHA before Nix admission
or lifecycle mutation. This binds the fetch to the admitted source without
repeating origin verification or upstream fetch. The host receives canonical
commit IDs, not product revision expressions, and retains later safety rechecks.

Styling qualification separately requires a clean checkout, captures current
full `HEAD`, and binds it to the host's clean `SourcePin` before Nix admission.
It retains its existing immutable local `git+file:…?rev=<commit>` harness
admission and does not require canonical `origin`, network access, or upstream
availability. Smoke uses the mutable current-checkout/path flake and has no
qualification admission. The root recipes invoke the pinned host executable
using the existing Nix launcher/package path. Nix source-specific harness
admission remains domain work because it selects a fixture graph from the
admitted immutable source, builds and roots it, verifies the executing
tool/helper against selected outputs, and records derivation/output/NAR
identity. Just is not a renamed call to the old monolithic public command: its
Git preflight, distinct-commit refusal, operation-specific recipe and plain
process-status contract are actual transferred behavior.

The retained focused host operations accept those normalized inputs and own only
responsibilities requiring the live host/domain contract: source-cleanliness and
drift checks at safety checkpoints; source-specific Nix harness/output selection
and provenance; acquiring/releasing the cooperative shared-run lease; restricted
workspaces and GC roots; NixOS VM, proxy, browser and backend lifecycle;
fixture/protocol comparisons; canary collection; the distinct evidence
validations below; cleanup; publication only when evidence itself is valid and
sanitized; and product/harness/infrastructure attribution. Ordinary baseline
product/protocol failures may be published as valid failed outcomes with nonzero
status. These functions stop constructing the generic
`CommandResult`/`StepResult` report envelope. The `xtask` CLI no longer exposes
the current `production-baseline` operation catalog; Just recipes are the public
catalog and dispatch to a narrow host execution leaf with explicit immutable
inputs. Remove only this workflow's `main.rs` generic
`invocation_trace::execute` → `result.report` → `.xtask/last-result.json` →
`xtask-done:` path. Preserve those general `main.rs`/`result.rs` contracts and
tests for unrelated xtask commands. Do not move host code into a Nix derivation
or move xtask into the product workspace.

COTS assessment is responsibility-sized, not whole-system substitution. Just's
documented dependencies run before dependents and stop on command failure, so it
owns recipe sequencing; Git owns discover/accept SHA resolution and their single
combined harness/source/target upstream check, while styling retains its local
immutable-source path with no upstream requirement; Nix owns immutable output
identities and harness/VM packaging. Ordinary JSON Schema validation already
uses the COTS Rust `jsonschema` Draft 2020-12 engine
(`jsonschema::draft202012::validate`, `xtask/src/production_baseline.rs`;
dependency in `xtask/Cargo.toml` and `Cargo.lock`). Retain this in-process
engine: a separate schema CLI would add another executable/process/serialization
edge without replacing schema-specific report projection, Markdown parity, or
evidence policy; `jq` predicates are not full schema validation. Keep the
existing narrow Rust integration rather than adding a CLI adapter or installing
another package.

For locking, the current lease is more than mutual exclusion: it securely opens
without following symlinks, requires a regular file owned by the expected user,
and uses a private runtime directory
(`xtask/src/production_baseline/platform/unix.rs:9-65`). The `flock(1)`
candidate supplies an advisory descriptor lock but not those safe-path/owner
checks. Replacing the in-process lease with a shell wrapper would still require
the secure open/validation code, inherited descriptor lifetime across all
recipes, and platform packaging, increasing rather than reducing the maintained
seam. Keep the existing focused lease owner. Process supervision is a separate
decision: Python subprocess timeout/kill/reap primitives do not by themselves
manage this workflow's multiple VM/proxy children, lifecycle ordering,
restricted diagnostics and cleanup. A migration would add another host runtime
and adapter without removing those domain contracts; retain focused host
supervision. `rename(2)` is only an atomic same-filesystem primitive, not
schema/Markdown validation, canary and prohibited-file/symlink scans, allowlist,
collision refusal, or publication ordering. No one-tool replacement is proposed.
Full authoritative citations and the bounded comparisons are in
`.xtask/run/issue-1895-review-evidence.md`.

Evidence contracts remain workflow-specific. Ordinary discover/accept validate
JSON-authoritative `summary.json` against the production-baseline schema,
mechanically derive and parity-check `summary.md`, enforce its exact two-file
allowlist, scan canaries/forbidden content and prohibited file/symlink types,
refuse an existing destination, and publish atomically only after successful
collection and source recheck. Styling has a separate current contract: validate
its styling evidence identity/phase/context in Rust; scan JSON and separately
generated Markdown against canaries; stage both privately; clean lifecycle
resources; re-verify the admitted source; then rename staging to its unique
review destination. It does not use the ordinary baseline schema or Markdown
parity validator, and the migration must not add those or change styling
evidence meaning. Local smoke publishes neither evidence contract. Raw
credentials, databases, backups, browser state and logs remain in restricted
ignored workspace. For ordinary discover/accept, a product/protocol failure may
still produce a valid, sanitized failed-outcome report: schema and publication
checks must pass, the report records the failed outcome, and the command returns
nonzero. Do not confuse that reportable result with publication-inadmissibility
failures: failed collection/rendering/schema or Markdown
parity/canary/prohibited file checks, required cleanup, source recheck, or
sanitization means no durable report. Styling keeps its existing behavior: only
fully validated and cleaned runs are published; a styling workflow error is
nonzero and must not gain a new failed-report contract.

The remaining decision is not whether to move the entire workflow into COTS: the
researched tools cover discrete responsibilities only. Compare total
Just/Nix/Rust/scripts/tests/docs maintenance; keep the smallest host domain
implementation for the specific gaps above, and make each workflow's public Just
preflight and generic-report retirement explicit.

## Acceptance

- The four public Just recipes are documented:
  `production-baseline-discover <revision>`,
  `production-baseline-accept <source> <target>`,
  `production-baseline-qualify-styling`, and `production-baseline-smoke`.
  Discover/accept preflight resolves product commit IDs, records expected clean
  harness `HEAD`, verifies canonical origin, and fetches harness/source/target
  SHAs together with
  `git fetch --quiet --no-write-fetch-head --refetch origin <commits>`, without
  updating FETCH_HEAD. The host compares expected harness SHA with its admitted
  clean `SourcePin` before lifecycle mutation; it does not duplicate the
  upstream check. Styling captures clean expected `HEAD` and binds it to
  `SourcePin`, but uses its existing local immutable Nix source and adds no
  origin/network requirement. Smoke uses the current path checkout and publishes
  no evidence.
- The host leaf is limited to source/drift safety checks, source-specific Nix
  harness/output admission and provenance, lease, restricted workspace/GC roots,
  VM/proxy/browser/backend lifecycle, fixture/protocol work, canary handling,
  workflow-specific evidence checks, cleanup/publication ordering, and failure
  attribution. It returns focused diagnostics and process status, not generic
  `CommandResult`/`StepResult`; it is host-only and outside the product
  workspace.
- Retire the public `ProductionBaselineCommand`/`Command::ProductionBaseline`
  catalog and old dispatch. For these commands only, bypass the generic report
  path in `xtask/src/main.rs` and remove `result.report` JSON/human output,
  `.xtask/last-result.json`, and `xtask-done:`. Preserve those shared
  types/contracts/tests for unrelated commands. Update CLI,
  `docs/production-baseline.md`, `docs/application-assets.md`, the baseline
  projection in `docs/ARCHITECTURE.md:3119-3132`, and generic report explanation
  at `:4879-4884`; retain browser tests as consumers. Review `main.rs`,
  CLI/dispatch, `result.rs`, and
  `xtask/tests/{invocation_trace,plain_process_boundaries,e2e_workflow,just_workflows}.rs`.
- Ordinary discover/accept keep their JSON-authoritative schema-validated
  summary, generated Markdown parity, exact allowlist, sanitization and atomic
  publication rules. A valid sanitized product/protocol-failed outcome remains
  publishable as a failed report and returns nonzero.
  Publication-inadmissibility failures
  (collection/rendering/schema/parity/sanitization, required cleanup or source
  recheck) produce no report. Styling keeps its separate validator,
  canary-scanned JSON/Markdown, private staging, cleanup, source recheck and
  rename contract; it gains no ordinary schema/parity or failed-report behavior.
  Smoke publishes neither; raw state remains restricted and ignored.
- Reuse the existing Rust `jsonschema::draft202012::validate` COTS engine;
  schema CLI adaptation adds a process boundary without replacing custom report
  projection or policy. Keep the safe lease wrapper: `flock` only supplies the
  advisory lock, not no-follow opening, regular-file/owner checks or private
  runtime-directory handling. Assess process supervision separately; Python
  subprocess primitives do not implement the workflow's multi-process cleanup
  and diagnostics. Atomic rename does not implement publication policy. The
  compact evidence artifact records capability and maintenance comparisons.
- Focused proof covers combined harness/source/target upstream preflight and
  clean-pin binding without a duplicate fetch, source/output admission, smoke
  separation, lease/lifecycle, both publication contracts, valid failed-outcome
  report with nonzero status, publication-inadmissibility refusal and diagnostic
  attribution. No native build is qualification evidence.
- Proposed numberless ADR `docs/adr/drafts/qualification-workflow-ownership.md`
  and its `docs/ARCHITECTURE.md` projection are authored together and reviewed
  with the feature. The projection links descriptively to the draft and labels
  the ownership split proposed/pending approval and implementation; it does not
  overwrite the current accepted-workflow description. Preserve ADR-0028/0141
  constraints, keep the draft `proposed`, do not edit generated `docs/README.md`
  or manually promote it. After spec/design approval, implementation updates the
  current architecture projection to match delivered behavior.

## Boundaries

This is an opt-in development/qualification workflow migration, not a new
qualification contract or production/release claim. Do not weaken qualification
criteria, change fixtures/schema/evidence meaning, claim native builds as
evidence, migrate xtask/tools into the product workspace, invoke host xtask
inside Nix derivations, alter unrelated verification gates, add generic
orchestration/receipt frameworks, or perform unrelated qualification
improvements. Just only owns the enumerated ordinary Git input preflight and
four public entrypoints; the host retains only the named domain operations. Do
not remove `CommandResult`/`StepResult`, last-result, or sentinel behavior for
unrelated xtask commands. Any materially different safety/provenance contract or
unproven COTS substitution returns to design approval. No implementation is
authorized until this spec (and any triggered outline) is approved.
