//! Git helpers for the verify gate: working-tree cleanliness (the `validate`
//! backstop) and self-healing `core.hooksPath` installation.

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
/// place the capture-and-check plumbing (formerly `adr::git_out`) lives.
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

/// `git mv <from> <to>`.
pub(crate) fn mv(dir: &Path, from: &str, to: &str) -> Result<()> {
    run(dir, &["mv", from, to])
}

/// `git add <path>`.
pub(crate) fn add(dir: &Path, path: &str) -> Result<()> {
    run(dir, &["add", path])
}

/// `git rev-parse --show-toplevel` — the working tree's root.
pub(crate) fn toplevel(dir: &Path) -> Result<String> {
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

    /// A fresh git repo under a pid-scoped temp dir, identity configured.
    fn temp_repo(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("jaunder-git-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "t@t"],
            &["config", "user.name", "t"],
        ] {
            assert!(at(&dir).args(args).status().unwrap().success());
        }
        dir
    }

    /// Write `rel` under `dir`, then `git add` + `git commit` it.
    fn commit(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
        assert!(at(dir).args(["add", rel]).status().unwrap().success());
        assert!(at(dir)
            .args(["commit", "-qm", "c"])
            .status()
            .unwrap()
            .success());
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
        assert!(at(&dir)
            .args(["checkout", "-q", "-b", "feature"])
            .status()
            .unwrap()
            .success());
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
}
