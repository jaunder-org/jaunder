//! `e2e-evidence` — artifact-only e2e verdicts over already-lifted producer
//! evidence; never builds or runs a VM. Prints one JSON report on stdout with a
//! single verdict step; the exit status is the verdict.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use host_proof::e2e::{E2eBackend, E2eBrowser, LiftedEvidence};
use host_proof::report::{CommandResult, StepResult};

#[derive(Parser)]
#[command(name = "e2e-evidence")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Verify one lane's explicit artifacts without building a VM.
    Verify {
        #[arg(long)]
        lane: String,
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        artifacts: PathBuf,
    },
    /// Reconcile one `{backend, browser}` pair's already-lifted split lane
    /// evidence without invoking Nix.
    Reconcile {
        #[arg(value_enum)]
        backend: E2eBackend,
        #[arg(value_enum)]
        browser: E2eBrowser,
        #[command(flatten)]
        evidence: LiftedEvidence,
    },
    /// Trace-derived `#[server]` fn flow coverage (#681): which server entry
    /// points the e2e suite actually drives.
    #[command(subcommand)]
    ServerFnCoverage(ServerFnCoverage),
}

#[derive(Subcommand)]
enum ServerFnCoverage {
    /// Explicitly regenerate the committed snapshot from the reconciled
    /// SQLite/Chromium lanes.
    Regenerate(LiftedEvidence),
    /// Compare the reconciled SQLite/Chromium lanes with the committed snapshot.
    Verify(LiftedEvidence),
}

fn verdict(name: &str, outcome: Result<String, String>) -> CommandResult {
    let mut result = CommandResult::new(name);
    result.push(match outcome {
        Ok(detail) => StepResult::ok(name).detail(detail),
        Err(detail) => StepResult::fail(name).detail(detail),
    });
    result
}

fn main() {
    let start = std::time::Instant::now();
    let mut result = match host_proof::report::parse_args::<Cli>().command {
        Command::Verify {
            lane,
            catalog,
            artifacts,
        } => verdict(
            "e2e-verify",
            host_proof::e2e_evidence::verify_lane(&lane, &catalog, &artifacts),
        ),
        Command::Reconcile {
            backend,
            browser,
            evidence,
        } => verdict(
            "e2e-reconcile",
            host_proof::e2e_evidence::reconciled_lanes(
                backend.as_str(),
                browser.as_str(),
                &evidence.catalog,
                &evidence.diagnostics_root,
            )
            .map(|(verdict, _)| verdict),
        ),
        Command::ServerFnCoverage(command) => {
            let (name, inputs, regenerate) = match &command {
                ServerFnCoverage::Verify(inputs) => ("server-fn-coverage-verify", inputs, false),
                ServerFnCoverage::Regenerate(inputs) => {
                    ("server-fn-coverage-regenerate", inputs, true)
                }
            };
            verdict(
                name,
                host_proof::steps::server_fn_coverage_check::from_reconciled(inputs, regenerate)
                    .map_err(|error| format!("{error:#}")),
            )
        }
    };
    result.duration_ms = start.elapsed().as_millis();
    std::process::exit(result.emit());
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command, ServerFnCoverage};
    use clap::Parser;

    #[test]
    fn lane_verification_requires_explicit_runtime_inputs() {
        assert!(Cli::try_parse_from(["e2e-evidence", "verify"]).is_err());
        let cli = Cli::try_parse_from([
            "e2e-evidence",
            "verify",
            "--lane",
            "sqlite-chromium-1-of-3",
            "--catalog",
            "lanes.json",
            "--artifacts",
            "output",
        ])
        .unwrap();
        assert!(matches!(cli.command, Command::Verify { .. }));
    }

    #[test]
    fn reconcile_requires_a_backend_and_a_browser() {
        let base = ["--diagnostics-root", "root", "--catalog", "lanes.json"];
        assert!(
            Cli::try_parse_from(
                ["e2e-evidence", "reconcile", "sqlite"]
                    .into_iter()
                    .chain(base)
            )
            .is_err()
        );
        let cli = Cli::try_parse_from(
            ["e2e-evidence", "reconcile", "sqlite", "chromium"]
                .into_iter()
                .chain(base),
        )
        .unwrap();
        assert!(matches!(cli.command, Command::Reconcile { .. }));
    }

    #[test]
    fn server_fn_coverage_parses_both_subcommands_and_requires_inputs() {
        // Both spellings appear in the gate's failure messages and CONTRIBUTING,
        // so a rename must break a test rather than a developer.
        assert!(Cli::try_parse_from(["e2e-evidence", "server-fn-coverage"]).is_err());
        for (action, regenerate) in [("regenerate", true), ("verify", false)] {
            assert!(Cli::try_parse_from(["e2e-evidence", "server-fn-coverage", action]).is_err());
            let cli = Cli::try_parse_from([
                "e2e-evidence",
                "server-fn-coverage",
                action,
                "--diagnostics-root",
                ".xtask/diagnostics",
                "--catalog",
                "lanes.json",
            ])
            .unwrap();
            let Command::ServerFnCoverage(command) = cli.command else {
                panic!("expected server-fn-coverage");
            };
            assert_eq!(
                matches!(command, ServerFnCoverage::Regenerate(_)),
                regenerate
            );
        }
    }
}
