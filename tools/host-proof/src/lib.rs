//! Focused host proof: the direct `test-local`, `e2e-local` and `pg-run`
//! executables and the private host lifecycle algorithms they share
//! (ADR-0246). Each executable prints one JSON report on stdout.

pub mod checkout;
pub mod compile_cache;
mod e2e_provenance;
pub mod git {
    pub use repository_policy::git::{at, toplevel};
}
pub mod invocation_trace;
pub mod nix_build;
pub mod report;
mod sh;
#[cfg(test)]
mod test_support;

pub mod steps {
    pub mod csr_prerequisite;
    pub mod e2e_local;
    pub mod host_server;
    mod host_signals;
    mod process;
    pub mod test_local;
}
