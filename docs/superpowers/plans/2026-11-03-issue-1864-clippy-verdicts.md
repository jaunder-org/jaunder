# #1864 six-unit Clippy implementation outline

> Triggered by human approval of the six-unit design; this outline itself
> requires approval before implementation. Human provenance: Manager request
> `2f0929a1-ebbd-4735-9a6c-b53e99a7a141` / `issue1864-clippy-v2-coverage`, exact
> answer “Approve six; design cached xtask”.

## Execution summary

Implement only the approved Clippy spec at
`docs/superpowers/specs/2026-11-03-issue-1864-clippy-verdicts.md`. Replace the
five historical Code Clippy invocations with exactly six named lint units plus
one narrow aggregate dependency; preserve all selected targets, features,
warning policy, root/tools lock and test contexts, existing consumers and
residual non-Clippy verdicts. Keep required host-only `xtask-clippy` distinct.
The explicitly approved removal is the former combined root Web(server+CSR)
configuration; no further coverage reduction is authorized. The cached raw xtask
design is separately authorized but is not a seventh implementation unit or part
of this scope. Do not implement it here. No source implementation or proof
execution begins until this outline is approved.

## Task outline

- [ ] **Task 1 — Define the six host Clippy units and preserve catalog
      semantics.** In `tools/devtool/src/check.rs`, replace the old five Code
      commands exactly once with `product-core-clippy`, `web-server-clippy`,
      `web-no-server-clippy`, `client-host-clippy`, `wasm-clippy`, and
      `tools-clippy`; preserve the selected commands/features/targets and
      `-D warnings` exactly as specified. Retain the full seven-package tools
      workspace unless effective performance lint/config parity is established
      and a scope decision authorizes a change. Keep `xtask-clippy` required and
      separate. Update `xtask/src/steps/static_checks.rs` host mapping to match,
      keeping all six units and the host xtask check represented once, and keep
      `--all` identities/order correct. Preserve unrelated catalog entries and
      their relative order; update only names whose selected population changes.
  - **Proof:** focused mock/unit tests pin all six command metadata,
    target/config selections, default targets, and catalog mapping. Verify
    meaningful selected lint targets are distinguished from packages compiled
    only as dependencies. Assert every one of the six occurs exactly once, all
    five old Code invocations are gone, host `xtask-clippy` remains, and
    unrelated catalog entries/order remain unchanged.

- [ ] **Task 2 — Add terminal Nix units, narrow aggregate, and required
      consumers.** In `nix/checks.nix`, define six independently buildable
      terminal Clippy derivations using the approved root/tools lock contexts
      and existing toolchains, target std, offline/vendor homes and input
      conventions. Preserve distinct root versus tools lock/test contexts
      (including the intentional `csr_bundle` overlap); preserve `performance`
      in tools absent established parity and explicit scope approval.
      Replace—not supplement—the five old Code executions; retain all non-Clippy
      Code checks. Add a narrow aggregate depending on all six outputs, with no
      lint rerun and no six serial build launches by a consumer. Wire the static
      consumer in `xtask/src/steps/nix.rs` to request that aggregate once, while
      preserving the existing required flake-check and residual Code
      composition. Nix must never invoke the xtask CLI. Update the applicable
      maintained contributor/architecture documentation and target-name
      references; no new generic framework or ADR by default.
  - **Proof:** metadata/config unit tests assert all six derivations and the
    aggregate dependency set; consumers reference each required verdict once and
    retain residual checks. Controlled leaf failures and aggregate failure
    propagation, plus valid reuse, must be demonstrated without forced
    rebuilds/cache purges. Check relevant source/config/tool identity,
    root/tools artifact and offline/vendor contexts, and unrelated-input
    retention only where the complete transitive closure permits. Account for
    broad tools/product and transitive CSS coupling honestly; do not claim false
    isolation. Prove transitive dependency aggregation rather than serial
    requests.

- [ ] **Task 3 — Integrate focused evidence and maintained docs.** Reconcile
      documentation to the exact final names, six-unit matrix, approved
      combined-config removal, separate host xtask check, and preserved coverage
      qualifications. Keep timing semantics unchanged: report aggregate Nix
      request duration separately from per-unit execution; do not infer per-unit
      runtime from aggregate duration or stale logs. Preserve current
      correlated-stream semantics/diagnostics and report current execution
      unavailable absent positive current-request attribution; do not add a
      timing protocol. Keep proof artifacts ignored and record exact references,
      revision and a fresh checkout inventory.
  - **Proof:** inspect the integrated diff against the approved spec/outline,
    verify all six dependencies/no duplicate old five, complete host mapping,
    target/catalog invariants, failure/reuse behavior and docs names. No blanket
    full E2E or broad gate is required as worker proof.

## Coordination and boundaries

No source implementation, build, gate, Nix/Cargo lint, metadata campaign,
staging, commit, tracker/ref/worktree operation is authorized by this outline
alone. Other slices have separate owners; do not duplicate them. After outline
approval and implementation checkout admission, independent work need not wait
for TypeScript/ERT completion; coordinate only exact unit conflicts and rebase
actual conflicts. Keep one writer per overlapping checkout. Follow the project’s
normal focused validation and Lead integration/acceptance process.

## Delivery and acceptance

Lead reviews the intended integrated diff and focused proofs, then follows
normal gates and hook-backed delivery: inspect formatter/hook mutations, approve
the exact intended tree, stage only after review, then commit/push and use the
normal PR/CI handoff. Commit the approved spec, approved outline, implementation
and maintained documentation. Include ignored proof references, delivered
revision and a fresh staged/unstaged/untracked inventory. No blanket full-E2E or
worker broad-gate requirement is introduced.
