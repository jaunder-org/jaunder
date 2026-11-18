//! `validate`'s wasm size budget (#836).
//!
//! Reads the same measurement `audit-wasm` produces for the shipped artifact, so
//! the gate and the tool can never disagree about what the bundle weighs. The
//! caller builds the bundle and names its root; the gate never runs Nix.

use anyhow::Result;

use crate::audit_wasm::{self, AuditReport};
use crate::report::{CommandResult, StepResult};
use crate::wasm_budget;

pub fn run(result: &mut CommandResult, site_path: &str) {
    run_with(result, site_path, audit_wasm::run);
}

pub(crate) fn run_with(
    result: &mut CommandResult,
    site_path: &str,
    audit: impl FnOnce(&str) -> Result<AuditReport>,
) {
    let step = match audit(site_path) {
        Err(error) => StepResult::fail("wasm-budget").detail(format!("{error:#}")),
        Ok(report) => {
            let raw_bytes = report
                .artifacts
                .iter()
                .find(|artifact| artifact.path.ends_with(".wasm"))
                .map(|artifact| artifact.raw_bytes);
            match raw_bytes {
                Some(raw_bytes) => {
                    let verdict =
                        wasm_budget::check(raw_bytes, wasm_budget::WASM_RAW_CEILING_BYTES);
                    if verdict.over {
                        StepResult::fail("wasm-budget")
                            .detail(wasm_budget::failure_message(&verdict))
                    } else {
                        // Report drift from the size #1549 achieved, not just pass/fail. A
                        // headroom budget's known weakness is that the win can erode
                        // quietly inside the headroom; naming the drift is what makes that
                        // erosion visible before it reaches the ceiling.
                        let achieved = wasm_budget::WASM_RAW_ACHIEVED_BYTES;
                        let drift = if verdict.actual >= achieved {
                            format!("+{}", verdict.actual - achieved)
                        } else {
                            format!("-{}", achieved - verdict.actual)
                        };
                        StepResult::ok("wasm-budget").detail(format!(
                            "{} raw bytes (ceiling {}, {drift} vs #1549)",
                            verdict.actual, verdict.ceiling
                        ))
                    }
                }
                None => {
                    StepResult::fail("wasm-budget").detail("audit-wasm reported no .wasm artifact")
                }
            }
        }
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::run_with;
    use crate::audit_wasm::{ArtifactMetrics, AuditReport};
    use crate::report::CommandResult;
    use crate::wasm_budget::WASM_RAW_CEILING_BYTES;

    fn report_with_wasm(raw_bytes: u64) -> AuditReport {
        AuditReport {
            site_path: "/fixture/csr-bundle".to_owned(),
            artifacts: vec![ArtifactMetrics {
                path: "/fixture/csr-bundle/pkg/app.wasm".to_owned(),
                raw_bytes,
                gzip_bytes: 0,
                brotli_bytes: 0,
            }],
        }
    }

    #[test]
    fn wasm_budget_verdict_has_no_discarded_report_metadata() {
        for bytes in [1, WASM_RAW_CEILING_BYTES, WASM_RAW_CEILING_BYTES + 1] {
            let mut result = CommandResult::new("check-leaf");
            run_with(&mut result, "/fixture/csr-bundle", |path| {
                assert_eq!(path, "/fixture/csr-bundle");
                Ok(report_with_wasm(bytes))
            });
            let step = result.steps.last().expect("wasm budget step");
            assert_eq!(step.ok, bytes <= WASM_RAW_CEILING_BYTES);
            assert!(step.detail.as_ref().unwrap().contains(&bytes.to_string()));
            assert!(result.evidence.is_none());
        }
    }

    #[test]
    fn failed_audit_keeps_the_original_diagnostic() {
        let mut result = CommandResult::new("check-leaf");
        run_with(&mut result, "/fixture/csr-bundle", |_| {
            Err(anyhow::anyhow!("reading artifact failed"))
        });
        let step = result.steps.last().unwrap();
        assert!(!step.ok);
        assert_eq!(step.detail.as_deref(), Some("reading artifact failed"));
    }

    #[test]
    fn missing_wasm_is_not_a_passing_budget() {
        let mut result = CommandResult::new("check-leaf");
        run_with(&mut result, "/fixture/csr-bundle", |_| {
            let mut report = report_with_wasm(1);
            report.artifacts.clear();
            Ok(report)
        });
        assert!(!result.ok);
        assert_eq!(
            result.steps[0].detail.as_deref(),
            Some("audit-wasm reported no .wasm artifact")
        );
    }
}
