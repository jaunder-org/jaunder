mod audit_wasm;
mod cache_safety;
mod cli;
mod compile_cache;
pub mod coverage;
mod digest;
mod dispatch;
pub use dispatch::artifact_verdict;
pub mod e2e_evidence;
pub mod e2e_ownership;
mod e2e_provenance;
pub mod elisp_coverage;
pub mod git;
mod host_tooling;
pub mod invocation_trace;
mod lifecycle;
pub use repository_policy::markers;
mod mutants;
mod nix_build;
mod nix_probe;
mod playwright_report;
pub mod production_baseline;
mod production_baseline_lifecycle;
mod qualification;
mod qualification_csr;
mod qualification_inventory;
mod qualification_run;
mod qualification_tooling;
mod result;
mod server_fn_coverage;
pub(crate) use repository_policy::server_fns;
mod sh;
#[cfg(test)]
mod test_support;
mod traces;
mod wasm_budget;
mod wasm_coverage;
mod wasm_sections;
mod wasm_symbols;

mod steps {
    pub mod boot_decomposition_coverage;
    pub mod check_leaf;
    pub mod csr_prerequisite;
    pub mod doctest_fences;
    pub mod duration_budget;
    pub mod e2e_local;
    pub mod host_server;
    mod host_signals;
    pub mod nix;
    mod process;
    pub mod server_fn_coverage_check;
    pub mod test_local;
    pub mod wasm_budget;
}

pub use cli::{
    CacheSafetyCommand, Cli, Command, CoverageCommand, E2eBackend, E2eBrowser, MutantsCommand,
    NixCommand, ServerFnCoverageCommand, TracesCommand, WasmCoverageCommand,
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
