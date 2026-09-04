use std::path::Path;

use xshell::Shell;

use crate::{
    adr, adr_readme, audit_wasm, census,
    cli::{
        AdrCommand, Cli, Command, CoverageCommand, NixCommand, PrCommand, ServerFnCoverageCommand,
        TracesCommand,
    },
    coverage, gate, issue, lifecycle, nix_probe, pr,
    result::{CommandResult, Mode, StepResult},
    server_fn_coverage, steps, traces,
};

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
        Command::Check { no_test } => {
            let policy = gate::execution_policy(&Command::Check { no_test });
            let sh = Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("check");
            gate::run_host_gate(&sh, Mode::Fix, policy, &mut result);
            if !no_test {
                steps::test_local::run(&sh, &mut result, &[]);
            }
            steps::nix::check_supporting_test_checks(&mut result, no_test);
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Precommit => {
            let policy = gate::execution_policy(&Command::Precommit);
            let sh = Shell::new()?;
            lifecycle::run_precommit_with_host_gate(Path::new("."), |class, result| {
                gate::dispatch_precommit_host_gate(class, &sh, policy, result);
            })
        }
        Command::Prepush => {
            let policy = gate::execution_policy(&Command::Prepush);
            let sh = Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("prepush");
            gate::run_prepush_with(
                &sh,
                policy,
                &mut result,
                || lifecycle::clean_tree_precheck(false),
                gate::run_local_push_gate,
            );
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Validate {
            no_e2e,
            allow_dirty,
        } => {
            let policy = gate::execution_policy(&Command::Validate {
                no_e2e,
                allow_dirty,
            });
            let sh = Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("validate");
            // Clean-tree backstop: refuse a dirty tree so what is measured equals the
            // committed tip (== what CI sees). Fail fast before the expensive steps.
            let precheck_start = std::time::Instant::now();
            let precheck =
                lifecycle::clean_tree_precheck(allow_dirty).with_duration(precheck_start.elapsed());
            let blocked = precheck.is_blocking_failure();
            result.push(precheck);
            if blocked {
                lifecycle::finalize(&mut result, start);
                return Ok(result);
            }
            gate::run_host_gate_without_tests(&sh, Mode::Check, policy, &mut result);
            steps::nix::static_checks(&mut result);
            // Deliberately in `validate` and not `check`: it costs a
            // `nix build .#site`, which the pre-commit gate should not pay (#836).
            steps::wasm_budget::run(&mut result);
            gate::HOST_TESTS_STEP.run(&sh, Mode::Check, policy, &mut result);
            steps::nix::test_checks(&mut result, false);
            if !no_e2e {
                // Each browser/backend combo is realized, lifted, and reconciled
                // separately; their same-named per-backend inputs cannot safely
                // survive the aggregate `e2e-checks` symlink join. The coverage
                // verifier therefore resolves the already-realized authoritative
                // combo's individual output rather than reading the join.
                let e2e = steps::nix::e2e(&mut result);
                steps::server_fn_coverage_check::verify_after_validate(
                    &mut result,
                    e2e.combinations_ok,
                );
            }
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Census => {
            let start = std::time::Instant::now();
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
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("audit-wasm");
            let step_start = std::time::Instant::now();
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
        Command::E2e { backend, browser } => {
            let start = std::time::Instant::now();
            let label = format!("e2e-{}-{}", backend.as_str(), browser.as_str());
            let mut result = CommandResult::new(&label);
            steps::nix::e2e_combo(&mut result, backend.as_str(), browser.as_str());
            // Surface retried-but-passed tests from the report `e2e_combo` just
            // lifted out of the VM (see steps::flaky). Informational — never fails
            // the combo.
            steps::flaky::collect(&mut result, backend.as_str(), browser.as_str());
            // #681: the e2e half of the flow-coverage gate. Only this per-combo path
            // has an uncollided capture (spec D8), and only the authoritative combo's
            // traces are used (D6) — `verify_after_combo` enforces both.
            steps::server_fn_coverage_check::verify_after_combo(
                &mut result,
                backend.as_str(),
                browser.as_str(),
            );
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Nix(NixCommand::ProbeSource) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("nix-probe-source");
            let step_start = std::time::Instant::now();
            result.push(nix_probe::probe_source().with_duration(step_start.elapsed()));
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Coverage(CoverageCommand::ProbeSource) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("coverage-probe-source");
            let step_start = std::time::Instant::now();
            result.push(coverage::probe::probe_source().with_duration(step_start.elapsed()));
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::E2eLocal {
            test,
            update_visual_snapshots,
        } => {
            let sh = Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("e2e-local");
            steps::e2e_local::run(&sh, &mut result, test.as_deref(), update_visual_snapshots);
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::TestLocal { nextest_args } => {
            let sh = Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("test-local");
            steps::test_local::run(&sh, &mut result, &nextest_args);
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::BuildCsr { release } => {
            let sh = Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("build-csr");
            steps::build_csr::run(&sh, &mut result, release);
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Adr(AdrCommand::SyncReadme) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("adr-sync-readme");
            let step_start = std::time::Instant::now();
            result.push(adr_readme::sync_readme().with_duration(step_start.elapsed()));
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Adr(AdrCommand::Promote) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("adr-promote");
            let step_start = std::time::Instant::now();
            result.push(adr::promote().with_duration(step_start.elapsed()));
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Adr(AdrCommand::Promoter) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("adr-promoter");
            let step_start = std::time::Instant::now();
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
            let start = std::time::Instant::now();
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
            let start = std::time::Instant::now();
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
            let start = std::time::Instant::now();
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
        Command::ServerFnCoverage(sub) => {
            let start = std::time::Instant::now();
            use steps::server_fn_coverage_check::{REGENERATE_STEP, VERIFY_STEP};
            let regenerate = matches!(sub, ServerFnCoverageCommand::Regenerate);
            let mut result = CommandResult::new(if regenerate {
                REGENERATE_STEP
            } else {
                VERIFY_STEP
            });
            // A missing/empty/unparseable capture propagates as Err → the exit-2
            // path, never a green run: treating a broken capture as "nothing
            // uncovered" would make the whole gate dishonest.
            let step = steps::server_fn_coverage_check::from_capture(
                Path::new(server_fn_coverage::io::CAPTURE_PATH),
                regenerate,
            )?;
            result.push(step);
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Issue(sub) => issue::execute(sub),
        Command::Pr(PrCommand::Cleanup { number }) => {
            let start = std::time::Instant::now();
            let mut result = pr::cleanup::execute(number);
            lifecycle::finalize(&mut result, start);
            Ok(result)
        }
        Command::Pr(sub) => {
            let start = std::time::Instant::now();
            let (operation, number, cfg) = match sub {
                PrCommand::Watch {
                    number,
                    interval,
                    timeout,
                    once,
                    until,
                } => (
                    pr::PrOperation::Watch,
                    number,
                    pr::watch::WatchConfig {
                        interval_secs: interval,
                        timeout_mins: timeout,
                        once,
                        stop_at_ready: until.is_none(),
                        ..Default::default()
                    },
                ),
                PrCommand::Land {
                    number,
                    interval,
                    timeout,
                } => (
                    pr::PrOperation::Land,
                    number,
                    pr::watch::WatchConfig {
                        interval_secs: interval,
                        timeout_mins: timeout,
                        once: false,
                        stop_at_ready: false,
                        ..Default::default()
                    },
                ),
                PrCommand::Cleanup { .. } => unreachable!("cleanup dispatches separately"),
            };
            // An `Err` here means the subject could not be established at all (exit
            // 2, no report). Every other failure — including `gh` being broken — is
            // a `watcher-error` report, so the outcome that most needs to be legible
            // always reaches the sidecar.
            let report = pr::execute(number, cfg, operation.is_landing())?;
            let mut result = pr::into_result(operation, report, start.elapsed());
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
