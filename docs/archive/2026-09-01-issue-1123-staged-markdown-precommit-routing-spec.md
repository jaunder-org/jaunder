# #1123 — Conservative staged-Markdown precommit routing

Issue: [#1123](https://github.com/jaunder-org/jaunder/issues/1123). Milestone:
Developer tooling & DX.

## Outcome

`cargo xtask precommit` recognizes an isolated staged-Markdown change and runs a
fixed Markdown-sensitive host surface instead of the broad host gate. Every
uncertain, mixed, or non-Markdown state falls back to the existing broad gate.
`prepush`, `check`, `validate`, and CI remain broad.

## Load-bearing decisions

- Routing applies only to `precommit`. Its input is the complete pre-run Git
  state, not a branch diff, merge-base inference, cached receipt, or post-run
  state.
- The narrow class is `staged-markdown-only`. It requires a nonempty dirty tree
  in which every dirty path is a staged-only, regular, case-sensitive `.md`
  addition or modification. There may be no unstaged, untracked,
  deleted/renamed, type-changing, non-Markdown, malformed, unparseable, or
  otherwise unknown entry.
- Git status collection and parsing must preserve the complete path/status
  population needed by that predicate. NUL-delimited input or an equivalently
  unambiguous representation is required. Quoted, non-UTF-8, malformed, or
  unsupported records never get guessed into the narrow class; they select the
  broad fallback.
- Classification is conservative routing, not validation. An inability to prove
  the narrow predicate selects the broad gate with a reason; it does not bypass
  the gate or turn a valid broad-gate invocation into a classification failure.
- The narrow surface is fixed and ordered like the corresponding broad host
  steps: Prettier, sequence/identifier collision checks, all three ADR checks,
  documentation links, flow-document parity, and the error-swallowing inventory.
  These are the complete existing host checks whose outcome or fix behavior can
  change when only tracked Markdown differs.
- The fixed surface is intentionally not further routed by Markdown path.
  Prettier owns the global `end2end` plus `**/*.md` fix scope; ADRs project into
  `docs/README.md` and `docs/ARCHITECTURE.md`; documentation links and flow
  documents consume repository-wide relationships. The isolated-tree predicate
  makes these global reads and formatter writes represent the staged Markdown
  state.
- `precommit-routing` is an informational successful `StepResult` emitted before
  gate work. Its detail reports the selected class and reason for both the
  narrow and broad routes, so human output and the JSON sidecar expose the same
  decision without a new result schema.
- Narrow-surface execution retains precommit's fail-fast policy. Work outside
  the selected surface is absent, not represented as successful skipped steps.
- Precommit always takes its after-snapshot and performs the existing
  conservative staging reconciliation, including after a narrow-surface failure.
  Safe formatter mutations to staged-clean Markdown remain eligible for
  restaging; unsafe or ambiguous mutations still fail closed.
- `xtask/tools-only` routing is explicitly rejected. Those workspaces implement
  gate definitions and repository scanners whose changes must be exercised
  against their full product, documentation, CI, and e2e input populations; a
  path-only shortcut would skip the behavior being changed.
- ADR-0029 gains a #1123 supplement recording the narrow route and broad
  fallback. `docs/ARCHITECTURE.md` and `CONTRIBUTING.md` project the same
  policy. No new ADR is required because this refines the accepted Git-hook
  gate.

## Acceptance

- A staged-only regular `.md` addition and a staged-only regular `.md`
  modification each select `staged-markdown-only` and execute exactly the fixed
  Markdown-sensitive surface in existing host-gate order.
- Empty staged state; any staged non-Markdown path; mixed Markdown and
  non-Markdown paths; uppercase or unusual Markdown extensions; deletions;
  renames; type changes; symlinks; unstaged changes; untracked paths; mixed
  staged/unstaged paths; malformed status; and unsupported or non-UTF-8 paths
  each select the broad host gate with a stable diagnostic reason.
- Narrow and broad decisions both emit `precommit-routing` before gate work in
  human output and `.xtask/last-result.json`.
- A synthetic failure in the narrow surface stops later narrow checks while
  precommit staging reconciliation still runs exactly once.
- A Prettier mutation to an initially staged-clean Markdown path is safely
  restaged. Existing mixed-state, user-unstaged, delete/rename, and untracked
  reconciliation behavior remains unchanged.
- Production command-graph tests consume the same classifier and fixed surface
  used by `precommit`; no shadow list defines the tested route.
- A representative broad classification emits `precommit-routing` first and then
  executes the unchanged complete precommit host gate in Fix mode.
- `prepush`, `check`, and `validate` command-graph tests prove their existing
  broad surfaces and execution policies are unchanged.
- Focused xtask tests and `cargo xtask check --no-test` pass.

## Boundaries

- No routing for `prepush`, `check`, `validate`, CI, product Rust, web/e2e,
  `xtask`, `tools`, or any non-Markdown class.
- No dependency graph, crate inference, branch-base inference, receipt cache,
  parallel execution, retry policy, timeout policy, or reordering of the broad
  host gate.
- No change to the membership of the broad precommit, prepush, check, validate,
  or CI surfaces.
- No weakening of fail-fast hooks, clean-tree prepush, staged-subset ownership,
  formatter restaging, or fail-closed Git/index safety.
- No path-specific Markdown routing and no new top-level command-result payload.
