//! Single-call boundary for existing checks, safety owners and residual adapters.
//!
//! Entries include substantive source/evidence algorithms, small textual/docs
//! policies, and execution/consumer glue; existing Rust code is not evidence that
//! every entry requires Rust or belongs permanently in xtask. Most owners still
//! accept CommandResult, adapted here to plain status/detail without serialization.
//! Just owns gate population, ordering and modes. Prepush docs retains raw/index
//! admission and mandatory post-request verification. The E2E workflow owns
//! authoritative flow verification; the static inventory remains a separate leaf.

use anyhow::{Result, bail, ensure};

use crate::{result::CommandResult, steps};

/// Run exactly one existing check or adapter, preserving its algorithm and leaf
/// diagnostic. An unknown name is a caller error rather than a skipped success.
pub(crate) fn run(name: &str) -> Result<crate::result::StepResult> {
    let mut result = CommandResult::new("check-leaf");
    match name {
        "wasm-budget" => steps::wasm_budget::run(&mut result),
        _ => bail!("unknown check leaf `{name}`"),
    }
    ensure!(
        result.steps.len() == 1,
        "check leaf `{name}` did not produce exactly one result"
    );
    Ok(result.steps.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrated_policy_leaves_are_not_compatibility_forwarders() {
        for name in [
            "flow-docs",
            "doc-links",
            "adr-format",
            "adr-readme-parity",
            "not-a-check",
        ] {
            assert!(run(name).is_err());
        }
    }
}
