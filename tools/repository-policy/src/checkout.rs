//! Policy executables admit the caller's checkout through `host-tool`, with this
//! package's own Nix source predicate.

/// Native Cargo reads live source; Git-backed Nix outputs additionally refuse omitted inputs.
pub fn prepare_policy_checkout(owner: &str) -> anyhow::Result<()> {
    host_tool::checkout::prepare_checkout(
        "tools/Cargo.toml",
        option_env!("JAUNDER_REPOSITORY_POLICY_SOURCE"),
    )?;
    host_tool::checkout::ensure_hooks_installed(owner);
    Ok(())
}
