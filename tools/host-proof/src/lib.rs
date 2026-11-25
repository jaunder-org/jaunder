//! Focused host proof (ADR-0246): artifact-only e2e verdicts, host
//! diagnostics, and the CSR input checks host tests depend on. Each verdict
//! executable prints one JSON report on stdout. The `e2e-local` lifecycle is
//! `dev e2e-local` in `tools/dev` (#2066).

pub mod audit_wasm;
pub mod cache_safety;
pub mod checkout;
pub use host_tool::report;
pub mod e2e;
pub mod e2e_evidence;
mod e2e_ownership;
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
    pub mod server_fn_coverage_check;
    pub mod wasm_budget;
}
