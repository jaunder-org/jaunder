//! Focused host proof: the direct `test-local`, `e2e-local` and `pg-run`
//! executables and the private host lifecycle algorithms they share
//! (ADR-0246). Each executable prints one JSON report on stdout.

pub mod audit_wasm;
pub mod cache_safety;
pub mod checkout;
pub use host_tool::{compile_cache, report};
pub mod e2e;
pub mod e2e_evidence;
mod e2e_ownership;
mod e2e_provenance;
pub mod git {
    pub use repository_policy::git::{at, toplevel};
}
pub mod invocation_trace;
pub mod nix_build;
mod nix_eval;
pub mod nix_probe;
pub mod playwright_report;
mod server_fn_coverage;
mod sh;
#[cfg(test)]
mod test_support;
pub mod traces;
mod wasm_budget;
mod wasm_sections;
mod wasm_symbols;

pub mod steps {
    pub mod boot_decomposition_coverage;
    pub mod csr_prerequisite;
    pub mod duration_budget;
    pub mod e2e_local;
    pub mod host_server;
    mod host_signals;
    mod process;
    pub mod server_fn_coverage_check;
    pub mod test_local;
    pub mod wasm_budget;
}
