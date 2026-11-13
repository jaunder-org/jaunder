use std::time::Instant;

use crate::{
    audit_wasm, cache_safety,
    cli::{
        CacheSafetyCommand, Cli, Command, CoverageCommand, MutantsCommand, NixCommand,
        WasmCoverageCommand,
    },
    coverage, lifecycle, nix_probe,
    result::{CommandResult, StepResult},
    steps, wasm_coverage,
};

/// Artifact-only commands bypass lifecycle reporting and all build hooks.
pub fn artifact_verdict(command: &Command) -> Option<Result<String, String>> {
    match command {
        Command::E2eVerify {
            lane,
            catalog,
            artifacts,
        } => Some(crate::e2e_evidence::verify_lane(lane, catalog, artifacts)),
        Command::E2eReconcile {
            backend,
            catalog,
            diagnostics_root,
        } => Some(crate::e2e_evidence::reconcile_files(
            backend.as_str(),
            catalog,
            diagnostics_root,
        )),
        Command::ServerFnCoverage(command) => {
            let (inputs, regenerate) = match command {
                crate::cli::ServerFnCoverageCommand::Verify(inputs) => (inputs, false),
                crate::cli::ServerFnCoverageCommand::Regenerate(inputs) => (inputs, true),
            };
            Some(
                steps::server_fn_coverage_check::from_producer(inputs, regenerate)
                    .map_err(|error| format!("{error:#}")),
            )
        }
        _ => None,
    }
}

pub fn run(cli: Cli) -> anyhow::Result<CommandResult> {
    // Reject --json for commands with no structured payload (the `traces` reporting
    // commands) before doing any work — a hollow envelope is worse than an error.
    if cli.json && !cli.command.produces_json_payload() {
        anyhow::bail!(
            "--json is not supported for `{}` (produces no structured output)",
            cli.command_name()
        );
    }
    match cli.command {
        Command::Mutants(MutantsCommand::VerifyEvidence {
            package,
            file,
            output,
            producer_status,
        }) => {
            let start = Instant::now();
            let report = crate::mutants::verify_evidence(&package, &file, &output, producer_status)
                .map_err(|error| error.context("verification failed"))?;
            let mut result = CommandResult::new("mutants-verify-evidence");
            result.push(StepResult::ok("mutants-verify-evidence").detail(report));
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::CheckLeaf { .. } => {
            anyhow::bail!(
                "internal leaf and precommit safety commands use the plain process boundary"
            )
        }
        Command::AuditWasm {
            site_path,
            breakdown,
            wasm,
        } => {
            let start = Instant::now();
            let mut result = CommandResult::new("audit-wasm");
            let step_start = Instant::now();
            if breakdown {
                match audit_wasm::breakdown(wasm.as_deref()) {
                    Ok(report) => {
                        let n = report.crates.len();
                        result.breakdown = Some(report);
                        result.push(
                            StepResult::ok("audit-wasm-breakdown")
                                .detail(format!("{n} crate(s) attributed"))
                                .with_duration(step_start.elapsed()),
                        );
                    }
                    Err(e) => {
                        result.push(
                            StepResult::fail("audit-wasm-breakdown")
                                .detail(format!("{e:#}"))
                                .with_duration(step_start.elapsed()),
                        );
                    }
                }
            } else {
                match audit_wasm::run(site_path.as_deref()) {
                    Ok(report) => {
                        let n = report.artifacts.len();
                        result.audit = Some(report);
                        result.push(
                            StepResult::ok("audit-wasm")
                                .detail(format!("{n} artifact(s)"))
                                .with_duration(step_start.elapsed()),
                        );
                    }
                    Err(e) => {
                        result.push(
                            StepResult::fail("audit-wasm")
                                .detail(format!("{e:#}"))
                                .with_duration(step_start.elapsed()),
                        );
                    }
                }
            }
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::E2eVerify { .. } | Command::E2eReconcile { .. } | Command::ServerFnCoverage(_) => {
            anyhow::bail!("artifact-input commands execute directly, without a CommandResult")
        }
        Command::CacheSafety(CacheSafetyCommand::Probe) => {
            let start = Instant::now();
            let mut result = CommandResult::new("cache-safety-probe");
            let step_start = Instant::now();
            result.push(cache_safety::probe().with_duration(step_start.elapsed()));
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Nix(NixCommand::ProbeSource) => {
            let start = Instant::now();
            let mut result = CommandResult::new("nix-probe-source");
            let step_start = Instant::now();
            result.push(nix_probe::probe_source().with_duration(step_start.elapsed()));
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::CoveragePolicy { out } => {
            let start = Instant::now();
            let mut result = CommandResult::new("coverage-policy");
            result.push(coverage::run(&out).0.with_duration(start.elapsed()));
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Coverage(CoverageCommand::ProbeSource) => {
            let start = Instant::now();
            let mut result = CommandResult::new("coverage-probe-source");
            let step_start = Instant::now();
            result.push(coverage::probe::probe_source().with_duration(step_start.elapsed()));
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Coverage(CoverageCommand::BenchmarkLocal { unloaded_system }) => {
            let start = Instant::now();
            let mut result = CommandResult::new("coverage-benchmark-local");
            let step_start = Instant::now();
            result.push(
                match coverage::benchmark::run(&unloaded_system) {
                    Ok(manifest) => StepResult::ok("coverage-benchmark-local").detail(format!(
                        "retained {} accepted observations at {}",
                        manifest.observations.len(),
                        coverage::benchmark::manifest_path(&manifest.revision).display()
                    )),
                    Err(error) => {
                        StepResult::fail("coverage-benchmark-local").detail(format!("{error:#}"))
                    }
                }
                .with_duration(step_start.elapsed()),
            );
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::WasmCoverage(WasmCoverageCommand::Probe) => {
            let start = Instant::now();
            let mut result = CommandResult::new("wasm-coverage-probe");
            let step_start = Instant::now();
            match wasm_coverage::probe() {
                Ok(aggregate) => {
                    let ok = aggregate.verdict == wasm_coverage::Verdict::Passed;
                    let detail = if ok {
                        "both browser reports map executed original Rust lines; merged evidence retained".to_owned()
                    } else {
                        aggregate.blockers.join("; ")
                    };
                    result.wasm_coverage = Some(aggregate);
                    result.push(
                        if ok {
                            StepResult::ok("wasm-coverage-probe")
                        } else {
                            StepResult::fail("wasm-coverage-probe")
                        }
                        .detail(detail)
                        .with_duration(step_start.elapsed()),
                    );
                }
                Err(error) => result.push(
                    StepResult::fail("wasm-coverage-probe")
                        .detail(format!("{error:#}"))
                        .with_duration(step_start.elapsed()),
                ),
            }
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::WasmCoverage(WasmCoverageCommand::Measure { quiescent_window }) => {
            let start = Instant::now();
            let mut result = CommandResult::new("wasm-coverage-measure");
            let step_start = Instant::now();
            result.push(match wasm_coverage::measure(&quiescent_window) {
                Ok(manifest) => StepResult::ok("wasm-coverage-measure").detail(format!(
                    "retained {} reconciled runs at .xtask/wasm-coverage/measurement/manifest-v1.json",
                    manifest.runs.len()
                )),
                Err(error) => StepResult::fail("wasm-coverage-measure").detail(format!("{error:#}")),
            }.with_duration(step_start.elapsed()));
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
    }
}
