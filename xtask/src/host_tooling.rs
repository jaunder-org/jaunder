//! Shared checkout admission for retained xtask consumers.
use anyhow::Result;

/// Preserve the selected xtask source predicate before hook preparation.
pub fn prepare_checkout() -> Result<()> {
    repository_policy::checkout::prepare_checkout(
        "xtask/Cargo.toml",
        (!env!("JAUNDER_XTASK_NIX_OUTPUT").is_empty()).then_some(".#xtask.sourceAdmits"),
    )
}
