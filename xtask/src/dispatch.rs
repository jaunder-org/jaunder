use std::time::Instant;

use crate::{
    audit_wasm, cache_safety,
    cli::{CacheSafetyCommand, Cli, Command, NixCommand},
    lifecycle, nix_probe,
    result::{CommandResult, StepResult},
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
    }
}
