//! Focused host proof: the `e2e-local` executable and its private host
//! lifecycle algorithms (ADR-0246), host diagnostics, and the CSR input checks
//! `dev test-local` runs as steps. Each verdict executable prints one JSON
//! report on stdout.

pub mod audit_wasm;
pub mod cache_safety;
pub mod checkout;
pub use host_tool::report;
pub mod e2e;
pub mod e2e_evidence;
mod e2e_ownership;
mod e2e_provenance;
pub mod git {
    pub use repository_policy::git::{at, toplevel};
}
pub mod nix_build;
pub mod playwright_report;
mod server_fn_coverage;
pub mod span_records;
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
    pub mod wasm_budget;
}
