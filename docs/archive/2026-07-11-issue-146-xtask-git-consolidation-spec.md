# Spec — #146: consolidate xtask git plumbing into one typed module

**Issue:** [#146](https://github.com/jaunder-org/jaunder/issues/146) **Status:**
proposed **Depends on:** nothing (all prerequisite surfaces already merged).

## Problem — and how the issue's premise has drifted

#146 was written against an older xtask git surface. Half its enumerated targets
no longer exist; the goal (one typed `git` module, no reinvented wrappers, no
duplicated env-scrubbing) still stands, but only against the _current_ code:

| #146 claim                                                                    | Reality on this branch's base                                                                 |
| ----------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `coverage::scrubbed_git` is a verbatim copy of `git::at`                      | **Gone.** `coverage/probe.rs` already routes through `git::at`.                               |
| `lib.rs` merge-driver helpers (`register_keepours`, `ensure_merge_driver`, …) | **Gone entirely** — zero occurrences in `xtask/src`.                                          |
| `adr.rs` `git_out`/`git_lines` reinvent exit-code handling                    | **Still true** — private to `adr.rs`, owns the grep exit-1/128 logic, `mv`, `add`.            |
| a raw `Command::new("git")` bypasses `git::at`                                | **Still true** — exactly one: `coverage/mod.rs::git_repo_root` (`rev-parse --show-toplevel`). |

xtask's git access currently splits into two families, and the goal is to
collapse them into **one** `git::at`-based module:

- **cwd + `xshell`** — `git::working_tree_status`, `hooks_path`,
  `ensure_hooks_path`. Called from `lib.rs` with a `Shell`, operate on the
  current directory, part of the gate's self-healing. These build the `git`
  command via `xshell::cmd!` — **without** env-scrubbing.
- **`-C <dir>` + `Command` via `git::at`** — the env-scrubbed, repo-targeted,
  unit-tested plumbing used by `adr.rs`, `coverage/probe.rs`, and
  `coverage/mod.rs`.

Three costs across the two families: (1) env-scrubbing is centralized in
`git::at` **except** for the raw `git_repo_root` Command _and_ the entire
`xshell` cwd family; (2) the plumbing wrappers (trimmed-stdout capture,
non-empty-line splitting, the grep no-match-vs-error distinction) live privately
in `adr.rs` rather than the shared module; and (3) there are three distinct ways
to invoke git (`at`+`Command`, `xshell::cmd!`, and each site's bespoke wrapper).

**The unscrubbed `xshell` family is a latent hazard, not just an
inconsistency.** The gate runs from the pre-push hook (the git-enforced gate),
which exports `GIT_DIR`/`GIT_INDEX_FILE`; an unscrubbed
`git config core.hooksPath` / `git status` then targets whatever repo the hook's
env points at rather than the working tree — precisely the failure mode
`git::at`'s `env_remove` was written to prevent. Routing these onto `git::at`
closes that gap as a side effect of the consolidation.

Two more git-via-`xshell` calls hide outside `git.rs`: `steps/build_csr.rs:18`
and `steps/e2e_local.rs:63` each run `cmd!(sh, "git rev-parse --show-toplevel")`
— the same toplevel lookup the new `toplevel` helper provides, open-coded and
unscrubbed.

This spec therefore migrates **every** git invocation in `xtask/src` onto the
one `git::at`-based module, with the sole exception of two intentional
fire-and-forget cleanup calls in `coverage/probe.rs` (documented below) that
already use `git::at` directly.

## Decision — no git library (re-affirmed)

Stay with shell-out; consolidate the wrappers. `git2`/`gix` were considered and
rejected in the issue body (C dependency awkward in the Nix-pure build; `gix`
porcelain immature). This spec does not revisit that; the rationale already
lives in the issue and no new ADR is warranted (mechanical consolidation of an
existing pattern, no novel architectural choice).

## Approach

Grow `xtask/src/git.rs` into the single home for **all** xtask git access, and
**drop the `xshell`/`Shell`/`cmd!` dependency from the module entirely**. All
helpers take `dir: &Path` (matching `adr.rs`'s existing testable shape) and
build on `git::at(dir)`. The pure predicates (`porcelain_is_dirty`,
`needs_hooks_path`) and `HOOKS_PATH` stay as-is.

**Primitives** (lifted from `adr.rs::git_out`/`git_lines`, bool dropped):

- `pub(crate) fn output(dir: &Path, args: &[&str]) -> Result<String>` — trimmed
  stdout; bail on any non-zero exit with `git {args:?} failed: {stderr}`.
- `pub(crate) fn lines(dir: &Path, args: &[&str]) -> Result<Vec<String>>` —
  `output` split on newlines, blank lines dropped.
- `pub(crate) fn run(dir: &Path, args: &[&str]) -> Result<()>` — run for effect,
  no capture; bail on non-zero. Absorbs the run-and-check shape reinvented in
  `coverage/probe.rs::git_run` and used by the config mutation below.

**Typed conveniences** — mostly thin wrappers over the primitives, one per
repeated call-site shape (`grep_files` and `toplevel` build on `git::at`
directly, since they need behavior `output` can't give — see their notes):

- `merge_base(dir, a, b) -> Result<String>`
- `diff_names(dir, range) -> Result<Vec<String>>` — `diff --name-only <range>`
  (all touched files).
- `diff_added(dir, range, pathspec) -> Result<Vec<String>>` —
  `diff --diff-filter=A --name-only <range> -- <pathspec>`.
- `grep_files(dir, pattern) -> Result<Vec<String>>` —
  `grep -l --fixed-strings <pattern>`, encapsulating grep's exit-code contract:
  **exit 1 = no match → `Ok(vec![])`; exit 128 (or any other non-zero) = real
  error → `Err`**. This is the only helper that tolerates a non-zero exit,
  replacing the `allow_no_match` bool that `git_out` threaded through every
  call. (Because it tolerates exit 1, it wraps `git::at` directly rather than
  `output`, which bails on any non-zero.)
- `mv(dir, from, to) -> Result<()>`
- `add(dir, path) -> Result<()>`
- `toplevel(dir) -> Result<String>` — `rev-parse --show-toplevel`, replacing the
  raw-`Command` `git_repo_root`. Note this **adds** env-scrubbing the original
  lacked (`git_repo_root` ran bare, full env; `toplevel` goes through `git::at`,
  so `-C dir` + `env_remove`) — strictly more correct in a hook context, and
  behavior-neutral for the normal `cargo xtask` invocation.
- `config_get(dir, key) -> Result<Option<String>>` — `config --get <key>`;
  tolerates exit 1 (key unset → `Ok(None)`), bails on other non-zero, maps a
  blank value to `None`. Wraps `git::at` directly (like `grep_files`). Replaces
  the `xshell` body of `hooks_path`. **Intentional divergence:** the old
  `hooks_path` did `.read().ok()`, swallowing _any_ error (incl. a
  corrupt-config exit 128) to `None`, which made `ensure_hooks_path` silently
  re-point `core.hooksPath`. `config_get` instead bails on exit 128, so a
  genuinely broken config now surfaces as an `Err` from `ensure_hooks_path`
  rather than a silent rewrite — fail-fast-precisely, consistent with the repo's
  gate ethos. The unset (exit 1) and set paths — the only ones that occur in
  practice — are unchanged.
- `config_set(dir, key, value) -> Result<()>` — `config <key> <value>` via
  `run`. Replaces the `xshell` mutation in `ensure_hooks_path`.

**Migrations:**

- `git.rs` cwd family: rewrite `working_tree_status`, `hooks_path`, and
  `ensure_hooks_path` onto the new helpers —
  `output(dir, &["status", "--porcelain"])`,
  `config_get(dir, "core.hooksPath")`, and `config_set`/`config_get`
  respectively. They take `dir: &Path` instead of a `Shell`. Drop
  `use xshell::…` from `git.rs`.
- `lib.rs`: update the two call sites (l.452 `ensure_hooks_path`, l.476
  `working_tree_status`) to pass the repo dir (`Path::new(".")`) instead of
  `&sh`. `lib.rs` keeps its `Shell` for its other uses.
- `adr.rs`: delete `git_out`/`git_lines`; route `run_renumber`/`run_promote`
  onto the new helpers (`merge_base`, `diff_added`, `diff_names`, `grep_files`,
  `mv`, `add`). Its existing integration tests (`renumber_*`, `promote_*`) stay
  green unchanged.
- `coverage/mod.rs`: delete `git_repo_root`; call
  `git::toplevel(Path::new("."))`.
- `steps/build_csr.rs` and `steps/e2e_local.rs`: replace each
  `cmd!(sh, "git rev-parse --show-toplevel").quiet().read()` with
  `git::toplevel(Path::new("."))` (both already `.trim()` and fall back to a
  `StepResult::fail` on error — preserve that `else`/`Err` handling). Their
  non-git `xshell` usage (the `cargo` invocations, `sh.change_dir`) stays.
- `coverage/probe.rs::git_run`: reduce to a thin call to `git::run` with the
  probe-specific `-c core.hooksPath=` hook-disable prefixed at the call
  (`git::run(dir, &["-c", "core.hooksPath=", …])`), removing the reinvented
  bail-on-non-zero logic while keeping the hook-disable local.
- `coverage/probe.rs` fire-and-forget cleanup calls (the two
  `let _ = git::at(…) .status()` sites: the `WorktreeGuard::drop` and the
  pre-run leftover-remove): **left as direct `git::at` calls.** They
  intentionally ignore failure, so the bail-on-non-zero `run` is the wrong
  shape; using the shared constructor directly is not reinvented duplication.

## Acceptance criteria

Each is observable — a reviewer can confirm delivered-vs-not from the diff and a
green gate.

1. **One way to invoke git.** Three greps, all observable:
   - `rg 'Command::new\("git"\)' xtask/src` → exactly one hit, the constructor
     body inside `git::at`.
   - `rg 'cmd!\([^)]*"git ' xtask/src` → **zero** hits: no git is invoked via
     `xshell` anywhere (the four current sites — `git.rs` ×3, `steps/*` ×2 minus
     overlap — are all gone).
   - `rg '\bxshell\b' xtask/src/git.rs` → no match: `git.rs` no longer depends
     on `xshell`.

   Together these establish that every git invocation in `xtask/src` flows
   through `git::at`, whether via a helper or (for the two documented
   fire-and-forget probe cleanups) directly.

2. **`adr.rs` owns no git wrapper.** `git_out` and `git_lines` no longer exist
   in `adr.rs` (`rg 'fn git_out|fn git_lines' xtask/src/adr.rs` → no match); all
   its git calls go through `crate::git::*` helpers.
3. **The grep no-match contract is encapsulated once.** No `allow_no_match`
   parameter survives anywhere in `xtask/src`; `git::grep_files` is the sole
   site distinguishing grep exit-1 (no match → empty) from exit-128 (error →
   bail), and has a direct unit test asserting both branches.
4. **Typed helpers exist and are used.** `git.rs` exposes `output`, `lines`,
   `run`, `merge_base`, `diff_names`, `diff_added`, `grep_files`, `mv`, `add`,
   `toplevel`, `config_get`, `config_set`; every one has at least one real call
   site (no dead API).
5. **Behavior is preserved.** The pre-existing adr integration tests
   (`renumber_bumps_newcomer_and_rewrites_refs`,
   `renumber_syncs_the_readme_table`,
   `renumber_assigns_distinct_numbers_to_multiple_newcomers`, all `promote_*`)
   pass unmodified, proving the migration is behavior-neutral.
6. **New git-module logic is covered.** The lifted primitives and typed helpers
   have host-runnable unit tests (real git against temp repos, as `adr.rs` tests
   already do) sufficient to satisfy the coverage gate — no new `cov:ignore`
   markers introduced for this code.
7. **The cwd family is scrubbed and behavior-preserving on the real paths.**
   `working_tree_status`, `hooks_path`, `ensure_hooks_path` now route through
   `git::at` (scrubbed); their observable results on the paths that occur in
   practice are unchanged — `hooks_path` still returns `None` when
   `core.hooksPath` is unset and `Some(trimmed)` when set; `ensure_hooks_path`
   still returns `true`/`false` on change/no-change. The **one** intended
   divergence (a corrupt-config exit 128 now `Err`s instead of being swallowed
   to `None`) is documented under `config_get`. A unit test covers the
   unset→`None` and set→`Some` paths of `config_get`.
8. **Gate green.** `cargo xtask check` passes (static + clippy + coverage),
   exercising the adr renumber/promote integration tests and the hooks-path
   self-healing.

## Out of scope

- Any change to `coverage/probe.rs` **behavior** (the worktree-probe flow is
  refactored onto `git::run` but stays behavior-identical).
- Migrating xtask's **non-git** `xshell` usage (sh.rs's runner, the `cargo`
  invocations and `sh.change_dir` in steps/\*). Only git invocations move; the
  `cmd!(sh, "cargo …")` calls and the `Shell` itself stay.
- Adopting a git library.
- New git operations not already performed by an existing call site.
