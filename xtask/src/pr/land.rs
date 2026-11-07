//! Native arming transport and verification predicate retained for the privileged
//! ADR promoter only. Ordinary landing is owned by the Just frontdoor.

use super::Subject;
use super::gh::{self, ApiError};

/// Arming is a mutation, so it is a separate capability from observing: no
/// `PrSource` can merge anything, whatever a future caller passes it.
pub trait PrArmer {
    fn arm_auto_merge(&self, subject: &Subject, expected_head: &str) -> Result<(), ApiError>;
}

/// GitHub represents a successful arm in either of two ways: a pending
/// auto-merge request or immediate membership in the live merge queue.
///
/// Keep this predicate shared with privileged automation so every caller
/// verifies the same GitHub factors after `gh pr merge --auto`.
pub(super) fn arm_is_verified(auto_merge_armed: bool, in_merge_queue: bool) -> bool {
    auto_merge_armed || in_merge_queue
}

pub struct GhArmer;

impl PrArmer for GhArmer {
    fn arm_auto_merge(&self, subject: &Subject, expected_head: &str) -> Result<(), ApiError> {
        // `run_gh_raw`, not `run_gh`: this prints a human sentence rather than JSON,
        // so parsing stdout would classify every *successful* arm as malformed. Its
        // output is not evidence either way — the next snapshot is.
        let number = subject.number.to_string();
        let repo = format!("{}/{}", subject.owner, subject.repo);
        gh::run_gh_raw(&merge_args(&number, &repo, expected_head))
    }
}

// Condition the mutation itself: a matching read cannot close the read/write race.
fn merge_args<'a>(number: &'a str, repo: &'a str, expected_head: &'a str) -> [&'a str; 9] {
    [
        "pr",
        "merge",
        number,
        "--repo",
        repo,
        "--auto",
        "--merge",
        "--match-head-commit",
        expected_head,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_merge_request_conditions_the_mutation_on_the_approved_head() {
        assert_eq!(
            merge_args("731", "owner/repo", "approved-A"),
            [
                "pr",
                "merge",
                "731",
                "--repo",
                "owner/repo",
                "--auto",
                "--merge",
                "--match-head-commit",
                "approved-A",
            ]
        );
    }
}
