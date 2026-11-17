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
pub fn output(dir: &Path, args: &[&str]) -> Result<String> {
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
pub fn lines(dir: &Path, args: &[&str]) -> Result<Vec<String>> {
    Ok(output(dir, args)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect())
}

/// Run a git command in `dir` for effect (no capture); bail on non-zero exit.
pub fn run(dir: &Path, args: &[&str]) -> Result<()> {
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

/// `git ls-files -- *.md` — tracked Markdown, repo-relative. The pathspec glob
/// matches at any depth, so filtering happens in git rather than in Rust.
pub fn ls_files_md(dir: &Path) -> Result<Vec<String>> {
    lines(dir, &["ls-files", "--", "*.md"])
}

/// `git rev-parse --show-toplevel` — the working tree's root.
pub fn toplevel(dir: &Path) -> Result<String> {
    output(dir, &["rev-parse", "--show-toplevel"])
}

/// `git config --get <key>` → the value, or `None` when unset (exit 1) or blank.
/// Bails on any other non-zero (e.g. exit 128 = corrupt config): a broken config
/// surfaces as an error rather than being silently treated as "unset" (see
/// [`output_or`]).
pub fn config_get(dir: &Path, key: &str) -> Result<Option<String>> {
    Ok(output_or(dir, &["config", "--get", key], 1)?.filter(|s| !s.is_empty()))
}

/// `git config <key> <value>`.
pub fn config_set(dir: &Path, key: &str, value: &str) -> Result<()> {
    run(dir, &["config", key, value])
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::commit;

    fn temp_repo(tag: &str) -> crate::test_support::TestRepo {
        crate::test_support::temp_repo("git", tag)
    }

    #[test]
    fn output_returns_trimmed_stdout_and_bails_on_error() {
        let dir = temp_repo("output");
        commit(&dir, "a.txt", "x\n");
        let head = output(&dir, &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(head.len(), 40, "full sha, trimmed: {head:?}");
        assert!(output(&dir, &["not-a-subcommand"]).is_err());
    }

    #[test]
    fn lines_drops_blank_lines() {
        let dir = temp_repo("lines");
        commit(&dir, "a.txt", "1\n");
        commit(&dir, "b.txt", "2\n");
        let subjects = lines(&dir, &["log", "--format=%s"]).unwrap();
        assert_eq!(subjects, vec!["c".to_string(), "c".to_string()]);
    }

    #[test]
    fn run_ok_on_success_err_on_failure() {
        let dir = temp_repo("run");
        commit(&dir, "a.txt", "x\n");
        assert!(run(&dir, &["status", "--porcelain"]).is_ok());
        assert!(run(&dir, &["mv", "nope", "nowhere"]).is_err());
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
    }

    #[test]
    fn ensure_hooks_path_sets_then_is_noop() {
        let dir = temp_repo("ensure-hooks");
        assert!(ensure_hooks_path(&dir).unwrap(), "first call sets it");
        assert!(!ensure_hooks_path(&dir).unwrap(), "second call is a no-op");
        assert_eq!(hooks_path(&dir).unwrap(), Some(HOOKS_PATH.to_string()));
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
    fn git_at_scrubs_repo_redirecting_env() {
        // Regression guard: without scrubbing these, a git op meant for `dir`
        // (a throwaway test repo) would be redirected at the hook's repo when run
        // inside a git hook, corrupting it. `get_envs()` yields `(key, None)` for a
        // removed var.
        let cmd = at(std::path::Path::new("/tmp/x"));
        let removed: std::collections::HashSet<std::ffi::OsString> = cmd
            .get_envs()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.to_owned())
            .collect();
        for var in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_COMMON_DIR",
            "GIT_NAMESPACE",
        ] {
            assert!(
                removed.contains(std::ffi::OsStr::new(var)),
                "{var} must be scrubbed so -C wins"
            );
        }
    }
}
