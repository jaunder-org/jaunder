//! `validate`'s wasm size budget (#836).
//!
//! Reads the same measurement `audit-wasm` produces for the shipped artifact, so
//! the gate and the tool can never disagree about what the bundle weighs.

use anyhow::Result;

use crate::audit_wasm::{self, AuditReport};
use crate::nix_build;
use crate::result::{CommandResult, NixReport, StepResult};
use crate::wasm_budget;

const SITE_INSTALLABLE: &str = ".#site";

pub fn run(result: &mut CommandResult) {
    let before = nix_build::observe(SITE_INSTALLABLE);
    run_with(
        result,
        || audit_wasm::run(None),
        || before.finish(SITE_INSTALLABLE),
    );
}

pub(crate) fn run_with(
    result: &mut CommandResult,
    audit: impl FnOnce() -> Result<AuditReport>,
    nix_report: impl FnOnce() -> NixReport,
) {
    let report = match audit() {
        Ok(report) => report,
        Err(error) => {
            result.push(StepResult::fail("wasm-budget").detail(format!("{error:#}")));
            return;
        }
    };
    let nix = nix_report();
    let raw_bytes = report
        .artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with(".wasm"))
        .map(|artifact| artifact.raw_bytes);
    // Keep the measurement: this step already paid for a `nix build .#site`, so
    // discarding the sizes would waste it on a `--json` run.
    result.audit = Some(report);

    match raw_bytes {
        Some(raw_bytes) => {
            let verdict = wasm_budget::check(raw_bytes, wasm_budget::WASM_RAW_CEILING_BYTES);
            if verdict.over {
                result.push(
                    StepResult::fail("wasm-budget")
                        .detail(wasm_budget::failure_message(&verdict))
                        .nix(nix),
                );
            } else {
                // Report drift from the size #836 achieved, not just pass/fail. A
                // headroom budget's known weakness is that the win can erode
                // quietly inside the headroom; naming the drift is what makes that
                // erosion visible before it reaches the ceiling.
                let achieved = wasm_budget::WASM_RAW_ACHIEVED_BYTES;
                let drift = if verdict.actual >= achieved {
                    format!("+{}", verdict.actual - achieved)
                } else {
                    format!("-{}", achieved - verdict.actual)
                };
                result.push(
                    StepResult::ok("wasm-budget")
                        .detail(format!(
                            "{} raw bytes (ceiling {}, {drift} vs #836)",
                            verdict.actual, verdict.ceiling
                        ))
                        .nix(nix),
                );
            }
        }
        None => result.push(
            StepResult::fail("wasm-budget")
                .detail("audit-wasm reported no .wasm artifact")
                .nix(nix),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{SITE_INSTALLABLE, run_with};
    use crate::audit_wasm::{self, ArtifactMetrics, AuditReport};
    use crate::result::{CommandResult, NixRealization, NixReport};

    fn report_with_wasm(raw_bytes: u64) -> AuditReport {
        AuditReport {
            site_path: "/nix/store/site".to_owned(),
            artifacts: vec![ArtifactMetrics {
                path: "/nix/store/site/pkg/app.wasm".to_owned(),
                raw_bytes,
                gzip_bytes: 0,
                brotli_bytes: 0,
            }],
        }
    }

    fn nix_report(realization: NixRealization) -> NixReport {
        NixReport {
            installable: SITE_INSTALLABLE.to_owned(),
            derivation: Some("/nix/store/site.drv".to_owned()),
            realization,
        }
    }

    #[test]
    fn successful_wasm_budget_attaches_the_injected_site_report() {
        for realization in [
            NixRealization::Reused,
            NixRealization::Realized,
            NixRealization::Unknown,
        ] {
            let mut result = CommandResult::new("validate");
            run_with(
                &mut result,
                || Ok(report_with_wasm(1)),
                || nix_report(realization),
            );

            let step = result.steps.last().expect("wasm budget step");
            assert!(step.ok);
            assert_eq!(
                step.nix.as_ref().expect("site report").installable,
                SITE_INSTALLABLE
            );
            assert_eq!(
                step.nix.as_ref().expect("site report").realization,
                realization
            );
        }
    }

    #[test]
    fn failed_audit_preserves_the_existing_failure_without_nix_evidence() {
        let mut result = CommandResult::new("validate");
        run_with(
            &mut result,
            || Err(anyhow::anyhow!("nix build failed")),
            || panic!("failed audit must not observe after completion"),
        );

        let step = result.steps.last().expect("wasm budget step");
        assert!(!step.ok);
        assert_eq!(step.detail.as_deref(), Some("nix build failed"));
        assert!(step.nix.is_none());
    }

    #[test]
    fn explicit_audit_path_is_not_a_gate_step() {
        assert_eq!(
            audit_wasm::resolve_site_path(Some("/nix/store/prebuilt-site")).unwrap(),
            "/nix/store/prebuilt-site"
        );
    }
}
