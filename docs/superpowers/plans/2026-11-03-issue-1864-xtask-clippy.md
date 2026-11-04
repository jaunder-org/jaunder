# #1864 cached raw xtask Clippy implementation outline

> Status: human-approved for implementation. Design approval: request
> `642deee8-e711-4467-9382-f6925a3135f4` / `issue1864-xtask-clippy-v1`, exact
> answer “Approve cached xtask spec”. Outline approval: request
> `aec9cc61-7d0c-448b-b7ab-14f62060e1ef` / `issue1864-xtask-clippy-outline-v1`,
> exact answer “Approve outline and begin”. The six-unit Clippy spec and outline
> have separate approvals; this outline does not alter or implement them.
> Proceed in the admitted checkout within the approved scope. The Nix derivation
> runs raw Clippy without invoking xtask or executing tests; normal host
> lifecycle execution remains authorized. Merge requires separate approval.

## Execution summary

Implement only `docs/superpowers/specs/2026-11-03-issue-1864-xtask-clippy.md`.
Add one cached raw xtask Clippy derivation, exposed through `nix flake check`
and requested once by the existing Linux hermetic validation lane alongside—but
not inside—the approved six-unit aggregate. Retain the existing required host
`xtask-clippy` check unchanged. Inside the Nix derivation, do not invoke the
xtask CLI or execute tests. Normal host hooks, focused tests, supported xtask
validation/consumer proof, and PR observation remain authorized as applicable.
Approval is recorded above.

## Task outline

- [ ] **Task 1 — Adapt the existing xtask package derivation for raw all-target
      Clippy.** Reuse `nix/xtask.nix`'s xtask package closure,
      `xtask/Cargo.lock`, pinned-fork vendor, native dependencies, and existing
      Nix build identity environment (`JAUNDER_XTASK_BUILD_NIX_OUTPUT`).
      Preserve the Git-free Nix identity branch in `xtask/build.rs`; do not
      claim equivalence with host Git provenance. Add or verify source admission
      needed by the exact command
      `cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings`,
      especially all-target compile-time includes and test-source fixtures such
      as `.github/workflows/ci.yml`, `.gitignore`, and xtask JSON/JSONL
      testdata. Include other required manifest, Cargo configuration, toolchain,
      and local path-dependency inputs as established by a bounded compile-input
      trace. Keep the new derivation cache boundary separate from `static-code`;
      do not create new vendor/native/offline frameworks.
  - **Proof:** focused source-admission checks cover every compile-time input
    used by selected targets. Relevant source/config/tool changes invalidate the
    unit; unrelated-input stability is assessed only against the complete
    transitive closure. Confirm the Nix identity path works without checkout Git
    qualification. Run raw Clippy only: Cargo may compile targets/build
    scripts/proc macros, but tests and the xtask CLI must not execute.
    Demonstrate unchanged valid local-store reuse without forced rebuild or
    cache purge.

- [ ] **Task 2 — Wire one required Nix verdict and reconcile the boundary.**
      Expose the raw unit in the flake checks and require it once through the
      existing Linux hermetic validation consumer, alongside and not within the
      six-unit aggregate. Reuse current static-check collection and
      failure-reporting conventions. Do not duplicate build requests or add an
      aggregate API solely for scheduling. Make the narrow ADR-0146
      check/source-boundary reconciliation needed to describe this independently
      cacheable raw Cargo lint; preserve ADR-0028's host execution boundary (no
      xtask CLI in derivations). Keep the host check required and unchanged. Do
      not change hooks, CI topology, the six-unit aggregate/matrix, devtool's
      check catalog, or in-progress #1859 aggregate APIs.
  - **Proof:** assert the flake and Linux validation consumer request the unit
    once and alongside—not inside—the six-unit aggregate. A controlled lint
    failure fails both the leaf and designated consumer. Restore/discard the
    fixture and confirm clean inputs yield successful leaf and consumer results.
    Preserve honest request-duration, store-reuse, and current correlated
    execution observations; historical logs or derivation identity alone do not
    prove current execution.

## Coordination and boundaries

This is a separate implementation slice from the six-unit Clippy work. The
six-unit human approval is request `2f0929a1-ebbd-4735-9a6c-b53e99a7a141` /
`issue1864-clippy-v2-coverage`, exact answer “Approve six; design cached xtask”;
it includes the approved combined-Web configuration removal and preserves
root/tools lock contexts and the performance qualification. Do not redo or
duplicate that implementation. Other family slices remain independent;
coordinate actual overlapping writers only. Manager reported the separate
six-unit implementation assignment startup is not confirmed and owns its
recovery; do not take over that assignment here.

**Historical pre-approval restriction (superseded by the approvals recorded
above):** before approval, no source implementation, Nix/Cargo build, Clippy,
test, gate, metadata campaign, formatter, tracker/ref/worktree operation,
staging, or commit was authorized. With approval, proceed with the outlined
implementation and normal host lifecycle; the Nix derivation alone must not
invoke the xtask CLI or execute tests. Keep proof artifacts ignored. Proceed
only in the admitted implementation checkout.

## Delivery and acceptance

Lead reviews the integrated diff and focused proof, then follows normal gate and
hook-backed delivery: inspect hook/formatter mutations, approve the exact
intended tree, stage only after review, and commit/push through the normal PR/CI
handoff. Deliver the approved spec, outline, implementation, and any narrowly
required ADR/documentation updates; include ignored proof references, delivered
revision, and fresh staged/unstaged/untracked inventory. No blanket full-E2E or
broad worker gate is introduced.
