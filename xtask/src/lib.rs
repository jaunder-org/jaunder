mod audit_wasm;
mod cache_safety;
mod cli;
pub mod coverage;
mod digest;
mod dispatch;
pub mod elisp_coverage;
pub mod git;
mod host_tooling;
mod lifecycle;
pub use repository_policy::markers;
mod mutants;
mod nix_probe;
mod result;
#[cfg(test)]
mod test_support;
mod wasm_budget;
mod wasm_coverage;
mod wasm_sections;
mod wasm_symbols;

mod steps {
    pub mod check_leaf;
    pub mod doctest_fences;
    pub mod nix;
    pub mod wasm_budget;
}

pub use cli::{
    CacheSafetyCommand, Cli, Command, CoverageCommand, E2eBackend, E2eBrowser, MutantsCommand,
    NixCommand, WasmCoverageCommand,
};
pub use dispatch::run;
pub use host_tooling::prepare_checkout;
pub use lifecycle::ensure_hooks_installed;
pub use result::{CommandResult, StepResult};

/// Run one check/safety adapter through the plain process-status boundary Just uses.
pub fn run_check_leaf_process(name: &str) -> Result<StepResult, String> {
    prepare_checkout().map_err(|error| format!("{error:#}"))?;
    ensure_hooks_installed();
    let start = std::time::Instant::now();
    let step = steps::check_leaf::run(name).map_err(|error| format!("{error:#}"))?;
    Ok(step.with_duration(start.elapsed()))
}
