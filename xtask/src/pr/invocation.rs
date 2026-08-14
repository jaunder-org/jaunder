use std::io::Write;

use super::watch;
use crate::git;
use anyhow::{Context, Result};

/// What the local repository says about where the caller is standing.
///
/// Passed in rather than read inside, so the divergence guard and the exit-2 messages
/// are reachable from a test instead of only from a real checkout.
#[derive(Debug, Clone, Default)]
pub struct GitFacts {
    pub branch: Option<String>,
    pub head_sha: Option<String>,
}

impl GitFacts {
    pub(super) fn read(dir: &std::path::Path, landing: bool) -> Result<Self> {
        Self::read_for_mode_with(
            dir,
            landing,
            || git::current_branch(dir),
            || git::head_sha(dir),
            &mut std::io::stderr(),
        )
    }

    fn read_for_mode_with(
        dir: &std::path::Path,
        landing: bool,
        branch: impl FnOnce() -> anyhow::Result<Option<String>>,
        head_sha: impl FnOnce() -> anyhow::Result<Option<String>>,
        stderr: &mut impl Write,
    ) -> Result<Self> {
        if landing {
            let branch = branch().with_context(|| {
                format!("reading local Git branch at {} for pr land", dir.display())
            })?;
            let head_sha = head_sha().with_context(|| {
                format!("reading local Git HEAD at {} for pr land", dir.display())
            })?;
            return Ok(Self { branch, head_sha });
        }

        let mut failed = false;
        let branch = branch().unwrap_or_else(|_| {
            failed = true;
            None
        });
        let head_sha = head_sha().unwrap_or_else(|_| {
            failed = true;
            None
        });
        if failed {
            let _ = writeln!(
                stderr,
                "xtask: warning: xtask.pr.git_facts: ignored failure while reading optional local Git facts"
            );
        }
        Ok(Self { branch, head_sha })
    }
}

/// One invocation's inputs, as a value rather than a parameter list.
pub struct Invocation<'a> {
    pub git: &'a GitFacts,
    pub number: Option<u64>,
    pub cfg: watch::WatchConfig,
    /// `true` for `pr land` — the only mode that may mutate anything.
    pub landing: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancillary_warning_git_facts_failures_preserve_optional_facts_and_json() {
        for branch_fails in [true, false] {
            let primary = crate::result::CommandResult::new("pr-watch");
            let before = serde_json::to_string(&primary).unwrap();
            let mut stderr = Vec::new();
            let facts = GitFacts::read_for_mode_with(
                std::path::Path::new("/repo"),
                false,
                || {
                    if branch_fails {
                        anyhow::bail!("sensitive branch")
                    }
                    Ok(Some("main".to_owned()))
                },
                || {
                    if branch_fails {
                        Ok(Some("abc123".to_owned()))
                    } else {
                        anyhow::bail!("sensitive sha")
                    }
                },
                &mut stderr,
            )
            .unwrap();
            assert_eq!(facts.branch.as_deref(), (!branch_fails).then_some("main"));
            assert_eq!(facts.head_sha.as_deref(), branch_fails.then_some("abc123"));
            assert_eq!(serde_json::to_string(&primary).unwrap(), before);
            let warning = String::from_utf8(stderr).unwrap();
            assert_eq!(warning.matches("xtask.pr.git_facts").count(), 1);
            assert_eq!(warning.lines().count(), 1);
            assert!(!warning.contains("sensitive"));
        }
    }

    fn io_error_kind(error: &(dyn std::error::Error + 'static)) -> Option<std::io::ErrorKind> {
        let mut current = Some(error);
        while let Some(error) = current {
            if let Some(error) = error.downcast_ref::<std::io::Error>() {
                return Some(error.kind());
            }
            current = error.source();
        }
        None
    }

    #[test]
    fn pr_land_git_fact_failures_are_typed_before_dispatch() {
        for branch_fails in [true, false] {
            let mut stderr = Vec::new();
            let error = GitFacts::read_for_mode_with(
                std::path::Path::new("/repo"),
                true,
                || {
                    if branch_fails {
                        return Err(anyhow::Error::new(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "injected branch",
                        )));
                    }
                    Ok(Some("topic".to_owned()))
                },
                || {
                    Err(anyhow::Error::new(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "injected head",
                    )))
                },
                &mut stderr,
            )
            .unwrap_err();
            let detail = format!("{error:#}");
            assert!(detail.contains("/repo"));
            assert!(detail.contains(if branch_fails {
                "local Git branch"
            } else {
                "local Git HEAD"
            }));
            assert_eq!(
                io_error_kind(error.root_cause()),
                Some(std::io::ErrorKind::PermissionDenied)
            );
            assert!(
                stderr.is_empty(),
                "land failures must not continue with a warning"
            );
        }
    }
}
