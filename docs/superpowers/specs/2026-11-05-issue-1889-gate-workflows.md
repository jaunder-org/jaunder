# #1889 — Simplify gate workflows

## Outcome

Replace Rust-owned gate orchestration with a small root `Justfile` as the
default one-command workflow interface. Just recipes invoke the pinned tools and
existing validation producers directly; Nix supplies the pinned Just/tool
runtime where needed, not a second workflow definition. Keep only the focused
Git/index safety logic that cannot be expressed as a safe, clear recipe. The
goal is less maintained machinery and fewer duplicated definitions—not moving
the Rust catalogs into another language.

## Approach and tradeoffs

The repository already has pinned `pkgs.just` in `nix/dev-shells.nix` and a root
`Justfile` with a `build-csr` recipe. Extend that entrypoint with ordinary
check, hooks, local validation and CI-lane workflows. Just recipes invoke direct
tools and callable leaf checks; use a Just dependency for ordinary success-path
ordering. Keep existing Rust-only check algorithms in Rust and expose only the
leaf invocation boundary Just needs—no population selection, workflow ordering,
or gate-wide reporting in that interface. Nix derivations remain leaves and
never invoke host-only xtask. A short single-process shebang recipe or helper is
allowed when necessary or genuinely simpler; do not build a framework or
translate Rust orchestration into shell/Nix.

Remove the Rust host gate catalogs and orchestration/result machinery in
`xtask/src/gate.rs` and gate-only paths in `dispatch.rs`, `cli.rs`,
`steps/static_checks.rs`, and lifecycle/result code. Preserve Rust-only check
functions and expose only a minimal single-leaf CLI boundary for checks Just
cannot invoke directly. It runs one named leaf and returns its process result—no
gate population, ordering, or report envelope. Preserve unrelated xtask commands
and dispatch (census, wasm audit, E2E maintenance, PR operations, and others).
Retain narrowly scoped Git/index classification, snapshot,
staging-reconciliation and shared-docs admission/post-check logic only where
required to prove the existing safety contract. Prefer `just` in hooks,
developer instructions and CI; retire old gate aliases unless a concrete caller
must retain one, rather than forwarding every legacy command by default.
Coordinate cutover of external agent gate selectors and their policy through the
Manager before changing those callers; do not modify external policies here.

Keep one recipe entrypoint per user workflow, not a second command inventory in
CI. CI may invoke the Just lane recipe from its existing job topology;
independent CI job results remain authoritative, while `last-result`/step
sidecars are diagnostic only. Preserve `e2e-lanes.json` as lane authority. Under
#1890's approved design-v2 interface, default full synchronous `just e2e` owns
the matrix, reconciliation and authoritative server-function coverage; its exit
status is authoritative. CI may call its minimal producer/reconcile recipes;
there is no shared result protocol or Nix-packaged workflow. Use this interface
only after #1890 lands. Until then, use currently landed entrypoints
independently—there is no implementation dependency or peer import. Coverage,
doctest and Elisp producer-consumer paths retain their owning entrypoints,
artifacts, and internal setup, ordering, cleanup and reconciliation. #1890 E2E
and #1891 coverage retain their independent workflows and checkout ownership. No
new cross-checkout dependency, global serialization, or import of unlanded
sibling work.

Preserve actual check populations and verdicts: host/static checks,
`static-docs` and `static-code`, backend parity, coverage/CRAP, wasm, doctests,
Elisp, E2E reconciliation and server-function flow coverage. Keep Fix behavior
for the interactive fixer workflow and Verify behavior for push/validation
workflows. Retain necessary producer ordering and cleanup. Remove decorative
per-step result envelopes, JSON summaries, mirrored ordering catalogs and
exhaustive-after-failure work where they are not consumed as an independent
required verdict. A failed recipe must return nonzero; required independent CI
lanes still run/reconcile according to their existing topology.

For precommit, classify the pre-run state conservatively: only the existing
staged-Markdown-only class takes the narrow route; uncertainty/unsupported state
takes the broad host route. Preserve the proven safety boundary: never stage
pre-existing untracked files or delete/rename state; re-stage only safe
formatter changes to initially staged tracked paths; fail closed on unsafe
reconciliation. A focused single-process owner reconciles exactly once after the
selected host gate, even on failure, preserves the original gate failure, and
returns failure if either gate or reconciliation fails. Only after successful
narrow-route checks and reconciliation may docs admission and
lock-write-disabled `static-docs` run on x86_64-linux. Always perform the docs
post-request state check; preserve the Nix failure if that check also fails. The
broad route remains host-only and does not run docs admission. Prepush retains
its dirty-tree refusal and verify-only behavior. No gate may infer success from
output or accept missing producer evidence.

### Why this is simpler

Today `gate.rs` maintains an ordered `HostGateStep` catalog, per-step metadata
and fail-fast/exhaustive policy; `dispatch.rs` and `cli.rs` sequence
check/precommit/prepush/validate/CI; result/lifecycle code adds envelopes and
snapshots; `static_checks.rs` maintains phases, commands, mode-specific
arguments and labels. CI separately calls lane commands while ADR-0203 describes
a shared validation catalog. The proposal deletes this Rust
workflow/catalog/result stack and directs users to one existing Justfile instead
of recreating the same catalog as Nix-built scripts plus Rust compatibility
dispatch. It also removes the #1887 docs-only `docsDevtoolBin`
source/vendor/build pipeline and consumer while sharing its three Prettier
definitions for Markdown, `end2end`, and CSS between host and hermetic callers.
Other devtool check definitions (including compiling checks, ast-grep, and
Leptos formatting) remain with devtool.

The measurable criterion is fewer maintained workflow definitions and less
gate-specific orchestration across Rust, CI, and Nix—not a line-count target.
Keep the existing one-command experience and gate verdicts; simplify incidental
diagnostic/result machinery rather than translating it.

### Approved tradeoffs

1. Retire `cargo xtask check/precommit/prepush/validate` and `ci-validate` as
   gate interfaces in favor of `just` recipes. Update hooks and repository docs;
   coordinate external agent selector/policy cutover through the Manager before
   removing the commands those selectors invoke. Do not retain aliases by
   default.
2. Gate recipes fail fast rather than preserve `check`/`validate`'s exhaustive
   diagnostic continuation after a failure. This may report fewer unrelated
   failures per local run; independent CI lanes and their results remain
   authoritative.
3. Retire the gate-wide structured result envelope, named per-step JSON/report
   contract and `last-result` gate summaries with the removed catalog, while
   retaining producer-owned artifacts and logs. CI job results are the verdict;
   sidecars are diagnostic only. Preserve an individual diagnostic field only at
   its owning boundary if a current consumer needs it.

## Acceptance

- A diff shows the Rust gate catalog/orchestration and redundant caller
  inventories are removed, not wrapped; ordinary workflows have one concise Just
  definition, and Nix adds only pinned runtime inputs needed to use it.
- Focused tests prove uncertain classification chooses broad routing; only a
  successful narrow Markdown route reaches docs admission; safe fixes affect
  eligible staged paths; pre-existing untracked/delete/rename state stays
  unstaged; unsafe reconciliation fails closed; and host failure still triggers
  exactly one reconciliation, preserves the failure and remains nonzero. Test
  the docs post-request check after both successful and failed requests.
- Host and hermetic Prettier callers share the #1887 definitions, and
  `docsDevtoolBin` plus its exclusive docs build/source plumbing are gone.
  Existing leaf verdicts, producer evidence, Fix/Verify behavior and CI
  aggregates remain authoritative.
- Validate the focused recipes and their safety seams first, then required
  hooks/CI. Use hermetic/full validation only for the corresponding confidence
  question, not as routine diagnostics.

## ADRs and boundaries

Accepted ADR Decisions are immutable (ADR-0127). Implementation must add one
narrow numberless superseding ADR draft and update `docs/ARCHITECTURE.md`; do
not edit accepted Decisions in place. Reconcile only changed ownership:
ADR-0028's host gate orchestration/result role and ADR-0203's shared local/CI
validation composition, while preserving xtask's host-only boundary, the four CI
lanes/required aggregates, `e2e-lanes.json`, and E2E producer/consumer topology.
ADR-0029's hook and Git/index guarantees remain; revise only superseded
orchestration ownership. ADR-0052/0146/0161/0178 remain for devtool-owned checks
except the three Prettier definitions moved by #1887 and the exclusive docs
pipeline removed there. Preserve ADR-0110's fail-closed population policy where
applicable. Do not create unrelated ADRs.

Combined #1889 spec approval includes #1887's Prettier component; it is not a
separate approval prerequisite. At actual component implementation start,
reconcile #1887 ownership and claim it In Progress if still unowned under the
existing Manager authorization; request a Manager decision only if execution
ownership remains unresolved. #1884–1886 remain with their owners; #1890/#1891
remain independent; #1898 remains the later retirement endpoint. The cross-owner
migration risk warranted the approved outline at
`docs/superpowers/plans/2026-11-05-issue-1889-gate-workflows.md`. Spec approval:
Manager relay `1d21c0df-6e73-4c89-a40e-ddf5b67d92ba`, `issue1889-spec-v2`,
“Approve revised design”. Outline approval: Manager relay
`38808dc8-5ac2-422f-a9fe-3b3843758f43`, `issue1889-outline-v1`, “Approve outline
and begin”. These approvals authorize preparation and the implementation cycle;
no source implementation is included in this preparatory change.
