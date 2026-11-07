use std::path::Path;
use std::time::Instant;

use xshell::Shell;

use crate::{
    adr, adr_readme, audit_wasm, cache_safety, census,
    cli::{
        AdrCommand, CacheSafetyCommand, Cli, Command, CoverageCommand, MutantsCommand, NixCommand,
        TracesCommand, WasmCoverageCommand,
    },
    coverage, lifecycle, nix_probe, pr,
    result::{CommandResult, StepResult},
    steps, traces, wasm_coverage,
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
        Command::QualificationLeaf { .. }
        | Command::CheckLeaf { .. }
        | Command::PrecommitSafety => {
            anyhow::bail!(
                "internal leaf and precommit safety commands use the plain process boundary"
            )
        }
        Command::Census => {
            let start = Instant::now();
            let mut result = CommandResult::new("census");
            let report = census::collect(Path::new("."), census::catalog())?;
            let failed = report.has_failed_cells();
            let cells = report.cell_count();
            result.census = Some(report);
            result.push(
                if failed {
                    StepResult::fail("census")
                        .detail(format!("{cells} cell(s); one or more collectors failed"))
                } else {
                    StepResult::ok("census").detail(format!("{cells} cell(s)"))
                }
                .with_duration(start.elapsed()),
            );
            lifecycle::finalize(&mut result, start);
            Ok(result)
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
        Command::E2eLocal {
            browser,
            test,
            update_visual_snapshots,
        } => {
            let sh = Shell::new()?;
            let start = Instant::now();
            let mut result = CommandResult::new("e2e-local");
            steps::e2e_local::run(
                &sh,
                &mut result,
                browser,
                test.as_deref(),
                update_visual_snapshots,
            );
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Sandbox {
            name,
            profile,
            reset,
            command,
        } => {
            let sh = Shell::new()?;
            let start = Instant::now();
            let mut result = CommandResult::new("sandbox");
            steps::sandbox::run(&sh, &mut result, name, profile, reset, command);
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::TestLocal { nextest_args } => {
            let sh = Shell::new()?;
            let mut result = CommandResult::new("test-local");
            steps::test_local::run(&sh, &mut result, &nextest_args);
            result.finished_at_unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            Ok(result)
        }
        Command::Adr(AdrCommand::SyncReadme) => {
            let start = Instant::now();
            let mut result = CommandResult::new("adr-sync-readme");
            let step_start = Instant::now();
            result.push(adr_readme::sync_readme().with_duration(step_start.elapsed()));
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Adr(AdrCommand::Promote) => {
            let start = Instant::now();
            let mut result = CommandResult::new("adr-promote");
            let step_start = Instant::now();
            result.push(adr::promote().with_duration(step_start.elapsed()));
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Adr(AdrCommand::Promoter) => {
            let start = Instant::now();
            let mut result = CommandResult::new("adr-promoter");
            let step_start = Instant::now();
            result.push(pr::promoter::execute().with_duration(step_start.elapsed()));
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Traces(TracesCommand::Analyze {
            top,
            trace,
            project,
            playwright_report,
            files,
        }) => {
            let start = Instant::now();
            let mut result = CommandResult::new("traces-analyze");
            let filters = traces::parse::Filters { trace, project };
            let reported = traces::report::ReportedDurations::from_paths(&playwright_report)?;
            let Some(analysis) = trace_attribute_owner_result(
                &mut result,
                "traces-analyze",
                traces::analyze::analyze(&files, filters, &reported),
            )?
            else {
                lifecycle::finalize(&mut result, start);
                return Ok(result);
            };
            let n = analysis.span_count;
            result.traces = Some(traces::render::render(&analysis, top as usize));
            result.push(
                StepResult::ok("traces-analyze")
                    .detail(format!("{n} span(s)"))
                    .with_duration(start.elapsed()),
            );
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Traces(TracesCommand::Run {
            top,
            trace,
            single_worker,
            browser,
        }) => {
            let start = Instant::now();
            let mut result = CommandResult::new("traces-run");
            // `_tmp` guards extracted traces until analysis ends. Collection and
            // Nix failures retain the trace command's top-level error contract.
            let (_tmp, files, reports) = traces::run::collect_trace_files(single_worker, browser)?;
            let n = files.len();
            let filters = traces::parse::Filters {
                trace,
                project: None,
            };
            // Paired per combo — see `ReportedDurations`: sqlite and postgres
            // share test+project+retry keys, so an unpaired merge would let one
            // backend's durations overwrite the other's.
            let reported = traces::report::ReportedDurations::from_labeled(&reports)?;
            let Some(analysis) = trace_attribute_owner_result(
                &mut result,
                "traces-run",
                traces::analyze::analyze(&files, filters, &reported),
            )?
            else {
                lifecycle::finalize(&mut result, start);
                return Ok(result);
            };
            result.traces = Some(traces::render::render(&analysis, top as usize));
            result.push(
                StepResult::ok("traces-run")
                    .detail(format!("{n} trace file(s)"))
                    .with_duration(start.elapsed()),
            );
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Traces(TracesCommand::BootPhases { files }) => {
            let start = Instant::now();
            let mut result = CommandResult::new("traces-boot-phases");
            let Some(rows) = trace_attribute_owner_result(
                &mut result,
                "traces-boot-phases",
                traces::boot_phases::boot_phases(&files),
            )?
            else {
                lifecycle::finalize(&mut result, start);
                return Ok(result);
            };
            let n = rows.len();
            result.traces = Some(traces::boot_phases::render(&rows));
            result.push(
                StepResult::ok("traces-boot-phases")
                    .detail(format!("{n} population(s)"))
                    .with_duration(start.elapsed()),
            );
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
    }
}

fn trace_attribute_owner_result<T>(
    result: &mut CommandResult,
    step: &'static str,
    value: anyhow::Result<T>,
) -> anyhow::Result<Option<T>> {
    match value {
        Ok(value) => Ok(Some(value)),
        Err(error)
            if error
                .downcast_ref::<traces::parse::MalformedJsonAttr>()
                .is_some() =>
        {
            result.push(
                StepResult::fail(step)
                    .detail(format!("{error:#}"))
                    .with_duration(std::time::Duration::ZERO),
            );
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn run_rejects_json_for_traces_analyze() {
        let cli = Cli {
            json: true,
            command: Command::Traces(TracesCommand::Analyze {
                top: 25,
                trace: None,
                project: None,
                playwright_report: Vec::new(),
                files: vec![PathBuf::from("x.jsonl")],
            }),
        };
        let err = match run(cli) {
            Ok(_) => panic!("expected --json to be rejected for traces analyze"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("--json"),
            "error explains the --json rejection: {err}"
        );
    }

    #[test]
    fn run_errors_on_missing_trace_file() {
        let path = "/no/such/trace.jsonl";
        let cli = Cli {
            json: false,
            command: Command::Traces(TracesCommand::Analyze {
                top: 25,
                trace: None,
                project: None,
                playwright_report: Vec::new(),
                files: vec![PathBuf::from(path)],
            }),
        };
        let error = match run(cli) {
            Ok(_) => panic!("missing trace file must remain a top-level error"),
            Err(error) => error,
        };
        assert!(format!("{error:#}").contains(path));
    }

    #[test]
    fn run_rejects_json_for_traces_run() {
        let cli = Cli {
            json: true,
            command: Command::Traces(TracesCommand::Run {
                top: 25,
                trace: None,
                single_worker: false,
                browser: None,
            }),
        };
        let err = match run(cli) {
            Ok(_) => panic!("expected --json to be rejected for traces run"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("--json"),
            "error explains the --json rejection: {err}"
        );
    }

    #[test]
    fn trace_json_attr_owner_returns_serializable_failed_command_result() {
        let span = serde_json::json!({
            "attributes": [{
                "key": "e2e.x",
                "value": { "stringValue": "{not json" }
            }]
        });
        let error = traces::parse::parse_json_attr(&span, "e2e.x", "source.jsonl").unwrap_err();
        let mut result = CommandResult::new("traces-analyze");
        let value: Option<()> =
            trace_attribute_owner_result(&mut result, "traces-analyze", Err(error)).unwrap();
        assert!(value.is_none());
        assert!(!result.ok);
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("source.jsonl"), "{json}");
        assert!(json.contains("e2e.x"), "{json}");
        assert!(json.contains("traces-analyze"), "{json}");
    }

    #[test]
    fn trace_non_attribute_failures_remain_top_level_errors() {
        let mut result = CommandResult::new("traces-run");
        let error = trace_attribute_owner_result::<()>(
            &mut result,
            "traces-run",
            Err(anyhow::anyhow!("nix build failed")),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("nix build failed"));
        assert!(result.steps.is_empty());
    }
}
