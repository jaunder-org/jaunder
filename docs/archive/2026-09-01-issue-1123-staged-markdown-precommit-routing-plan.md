# #1123 — Staged-Markdown precommit routing implementation outline

Spec:
[`docs/archive/2026-09-01-issue-1123-staged-markdown-precommit-routing-spec.md`](2026-09-01-issue-1123-staged-markdown-precommit-routing-spec.md)

## Risk trigger

This work changes Git-status interpretation and the precommit command graph. A
small outline is required so classification, execution, and staging
reconciliation share one contract rather than growing parallel policy tables.

## Contracts

- `git` owns an unambiguous pre-run snapshot built from NUL-delimited porcelain
  status plus NUL-delimited cached raw-diff records. The latter supplies the
  staged status and index mode needed to distinguish regular blobs
  (`100644`/`100755`) from symlinks, gitlinks, and unsupported modes. The two
  populations are cross-checked; missing, malformed, conflicting, or non-UTF-8
  evidence remains explicit uncertainty.
- The code contract is
  `git::classify_precommit_change(&GitStatusSnapshot) -> PrecommitChangeClass`,
  where `PrecommitChangeClass` is `StagedMarkdownOnly` or
  `Broad(PrecommitBroadReason)`. `PrecommitBroadReason` has stable variants,
  evaluated in this order: uncertain status, empty state, untracked path,
  unstaged path, delete-or-rename, unsupported change, unsupported index mode,
  and non-Markdown path.
- `PrecommitChangeClass` owns the routing detail encoding:
  `class=staged-markdown-only reason=isolated-staged-markdown` or
  `class=broad reason=<stable-kebab-case-reason>`. Code and documentation use
  those terms rather than inventing parallel labels.
- Unsupported records cannot disappear from post-gate staging safety:
  classification routes broadly and staging reconciliation fails closed if the
  before/after snapshot cannot represent the complete mutation population.
- Every production host/static check carries its Markdown-route eligibility with
  its existing runner/spec definition. Broad and narrow precommit both traverse
  the same ordered production catalogs; the narrow route filters that catalog
  metadata. No second runner list or name-matching policy table is permitted.
- `precommit-routing` is pushed before gate execution. Both routes use the
  existing fail-fast runner; post-gate snapshot and staging reconciliation run
  once regardless of gate outcome.
- Documentation projects the code-owned class, reasons, and eligibility policy.
  It does not define an additional path or check table.

## Implementation slices

### 1. Classifier and precommit command graph

Own `xtask/src/git.rs`, `xtask/src/lib.rs`, `xtask/src/steps/static_checks.rs`
if eligibility must reach an individual static spec, and their
unit/orchestration tests.

- Make status collection/path parsing unambiguous for whitespace and rename-like
  path text. Combine NUL-delimited full status with cached raw-diff
  modes/statuses, cross-check their staged populations, and retain explicit
  uncertainty.
- Preserve existing `GitStatusSnapshot` and `precommit_stage_plan` behavior for
  safe restaging; fail closed if a record cannot be represented.
- Implement the specified classifier API, reason precedence, and detail
  encoding. Admit only a nonempty snapshot whose complete dirty population
  consists of staged-only regular case-sensitive `.md` additions/modifications.
- Mark the corresponding production checks as Markdown-sensitive: Prettier,
  sequence-check, the ADR bundle, doc-links, flow-docs, and
  error-swallowing-inventory. Select the narrow graph as an ordered filter over
  the same broad host/static catalogs.
- Refactor `run_precommit_with_host_gate` to classify the before-snapshot, emit
  `precommit-routing`, dispatch the filtered or complete Fix-mode host graph,
  then perform the unchanged after-snapshot/staging plan exactly once.
- Keep fail-fast semantics within either selected route. Do not synthesize
  skipped steps.
- Test additions, modifications, empty state, non-Markdown and mixed paths,
  uppercase extensions, deletes/renames, type changes, symlinks/gitlinks,
  unstaged, untracked, mixed index/worktree state, whitespace paths, malformed
  and conflicting records, and unsupported/non-UTF-8 input.
- Add production-graph tests proving the narrow graph is the required ordered
  subsequence of the broad graph, broad precommit remains complete, routing
  detail is stable, an early narrow failure still reconciles once, and
  prepush/check/validate graphs remain unchanged.
- Extend the orchestration fixture so a narrow-route Markdown formatter mutation
  is safely restaged.

### 2. Policy projection

Own `docs/adr/0029-git-enforced-verify-gate.md`, `docs/ARCHITECTURE.md`, and
`CONTRIBUTING.md`.

- Record the isolated staged-Markdown predicate, fixed Markdown surface,
  informational routing result, global formatter/projection rationale, and broad
  fallback.
- State that prepush/check/validate/CI remain broad and that xtask/tools-only
  routing was rejected because those workspaces define and exercise repository
  scanners.
- Keep terminology and command authority consistent across all three documents.

## Integration order

1. Develop the classifier/command-graph slice and policy-projection slice in
   parallel against the exact class, reason, detail, and eligibility contracts
   above. They own disjoint files.
2. Integrate code first, then reconcile documentation wording against the
   production names without changing the contract.
3. Run formatting once across touched files.

## Verification

1. Focused xtask unit/orchestration tests covering classification, route
   membership/order, fail-fast reconciliation, and safe Markdown restaging:
   `devtool run -- cargo xtask test-local -- --manifest-path xtask/Cargo.toml`.
2. Static and compile checks after the focused suite:
   `devtool run -- cargo xtask check --no-test`.
3. Stage the intended tree and run the commit gate through the normal commit
   path; inspect the parked result and routing step. Because the staged tree
   then includes Rust and documentation, this must select the broad route.
4. After commit, exercise the actual narrow surface in a disposable temporary
   repository fixture or equivalent command-level smoke test with one isolated
   staged Markdown modification; observe `precommit-routing` select
   `staged-markdown-only`, the fixed step sequence, and safe restaging.
5. Run the clean-tree push gate before opening the pull request:
   `devtool run -- cargo xtask prepush`.
