# Issue #185 — Format Markdown with prettier (flag-day + check/validate gate)

## Goal

Bring tracked `*.md` under **prettier** for consistent formatting, gated in the
host verify ladder: auto-fix under `cargo xtask check`, verify-only under
`cargo xtask validate`. This **extends the existing host `prettier` StepSpec**
(`xtask/src/steps/static_checks.rs`, currently `end2end/` only) to markdown —
not a new tool.

## Settled config decisions (from the issue)

- **`proseWrap: always`.** Prettier owns line-wrapping; authors/agents write
  prose freely and `check` re-wraps on commit. Chosen over `preserve` after a
  documented preview (the reflow is a one-time reconciliation, not recurring
  instability — after the flag day the tree is in prettier's canonical wrap and
  an edit reflows only its own paragraph).
- **`printWidth` omitted.** Prettier's default is already 80, and the project's
  standing rule is "don't reinforce defaults." The effective wrap column is
  locked by the flake.lock pin on `pkgs.prettier` (a prettier bump = flag-day
  #2), so the default needs no separate regression lock. Config is
  `{ "proseWrap": "always" }` only.
- **Emphasis marker `_`, forced.** Prettier hardcodes `_italic_` / `**bold**`
  (not configurable). The flag day converts every `*italic*` → `_italic_`
  tree-wide; `_` becomes house style.

## Architecture

### 1. Config files (new, repo root)

`.prettierrc.json`:

```json
{
  "proseWrap": "always"
}
```

This root config also applies to the pre-existing `end2end/` run — safe:
`proseWrap` affects only markdown, and `printWidth` stays at prettier's default
80 (already the default for the `end2end/` TS/CSS).

`.prettierignore`:

```
docs/archive/
target/
result
result-*
.claude/
```

- `docs/archive/` — ~130 of the 211 tracked `.md` are frozen, write-once
  specs/plans. Excluding them keeps the flag-day diff to the ~80 living docs and
  skips pointless reflow of historical record.
- `target/`, `result`, `result-*` — non-hidden build/output dirs that a
  `**/*.md` glob would otherwise descend into.
- `.claude/` — defensive: `**/*.md` (fast-glob, `dot:false`) does not match
  `.`-prefixed dirs, but running prettier from the **main** repo would otherwise
  reach the full checkouts under `.claude/worktrees/`. `node_modules` is ignored
  by prettier automatically.

### 2. The gate — extend the host prettier StepSpec

`xtask/src/steps/static_checks.rs` (currently `--check end2end` / `-w end2end`):

```rust
let prettier_args = match mode {
    Mode::Check => vec!["--check", "end2end", "**/*.md"],
    Mode::Fix   => vec!["-w", "end2end", "**/*.md"],
};
```

Prettier globs `**/*.md` internally (a literal argv, no shell — same pattern as
leptosfmt's `**/*.rs`); `.prettierignore` scopes it. This is the **real,
merge-blocking gate**: CI runs it via `cargo xtask validate --no-e2e`
(`.github/workflows/ci.yml:41`), and the pre-commit hook auto-fixes in `check`
(Fix mode → `-w`).

**Version pin — already satisfied.** Prettier comes from `pkgs.prettier` on both
host (devShell `ciInputs`) and nix, pinned by flake.lock. No new pinning work;
the lock _is_ the pin.

**`prettier-check` nix derivation — deliberately untouched.** `flake.nix`'s
`prettier-check` (checks `end2endSrc`) is one of a class of static-check nix
`*-check` siblings (`rustfmt`, `clippy`, `leptosfmt-check`, `ert-check`,
`elisp-fmt-check`, `deny`) reachable only via `nix flake check` — which neither
CI nor the steered `cargo xtask` workflow runs. Each duplicates a host StepSpec
running the identical tool, so the class is redundant with
`cargo xtask validate`. Extending `prettier-check` to markdown would add surface
to a mechanism whose existence is in question. Its fate (and ADR-0031's
convention) is routed to a separate issue (see Separable concerns). This is
distinct from the load-bearing nix derivations xtask _does_ orchestrate —
`coverage`, `e2e-*`, `elisp-integration` — which require the
sandbox/VM/instrumentation and cannot be host StepSpecs.

### 3. Coverage denylist (flake.nix:1177-1183)

Add a suffix exclusion to the coverage-source filter:

```nix
&& !(pkgs.lib.hasSuffix ".md" path)
```

Currently only `/docs/` is denied, so root-level `.md` (`README.md`,
`CONTEXT.md`, `CONTRIBUTING.md`) sit inside the coverage source and bust the
coverage cache on every edit. Denying `.md` decouples all markdown from the
coverage hash — which is what makes a `validate`-time md gate net-positive
rather than a coverage-rebuild tax on every doc edit. Coverage **numbers** are
unaffected: markdown is never instrumented; it was only in the src as filter
residue.

## Flag-day sequencing

Three commits, ordered so each passes its own per-commit gate and no coverage
rebuild is wasted:

1. **Plumbing** — add `.prettierrc.json` + `.prettierignore`; add the `.md`
   coverage denylist to `flake.nix`. (The flake.nix change rebuilds coverage
   once regardless; landing the denylist here means the flag-day reformat that
   follows does not bust it.)
2. **Flag day** — `prettier --write` over the tree: a **pure reformat** commit,
   reviewable as such (`_` emphasis + `always` reflow across the ~80 living
   docs).
3. **Gate** — widen the host prettier StepSpec to `**/*.md`; update
   `CONTRIBUTING.md:106` to note markdown is covered.

## Testing

Tooling/formatting work — the "test" is the gate itself:

- After the flag day, `cargo xtask validate --no-e2e` passes (prettier `--check`
  clean tree).
- A deliberately-misformatted `.md` makes `cargo xtask validate --no-e2e`
  **fail** (manual negative check) — proving the gate actually gates.

No unit tests; acceptance is the green gate plus the negative check.

## Separable concerns (plan task 1)

File a GitHub issue: **"Remove redundant static-check nix `*-check` siblings
(`prettier-check`, `rustfmt`, `clippy`, `leptosfmt-check`, `ert-check`,
`elisp-fmt-check`, `deny`) — overlap with `cargo xtask validate`;
amend/supersede ADR-0031."** Rationale: these siblings are exercised only by
`nix flake check`, which is not part of the enforced/steered flow, so they
duplicate host StepSpecs. This is broader than markdown and touches a documented
ADR + ~7 derivations, so it gets its own cycle rather than being folded into
#185.

## Acceptance

- Explicit prettier config committed: `.prettierrc.json` =
  `{ "proseWrap": "always" }`; `.prettierignore` covering `docs/archive/`,
  build/output dirs, and `.claude/`.
- All tracked, non-ignored `*.md` are prettier-clean.
- Host prettier StepSpec auto-formats md under `cargo xtask check` and verifies
  under `cargo xtask validate` (fails on unformatted); confirmed by a negative
  check.
- `.md` excluded from the coverage source denylist (doc edits no longer bust the
  coverage cache).
- Prettier version pin confirmed (flake.lock `pkgs.prettier`; no new work).
- Separable static-sibling cleanup filed as its own issue.

## Notes / non-goals

- `CLAUDE.md`, `AGENTS.md`, `end2end/CLAUDE.md` are currently **untracked** →
  out of scope until tracked.
- Timing: the flag-day diff touches every living `.md` → will conflict with
  in-flight branches (#172, #173). Expect a rebase; the reformat is
  deterministic so re-running `prettier --write` after a rebase reconciles
  cleanly.
