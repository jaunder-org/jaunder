//! `validate`'s wasm size budget (#836).
//!
//! Reads the same measurement `audit-wasm` produces for the shipped artifact, so
//! the gate and the tool can never disagree about what the bundle weighs.

use crate::result::{CommandResult, StepResult};

pub fn run(result: &mut CommandResult) {
    let report = match crate::audit_wasm::run(None) {
        Ok(r) => r,
        Err(e) => {
            result.push(StepResult::fail("wasm-budget").detail(format!("{e:#}")));
            return;
        }
    };

    let raw_bytes = report
        .artifacts
        .iter()
        .find(|a| a.path.ends_with(".wasm"))
        .map(|a| a.raw_bytes);
    // Keep the measurement: this step already paid for a `nix build .#site`, so
    // discarding the sizes would waste it on a `--json` run.
    result.audit = Some(report);

    match raw_bytes {
        Some(raw_bytes) => {
            let verdict =
                crate::wasm_budget::check(raw_bytes, crate::wasm_budget::WASM_RAW_CEILING_BYTES);
            if verdict.over {
                result.push(
                    StepResult::fail("wasm-budget")
                        .detail(crate::wasm_budget::failure_message(&verdict)),
                );
            } else {
                // Report drift from the size #836 achieved, not just pass/fail. A
                // headroom budget's known weakness is that the win can erode
                // quietly inside the headroom; naming the drift is what makes that
                // erosion visible before it reaches the ceiling.
                let achieved = crate::wasm_budget::WASM_RAW_ACHIEVED_BYTES;
                let drift = if verdict.actual >= achieved {
                    format!("+{}", verdict.actual - achieved)
                } else {
                    format!("-{}", achieved - verdict.actual)
                };
                result.push(StepResult::ok("wasm-budget").detail(format!(
                    "{} raw bytes (ceiling {}, {drift} vs #836)",
                    verdict.actual, verdict.ceiling
                )));
            }
        }
        None => result
            .push(StepResult::fail("wasm-budget").detail("audit-wasm reported no .wasm artifact")),
    }
}
