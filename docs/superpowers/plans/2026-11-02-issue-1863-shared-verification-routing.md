# Issue #1863 outline — shared Markdown verdict

**Replacement outline.** Supersedes the earlier unapproved
resolved-source/full-inventory plan; do not implement that machinery.

## Scope

On `x86_64-linux`, existing narrow staged-Markdown precommit runs host Fix and
its distinct checks, performs its existing final safe restaging once, then
requests `static-docs` only on success. Clean prepush replaces only host
Markdown Verify. CI retains its existing hermetic request and aggregate. Non-x86
behavior and broad precommit stay unchanged.

## Ordered tasks

1. Add the leaf request after existing precommit snapshot/planning/application.
   Run it only when host checks, reconciliation and the focused admission pass.
   Preserve one reconciliation on every path; host or stage failure skips Nix,
   leaf failure fails the hook without restaging again.
2. Use existing Git primitives to require no residual unstaged or nonignored
   untracked state in precommit. Compare raw worktree blob OIDs with index OIDs
   for every tracked `staticDocsSrc` input (Markdown plus any-depth
   `.prettierrc.json`/`.prettierignore`, excluding `.claude`), including clean
   files. Apply the same identity check to clean prepush. After Nix, compare Git
   status and raw cached-diff state, then recheck relevant raw/index identity;
   report mutation/query failure without restaging, preserving any leaf failure.
   Keep this bounded: no index reconstruction, resolved-source override,
   persisted inventory, cache, or general framework. Use existing `build_check`
   and explicit no-lock-update/no-lock-write flags.
3. Replace only host `prettier-markdown` Verify in clean x86_64 prepush at its
   current order position. Preserve clean-tree refusal, all other checks/phases
   and fail-fast behavior. Keep existing non-x86 and broad-precommit behavior.
4. Focused tests: ordering/platform fallback; host/stage/admission skip with one
   reconciliation; leaf failure after reconciliation; clean and changed
   Markdown/config raw-index identity including conversion refusals; post-leaf
   tracked/config/lock mutation and Git-query failures; unchanged `flake.lock`.
   Reuse #1862 evidence for existing leaf-level harness controls.
5. CI: verify actual `ci-validate hermetic` failure propagation, the exact
   required aggregate's rejection of a controlled failed-hermetic result, and
   hosted required CI success on the exact ref. No workflow duplicate.
6. Narrow ADR-0029 supplement and, after implementation, update CONTRIBUTING,
   the pilot page and architecture verification section. No #1862 ordering
   change or other ADR/CI topology amendment.

## Main risk

The bounded assumption is cooperative single-writer execution: after successful
safe restaging, every tracked Markdown/config byte selected by the Git-backed
leaf matches its index blob, and no later precommit host hook work follows the
leaf. The focused Git-flake proof must confirm actual behavior; accepted `path:`
harness evidence alone is insufficient. If correspondence cannot be established
with these existing bounded Git primitives, stop and return to design rather
than add snapshot infrastructure.
