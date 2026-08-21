//! Git helpers for the verify gate: working-tree cleanliness (the `validate`
//! backstop) and self-healing `core.hooksPath` installation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

/// Repo-relative hooks directory the gate routes git to. Relative (not absolute)
/// so each worktree resolves to its own `.githooks` checkout.
pub const HOOKS_PATH: &str = ".githooks";

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

/// True when `git status --porcelain` output denotes a dirty tree. Porcelain lists
/// staged + unstaged tracked changes AND untracked non-gitignored files (`??`), and
/// omits gitignored paths — exactly the surface the Nix coverage source picks up.
/// Any non-blank line means dirty.
pub fn porcelain_is_dirty(porcelain: &str) -> bool {
    porcelain.lines().any(|line| !line.trim().is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusSnapshot {
    pub paths: BTreeMap<String, GitPathStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitPathStatus {
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
    pub delete_or_rename: bool,
    pub worktree_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecommitStagePlan {
    pub stage_paths: Vec<String>,
    pub failures: Vec<String>,
}

/// Whether `core.hooksPath` needs (re)pointing at [`HOOKS_PATH`], given its current
/// value (`None` = unset).
pub fn needs_hooks_path(current: Option<&str>) -> bool {
    match current {
        Some(value) => value.trim() != HOOKS_PATH,
        None => true,
    }
}

/// `git status --porcelain` text. Errors only if git itself cannot run.
pub fn working_tree_status(dir: &Path) -> Result<String> {
    output(dir, &["status", "--porcelain"])
}

/// Snapshot the full dirty working tree visible to pre-commit. `--untracked-files=all`
/// is load-bearing: without it, Git can collapse a dirty untracked directory to
/// `?? dir/` and hide a new file created under that directory during the gate.
/// `--find-renames` is also load-bearing: it preserves rename detection while
/// overriding user config such as `status.renames=copies`, which can otherwise
/// emit synthetic `C  old -> new` paths that cannot be fingerprinted or staged.
pub fn status_snapshot(dir: &Path) -> Result<GitStatusSnapshot> {
    let out = at(dir)
        .args([
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--find-renames",
        ])
        .output()
        .with_context(|| "running git status --porcelain --untracked-files=all --find-renames")?;
    if !out.status.success() {
        anyhow::bail!(
            "git status --porcelain --untracked-files=all --find-renames failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let mut snapshot = parse_status_snapshot(&String::from_utf8_lossy(&out.stdout));
    for (path, status) in &mut snapshot.paths {
        if !status.untracked && !status.delete_or_rename {
            status.worktree_fingerprint = Some(output(dir, &["hash-object", "--", path])?);
        }
    }
    Ok(snapshot)
}

pub fn parse_status_snapshot(porcelain: &str) -> GitStatusSnapshot {
    let mut paths = BTreeMap::new();
    for line in porcelain.lines().filter(|line| !line.trim().is_empty()) {
        let bytes = line.as_bytes();
        if bytes.len() < 3 {
            continue;
        }
        let path = line.get(3..).unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        let status = paths
            .entry(path.to_string())
            .or_insert_with(GitPathStatus::default);
        let index = bytes[0];
        let worktree = bytes[1];
        if index == b'?' && worktree == b'?' {
            status.untracked = true;
            continue;
        }
        status.staged |= index != b' ' && index != b'?';
        status.unstaged |= worktree != b' ' && worktree != b'?';
        status.delete_or_rename |= matches!(index, b'D' | b'R') || matches!(worktree, b'D' | b'R');
    }
    GitStatusSnapshot { paths }
}

pub fn precommit_stage_plan(
    before: &GitStatusSnapshot,
    after: &GitStatusSnapshot,
) -> PrecommitStagePlan {
    let paths: BTreeSet<_> = before.paths.keys().chain(after.paths.keys()).collect();
    let mut stage_paths = Vec::new();
    let mut failures = Vec::new();
    for path in paths {
        let before_status = before.paths.get(path);
        let after_status = after.paths.get(path);
        if before_status.is_some_and(|s| s.delete_or_rename)
            || after_status.is_some_and(|s| s.delete_or_rename)
        {
            failures.push(format!(
                "{path}: delete/rename status is unsafe for auto-staging"
            ));
            continue;
        }
        if after_status.is_some_and(|s| s.untracked) {
            if before_status.is_none() {
                failures.push(format!(
                    "{path}: new untracked file created during precommit"
                ));
            }
            continue;
        }
        if before_status.is_some_and(|s| s.untracked) {
            continue;
        }
        if !precommit_path_changed(before_status, after_status) {
            continue;
        }
        match before_status {
            Some(status) if status.staged && !status.unstaged => stage_paths.push(path.clone()),
            Some(status) if status.staged && status.unstaged => {
                failures.push(format!(
                    "{path}: pre-existing mixed staged/unstaged state made auto-staging unsafe"
                ));
            }
            _ => failures.push(format!("{path}: will not add work the user did not stage")),
        }
    }
    stage_paths.sort();
    stage_paths.dedup();
    failures.sort();
    failures.dedup();
    PrecommitStagePlan {
        stage_paths,
        failures,
    }
}

fn precommit_path_changed(before: Option<&GitPathStatus>, after: Option<&GitPathStatus>) -> bool {
    fn comparable(
        status: Option<&GitPathStatus>,
    ) -> Option<(bool, bool, bool, bool, &Option<String>)> {
        status.map(|s| {
            (
                s.staged,
                s.unstaged,
                s.untracked,
                s.delete_or_rename,
                &s.worktree_fingerprint,
            )
        })
    }
    comparable(before) != comparable(after)
}

pub fn apply_precommit_stage_plan(dir: &Path, plan: &PrecommitStagePlan) -> crate::StepResult {
    let mut failures = plan.failures.clone();
    for path in &plan.stage_paths {
        if let Err(err) = run(dir, &["add", "--", path]) {
            failures.push(err.to_string());
        }
    }
    if !failures.is_empty() {
        failures.sort();
        failures.dedup();
        return crate::StepResult::fail("precommit-staging").detail(failures.join("\n"));
    }
    if plan.stage_paths.is_empty() {
        return crate::StepResult::ok("precommit-staging").detail("no staged fixes");
    }
    crate::StepResult::ok("precommit-staging")
        .detail(format!("staged: {}", plan.stage_paths.join(", ")))
}

/// Tracked files matching `glob`, sorted, relative to `dir`.
///
/// `git ls-files` rather than a filesystem walk: the walk would descend into
/// `target/` and into nested worktrees under `.claude/worktrees/`, so a gate that
/// enumerates "every source file" would police other checkouts' code.
///
/// `ls-files` lists only what is under `dir`, so a caller that means "every tracked
/// file in the repo" must pass [`toplevel`] — a test run with `--manifest-path
/// xtask/Cargo.toml` executes with `xtask/` as its cwd and would otherwise see a
/// partial tree, with paths relative to it rather than to the repo.
pub fn tracked_files(dir: &Path, glob: &str) -> Result<Vec<String>> {
    let out = output(dir, &["ls-files", "--", glob])?;
    let mut files: Vec<String> = out.lines().map(str::to_string).collect();
    files.sort();
    Ok(files)
}

/// The checked-out branch, or `None` when HEAD is detached (`--show-current`
/// prints nothing there). Used by `pr land` to tell "I am standing on this PR's
/// branch" from "I am elsewhere".
pub fn current_branch(dir: &Path) -> Result<Option<String>> {
    let name = output(dir, &["branch", "--show-current"])?;
    Ok((!name.is_empty()).then_some(name))
}

/// The local HEAD commit, or `None` in a repo with no commits yet (exit 128).
pub fn head_sha(dir: &Path) -> Result<Option<String>> {
    output_or(dir, &["rev-parse", "HEAD"], 128)
}

/// A remote's URL, or `None` when that remote is not configured. Read through
/// `config --get` rather than `remote get-url` so the unset case reuses the
/// exit-1-is-a-valid-nothing path already established here.
pub fn remote_url(dir: &Path, name: &str) -> Result<Option<String>> {
    config_get(dir, &format!("remote.{name}.url"))
}

/// Current `core.hooksPath`, or `None` when unset/blank (see [`config_get`]).
pub fn hooks_path(dir: &Path) -> Result<Option<String>> {
    config_get(dir, "core.hooksPath")
}

/// Ensure `core.hooksPath` points at [`HOOKS_PATH`]; set it if unset/wrong. Returns
/// `true` when it changed the config.
pub fn ensure_hooks_path(dir: &Path) -> Result<bool> {
    if needs_hooks_path(hooks_path(dir)?.as_deref()) {
        config_set(dir, "core.hooksPath", HOOKS_PATH)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Trimmed stdout of a git command in `dir`; bail on any non-zero exit. The one
/// place the capture-and-check plumbing lives.
pub(crate) fn output(dir: &Path, args: &[&str]) -> Result<String> {
    let out = at(dir)
        .args(args)
        .output()
        .with_context(|| format!("running git {args:?}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Non-empty lines of [`output`].
pub(crate) fn lines(dir: &Path, args: &[&str]) -> Result<Vec<String>> {
    Ok(output(dir, args)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect())
}

/// Run a git command in `dir` for effect (no capture); bail on non-zero exit.
pub(crate) fn run(dir: &Path, args: &[&str]) -> Result<()> {
    let ok = at(dir)
        .args(args)
        .status()
        .with_context(|| format!("running git {args:?}"))?
        .success();
    if !ok {
        anyhow::bail!("git {args:?} failed");
    }
    Ok(())
}

/// Trimmed stdout of a git command, or `None` when it exits with `tolerated`
/// instead of bailing — the shared core of the two helpers that read one exit
/// code as a valid "nothing" answer (`grep`'s exit 1 = no match, `config --get`'s
/// exit 1 = unset). Any other non-zero still bails.
fn output_or(dir: &Path, args: &[&str], tolerated: i32) -> Result<Option<String>> {
    let out = at(dir)
        .args(args)
        .output()
        .with_context(|| format!("running git {args:?}"))?;
    match out.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        )),
        Some(c) if c == tolerated => Ok(None),
        _ => anyhow::bail!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

/// `git merge-base <a> <b>`.
pub(crate) fn merge_base(dir: &Path, a: &str, b: &str) -> Result<String> {
    output(dir, &["merge-base", a, b])
}

/// `git diff --name-only <range>` — every file touched in the range.
pub(crate) fn diff_names(dir: &Path, range: &str) -> Result<Vec<String>> {
    lines(dir, &["diff", "--name-only", range])
}

/// `git diff --diff-filter=A --name-only <range> -- <pathspec>` — files ADDED in
/// the range, scoped to `pathspec`.
pub(crate) fn diff_added(dir: &Path, range: &str, pathspec: &str) -> Result<Vec<String>> {
    lines(
        dir,
        &[
            "diff",
            "--diff-filter=A",
            "--name-only",
            range,
            "--",
            pathspec,
        ],
    )
}

/// `git grep -l --fixed-strings <pattern>` — files containing `pattern`.
/// Grep's exit 1 = no match → `Ok(vec![])`; exit 128 (or any other non-zero) =
/// real error → `Err` (see [`output_or`]).
pub(crate) fn grep_files(dir: &Path, pattern: &str) -> Result<Vec<String>> {
    let matched = output_or(dir, &["grep", "-l", "--fixed-strings", pattern], 1)?;
    Ok(match matched {
        Some(out) => out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect(),
        None => Vec::new(),
    })
}

/// `git ls-files -- *.md` — tracked Markdown, repo-relative. The pathspec glob
/// matches at any depth, so filtering happens in git rather than in Rust.
pub(crate) fn ls_files_md(dir: &Path) -> Result<Vec<String>> {
    lines(dir, &["ls-files", "--", "*.md"])
}

/// `git mv <from> <to>`.
pub(crate) fn mv(dir: &Path, from: &str, to: &str) -> Result<()> {
    run(dir, &["mv", from, to])
}

/// `git add <path>`.
pub(crate) fn add(dir: &Path, path: &str) -> Result<()> {
    run(dir, &["add", path])
}

/// `git rev-parse --show-toplevel` — the working tree's root.
pub fn toplevel(dir: &Path) -> Result<String> {
    output(dir, &["rev-parse", "--show-toplevel"])
}

/// `git config --get <key>` → the value, or `None` when unset (exit 1) or blank.
/// Bails on any other non-zero (e.g. exit 128 = corrupt config): a broken config
/// surfaces as an error rather than being silently treated as "unset" (see
/// [`output_or`]).
pub(crate) fn config_get(dir: &Path, key: &str) -> Result<Option<String>> {
    Ok(output_or(dir, &["config", "--get", key], 1)?.filter(|s| !s.is_empty()))
}

/// `git config <key> <value>`.
pub(crate) fn config_set(dir: &Path, key: &str, value: &str) -> Result<()> {
    run(dir, &["config", key, value])
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{commit, git_ok, write};

    fn temp_repo(tag: &str) -> std::path::PathBuf {
        crate::test_support::temp_repo("git", tag)
    }

    #[test]
    fn output_returns_trimmed_stdout_and_bails_on_error() {
        let dir = temp_repo("output");
        commit(&dir, "a.txt", "x\n");
        let head = output(&dir, &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(head.len(), 40, "full sha, trimmed: {head:?}");
        assert!(output(&dir, &["not-a-subcommand"]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lines_drops_blank_lines() {
        let dir = temp_repo("lines");
        commit(&dir, "a.txt", "1\n");
        commit(&dir, "b.txt", "2\n");
        let subjects = lines(&dir, &["log", "--format=%s"]).unwrap();
        assert_eq!(subjects, vec!["c".to_string(), "c".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_ok_on_success_err_on_failure() {
        let dir = temp_repo("run");
        commit(&dir, "a.txt", "x\n");
        assert!(run(&dir, &["status", "--porcelain"]).is_ok());
        assert!(run(&dir, &["mv", "nope", "nowhere"]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn grep_files_match_no_match_and_error() {
        let dir = temp_repo("grep");
        commit(&dir, "hay.txt", "a needle here\n");
        commit(&dir, "other.txt", "nothing\n");
        assert_eq!(
            grep_files(&dir, "needle").unwrap(),
            vec!["hay.txt".to_string()]
        );
        assert!(grep_files(&dir, "absent-token").unwrap().is_empty()); // exit 1
        // A nonexistent dir → git can't chdir → exit 128 → Err (NOT an empty
        // match). Deterministic regardless of whether $TMPDIR sits under a repo.
        let missing =
            std::env::temp_dir().join(format!("jaunder-git-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&missing);
        assert!(grep_files(&missing, "x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_base_diff_added_and_diff_names() {
        let dir = temp_repo("diff");
        commit(&dir, "base.txt", "b\n");
        let base = output(&dir, &["rev-parse", "HEAD"]).unwrap();
        assert!(
            at(&dir)
                .args(["checkout", "-q", "-b", "feature"])
                .status()
                .unwrap()
                .success()
        );
        commit(&dir, "docs/new.md", "n\n");
        let range = format!("{base}..HEAD");
        assert_eq!(merge_base(&dir, "main", "HEAD").unwrap(), base);
        assert_eq!(
            diff_names(&dir, &range).unwrap(),
            vec!["docs/new.md".to_string()]
        );
        assert_eq!(
            diff_added(&dir, &range, "docs").unwrap(),
            vec!["docs/new.md".to_string()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toplevel_returns_repo_root() {
        let dir = temp_repo("toplevel");
        commit(&dir, "a.txt", "x\n");
        let root = toplevel(&dir).unwrap();
        // Compare canonically — /tmp may be a symlink.
        assert_eq!(
            std::fs::canonicalize(&root).unwrap(),
            std::fs::canonicalize(&dir).unwrap()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_get_none_when_unset_some_after_set() {
        let dir = temp_repo("config");
        assert_eq!(config_get(&dir, "core.hooksPath").unwrap(), None);
        config_set(&dir, "core.hooksPath", ".githooks").unwrap();
        assert_eq!(
            config_get(&dir, "core.hooksPath").unwrap(),
            Some(".githooks".to_string())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_hooks_path_sets_then_is_noop() {
        let dir = temp_repo("ensure-hooks");
        assert!(ensure_hooks_path(&dir).unwrap(), "first call sets it");
        assert!(!ensure_hooks_path(&dir).unwrap(), "second call is a no-op");
        assert_eq!(hooks_path(&dir).unwrap(), Some(HOOKS_PATH.to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn current_branch_head_sha_and_remote_read_a_real_repo() {
        let dir = temp_repo("readers");
        commit(&dir, "seed.txt", "x\n");
        assert!(current_branch(&dir).unwrap().is_some());
        let sha = head_sha(&dir).unwrap().expect("a seeded repo has a HEAD");
        assert_eq!(sha.len(), 40, "full sha, not abbreviated");
        assert_eq!(
            remote_url(&dir, "origin").unwrap(),
            None,
            "a throwaway repo has no remote"
        );
        config_set(&dir, "remote.origin.url", "git@github.com:o/r.git").unwrap();
        assert_eq!(
            remote_url(&dir, "origin").unwrap(),
            Some("git@github.com:o/r.git".to_string())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn porcelain_blank_is_clean() {
        assert!(!porcelain_is_dirty(""));
        assert!(!porcelain_is_dirty("\n"));
        assert!(!porcelain_is_dirty("   \n  \n"));
    }

    #[test]
    fn porcelain_untracked_is_dirty() {
        assert!(porcelain_is_dirty("?? new_file.rs"));
    }

    #[test]
    fn porcelain_staged_or_modified_is_dirty() {
        assert!(porcelain_is_dirty(" M src/lib.rs"));
        assert!(porcelain_is_dirty("A  staged.rs"));
        assert!(porcelain_is_dirty("?? a\n M b"));
    }

    #[test]
    fn needs_hooks_path_when_unset_or_wrong() {
        assert!(needs_hooks_path(None));
        assert!(needs_hooks_path(Some(".git/hooks")));
        assert!(needs_hooks_path(Some("")));
    }

    #[test]
    fn no_need_when_hooks_path_already_correct() {
        assert!(!needs_hooks_path(Some(".githooks")));
        assert!(!needs_hooks_path(Some(" .githooks \n")));
    }

    #[test]
    fn precommit_status_parser_classifies_index_worktree_and_untracked() {
        let snap = parse_status_snapshot("M  src/a.rs\n M src/b.rs\nMM src/c.rs\n?? scratch.rs\n");
        assert!(snap.paths["src/a.rs"].staged);
        assert!(!snap.paths["src/a.rs"].unstaged);
        assert!(!snap.paths["src/b.rs"].staged);
        assert!(snap.paths["src/b.rs"].unstaged);
        assert!(snap.paths["src/c.rs"].staged);
        assert!(snap.paths["src/c.rs"].unstaged);
        assert!(snap.paths["scratch.rs"].untracked);
    }

    #[test]
    fn precommit_status_parser_marks_delete_and_rename_unsafe() {
        let snap = parse_status_snapshot("D  gone.rs\n D missing.rs\nR  old.rs -> new.rs\n");
        assert!(snap.paths["gone.rs"].delete_or_rename);
        assert!(snap.paths["missing.rs"].delete_or_rename);
        assert!(snap.paths["old.rs -> new.rs"].delete_or_rename);
    }

    fn fp(mut snap: GitStatusSnapshot, path: &str, value: &str) -> GitStatusSnapshot {
        snap.paths.get_mut(path).unwrap().worktree_fingerprint = Some(value.to_string());
        snap
    }

    #[test]
    fn precommit_stage_plan_stages_only_clean_previously_staged_tracked_paths() {
        let before = fp(
            fp(
                parse_status_snapshot("M  a.rs\n M b.rs\n?? scratch.rs\n"),
                "a.rs",
                "old-a",
            ),
            "b.rs",
            "old-b",
        );
        let after = fp(
            fp(
                parse_status_snapshot("MM a.rs\n M b.rs\n?? scratch.rs\n"),
                "a.rs",
                "new-a",
            ),
            "b.rs",
            "old-b",
        );
        let plan = precommit_stage_plan(&before, &after);
        assert_eq!(plan.stage_paths, vec!["a.rs".to_string()]);
        assert!(plan.failures.is_empty());
    }

    #[test]
    fn precommit_stage_plan_rejects_mixed_and_unstaged_only_mutations() {
        let before = fp(
            fp(
                parse_status_snapshot("MM mixed.rs\n M unstaged.rs\n"),
                "mixed.rs",
                "old-mixed",
            ),
            "unstaged.rs",
            "old-unstaged",
        );
        let after = fp(
            fp(
                parse_status_snapshot("MM mixed.rs\n M unstaged.rs\n"),
                "mixed.rs",
                "new-mixed",
            ),
            "unstaged.rs",
            "new-unstaged",
        );
        let plan = precommit_stage_plan(&before, &after);
        assert!(plan.stage_paths.is_empty());
        assert!(
            plan.failures
                .iter()
                .any(|f| f.contains("mixed.rs") && f.contains("pre-existing mixed"))
        );
        assert!(
            plan.failures.iter().any(|f| f.contains("unstaged.rs")
                && f.contains("will not add work the user did not stage"))
        );
    }

    #[test]
    fn precommit_stage_plan_rejects_clean_before_tracked_mutation() {
        let before = parse_status_snapshot("");
        let after = fp(
            parse_status_snapshot(" M clean.rs\n"),
            "clean.rs",
            "new-clean",
        );
        let plan = precommit_stage_plan(&before, &after);
        assert!(plan.stage_paths.is_empty());
        assert!(
            plan.failures.iter().any(|f| f.contains("clean.rs")
                && f.contains("will not add work the user did not stage"))
        );
    }

    #[test]
    fn precommit_stage_plan_tolerates_old_untracked_and_rejects_new_untracked() {
        let before = parse_status_snapshot("?? old.tmp\n");
        let after = parse_status_snapshot("?? old.tmp\n?? new.tmp\n");
        let plan = precommit_stage_plan(&before, &after);
        assert!(plan.stage_paths.is_empty());
        assert_eq!(plan.failures.len(), 1);
        assert!(plan.failures[0].contains("new.tmp"));
        assert!(plan.failures[0].contains("new untracked"));
    }

    #[test]
    fn precommit_stage_plan_rejects_delete_or_rename_states() {
        let before = parse_status_snapshot("M  keep.rs\n");
        let after = parse_status_snapshot("D  keep.rs\nR  old.rs -> new.rs\n");
        let plan = precommit_stage_plan(&before, &after);
        assert!(plan.stage_paths.is_empty());
        assert!(
            plan.failures
                .iter()
                .any(|f| f.contains("keep.rs") && f.contains("delete/rename"))
        );
        assert!(
            plan.failures
                .iter()
                .any(|f| f.contains("old.rs -> new.rs") && f.contains("delete/rename"))
        );
    }

    #[test]
    fn precommit_stage_plan_preserves_delete_recreate_as_delete_rename() {
        let after = parse_status_snapshot("D  a.rs\n?? a.rs\n");
        assert!(after.paths["a.rs"].delete_or_rename);
        let plan = precommit_stage_plan(&parse_status_snapshot(""), &after);
        assert!(plan.stage_paths.is_empty());
        assert!(
            plan.failures
                .iter()
                .any(|f| f.contains("a.rs") && f.contains("delete/rename"))
        );
    }

    #[test]
    fn precommit_apply_restages_only_the_previously_staged_file() {
        let dir = temp_repo("precommit-restage");
        commit(&dir, "a.rs", "fn a(){}\n");
        commit(&dir, "b.rs", "fn b(){}\n");

        write(&dir, "a.rs", "fn a() { }\n");
        git_ok(&dir, &["add", "a.rs"]);
        write(&dir, "b.rs", "fn b() { }\n");
        let before = status_snapshot(&dir).unwrap();

        write(&dir, "a.rs", "fn a() { }\n// formatted\n");
        let after = status_snapshot(&dir).unwrap();
        let plan = precommit_stage_plan(&before, &after);
        let step = apply_precommit_stage_plan(&dir, &plan);

        assert!(step.ok, "{step:?}");
        assert_eq!(
            output(&dir, &["diff", "--cached", "--name-only"]).unwrap(),
            "a.rs"
        );
        assert_eq!(output(&dir, &["diff", "--name-only"]).unwrap(), "b.rs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_snapshot_forces_rename_detection_not_copy_detection() {
        let dir = temp_repo("status-no-copies");
        commit(&dir, "a.rs", "same\n");
        git_ok(&dir, &["config", "status.renames", "copies"]);
        write(&dir, "b.rs", "same\n");
        git_ok(&dir, &["add", "b.rs"]);

        let snap = status_snapshot(&dir).unwrap();

        assert!(snap.paths.contains_key("b.rs"));
        assert!(!snap.paths.keys().any(|path| path.contains(" -> ")));
        assert!(snap.paths["b.rs"].worktree_fingerprint.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn precommit_apply_stages_safe_paths_even_when_other_failures_exist() {
        let dir = temp_repo("precommit-partial-failure");
        commit(&dir, "a.rs", "one\n");
        write(&dir, "a.rs", "two\n");
        git_ok(&dir, &["add", "a.rs"]);
        let before = status_snapshot(&dir).unwrap();

        write(&dir, "a.rs", "three\n");
        write(&dir, "new.tmp", "new\n");
        let after = status_snapshot(&dir).unwrap();
        let plan = precommit_stage_plan(&before, &after);
        let step = apply_precommit_stage_plan(&dir, &plan);

        assert!(!step.ok);
        assert!(step.detail.as_deref().unwrap().contains("new untracked"));
        assert_eq!(output(&dir, &["show", ":a.rs"]).unwrap(), "three");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn precommit_apply_refuses_mixed_staged_unstaged_file() {
        let dir = temp_repo("precommit-mixed");
        commit(&dir, "a.rs", "one\n");
        write(&dir, "a.rs", "two\n");
        git_ok(&dir, &["add", "a.rs"]);
        write(&dir, "a.rs", "three\n");
        let before = status_snapshot(&dir).unwrap();

        write(&dir, "a.rs", "four\n");
        let after = status_snapshot(&dir).unwrap();
        let plan = precommit_stage_plan(&before, &after);
        let step = apply_precommit_stage_plan(&dir, &plan);

        assert!(!step.ok);
        assert!(
            step.detail
                .as_deref()
                .unwrap()
                .contains("pre-existing mixed")
        );
        assert_eq!(output(&dir, &["show", ":a.rs"]).unwrap(), "two");
        assert_eq!(std::fs::read_to_string(dir.join("a.rs")).unwrap(), "four\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn precommit_apply_refuses_unstaged_only_file() {
        let dir = temp_repo("precommit-unstaged");
        commit(&dir, "a.rs", "one\n");
        let before = status_snapshot(&dir).unwrap();

        write(&dir, "a.rs", "two\n");
        let after = status_snapshot(&dir).unwrap();
        let plan = precommit_stage_plan(&before, &after);
        let step = apply_precommit_stage_plan(&dir, &plan);

        assert!(!step.ok);
        assert!(
            step.detail
                .as_deref()
                .unwrap()
                .contains("will not add work the user did not stage")
        );
        assert!(
            output(&dir, &["diff", "--cached", "--name-only"])
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn precommit_apply_refuses_new_untracked_file_without_staging_old_untracked() {
        let dir = temp_repo("precommit-untracked");
        commit(&dir, "tracked.rs", "one\n");
        write(&dir, "old.tmp", "old\n");
        let before = status_snapshot(&dir).unwrap();

        write(&dir, "new.tmp", "new\n");
        let after = status_snapshot(&dir).unwrap();
        let plan = precommit_stage_plan(&before, &after);
        let step = apply_precommit_stage_plan(&dir, &plan);

        assert!(!step.ok);
        assert!(step.detail.as_deref().unwrap().contains("new untracked"));
        assert!(
            output(&dir, &["diff", "--cached", "--name-only"])
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn precommit_apply_refuses_new_file_inside_preexisting_untracked_dir() {
        let dir = temp_repo("precommit-untracked-dir");
        commit(&dir, "tracked.rs", "one\n");
        write(&dir, "scratch/old.tmp", "old\n");
        let before = status_snapshot(&dir).unwrap();

        write(&dir, "scratch/new.tmp", "new\n");
        let after = status_snapshot(&dir).unwrap();
        let plan = precommit_stage_plan(&before, &after);
        let step = apply_precommit_stage_plan(&dir, &plan);

        assert!(!step.ok);
        assert!(step.detail.as_deref().unwrap().contains("scratch/new.tmp"));
        assert!(step.detail.as_deref().unwrap().contains("new untracked"));
        assert!(
            output(&dir, &["diff", "--cached", "--name-only"])
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
