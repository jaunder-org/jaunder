//! Focused host proof (ADR-0246): artifact-only e2e verdicts and host
//! diagnostics. Each verdict executable prints one JSON report on stdout. The
//! `e2e-local` lifecycle is `dev e2e-local` in `tools/dev` (#2066).

pub use host_tool::report;
pub mod e2e;
pub mod e2e_evidence;
mod e2e_ownership;
pub mod nix_build;
pub mod playwright_report;
mod server_fn_coverage;
#[cfg(test)]
mod test_support;
pub mod traces;

pub mod steps {
    pub mod boot_decomposition_coverage;
    pub mod duration_budget;
    pub mod server_fn_coverage_check;
}
