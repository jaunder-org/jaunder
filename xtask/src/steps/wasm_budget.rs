//! `validate`'s wasm size budget (#836).
//!
//! Reads the same measurement `audit-wasm` produces for the shipped artifact, so
//! the gate and the tool can never disagree about what the bundle weighs.

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::audit_wasm::{self, AuditReport};
use crate::nix_build;
use crate::result::{CommandResult, NixReport, StepResult};
use crate::wasm_budget;

const SITE_INSTALLABLE: &str = ".#site";

pub fn run(result: &mut CommandResult) {
    let start = Instant::now();
    let before = nix_build::observe(SITE_INSTALLABLE);
    run_with(
        result,
        || audit_wasm::resolve_site_path(None),
        |site_path| audit_wasm::run(Some(site_path)),
        || before.finish(SITE_INSTALLABLE),
        || start.elapsed(),
    );
}

pub(crate) fn run_with(
    result: &mut CommandResult,
    build: impl FnOnce() -> Result<String>,
    audit: impl FnOnce(&str) -> Result<AuditReport>,
    nix_report: impl FnOnce() -> NixReport,
    elapsed: impl FnOnce() -> Duration,
) {
    let site_path = match build() {
        Ok(site_path) => site_path,
        Err(error) => {
            result.push(
                StepResult::fail("wasm-budget")
                    .detail(format!("{error:#}"))
                    .with_duration(elapsed()),
            );
            return;
        }
    };
    let nix = nix_report();
    let step = match audit(&site_path) {
        Err(error) => StepResult::fail("wasm-budget").detail(format!("{error:#}")),
        Ok(report) => {
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
                    let verdict =
                        wasm_budget::check(raw_bytes, wasm_budget::WASM_RAW_CEILING_BYTES);
                    if verdict.over {
                        StepResult::fail("wasm-budget")
                            .detail(wasm_budget::failure_message(&verdict))
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
                        StepResult::ok("wasm-budget").detail(format!(
                            "{} raw bytes (ceiling {}, {drift} vs #836)",
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
    result.push(step.nix(nix).with_duration(elapsed()));
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

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
                || Ok("/nix/store/site".to_owned()),
                |_| Ok(report_with_wasm(1)),
                || nix_report(realization),
                || Duration::from_millis(321),
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
            assert_eq!(step.duration_ms, 321);
        }
    }

    #[test]
    fn failed_build_preserves_the_existing_failure_without_nix_evidence() {
        let mut result = CommandResult::new("validate");
        run_with(
            &mut result,
            || Err(anyhow::anyhow!("nix build failed")),
            |_| panic!("failed build must not audit artifacts"),
            || panic!("failed build must not observe after completion"),
            || Duration::from_millis(123),
        );

        let step = result.steps.last().expect("wasm budget step");
        assert!(!step.ok);
        assert_eq!(step.detail.as_deref(), Some("nix build failed"));
        assert_eq!(step.duration_ms, 123);
        assert!(step.nix.is_none());
    }

    #[test]
    fn post_build_audit_failure_retains_nix_evidence() {
        let mut result = CommandResult::new("validate");
        run_with(
            &mut result,
            || Ok("/nix/store/site".to_owned()),
            |_| Err(anyhow::anyhow!("reading artifact failed")),
            || nix_report(NixRealization::Realized),
            || Duration::from_millis(456),
        );

        let step = result.steps.last().expect("wasm budget step");
        assert!(!step.ok);
        assert_eq!(step.detail.as_deref(), Some("reading artifact failed"));
        assert_eq!(step.duration_ms, 456);
        assert_eq!(
            step.nix.as_ref().expect("site report").realization,
            NixRealization::Realized
        );
    }

    #[test]
    fn explicit_audit_path_is_not_a_gate_step() {
        assert_eq!(
            audit_wasm::resolve_site_path(Some("/nix/store/prebuilt-site")).unwrap(),
            "/nix/store/prebuilt-site"
        );
    }
}
