//! Git helpers for the verify gate: working-tree cleanliness (the `validate`
//! backstop) and self-healing `core.hooksPath` installation.

use anyhow::{Context, Result};
use xshell::{cmd, Shell};

/// Repo-relative hooks directory the gate routes git to. Relative (not absolute)
/// so each worktree resolves to its own `.githooks` checkout.
pub const HOOKS_PATH: &str = ".githooks";

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
pub fn working_tree_status(sh: &Shell) -> Result<String> {
    cmd!(sh, "git status --porcelain")
        .quiet()
        .read()
        .context("running `git status --porcelain`")
}

/// Current `core.hooksPath`, or `None` when unset/blank. `--get` exits non-zero when
/// the key is missing, so the status is ignored and an empty read maps to `None`.
pub fn hooks_path(sh: &Shell) -> Option<String> {
    cmd!(sh, "git config --get core.hooksPath")
        .quiet()
        .ignore_status()
        .read()
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Ensure `core.hooksPath` points at [`HOOKS_PATH`]; set it if unset/wrong. Returns
/// `true` when it changed the config.
pub fn ensure_hooks_path(sh: &Shell) -> Result<bool> {
    if needs_hooks_path(hooks_path(sh).as_deref()) {
        cmd!(sh, "git config core.hooksPath {HOOKS_PATH}")
            .quiet()
            .run()
            .context("setting core.hooksPath")?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
