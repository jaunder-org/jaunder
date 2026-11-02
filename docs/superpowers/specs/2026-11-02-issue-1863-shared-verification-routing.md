# Issue #1863 design — share the Markdown verification verdict

**Replacement approval draft.** This concise proposal supersedes the earlier
unapproved resolved-source/full-inventory and dual-guard draft in this file's
history; that extra machinery is rejected, not an approved alternative.

## Outcome and scope

On production `x86_64-linux`, the existing narrow staged-Markdown precommit
keeps its host Fix and distinct host checks, then requests the existing
`static-docs` Verify after its existing one-time safe restaging. Clean prepush
replaces only host `prettier-markdown` Verify with that leaf. CI keeps its
current required hermetic leaf and aggregate. No precommit eligibility is
narrowed or deferred.

## Concrete ordering

**Narrow precommit:** existing staged-only regular Markdown admission → host
`prettier-markdown` Fix → distinct `prettier-end2end` Fix → sequence/identifier,
ADR, document-link and flow checks → existing final snapshot and safe stage-plan
application exactly once → admission check → Nix Verify only if every host
check, stage reconciliation and admission check passed.

If a host check fails, still perform today's final reconciliation so safe
formatter fixes are restaged, but skip Nix and return failure. If
restaging/admission fails, skip Nix and return failure. If the leaf fails, it
fails the hook; do not stage again. Host Fix is a write operation, not duplicate
Verify. The broad precommit route remains unchanged.

**Clean prepush:** existing clean-tree refusal first; at the current host
`prettier-markdown` Verify position request the leaf; preserve all other
host/static checks, auxiliary tests, product tests and workspace doctests in
their existing order. A dirty tree prevents the request. Nix failure is the
prepush failure and existing fail-fast stops later work.

**CI:** preserve the current `validate-hermetic` request through
`cargo xtask ci-validate hermetic`, and the required `Validate (no e2e)`
aggregate that requires every lane. Add no second leaf request or alternate pass
signal.

## Admission and request

Use the existing `run_precommit_with_host_gate` lifecycle
(`xtask/src/lifecycle.rs`), narrow classifier and `precommit_stage_plan`
(`xtask/src/git.rs`), and Nix `build_check` (`xtask/src/steps/nix.rs`). After
the single safe restage, require the existing Git/status evidence to show no
unstaged changes and no nonignored untracked files; status/diff query failure
refuses the Nix request. The narrow classifier already limits the starting state
to staged-only regular Markdown additions/modifications. Under the cooperative
single-writer assumption, successful reconciliation makes the admitted worktree
content the intended staged content; do not reconstruct a repository-wide index
or add a second snapshot framework.

Before either the precommit or clean-prepush request, compare raw worktree blob
OIDs with index OIDs for the complete existing tracked input population selected
by `staticDocsSrc`: every Markdown file and every `.prettierrc.json` and
`.prettierignore` at any depth, excluding `.claude`. This catches clean/EOL
conversion even for unchanged files without reconstructing the index or adding
manifests/cache machinery. Precommit additionally retains its staged-only
Markdown admission and refuses remaining unstaged/nonignored-untracked state;
prepush retains its clean-tree refusal. After the request, compare existing Git
status and raw cached-diff evidence, then recheck the static-docs raw/index
identity. Fail on tracked, staged, or nonignored-untracked mutation or query
failure, preserve the original Nix failure, and never restage a second time. Use
the ordinary Git-backed `.#checks.x86_64-linux.static-docs` request through
existing `build_check`, with explicit
`--no-update-lock-file --no-write-lock-file` flags for request/observation. Do
not add resolved-source overrides, reconstructed indexes, saved manifests or
cache/skip state. Confirm tracked `flake.lock` remains unchanged; ordinary Nix
logs, GC roots and other expected ignored `.xtask/` outputs are not source
changes.

The qualification is the repository's cooperative single-writer lifecycle, not
immunity to a concurrent external writer. Nix runs after all host checks and
final restaging, leaving no later precommit hook work that can stale the
verdict.

## Platform and retained checks

The proof/production route is `x86_64-linux`. The flake also declares Linux
checks for `aarch64-linux`, but they are not proven/routed by this change; keep
existing non-x86 behavior: narrow precommit remains host Fix-only and prepush
keeps host Markdown Verify. Darwin behavior also remains unchanged. Do not claim
cross-platform equivalence or broaden proof.

`static-docs` runs `devtool check --group docs` over the existing
`staticDocsSrc`; preserve its Markdown/config selector and docs-tool/lock/tool
closure. Keep `prettier-end2end`, identifier/sequence, ADR, links, flow checks,
all other prepush phases, CI lanes/aggregate and producer-plus-verdict coupling
unchanged. The accepted #1862 harness evidence covers the bounded leaf contract,
input/closure reasoning and modelled failure/reuse; it does **not** by itself
prove the production Git-backed hook request.

## Decisions and acceptance

Amend **ADR-0029 only** with the actual narrow precommit order (host checks →
one final reconciliation → conditional leaf request), prepush Verify replacement
and platform fallback. **No #1862 ordering amendment**: this restores the
approved Fix-and-safe-restage-before-request contract. After implementation,
update `CONTRIBUTING.md`, `docs/shared-verification-pilot.md` and the
verification section of `docs/ARCHITECTURE.md`.
ADR-0028/0052/0141/0146/0178/0192/0203 and CI topology remain unchanged.

Focused acceptance: (1) test ordering and prove host/stage/admission failures
skip Nix while safe reconciliation still occurs once; leaf failure returns hook
failure without a second reconciliation; (2) prove raw/index correspondence for
changed and unchanged Markdown plus formatter config, including clean conversion
refusal before both consumers; prove actual Git-backed `.#` sees the
post-restage intended Markdown; test post-request tracked/config/lock mutation
and query failure while preserving the leaf verdict; reuse #1862 receipts for
prior harness-only controls; (3) exercise actual prepush request/failure and
preserve its other phases; (4) exercise actual `ci-validate hermetic` leaf
failure, a controlled negative test of the exact aggregate predicate, and
required hosted CI success on the exact ref; (5) assert request/observation uses
no-lock-write flags and does not mutate tracked `flake.lock`.

**Approval requested:** approve this integrated consumer scope/order, focused
clean/EOL admission refusal, x86_64-only routing and non-x86 fallback, and
ADR-0029/document updates. This is design only; no implementation, experiment,
push or PR is authorized here. The native #1863→#1862 tracker link remains
context under integrated Manager authority, not a separate
documentation-shipment prerequisite. Human design/outline approval remains the
implementation gate.
