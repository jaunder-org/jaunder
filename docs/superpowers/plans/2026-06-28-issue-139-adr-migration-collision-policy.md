# ADR & Migration Identifier-Collision Policy — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Execution status (2026-06-28): COMPLETE.** Executed inline, committed at
> gate-green deliverable boundaries rather than strictly per plan-task (the
> pre-commit hook stages the whole tree, and `next_number` had no consumer until
> the renumber tool — committing Task 1 alone would have tripped dead-code):
> - Tasks 1–2 (ids helpers + `identifier-collisions` check) → commit `cbe4dc1`.
> - Tasks 3–4 (`git::at`, `adr renumber` + integration test) → commit `0d420ca`.
> - Task 5 (ADR + CONTRIBUTING) → commit `43915d4`. ADR is **0036**, not the
>   plan's placeholder: `origin/main` was at 0033 but 0034/0035 are in flight on
>   other branches.
> All 105 xtask tests pass; `cargo xtask check` is green.

**Goal:** Make concurrent ADR creation collision-loud (a verify-gate check) and collision-cheap (a one-command renumber tool), and make migration number/parity collisions loud, without changing the sequential naming convention.

**Architecture:** Two additions to the `xtask` crate: (1) a build-free static check, run inside `cargo xtask check`/`validate`, that scans the ADR and migration directories for duplicate numeric prefixes and for sqlite/postgres parity; (2) a `cargo xtask adr renumber` subcommand that bumps the branch's newly-added ADR to the next free number and rewrites references, treating the ADR already on `origin/main` as immutable. All logic follows the crate's existing pattern: a pure, unit-tested core (`ids.rs`, pure rewrite helpers) behind a thin I/O wrapper that returns a `StepResult`.

**Tech Stack:** Rust, `xtask` crate (clap, xshell, serde, anyhow), git CLI invoked via `std::process::Command`.

## Global Constraints

- No `Co-Authored-By` trailers in commits (repo policy, overrides global default).
- Per-task gate before each commit: `cargo xtask check --no-test` (static + clippy + fmt + host xtask unit tests; no Nix). Final gate before PR: `cargo xtask validate --no-e2e`.
- Pure logic must be I/O-free and unit-tested in isolation; I/O wrappers produce `StepResult`. Follow the existing `static_checks.rs` / `coverage/*` pattern.
- Git commands that target a specific repo dir must use the env-scrubbed builder (see Task 3) so ambient `GIT_DIR`/etc. (exported when run inside a hook) cannot redirect the operation at another repo.
- Migrations keep sequential `NNNN_slug.sql` naming; ADRs keep `NNNN-slug.md`. Do not introduce timestamps.

---

## File Structure

**Create:**
- `xtask/src/ids.rs` — pure helpers: parse leading number, detect duplicate prefixes, backend parity, next free number.
- `xtask/src/steps/sequence_check.rs` — I/O wrapper: scan the three directories, build the `identifier-collisions` `StepResult`.
- `xtask/src/adr.rs` — pure rewrite helpers + git orchestration for `adr renumber`.
- `docs/adr/00NN-identifier-collision-policy.md` — the ADR recording this decision (number chosen at implementation time).

**Modify:**
- `xtask/src/lib.rs` — declare `mod ids;`, `mod adr;`, `pub mod sequence_check;`; add the `Adr` subcommand + `command_name` arm + dispatch; call `sequence_check::run` in the `Check` and `Validate` arms.
- `xtask/src/git.rs` — add `pub fn at(dir: &Path) -> std::process::Command` (the env-scrubbed builder, promoted from lib.rs's private `git_at`).
- `CONTRIBUTING.md` — short subsection: how to add an ADR, the duplicate/parity check, and the `cargo xtask adr renumber` recovery path.

---

### Task 1: Pure identifier helpers (`ids.rs`)

**Files:**
- Create: `xtask/src/ids.rs`
- Modify: `xtask/src/lib.rs` (add `mod ids;` near the other `mod` lines, e.g. after `mod coverage;`)

**Interfaces:**
- Produces:
  - `ids::leading_number(filename: &str) -> Option<u32>`
  - `ids::duplicate_prefixes(filenames: &[String]) -> Vec<(u32, Vec<String>)>`
  - `ids::parity_mismatch(sqlite: &[String], postgres: &[String]) -> Vec<String>`
  - `ids::next_number(filenames: &[String]) -> u32`

- [ ] **Step 1: Write the failing tests**

Create `xtask/src/ids.rs` with the tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_number_parses_both_separators() {
        assert_eq!(leading_number("0034-foo.md"), Some(34));
        assert_eq!(leading_number("0023_create_x.sql"), Some(23));
        assert_eq!(leading_number("README.md"), None);
        assert_eq!(leading_number("template.md"), None);
    }

    #[test]
    fn duplicate_prefixes_reports_only_collisions_sorted() {
        let files = vec![
            "0034-bar.md".to_string(),
            "0034-foo.md".to_string(),
            "0033-solo.md".to_string(),
            "notes.md".to_string(),
        ];
        let dups = duplicate_prefixes(&files);
        assert_eq!(
            dups,
            vec![(34, vec!["0034-bar.md".to_string(), "0034-foo.md".to_string()])]
        );
    }

    #[test]
    fn duplicate_prefixes_empty_when_unique() {
        let files = vec!["0001_a.sql".to_string(), "0002_b.sql".to_string()];
        assert!(duplicate_prefixes(&files).is_empty());
    }

    #[test]
    fn parity_mismatch_flags_each_side() {
        let sqlite = vec!["0001_a.sql".to_string(), "0002_only_sqlite.sql".to_string()];
        let postgres = vec!["0001_a.sql".to_string(), "0003_only_pg.sql".to_string()];
        let m = parity_mismatch(&sqlite, &postgres);
        assert_eq!(
            m,
            vec![
                "0002_only_sqlite (sqlite only)".to_string(),
                "0003_only_pg (postgres only)".to_string(),
            ]
        );
    }

    #[test]
    fn parity_mismatch_empty_when_identical() {
        let s = vec!["0001_a.sql".to_string()];
        assert!(parity_mismatch(&s, &s).is_empty());
    }

    #[test]
    fn next_number_is_max_plus_one() {
        let files = vec!["0001_a.sql".to_string(), "0007_b.sql".to_string()];
        assert_eq!(next_number(&files), 8);
        assert_eq!(next_number(&[]), 0);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml ids::`
Expected: FAIL — `cannot find function leading_number` etc. (also requires `mod ids;` in lib.rs; add it now so the module is compiled).

- [ ] **Step 3: Implement the pure helpers**

Prepend to `xtask/src/ids.rs` (above the test module):

```rust
//! Pure helpers for sequence-numbered identifier files (ADRs, migrations):
//! parsing the leading number, detecting duplicate prefixes, checking backend
//! parity, and choosing the next free number. No I/O — unit-tested in isolation.

use std::collections::{BTreeMap, BTreeSet};

/// The leading integer of a filename like `0034-foo.md` or `0023_create_x.sql`.
/// `None` when the name does not start with a digit.
pub fn leading_number(filename: &str) -> Option<u32> {
    let digits: String = filename.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Filename without its final extension: `0023_create_x.sql` -> `0023_create_x`.
fn stem(filename: &str) -> &str {
    match filename.rfind('.') {
        Some(i) => &filename[..i],
        None => filename,
    }
}

/// Numbers used by more than one file, each with its sorted filenames. Files
/// without a leading number are ignored. Sorted by number for stable output.
pub fn duplicate_prefixes(filenames: &[String]) -> Vec<(u32, Vec<String>)> {
    let mut by_number: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for name in filenames {
        if let Some(n) = leading_number(name) {
            by_number.entry(n).or_default().push(name.clone());
        }
    }
    by_number
        .into_iter()
        .filter(|(_, names)| names.len() > 1)
        .map(|(n, mut names)| {
            names.sort();
            (n, names)
        })
        .collect()
}

/// Migration stems present in one backend directory but not the other — a
/// backend-parity violation. Returns sorted `"<stem> (<backend> only)"` lines.
pub fn parity_mismatch(sqlite: &[String], postgres: &[String]) -> Vec<String> {
    let set = |names: &[String]| -> BTreeSet<String> {
        names.iter().map(|n| stem(n).to_string()).collect()
    };
    let s = set(sqlite);
    let p = set(postgres);
    let mut out = Vec::new();
    out.extend(s.difference(&p).map(|x| format!("{x} (sqlite only)")));
    out.extend(p.difference(&s).map(|x| format!("{x} (postgres only)")));
    out.sort();
    out
}

/// One greater than the maximum leading number across `filenames`; `0` when none
/// have a number. Monotonic — never reuses a gap left by a deleted file.
pub fn next_number(filenames: &[String]) -> u32 {
    filenames
        .iter()
        .filter_map(|n| leading_number(n))
        .max()
        .map_or(0, |m| m + 1)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml ids::`
Expected: PASS (6 tests).

- [ ] **Step 5: Per-task gate + commit**

Run: `cargo xtask check --no-test`
Expected: all steps `[ ok ]`, `xtask check PASSED`.

```bash
git add xtask/src/ids.rs xtask/src/lib.rs
git commit -m "feat(xtask): pure identifier helpers for sequence collisions"
```

---

### Task 2: Duplicate-prefix + parity check step (`sequence_check.rs`)

**Files:**
- Create: `xtask/src/steps/sequence_check.rs`
- Modify: `xtask/src/lib.rs` (declare the module in the `mod steps { ... }` block; call `steps::sequence_check::run(&mut result)` in the `Check` and `Validate` arms, immediately after `steps::static_checks::run(...)`)

**Interfaces:**
- Consumes: `ids::duplicate_prefixes`, `ids::parity_mismatch` (Task 1); `CommandResult`, `StepResult`.
- Produces: `sequence_check::run(result: &mut CommandResult)` — pushes a step named `"identifier-collisions"`; `sequence_check::problems(adr, sqlite, postgres) -> Option<String>` (pure, for tests).

- [ ] **Step 1: Write the failing tests**

Create `xtask/src/steps/sequence_check.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_dirs_report_no_problems() {
        let adr = vec!["0001-a.md".to_string(), "0002-b.md".to_string()];
        let mig = vec!["0001_x.sql".to_string()];
        assert_eq!(problems(&adr, &mig, &mig), None);
    }

    #[test]
    fn adr_collision_includes_recovery_command() {
        let adr = vec!["0034-foo.md".to_string(), "0034-bar.md".to_string()];
        let detail = problems(&adr, &[], &[]).expect("a problem");
        assert!(detail.contains("ADR number 0034"));
        assert!(detail.contains("0034-bar.md"));
        assert!(detail.contains("cargo xtask adr renumber"));
    }

    #[test]
    fn migration_collision_has_no_adr_recovery_line() {
        let mig = vec!["0007_a.sql".to_string(), "0007_b.sql".to_string()];
        let detail = problems(&[], &mig, &mig).expect("a problem");
        assert!(detail.contains("sqlite migration 0007"));
        assert!(!detail.contains("cargo xtask adr renumber"));
    }

    #[test]
    fn parity_gap_is_reported() {
        let sqlite = vec!["0001_a.sql".to_string()];
        let postgres = vec!["0001_a.sql".to_string(), "0002_b.sql".to_string()];
        let detail = problems(&[], &sqlite, &postgres).expect("a problem");
        assert!(detail.contains("backend parity"));
        assert!(detail.contains("0002_b (postgres only)"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

First wire the module so it compiles: in `xtask/src/lib.rs`, inside the existing `mod steps { ... }` block add `pub mod sequence_check;`.

Run: `cargo test --manifest-path xtask/Cargo.toml sequence_check::`
Expected: FAIL — `cannot find function problems`.

- [ ] **Step 3: Implement the check**

Prepend to `xtask/src/steps/sequence_check.rs`:

```rust
//! The `identifier-collisions` static check: scans the ADR and migration
//! directories for duplicate numeric prefixes (which git merges silently because
//! the filenames differ) and for sqlite/postgres backend parity. Read-only in
//! every mode — resolution for ADRs is the separate `adr renumber` command.

use std::path::Path;

use crate::ids;
use crate::result::{CommandResult, StepResult};

const ADR_DIR: &str = "docs/adr";
const SQLITE_DIR: &str = "storage/migrations/sqlite";
const PG_DIR: &str = "storage/migrations/postgres";

/// Filenames of regular files directly in `dir`. A missing directory yields an
/// empty list (the check is a no-op rather than an error).
fn filenames(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// The failure detail for all collisions/parity problems, or `None` when clean.
/// Pure given the three filename lists, so it is unit-tested directly.
pub fn problems(adr: &[String], sqlite: &[String], postgres: &[String]) -> Option<String> {
    let mut lines = Vec::new();

    let adr_dups = ids::duplicate_prefixes(adr);
    for (number, files) in &adr_dups {
        lines.push(format!(
            "ADR number {number:04} is used by multiple files: {}",
            files.join(", ")
        ));
    }
    if !adr_dups.is_empty() {
        lines.push("  recovery: cargo xtask adr renumber".to_string());
    }

    for (number, files) in ids::duplicate_prefixes(sqlite) {
        lines.push(format!(
            "sqlite migration {number:04} is used by multiple files: {}",
            files.join(", ")
        ));
    }
    for (number, files) in ids::duplicate_prefixes(postgres) {
        lines.push(format!(
            "postgres migration {number:04} is used by multiple files: {}",
            files.join(", ")
        ));
    }
    for mismatch in ids::parity_mismatch(sqlite, postgres) {
        lines.push(format!("migration backend parity: {mismatch}"));
    }

    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Scan the repo's identifier directories and push the result step.
pub fn run(result: &mut CommandResult) {
    let adr = filenames(Path::new(ADR_DIR));
    let sqlite = filenames(Path::new(SQLITE_DIR));
    let postgres = filenames(Path::new(PG_DIR));
    let step = match problems(&adr, &sqlite, &postgres) {
        None => StepResult::ok("identifier-collisions"),
        Some(detail) => StepResult::fail("identifier-collisions").detail(detail),
    };
    result.push(step);
}
```

- [ ] **Step 4: Wire the step into `check` and `validate`**

In `xtask/src/lib.rs`, in the `Command::Check` arm, after `steps::static_checks::run(&sh, Mode::Fix, &mut result);` add:

```rust
            steps::sequence_check::run(&mut result);
```

In the `Command::Validate` arm, after `steps::static_checks::run(&sh, Mode::Check, &mut result);` add:

```rust
            steps::sequence_check::run(&mut result);
```

- [ ] **Step 5: Run the unit tests + a live check**

Run: `cargo test --manifest-path xtask/Cargo.toml sequence_check::`
Expected: PASS (4 tests).

Run: `cargo xtask check --no-test`
Expected: a new `[ ok ] identifier-collisions` line appears; `xtask check PASSED` (the real tree is currently collision-free and parity-correct).

- [ ] **Step 6: Per-task gate + commit**

Run: `cargo xtask check --no-test`
Expected: all `[ ok ]`.

```bash
git add xtask/src/steps/sequence_check.rs xtask/src/lib.rs
git commit -m "feat(xtask): identifier-collisions check for ADRs and migrations"
```

---

### Task 3: `adr renumber` — rewrite helpers, env-scrubbed git builder, CLI surface

**Files:**
- Create: `xtask/src/adr.rs`
- Modify: `xtask/src/git.rs` (add `pub fn at`); `xtask/src/lib.rs` (use `git::at` where the private `git_at` is used today; declare `mod adr;`; add the `Adr` subcommand, its `command_name` arm, and dispatch)

**Interfaces:**
- Consumes: `ids::*` (Task 1).
- Produces:
  - `git::at(dir: &Path) -> std::process::Command` (env-scrubbed git command builder)
  - `adr::pad(n: u32) -> String`
  - `adr::rewrite_stem(content: &str, old_stem: &str, new_stem: &str) -> String`
  - `adr::rewrite_bare(content: &str, old: u32, new: u32) -> String`
  - `adr::replace_number(filename: &str, new: u32) -> String`
  - `adr::renumber() -> StepResult` (real entry point; orchestration body lands in Task 4)

- [ ] **Step 1: Promote the env-scrubbed git builder into `git.rs`**

Add to `xtask/src/git.rs` (top-level), copying the scrubbing logic verbatim from lib.rs's private `git_at`:

```rust
use std::path::Path;
use std::process::Command;

/// A `git -C <dir>` command scrubbed of the ambient env vars that redirect git at
/// a different repository. A git hook (e.g. `.githooks/pre-push`) exports
/// `GIT_DIR`/`GIT_INDEX_FILE`; those would make `git -C <dir>` operate on the
/// hook's repo instead of `dir`. Clearing them pins the target to `-C <dir>`.
pub fn at(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir);
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
    ] {
        cmd.env_remove(var);
    }
    cmd
}
```

Then in `xtask/src/lib.rs`, delete the private `fn git_at(...)` and replace its call sites (`register_keepours`, `merge_driver_value`, and the `merge_driver_tests` `use super::...` / `git_at(...)` references) with `crate::git::at` / `git::at`. Update the test module import `use super::{ensure_merge_driver, git_at, ...}` to drop `git_at` and call `crate::git::at` instead.

- [ ] **Step 2: Write failing tests for the pure rewrite helpers**

Create `xtask/src/adr.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_is_four_digits() {
        assert_eq!(pad(34), "0034");
        assert_eq!(pad(5), "0005");
    }

    #[test]
    fn replace_number_keeps_slug_and_extension() {
        assert_eq!(replace_number("0034-bar.md", 35), "0035-bar.md");
        assert_eq!(replace_number("0034-multi-word-slug.md", 35), "0035-multi-word-slug.md");
    }

    #[test]
    fn rewrite_stem_replaces_path_form_refs() {
        let content = "See [the ADR](docs/adr/0034-bar.md) and 0034-bar.md again.";
        let out = rewrite_stem(content, "0034-bar", "0035-bar");
        assert_eq!(out, "See [the ADR](docs/adr/0035-bar.md) and 0035-bar.md again.");
    }

    #[test]
    fn rewrite_bare_replaces_only_the_padded_token() {
        let content = "ADR-0034 governs this. Unrelated number 10034 stays.";
        let out = rewrite_bare(content, 34, 35);
        assert_eq!(out, "ADR-0035 governs this. Unrelated number 10034 stays.");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Add `mod adr;` to `xtask/src/lib.rs` (near `mod coverage;`) so the module compiles.

Run: `cargo test --manifest-path xtask/Cargo.toml adr::`
Expected: FAIL — `cannot find function pad`.

- [ ] **Step 4: Implement the pure helpers + entry-point stub**

Prepend to `xtask/src/adr.rs`:

```rust
//! `cargo xtask adr renumber`: resolve an ADR number collision by bumping the
//! branch's newly-added ADR to the next free number and rewriting references.
//! The ADR already reachable from `origin/main` is immutable; only branch
//! additions move. Path-form references (which carry the slug) are rewritten
//! repo-wide; bare `ADR-NNNN` references are rewritten only in branch-touched
//! files, so `main`'s references to the other number are never clobbered.

use crate::result::StepResult;

/// Four-digit zero-padded number, e.g. `34 -> "0034"`.
pub fn pad(n: u32) -> String {
    format!("{n:04}")
}

/// Replace the leading number of `filename`, preserving the separator, slug, and
/// extension: `replace_number("0034-bar.md", 35) -> "0035-bar.md"`.
pub fn replace_number(filename: &str, new: u32) -> String {
    let rest = filename.trim_start_matches(|c: char| c.is_ascii_digit());
    format!("{}{rest}", pad(new))
}

/// Replace every occurrence of `old_stem` with `new_stem`. The stem carries the
/// slug (`0034-bar`), so it is unambiguous and safe to rewrite repo-wide.
pub fn rewrite_stem(content: &str, old_stem: &str, new_stem: &str) -> String {
    content.replace(old_stem, new_stem)
}

/// Replace bare `ADR-NNNN` references for `old` -> `new`. The padded `ADR-` prefix
/// keeps `10034`-style substrings from matching. Caller scopes this to
/// branch-touched files because the bare form lacks a slug.
pub fn rewrite_bare(content: &str, old: u32, new: u32) -> String {
    content.replace(&format!("ADR-{}", pad(old)), &format!("ADR-{}", pad(new)))
}

/// Entry point for the `adr renumber` subcommand. Orchestration body added in
/// Task 4; for now report that it is unimplemented so the wiring is testable.
pub fn renumber() -> StepResult {
    StepResult::fail("adr-renumber").detail("not yet implemented")
}
```

- [ ] **Step 5: Add the CLI subcommand**

In `xtask/src/lib.rs`:

In the `Command` enum, add:

```rust
    /// Resolve an ADR number collision: bump this branch's newly-added ADR to the
    /// next free number and rewrite references. The ADR already on `origin/main`
    /// keeps its number. Run after rebasing onto the latest `origin/main`.
    #[command(subcommand)]
    Adr(AdrCommand),
```

After the `CoverageCommand` enum, add:

```rust
/// `adr` subcommands.
#[derive(Subcommand)]
pub enum AdrCommand {
    /// Renumber this branch's colliding ADR to the next free number and rewrite
    /// references (path-form repo-wide; bare `ADR-NNNN` in branch-touched files).
    Renumber,
}
```

In `command_name`, add the arm:

```rust
            Command::Adr(AdrCommand::Renumber) => "adr-renumber",
```

In `run`, add the match arm:

```rust
        Command::Adr(AdrCommand::Renumber) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("adr-renumber");
            result.push(adr::renumber());
            finalize(&mut result, start);
            Ok(result)
        }
```

- [ ] **Step 6: Write + run the CLI parse test**

Add to the `cli_tests` module in `xtask/src/lib.rs`:

```rust
    #[test]
    fn adr_renumber_parses() {
        let cli = Cli::try_parse_from(["xtask", "adr", "renumber"]).unwrap();
        assert_eq!(cli.command_name(), "adr-renumber");
    }
```

Run: `cargo test --manifest-path xtask/Cargo.toml`
Expected: PASS — `adr::` helper tests, the new CLI parse test, and the unchanged merge-driver tests (now using `git::at`).

- [ ] **Step 7: Per-task gate + commit**

Run: `cargo xtask check --no-test`
Expected: all `[ ok ]`.

```bash
git add xtask/src/adr.rs xtask/src/git.rs xtask/src/lib.rs
git commit -m "feat(xtask): adr renumber scaffolding + shared git::at builder"
```

---

### Task 4: `adr renumber` git orchestration + integration test

**Files:**
- Modify: `xtask/src/adr.rs` (replace the `renumber` stub with the real body + git helpers + an integration test)

**Interfaces:**
- Consumes: `ids::leading_number`, `ids::next_number` (Task 1); `git::at` (Task 3); `pad`, `replace_number`, `rewrite_stem`, `rewrite_bare` (Task 3).
- Produces: `adr::renumber()` resolves a real collision; internal `run_renumber(repo: &Path, main_ref: &str) -> anyhow::Result<String>` is the testable core.

- [ ] **Step 1: Write the failing integration test**

Add to the `tests` module in `xtask/src/adr.rs` (the `run_renumber` signature lets the test pass a local `main` branch instead of `origin/main`):

```rust
    use std::path::Path;

    fn git(dir: &Path, args: &[&str]) {
        let ok = crate::git::at(dir).args(args).status().unwrap().success();
        assert!(ok, "git {args:?} failed");
    }

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn renumber_bumps_newcomer_and_rewrites_refs() {
        let tmp = std::env::temp_dir().join(format!("jaunder-adr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        git(&tmp, &["init", "-q", "-b", "main"]);
        git(&tmp, &["config", "user.email", "t@t"]);
        git(&tmp, &["config", "user.name", "t"]);

        // main: ADR-0034-foo plus a doc that references it by both forms.
        write(&tmp, "docs/adr/0034-foo.md", "# ADR-0034: Foo\n");
        write(&tmp, "CONTRIBUTING.md", "See ADR-0034 at docs/adr/0034-foo.md.\n");
        git(&tmp, &["add", "."]);
        git(&tmp, &["commit", "-qm", "main: 0034-foo"]);

        // branch: a colliding ADR-0034-bar plus a NEW file referencing it.
        git(&tmp, &["checkout", "-q", "-b", "feature"]);
        write(&tmp, "docs/adr/0034-bar.md", "# ADR-0034: Bar\nsee docs/adr/0034-bar.md\n");
        write(&tmp, "docs/notes.md", "Decided in ADR-0034 (docs/adr/0034-bar.md).\n");
        git(&tmp, &["add", "."]);
        git(&tmp, &["commit", "-qm", "feature: 0034-bar"]);

        let summary = run_renumber(&tmp, "main").unwrap();
        assert!(summary.contains("0034-bar.md -> 0035-bar.md"), "summary: {summary}");

        // The newcomer moved; main's ADR is untouched.
        assert!(tmp.join("docs/adr/0035-bar.md").exists());
        assert!(!tmp.join("docs/adr/0034-bar.md").exists());
        assert!(tmp.join("docs/adr/0034-foo.md").exists());

        // Branch-added file: both forms rewritten to 0035.
        let notes = std::fs::read_to_string(tmp.join("docs/notes.md")).unwrap();
        assert_eq!(notes, "Decided in ADR-0035 (docs/adr/0035-bar.md).\n");

        // The moved ADR's own title (bare form, in a branch-touched file) rewritten.
        let bar = std::fs::read_to_string(tmp.join("docs/adr/0035-bar.md")).unwrap();
        assert!(bar.contains("# ADR-0035: Bar"));
        assert!(bar.contains("docs/adr/0035-bar.md"));

        // main's pre-existing file keeps its bare ADR-0034 (NOT branch-touched).
        let contributing = std::fs::read_to_string(tmp.join("CONTRIBUTING.md")).unwrap();
        assert_eq!(contributing, "See ADR-0034 at docs/adr/0034-foo.md.\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path xtask/Cargo.toml adr::tests::renumber_bumps`
Expected: FAIL — `cannot find function run_renumber` (and the stub returns "not yet implemented").

- [ ] **Step 3: Implement the orchestration**

In `xtask/src/adr.rs`, add the imports and replace the `renumber` stub body. Add near the top:

```rust
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ids;
```

Replace `pub fn renumber()` with:

```rust
const ADR_DIR: &str = "docs/adr";

/// Real entry point: operate on the current repo against `origin/main`.
pub fn renumber() -> StepResult {
    match run_renumber(Path::new("."), "origin/main") {
        Ok(summary) => StepResult::ok("adr-renumber").detail(summary),
        Err(e) => StepResult::fail("adr-renumber").detail(format!("{e:#}")),
    }
}

/// Trimmed stdout of a git command in `repo`, or an error with the stderr. An
/// empty result (e.g. `grep`/`diff` with no matches) is `Ok(String::new())`;
/// callers that tolerate "no matches" pass `allow_failure = true`.
fn git_out(repo: &Path, args: &[&str], allow_failure: bool) -> Result<String> {
    let out = crate::git::at(repo)
        .args(args)
        .output()
        .with_context(|| format!("running git {args:?}"))?;
    if !out.status.success() && !allow_failure {
        anyhow::bail!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Non-empty lines of a git command's stdout.
fn git_lines(repo: &Path, args: &[&str], allow_failure: bool) -> Result<Vec<String>> {
    Ok(git_out(repo, args, allow_failure)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect())
}

/// ADR filenames currently in `repo`'s `docs/adr`.
fn adr_filenames(repo: &Path) -> Vec<String> {
    let dir = repo.join(ADR_DIR);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// Rewrite `needle`/number references across the relevant files and `git mv` the
/// colliding ADRs. Returns a human summary of the moves.
fn run_renumber(repo: &Path, main_ref: &str) -> Result<String> {
    let base = git_out(repo, &["merge-base", main_ref, "HEAD"], false)
        .context("finding merge-base with main")?;
    let range = format!("{base}..HEAD");

    // ADR files this branch ADDED (filenames only).
    let added: Vec<String> = git_lines(
        repo,
        &["diff", "--diff-filter=A", "--name-only", &range, "--", ADR_DIR],
        false,
    )?
    .into_iter()
    .filter_map(|p| p.rsplit('/').next().map(str::to_string))
    .collect();

    // Files this branch touched at all (scope for bare-ref rewrites).
    let touched: Vec<String> = git_lines(repo, &["diff", "--name-only", &range], false)?;

    let mut all = adr_filenames(repo);
    let mut summary = Vec::new();

    for added_name in &added {
        let Some(num) = ids::leading_number(added_name) else {
            continue;
        };
        let collides = all
            .iter()
            .filter(|n| ids::leading_number(n) == Some(num))
            .count()
            > 1;
        if !collides {
            continue;
        }

        let new_num = ids::next_number(&all);
        let new_name = replace_number(added_name, new_num);
        let old_stem = added_name.rsplit_once('.').map_or(added_name.as_str(), |(s, _)| s);
        let new_stem = new_name.rsplit_once('.').map_or(new_name.as_str(), |(s, _)| s);

        // 1. git mv the colliding newcomer.
        let old_rel = format!("{ADR_DIR}/{added_name}");
        let new_rel = format!("{ADR_DIR}/{new_name}");
        git_out(repo, &["mv", &old_rel, &new_rel], false)?;

        // 2. Path-form (slug-bearing) refs: rewrite repo-wide.
        for file in git_lines(repo, &["grep", "-l", "--fixed-strings", old_stem], true)? {
            rewrite_file(repo, &file, |c| rewrite_stem(c, old_stem, new_stem))?;
        }

        // 3. Bare `ADR-NNNN` refs: rewrite only in branch-touched files (path
        //    after the mv is `new_rel`, so consider both the old and new path).
        let bare_token = format!("ADR-{}", pad(num));
        for file in git_lines(repo, &["grep", "-l", "--fixed-strings", &bare_token], true)? {
            let touched_by_branch =
                touched.iter().any(|t| t == &file) || file == new_rel || file == old_rel;
            if touched_by_branch {
                rewrite_file(repo, &file, |c| rewrite_bare(c, num, new_num))?;
            }
        }

        // Reflect the rename so a second newcomer gets a fresh number.
        all.retain(|n| n != added_name);
        all.push(new_name.clone());
        summary.push(format!("{added_name} -> {new_name}"));
    }

    if summary.is_empty() {
        Ok("no ADR collisions to resolve".to_string())
    } else {
        Ok(summary.join("; "))
    }
}

/// Read `rel` under `repo`, apply `f`, write it back only if it changed.
fn rewrite_file(repo: &Path, rel: &str, f: impl Fn(&str) -> String) -> Result<()> {
    let path: PathBuf = repo.join(rel);
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let updated = f(&content);
    if updated != content {
        std::fs::write(&path, updated).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}
```

- [ ] **Step 4: Run the integration test to verify it passes**

Run: `cargo test --manifest-path xtask/Cargo.toml adr::tests::renumber_bumps`
Expected: PASS.

- [ ] **Step 5: Run the full xtask suite**

Run: `cargo test --manifest-path xtask/Cargo.toml`
Expected: PASS (Tasks 1–4 tests + existing suite).

- [ ] **Step 6: Per-task gate + commit**

Run: `cargo xtask check --no-test`
Expected: all `[ ok ]`.

```bash
git add xtask/src/adr.rs
git commit -m "feat(xtask): implement adr renumber collision resolver"
```

---

### Task 5: ADR record + CONTRIBUTING note

**Files:**
- Create: `docs/adr/00NN-identifier-collision-policy.md` (number = next free at implementation time)
- Modify: `CONTRIBUTING.md`

- [ ] **Step 1: Determine the next free ADR number**

Run: `ls docs/adr`
Expected: confirm the highest existing number; the new ADR is max + 1. (Note: `CLAUDE.md` already cites `ADR-0034` for the CI/e2e-matrix decision, so verify whether `0034` is taken before choosing.) Use the chosen `NNNN` consistently below.

- [ ] **Step 2: Write the ADR**

Create `docs/adr/00NN-identifier-collision-policy.md` (MADR style, matching siblings):

```markdown
# ADR-00NN: Identifier-collision policy for ADRs and migrations

* Status: accepted
* Date: 2026-06-28

## Context and Problem Statement

Sequentially-numbered files created on concurrent branches collide: two branches
each pick the next number, and because the filenames differ (`0034-foo.md` vs
`0034-bar.md`) git merges them with no conflict — the collision is silent and
only surfaces as confusion later. ADRs (`docs/adr/NNNN-slug.md`) hit this often;
migrations (`storage/migrations/{sqlite,postgres}/NNNN_slug.sql`) have the same
shape but have not yet collided, and their number is referenced nowhere outside
the directory.

## Decision Drivers

* Make a collision loud rather than silent.
* Make ADR resolution cheap (ADR numbers are referenced in code, `clippy.toml`,
  and docs, so the sequence has value and is worth preserving).
* Proportionality: do not add machinery migrations do not need.

## Decision Outcome

The governing rule: **a branch must never allocate a shared identifier by reading
the current maximum and hoping it survives the merge.**

* A build-free `identifier-collisions` check runs inside `cargo xtask
  check`/`validate`. It fails on a duplicate numeric prefix within `docs/adr`,
  `storage/migrations/sqlite`, or `storage/migrations/postgres`, and on
  sqlite/postgres backend-parity gaps. This makes every collision loud on the
  branch (after rebase) and on `main`'s CI.
* `cargo xtask adr renumber` resolves an ADR collision in one command: the ADR
  already reachable from `origin/main` is immutable; the branch's newly-added ADR
  is bumped to the next free number, with path-form references rewritten repo-wide
  and bare `ADR-NNNN` references rewritten in branch-touched files.
* Migrations keep sequential numbering with the detection check only — no
  renumber tool, no timestamps. Timestamps were rejected: they are collision-free
  but not monotonic with respect to merge order, and a later-merged migration with
  an earlier timestamp can trip sqlx's out-of-order detection on a persistent DB.

## Consequences

* Good: collisions cannot ship silently; ADR collisions are a one-command fix.
* Good: no change to the established sequential naming convention.
* Bad: the `adr renumber` heuristic cannot disambiguate a bare `ADR-NNNN` that a
  branch adds into a pre-existing file already citing the other number; that rare
  case is left to the human, and the detection check still guards correctness.
```

- [ ] **Step 3: Add the CONTRIBUTING note**

In `CONTRIBUTING.md`, add a short subsection near the existing ADR/documentation guidance:

```markdown
### Adding an ADR

ADRs are `docs/adr/NNNN-slug.md` with a sequential number. The
`identifier-collisions` step of `cargo xtask check`/`validate` fails if two ADRs
(or two migrations, per backend) share a number, or if the sqlite/postgres
migration sets diverge. If a concurrent branch took "your" number, the check goes
red after you rebase onto `main`; run `cargo xtask adr renumber` to bump your new
ADR to the next free number and rewrite references automatically. Migrations use
the same sequential convention and the same detection check, but are renumbered
by hand on the rare occasion it is needed.
```

- [ ] **Step 4: Verify the new ADR does not trip the check**

Run: `cargo xtask check --no-test`
Expected: `[ ok ] identifier-collisions` (the new ADR has a unique number); all `[ ok ]`.

- [ ] **Step 5: Commit**

```bash
git add docs/adr/00NN-identifier-collision-policy.md CONTRIBUTING.md
git commit -m "docs: ADR + CONTRIBUTING note for identifier-collision policy"
```

---

## Final Gate

- [ ] Run the full local gate: `cargo xtask validate --no-e2e`
  Expected: all steps `[ ok ]`, `xtask validate PASSED`. (This is the pre-push gate; e2e is unaffected by xtask-only changes, but run full `cargo xtask validate` if you want the complete sweep.)

- [ ] Manual smoke (optional): in a throwaway branch, create a deliberately colliding `docs/adr/<existing-number>-test.md`, run `cargo xtask check --no-test` (see it fail with the recovery line), run `cargo xtask adr renumber`, re-run the check (see it pass), then discard the branch.

---

## Self-Review

**Spec coverage:**
- Duplicate-prefix check (ADRs + migrations) → Task 2.
- Backend-parity assertion (required) → Task 1 (`parity_mismatch`) + Task 2 (`problems`, `parity_gap_is_reported` test).
- Runs in the verify ladder (`check`/`validate`, build-free) → Task 2 Step 4.
- Failure prints number + filenames + ADR recovery command → Task 2 (`problems`).
- `cargo xtask adr renumber`: keep-`origin/main`-immutable, bump newcomer → Task 4 (`collides` check over branch-added ADRs; `next_number`).
- Path-form refs rewritten repo-wide; bare refs scoped to branch-touched files → Task 4 Steps 2–3 of the orchestration; asserted by the integration test (CONTRIBUTING's bare ref untouched).
- Migrations: test only, keep sequential, no timestamps → no renumber tool authored; ADR records the rejection.
- New ADR + CONTRIBUTING note → Task 5.

**Placeholder scan:** The only intentional placeholder is the ADR number `00NN` in Task 5, resolved in Task 5 Step 1 against the live tree. No `TODO`/`TBD`/"add error handling" left in code.

**Type consistency:** `leading_number`/`next_number`/`duplicate_prefixes`/`parity_mismatch` signatures are identical between Task 1 (definition) and Tasks 2/4 (use). `pad`/`rewrite_stem`/`rewrite_bare`/`replace_number` defined in Task 3, used in Task 4. `git::at` defined in Task 3, used in Task 4 and the existing merge-driver code. Step name `"identifier-collisions"` and command name `"adr-renumber"` are consistent across tasks.
