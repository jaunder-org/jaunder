//! `coverage-check` — host-run coverage evidence consumers; none runs inside
//! Nix. Prints one JSON report on stdout; the exit status is the verdict.

use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, Subcommand};
use coverage::host::{mutants, nix, policy, wasm_evidence};
use host_tool::report::{CommandResult, StepResult, parse_args};

#[derive(Parser)]
#[command(name = "coverage-check")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

fn nonempty(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        Err("value must not be empty".into())
    } else {
        Ok(value.to_owned())
    }
}

#[derive(Subcommand)]
enum Command {
    /// Apply source-aware coverage/CRAP policy to realized reports; never runs Nix.
    Policy {
        #[arg(long)]
        out: PathBuf,
    },
    /// Guard the Nix coverage derivation's source filter against silent drift:
    /// staging an excluded file leaves `coverage.drvPath` unchanged and staging an
    /// instrumented `.rs` changes it. Eval-only; runs in CI and on request (#241).
    ProbeSource,
    /// Measure the coverage producer under a declared unloaded host window. An
    /// opt-in local experiment; it does not alter production coverage.
    #[command(
        after_help = "EXAMPLES:\n  just coverage-check benchmark-local --unloaded-system 'operator confirms no competing host load'"
    )]
    BenchmarkLocal {
        /// Operator assertion that the host has no competing workload.
        #[arg(long, value_parser = nonempty)]
        unloaded_system: String,
    },
    /// Realize each browser WASM evidence producer, validate its archive, then
    /// conditionally count-sum their profiles with the pinned LLVM tools. The
    /// aggregate is written to `.xtask/wasm-coverage/status.json` and reported.
    WasmProbe,
    /// Measure the baseline and instrumented diagnostic bundles during a
    /// coordinated quiet host window; numbers are invalid under competing load.
    #[command(
        after_help = "EXAMPLES:\n  just coverage-check wasm-measure --quiescent-window '2026-09-06 coordinated window'"
    )]
    WasmMeasure {
        #[arg(long, value_parser = nonempty)]
        quiescent_window: String,
    },
    /// Lift and reconcile the Emacs Lisp coverage producer's artifacts.
    ElispConsume,
    /// Report counts only for completed, applicable and consistent mutation
    /// evidence.
    MutantsVerify {
        #[arg(long, value_parser = nonempty)]
        package: String,
        #[arg(long, value_parser = nonempty)]
        file: String,
        /// Producer output root containing mutants.out/.
        #[arg(long)]
        output: PathBuf,
        #[arg(long, allow_hyphen_values = true)]
        producer_status: i32,
    },
}

fn step(name: &str, outcome: anyhow::Result<String>, start: Instant) -> StepResult {
    match outcome {
        Ok(detail) => StepResult::ok(name).detail(detail),
        Err(error) => StepResult::fail(name).detail(format!("{error:#}")),
    }
    .with_duration(start.elapsed())
}

fn run(command: Command) -> CommandResult {
    let start = Instant::now();
    let mut result;
    match command {
        Command::Policy { out } => {
            result = CommandResult::new("coverage-policy");
            result.push(policy::run(&out).0.with_duration(start.elapsed()));
        }
        Command::ProbeSource => {
            result = CommandResult::new("coverage-probe-source");
            result.push(policy::probe::probe_source().with_duration(start.elapsed()));
        }
        Command::BenchmarkLocal { unloaded_system } => {
            result = CommandResult::new("coverage-benchmark-local");
            let outcome = policy::benchmark::run(&unloaded_system).map(|manifest| {
                format!(
                    "retained {} accepted observations at {}",
                    manifest.observations.len(),
                    policy::benchmark::manifest_path(&manifest.revision).display()
                )
            });
            result.push(step("coverage-benchmark-local", outcome, start));
        }
        Command::WasmProbe => {
            result = CommandResult::new("wasm-coverage-probe");
            match wasm_evidence::probe() {
                Ok(aggregate) => {
                    let ok = aggregate.verdict == wasm_evidence::Verdict::Passed;
                    let detail = if ok {
                        "both browser reports map executed original Rust lines; merged evidence retained".to_owned()
                    } else {
                        aggregate.blockers.join("; ")
                    };
                    result.evidence = serde_json::to_value(&aggregate).ok();
                    let verdict = if ok {
                        StepResult::ok("wasm-coverage-probe")
                    } else {
                        StepResult::fail("wasm-coverage-probe")
                    };
                    result.push(verdict.detail(detail).with_duration(start.elapsed()));
                }
                Err(error) => result.push(step("wasm-coverage-probe", Err(error), start)),
            }
        }
        Command::WasmMeasure { quiescent_window } => {
            result = CommandResult::new("wasm-coverage-measure");
            let outcome = wasm_evidence::measure(&quiescent_window).map(|manifest| {
                format!(
                    "retained {} reconciled runs at .xtask/wasm-coverage/measurement/manifest-v1.json",
                    manifest.runs.len()
                )
            });
            result.push(step("wasm-coverage-measure", outcome, start));
        }
        Command::ElispConsume => {
            result = CommandResult::new("elisp-coverage-consume");
            nix::consume_elisp_coverage(&mut result);
        }
        Command::MutantsVerify {
            package,
            file,
            output,
            producer_status,
        } => {
            result = CommandResult::new("mutants-verify-evidence");
            let outcome = mutants::verify_evidence(&package, &file, &output, producer_status)
                .map_err(|error| error.context("verification failed"));
            result.push(step("mutants-verify-evidence", outcome, start));
        }
    }
    result.duration_ms = start.elapsed().as_millis();
    result
}

fn main() {
    let cli = parse_args::<Cli>();
    if let Err(error) = repository_policy::checkout::prepare_checkout("tools/Cargo.toml", None) {
        std::process::exit(CommandResult::refused("coverage-check", format!("{error:#}")).emit());
    }
    repository_policy::checkout::ensure_hooks_installed("coverage-check");
    std::process::exit(run(cli.command).emit());
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command};
    use clap::Parser;

    #[test]
    fn benchmark_requires_an_unloaded_system_assertion_and_wasm_probe_parses() {
        assert!(Cli::try_parse_from(["coverage-check", "benchmark-local"]).is_err());
        assert!(
            Cli::try_parse_from([
                "coverage-check",
                "benchmark-local",
                "--unloaded-system",
                " "
            ])
            .is_err()
        );
        let cli = Cli::try_parse_from([
            "coverage-check",
            "benchmark-local",
            "--unloaded-system",
            "operator confirms no competing host load",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::BenchmarkLocal { unloaded_system }
                if unloaded_system == "operator confirms no competing host load"
        ));
        assert!(matches!(
            Cli::try_parse_from(["coverage-check", "wasm-probe"])
                .unwrap()
                .command,
            Command::WasmProbe
        ));
    }
}
