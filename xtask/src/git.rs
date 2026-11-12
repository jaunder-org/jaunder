//! Shared native Git evidence and checkout helpers for retained xtask consumers.
pub use repository_policy::git::{
    GitPathStatus, GitStatusSnapshot, HOOKS_PATH, PrecommitBroadReason, PrecommitChangeClass,
    PrecommitStagePlan, SharedVerdictGitState, at, classify_precommit_change, config_get,
    config_set, current_branch, ensure_hooks_path, head_sha, hooks_path, lines, ls_files_md,
    needs_hooks_path, output, parse_status_snapshot, porcelain_is_dirty, precommit_stage_plan,
    remote_url, run, shared_markdown_admission, shared_static_docs_clean_admission,
    shared_verdict_git_state, shared_verdict_state_unchanged, status_snapshot, toplevel,
    tracked_files, working_tree_status,
};
