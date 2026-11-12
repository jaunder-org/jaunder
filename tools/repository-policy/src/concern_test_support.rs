//! Shared source-policy fixtures reuse the existing scrubbed Git fixture owner.
pub(crate) use crate::test_support::{TestRepo, commit, git_ok, temp_repo, write};

/// Deliberate negative fixture for the retired server attribute spelling.
pub fn retired_server_fn(attr_args: &str, item: &str) -> String {
    format!("#[{}{attr_args}]\n{item}\n", "server")
}
