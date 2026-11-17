//! Git helpers shared by the tools: a `git -C` that hook overrides cannot
//! redirect, tracked-file and config queries, and the `core.hooksPath`
//! installation of checkout admission. Index custody is `dev`'s
//! (`tools/dev/dev/index.py`), not this module's.

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

/// Whether `core.hooksPath` needs (re)pointing at [`HOOKS_PATH`], given its current
/// value (`None` = unset).
pub fn needs_hooks_path(current: Option<&str>) -> bool {
    match current {
        Some(value) => value.trim() != HOOKS_PATH,
        None => true,
    }
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
/// surfaces as an error rather than being silently treated as "unset".
pub fn config_get(dir: &Path, key: &str) -> Result<Option<String>> {
    let args = ["config", "--get", key];
    let out = at(dir)
        .args(args)
        .output()
        .with_context(|| format!("running git {args:?}"))?;
    match out.status.code() {
        Some(0) => {
            let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Ok((!value.is_empty()).then_some(value))
        }
        Some(1) => Ok(None),
        _ => anyhow::bail!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
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
    fn config_get_bails_on_a_broken_config_rather_than_reading_unset() {
        let dir = temp_repo("broken-config");
        std::fs::write(dir.join(".git/config"), "[core\n").unwrap();
        assert!(config_get(&dir, "core.hooksPath").is_err());
    }

    #[test]
    fn ensure_hooks_path_sets_then_is_noop() {
        let dir = temp_repo("ensure-hooks");
        assert!(ensure_hooks_path(&dir).unwrap(), "first call sets it");
        assert!(!ensure_hooks_path(&dir).unwrap(), "second call is a no-op");
        assert_eq!(hooks_path(&dir).unwrap(), Some(HOOKS_PATH.to_string()));
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
