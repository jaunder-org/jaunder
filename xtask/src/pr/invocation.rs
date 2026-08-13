use super::watch;
use crate::git;

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
    pub(super) fn read(dir: &std::path::Path) -> Self {
        Self {
            branch: git::current_branch(dir).ok().flatten(),
            head_sha: git::head_sha(dir).ok().flatten(),
        }
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
