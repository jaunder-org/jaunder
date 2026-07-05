# Coverage source-drift probe (#241) — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Spec:**
[`docs/superpowers/specs/2026-07-04-issue-241-coverage-source-drift-probe.md`](../specs/2026-07-04-issue-241-coverage-source-drift-probe.md)
— the "what/why". This plan is the "how"; it does not restate the spec.

**Goal:** Add an on-demand `cargo xtask coverage probe-source` that fails loudly
if the Nix `coverage` derivation's `src` filter drifts (admits junk → #37
impurity, or drops source → coverage hole), and run it in CI.

**Architecture:** A pure `probe_verdict(base, junk, rs)` encodes the two filter
invariants over three `coverage.drvPath` strings. An impure orchestrator
measures those three drvPaths in an ephemeral, git-ignored worktree, **staging**
each probe file with `git add` (per the spec's empirically-verified visibility
model: nix ignores untracked _new_ files even on a dirty tree). An RAII guard
removes the worktree on every exit path. A CI step in the existing
`validate-no-e2e` job runs it.

**Tech Stack:** Rust (xtask, clap, anyhow), `nix eval --raw`, `git worktree`,
GitHub Actions.

## Global Constraints

- **Probe-file paths (verbatim):** junk = `probe.txt` (repo root); source =
  `server/src/__drift_probe.rs`. Both confirmed non-gitignored; the first is
  filter-excluded, the second `filterCargoSources`-admitted.
- **Inclusion mechanism:** new probe files MUST be `git add`-ed (staged) to be
  visible to nix — dirtying alone is insufficient (spec "Load-bearing
  subtlety"). Edits to already-tracked files need no staging, but the probe only
  adds _new_ files.
- **Eval target:** `.#checks.x86_64-linux.coverage.drvPath`,
  `nix eval --raw --accept-flake-config`, cwd = the flake dir (so `.#` resolves
  that worktree's git state). System string is `x86_64-linux` (the private
  `SYSTEM` const in `steps/nix.rs`).
- **Ephemeral worktree:** under `.xtask/` (git-ignored, `.gitignore:39`);
  created with `git worktree add --detach <tmp> HEAD`; removed via an RAII
  `Drop` guard (`git worktree remove --force`).
- **Git invocations:** use `crate::git::at(dir)` (scrubs
  `GIT_DIR`/`GIT_INDEX_FILE`/… — the hook env hazard) and disable hooks with
  `-c core.hooksPath=` (defensive: `worktree add` can fire a `post-checkout`
  hook).
- **Not per-commit:** do NOT add the probe to `Command::Check` or
  `Command::Validate` dispatch.
- **Commits:** run the per-commit gate (`cargo xtask check`) clean before each
  commit (**jaunder-commit**). **No `Co-Authored-By` trailer.**

---

## Review header

**Scope — in:** the `probe-source` subcommand + pure verdict + nix-eval helper +
CI step + docs. **Out:** any change to the filter's behavior; per-commit wiring;
#246. **Separable concerns:** none surfaced (spec's out-of-scope items are
already separate issues/deferrals) — so no issue-filing first task.

**Tasks:**

1. Pure `probe_verdict` + `DriftError` (unit-tested) — spec AC4.
2. `eval_coverage_drvpath` helper + `WorktreeGuard` + `probe_source`
   orchestrator + `coverage probe-source` CLI wiring; verify AC1/AC2/AC3/AC5/AC7
   by running it.
3. CI step in `validate-no-e2e` (`if: always()`) — AC6.
4. Docs: on-demand probe + CI role — AC8.

**Key risks/decisions:** the staging mechanism is load-bearing and empirically
verified (spec appendix) — Task 2's AC2/AC3 demonstrations re-prove the guard
actually fires. The orchestrator is I/O (nix + git worktree); its unit-testable
core is the Task 1 verdict, and its integration is verified by running the real
subcommand (there is no cheap unit test for the shell-out, and the plan says so
rather than faking one).

---

### Task 1: Pure `probe_verdict` + `DriftError`

**Files:**

- Create: `xtask/src/coverage/probe.rs`
- Modify: `xtask/src/coverage/mod.rs` (add `pub mod probe;` alongside the
  existing `pub mod crap; …`)
- Test: in-file `#[cfg(test)]` in `probe.rs` (xtask convention — `result.rs`,
  `git.rs` use in-file tests)

**Interfaces:**

- Consumes: nothing.
- Produces:
  - `pub enum DriftError { AdmitsJunk { base: String, junk: String }, DropsSource { base: String } }`
    — `#[derive(Debug, Clone, PartialEq, Eq)]`, with `Display` +
    `std::error::Error` impls (so the orchestrator can `anyhow::Error::from`
    it).
  - `pub fn probe_verdict(base: &str, junk: &str, rs: &str) -> Result<(), DriftError>`

- [x] **Step 1: Write the failing tests** (one per branch)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_when_junk_excluded_and_source_measured() {
        assert_eq!(probe_verdict("d-base", "d-base", "d-rs"), Ok(()));
    }

    #[test]
    fn admits_junk_when_junk_moves_drvpath() {
        assert_eq!(
            probe_verdict("d-base", "d-JUNKMOVED", "d-rs"),
            Err(DriftError::AdmitsJunk { base: "d-base".into(), junk: "d-JUNKMOVED".into() })
        );
    }

    #[test]
    fn drops_source_when_rs_does_not_move_drvpath() {
        assert_eq!(
            probe_verdict("d-base", "d-base", "d-base"),
            Err(DriftError::DropsSource { base: "d-base".into() })
        );
    }

    #[test]
    fn admits_junk_takes_precedence_over_drops_source() {
        // Both broken: junk moved AND rs == base. Junk (impurity) is checked first.
        assert_eq!(
            probe_verdict("d-base", "d-JUNKMOVED", "d-base"),
            Err(DriftError::AdmitsJunk { base: "d-base".into(), junk: "d-JUNKMOVED".into() })
        );
    }

    #[test]
    fn display_names_the_broken_invariant() {
        let j = DriftError::AdmitsJunk { base: "b".into(), junk: "j".into() };
        assert!(j.to_string().contains("admits") && j.to_string().contains("junk"));
        let s = DriftError::DropsSource { base: "b".into() };
        assert!(s.to_string().contains("drops") && s.to_string().contains("source"));
    }
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p xtask coverage::probe` Expected: FAIL —
`probe_verdict` / `DriftError` not defined.

- [x] **Step 3: Implement against the tests**

Every branch is pinned by a test (both-hold, junk-moved, rs-static, precedence,
Display). To signature
`probe_verdict(base: &str, junk: &str, rs: &str) -> Result<(), DriftError>`:
check `junk != base` first → `AdmitsJunk`; then `rs == base` → `DropsSource`;
else `Ok(())`. `DriftError`'s `Display` must contain "admits"/"junk" and
"drops"/"source" respectively (Step-5 tests + the orchestrator's `StepResult`
detail depend on it). Module doc-comment: what the probe guards and why (link
#241/#37).

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p xtask coverage::probe` Expected: PASS (5 tests).

- [x] **Step 5: Commit**

```bash
git add xtask/src/coverage/probe.rs xtask/src/coverage/mod.rs
git commit -m "feat(coverage): pure probe_verdict for source-filter drift (#241)"
```

Run `cargo xtask check` first so the commit gate passes clean
(**jaunder-commit**).

---

### Task 2: nix-eval helper, worktree guard, orchestrator, and CLI wiring

**Files:**

- Modify: `xtask/src/steps/nix.rs` (add `eval_coverage_drvpath`; add
  `use std::path::Path;` and `use anyhow::{bail, Context, Result};` if not
  present)
- Modify: `xtask/src/coverage/probe.rs` (add `WorktreeGuard` + `probe_source` +
  `run_probe`)
- Modify: `xtask/src/lib.rs` (`CoverageCommand` enum;
  `Coverage(CoverageCommand)` arm on `Command`; dispatch arm; **`command_name`
  arm** — that match is exhaustive)

**Interfaces:**

- Consumes: `probe_verdict`, `DriftError` (Task 1); `crate::git::at`
  (`xtask/src/git.rs:18`); `StepResult::{ok,fail,detail}`
  (`xtask/src/result.rs:21`); the dispatch pattern at `xtask/src/lib.rs:309-328`
  (`Adr` arms) and `finalize`.
- Produces:
  - `pub fn crate::steps::nix::eval_coverage_drvpath(flake_dir: &Path) -> anyhow::Result<String>`
  - `pub fn crate::coverage::probe::probe_source() -> StepResult` (label
    `"coverage-probe-source"`)
  - `Command::Coverage(CoverageCommand::ProbeSource)` → runnable as
    `cargo xtask coverage probe-source`.

- [x] **Step 1: Add the nix-eval helper** in `xtask/src/steps/nix.rs`

```rust
/// Evaluate the coverage check's `.drvPath` for the flake rooted at `flake_dir`.
/// cwd = `flake_dir` so `.#` resolves *that* worktree's git state (staged/dirty),
/// matching the `.#`-ref semantics the source-drift probe (#241) relies on.
pub fn eval_coverage_drvpath(flake_dir: &Path) -> Result<String> {
    let attr = format!(".#checks.{SYSTEM}.coverage.drvPath");
    let out = Command::new("nix")
        .current_dir(flake_dir)
        .args(["eval", "--raw", "--accept-flake-config", &attr])
        .output()
        .context("spawning `nix eval` for coverage.drvPath")?;
    if !out.status.success() {
        bail!("`nix eval {attr}` failed:\n{}", String::from_utf8_lossy(&out.stderr));
    }
    let path = String::from_utf8(out.stdout)
        .context("`nix eval` output was not UTF-8")?
        .trim()
        .to_owned();
    if path.is_empty() {
        bail!("`nix eval {attr}` returned an empty path");
    }
    Ok(path)
}
```

- [x] **Step 2: Add `WorktreeGuard` + orchestrator** in
      `xtask/src/coverage/probe.rs`

Uses (add to the file's imports): `std::fs`, `std::path::{Path, PathBuf}`,
`anyhow::{Context, Result}`,
`crate::{git, result::StepResult, steps::nix::eval_coverage_drvpath}`.

```rust
/// Removes the ephemeral probe worktree on every exit path (return, error, panic).
struct WorktreeGuard {
    repo_root: PathBuf,
    path: PathBuf,
}
impl Drop for WorktreeGuard {
    fn drop(&mut self) {
        let _ = git::at(&self.repo_root)
            .args(["-c", "core.hooksPath="])
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .status();
    }
}

/// Run a git subcommand in `dir` with hooks disabled; bail on non-zero exit.
fn git_run(dir: &Path, args: &[&str]) -> Result<()> {
    let ok = git::at(dir)
        .args(["-c", "core.hooksPath="])
        .args(args)
        .status()
        .with_context(|| format!("running git {args:?} in {}", dir.display()))?
        .success();
    if !ok {
        anyhow::bail!("git {args:?} failed in {}", dir.display());
    }
    Ok(())
}

/// The user-facing step: measure the three drvPaths and apply [`probe_verdict`].
pub fn probe_source() -> StepResult {
    match run_probe() {
        Ok(()) => StepResult::ok("coverage-probe-source")
            .detail("coverage src filter contract holds (junk excluded, source measured)"),
        Err(e) => StepResult::fail("coverage-probe-source").detail(format!("{e:#}")),
    }
}

fn run_probe() -> Result<()> {
    let repo_root = std::env::current_dir().context("resolving cwd")?;
    let tmp = repo_root.join(".xtask/coverage-probe.worktree");
    fs::create_dir_all(repo_root.join(".xtask")).context("creating .xtask")?;
    // Clear any leftover from a prior crash (ignore failure: usually nothing there).
    let _ = git::at(&repo_root)
        .args(["-c", "core.hooksPath=", "worktree", "remove", "--force"])
        .arg(&tmp)
        .status();

    git_run(&repo_root, &["worktree", "add", "--detach",
                          tmp.to_str().context("tmp path not UTF-8")?, "HEAD"])?;
    let _guard = WorktreeGuard { repo_root: repo_root.clone(), path: tmp.clone() };

    // State A: clean HEAD.
    let base = eval_coverage_drvpath(&tmp)?;

    // State B: staged junk (filter-excluded) → expect unchanged.
    fs::write(tmp.join("probe.txt"), b"").context("writing probe.txt")?;
    git_run(&tmp, &["add", "probe.txt"])?;
    let junk = eval_coverage_drvpath(&tmp)?;
    git_run(&tmp, &["rm", "--cached", "--quiet", "probe.txt"])?;
    fs::remove_file(tmp.join("probe.txt")).context("removing probe.txt")?;

    // State C: staged .rs under an instrumented dir → expect changed.
    let rs_rel = "server/src/__drift_probe.rs";
    fs::write(tmp.join(rs_rel), b"// coverage source-drift probe (#241)\n")
        .context("writing probe .rs")?;
    git_run(&tmp, &["add", rs_rel])?;
    let rs = eval_coverage_drvpath(&tmp)?;

    probe_verdict(&base, &junk, &rs)?; // DriftError: Error → anyhow via `?`
    Ok(())
}
```

(`probe_verdict(&base, &junk, &rs)?` relies on `DriftError: std::error::Error`
from Task 1.)

- [x] **Step 3: Wire the CLI** in `xtask/src/lib.rs`

Add the nested enum (mirroring `AdrCommand`, `lib.rs:124`):

```rust
/// `coverage` subcommands.
#[derive(Subcommand)]
pub enum CoverageCommand {
    /// Guard the Nix coverage derivation's source filter against silent drift:
    /// assert an excluded file leaves `coverage.drvPath` unchanged and an
    /// instrumented `.rs` changes it. Eval-only; runs in CI / on request, NOT in
    /// per-commit `check`/`validate` (#241).
    #[command(after_help = "EXAMPLES:\n  cargo xtask coverage probe-source")]
    ProbeSource,
}
```

Add to `Command` (near the `Adr`/`Traces` arms, `lib.rs:111-116`):

```rust
    /// Coverage tooling (the source-filter drift probe; #241).
    #[command(subcommand)]
    Coverage(CoverageCommand),
```

Add the dispatch arm (pattern from `lib.rs:309-315`):

```rust
        Command::Coverage(CoverageCommand::ProbeSource) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("coverage-probe-source");
            result.push(coverage::probe::probe_source());
            finalize(&mut result, start);
            Ok(result)
        }
```

Add the `command_name` arm (the `match self.command` at `lib.rs:200` is
**exhaustive — no `_`**, so this is required to compile):

```rust
            Command::Coverage(CoverageCommand::ProbeSource) => "coverage-probe-source",
```

(No `produces_json_payload` change: it uses `!matches!(…Traces…)`, so `Coverage`
defaults to `true` — correct, the probe emits a normal `StepResult` envelope.
The `match cli.command` sites in tests all have `_ =>` catch-alls, so only
`command_name` needs the new arm.)

- [x] **Step 4: Build + run the real subcommand (AC1)**

Run: `cargo xtask coverage probe-source` Expected: exits 0; human output
`[ ok ] coverage-probe-source — …contract holds…`.

- [x] **Step 5: Verify no real-tree mutation (AC5) and not-per-commit (AC7)**

Run: `git -C <this worktree> status --porcelain` → unchanged (no `probe.txt`, no
`__drift_probe.rs`); `git worktree list` shows no
`.xtask/coverage-probe.worktree`. Confirm AC7 by inspection: the
`Check`/`Validate` dispatch arms (`lib.rs:239-283`) do not call
`coverage::probe`.

- [x] **Step 6: Commit**

```bash
git add xtask/src/steps/nix.rs xtask/src/coverage/probe.rs xtask/src/lib.rs
git commit -m "feat(coverage): cargo xtask coverage probe-source drift guard (#241)"
```

Run `cargo xtask check` first (**jaunder-commit**) — the crate must compile
clean (incl. the new `command_name` arm).

- [x] **Step 7: Demonstrate the guard fires (AC2 + AC3)** — throwaway
      **committed** filter edits, then reset.

The probe evals an ephemeral `git worktree add --detach … HEAD` checkout, so it
measures the **committed (HEAD)** filter — an _uncommitted_ `flake.nix` edit in
this worktree is not visible to it (it would falsely pass). The demo therefore
commits each throwaway edit (with `--no-verify` to skip the slow gate hook) and
resets after. Run this only now, with the tree clean after Step 6, so
`reset --hard` is safe.

```bash
# AC2 — broadening: filter admits probe.txt
#   edit flake.nix coverage src filter, adding: || (pkgs.lib.hasSuffix "probe.txt" path)
git commit -am "TEMP drift demo: admit junk" --no-verify
cargo xtask coverage probe-source    # Expected: exit 1, detail names admits-junk / impurity
git reset --hard HEAD~1

# AC3 — narrowing: filter drops the probe .rs
#   edit flake.nix, adding to the exclusion chain: && !(pkgs.lib.hasInfix "__drift_probe" path)
git commit -am "TEMP drift demo: drop source" --no-verify
cargo xtask coverage probe-source    # Expected: exit 1, detail names drops-source / coverage-hole
git reset --hard HEAD~1
```

Confirm `git log --oneline -1` is back at the Step-6 commit and `git status` is
clean. No code change results from this step — it is pure verification.

---

### Task 3: CI wiring

**Files:**

- Modify: `.github/workflows/ci.yml` (the `validate-no-e2e` job, `ci.yml:9-53`)

**Interfaces:**

- Consumes: `cargo xtask coverage probe-source` (Task 2).
- Produces: a CI step that fails a PR on drift.

- [x] **Step 1: Add the step** after "Validate (static + clippy + coverage, via
      xtask)" (`ci.yml:40-41`), before the diagnostics upload:

```yaml
- name: Coverage source-drift probe (#241)
  if: always()
  run:
    nix develop .#ci --accept-flake-config -c cargo xtask coverage probe-source
```

Rationale inline-comment: `if: always()` so a drift signal isn't masked by an
unrelated `validate` failure; placed here to reuse the job's nix + cachix +
xtask cache (eval-only, ~seconds).

- [x] **Step 2: Validate the workflow YAML**

Run: `cargo xtask check` (its `prettier`/static steps format & check
`.github/**/*.yml`) — Expected: PASS, `ci.yml` unchanged by formatting (or
auto-formatted, then re-inspect the diff). Inspect: the step sits inside
`validate-no-e2e`, not the `e2e` matrix.

- [x] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci(coverage): run source-drift probe in validate-no-e2e (#241)"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

### Task 4: Documentation

**Files:**

- Modify: `CONTRIBUTING.md` (the coverage-policy / `nix flake check` section
  that #231 touched)

**Interfaces:**

- Consumes: the shipped subcommand + CI step.
- Produces: contributor-facing description of the probe and its CI role.

- [x] **Step 1: Document the probe**

Add a short paragraph under the coverage section: what
`cargo xtask coverage probe-source` guards (the `src` filter's two invariants —
excluded files must not move `coverage.drvPath`, instrumented `.rs` must), the
**staging** subtlety (new files must be `git add`-ed to be measured — link the
spec's load-bearing note), that it evaluates the **committed (HEAD)** filter in
an ephemeral worktree (so it guards what CI/PRs carry, not local uncommitted
edits), that it runs in CI (`validate-no-e2e`) and on request, and that it is
deliberately **not** in per-commit `check`/`validate`.

- [x] **Step 2: Verify docs formatting**

Run: `cargo xtask check` — Expected: PASS (prettier covers Markdown; ADR/readme
parity unaffected).

- [x] **Step 3: Commit**

```bash
git add CONTRIBUTING.md
git commit -m "docs(coverage): document the source-drift probe and its CI role (#241)"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

## Self-review

- **Spec coverage:** AC1 → T2/S4; AC2 → T2/S7; AC3 → T2/S7; AC4 → T1; AC5 →
  T2/S5; AC6 → T3; AC7 → T2/S5 (by construction); AC8 → T4. All eight mapped.
- **Placeholders:** none — every step carries real Rust/YAML and exact commands.
- **Type consistency:** `probe_verdict`/`DriftError` (T1) consumed unchanged in
  T2; `eval_coverage_drvpath(&Path) -> Result<String>` defined and consumed in
  T2; `probe_source() -> StepResult` label `"coverage-probe-source"` matches the
  `CommandResult::new` label and the dispatch arm; `git::at` signature matches
  `git.rs:18`.
