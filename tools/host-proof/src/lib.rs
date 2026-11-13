//! Focused host proof: the direct `test-local`, `e2e-local` and `pg-run`
//! executables and the private host lifecycle algorithms they share
//! (ADR-0246). Each executable prints one JSON report on stdout.

pub mod checkout;
pub mod compile_cache;
pub mod e2e;
pub mod e2e_evidence;
mod e2e_ownership;
mod e2e_provenance;
pub mod git {
    pub use repository_policy::git::{at, toplevel};
}
pub mod invocation_trace;
pub mod nix_build;
pub mod playwright_report;
pub mod report;
mod server_fn_coverage;
mod sh;
#[cfg(test)]
mod test_support;
pub mod traces;

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
}
