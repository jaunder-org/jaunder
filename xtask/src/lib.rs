mod adr;
mod adr_readme;
mod audit_wasm;
mod cache_safety;
mod census;
mod cli;
mod compile_cache;
pub mod coverage;
mod digest;
mod dispatch;
pub use dispatch::artifact_verdict;
mod doc_links;
pub mod e2e_evidence;
pub mod e2e_ownership;
mod e2e_provenance;
pub mod elisp_coverage;
mod files;
pub mod git;
mod host_tooling;
mod ids;
pub mod invocation_trace;
mod lifecycle;
pub mod markers;
mod mutants;
mod nix_build;
mod nix_probe;
mod performance;
mod playwright_report;
pub mod pr;
pub mod production_baseline;
mod production_baseline_lifecycle;
mod qualification;
mod qualification_csr;
mod qualification_inventory;
mod qualification_run;
mod qualification_tooling;
mod result;
mod server_fn_coverage;
mod server_fns;
mod sh;
#[cfg(test)]
mod test_support;
mod traces;
mod wasm_budget;
mod wasm_coverage;
mod wasm_sections;
mod wasm_symbols;
mod web_server_fns;
mod steps {
    pub mod adr_check;
    pub mod boot_decomposition_coverage;
    pub mod check_leaf;
    pub mod common_host_target_closure;
    pub mod csr_prerequisite;
    pub mod doc_links;
    pub mod doctest_fences;
    pub mod duration_budget;
    pub mod e2e_collector_config_check;
    pub mod e2e_goto_wrapper_check;
    pub mod e2e_local;
    pub mod e2e_scaffold_check;
    pub mod e2e_server_fn_endpoint_check;
    pub mod e2e_telemetry_boundary_check;
    pub mod flow_docs;
    pub mod host_server;
    mod host_signals;
    pub mod html_sink_check;
    pub mod ident_gate;
    pub mod lint_suppression_check;
    pub mod nix;
    mod process;
    pub mod proffered_secret_check;
    pub mod raw_html_door_check;
    pub mod sandbox;
    pub mod scan;
    pub mod sequence_check;
    pub mod server_fn_coverage_check;
    pub mod server_fn_registrar_check;
    pub mod server_fn_tracing_check;
    pub mod server_fn_wire_arg_error_check;
    pub mod sqlx_newtype_bind_check;
    pub mod sqlx_newtype_decode_check;
    pub mod target_arch_placement_check;
    pub mod test_local;
    pub mod test_pattern_check;
    pub mod thin_components;
    pub mod traced_context_check;
    pub mod wasm_budget;
    pub mod write_transaction_contract_check;
    pub mod xlang_literal_check;
}

pub use cli::{
    AdrCommand, CacheSafetyCommand, Cli, Command, CoverageCommand, E2eBackend, E2eBrowser,
    MutantsCommand, NixCommand, PerformanceBackend, PerformanceBrowser, PerformanceCommand,
    PerformanceProfile, PerformanceRunArgs, SandboxProfile, ServerFnCoverageCommand, TracesCommand,
    WasmCoverageCommand,
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

/// Run precommit's narrow Git/index safety wrapper without a result envelope.
pub fn run_precommit_safety_process() -> Result<(), String> {
    prepare_checkout().map_err(|error| format!("{error:#}"))?;
    ensure_hooks_installed();
    lifecycle::run_precommit_safety(std::path::Path::new("."))
}
