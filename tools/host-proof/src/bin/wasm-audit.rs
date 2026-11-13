//! `wasm-audit` — frontend WASM/JS bundle size (raw, gzip, brotli) from the
//! manifest-selected identity assets of deterministic `nix build .#csrBundle`,
//! and the `budget` gate that reads the same measurement (#836). Prints one JSON
//! report on stdout; tables ride in `report`, measurements in `evidence`.

use clap::{Parser, Subcommand};
use host_proof::audit_wasm;
use host_proof::report::{CommandResult, StepResult, parse_args};

#[derive(Parser)]
#[command(name = "wasm-audit")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Measure the shipped bundle. Run after a change expected to move it (a new
    /// dependency or a client feature), or periodically to watch the trend.
    #[command(after_help = "EXAMPLES:\n  \
        just wasm-audit report\n  \
        just wasm-audit report --site-path /nix/store/...-jaunder-csr-wasm-bundle\n  \
        just wasm-audit report --breakdown")]
    Report {
        /// Audit a prebuilt CSR bundle root instead of running `nix build`.
        #[arg(long)]
        site_path: Option<String>,
        /// Report per-section and per-crate byte attribution instead of totals.
        ///
        /// Measured on the pre-wasm-bindgen, unstripped `.#csrWasm` artifact,
        /// which still carries a name section — `wasm-opt` strips names from the
        /// shipped bundle, so the shipped file cannot be attributed (#836). Its
        /// total is NOT the shipped bundle size.
        #[arg(long)]
        breakdown: bool,
        /// Break down this wasm file instead of building `.#csrWasm`.
        #[arg(long, requires = "breakdown")]
        wasm: Option<String>,
    },
    /// Fail when the manifest-selected WASM identity artifact exceeds the
    /// committed ceiling.
    Budget,
}

fn main() {
    let cli = parse_args::<Cli>();
    let start = std::time::Instant::now();
    let mut result;
    match cli.command {
        Command::Report {
            site_path,
            breakdown,
            wasm,
        } if breakdown => {
            result = CommandResult::new("audit-wasm-breakdown");
            let step = match audit_wasm::breakdown(wasm.as_deref()) {
                Ok(report) => {
                    result.report = Some(audit_wasm::render_breakdown(&report));
                    result.evidence = serde_json::to_value(&report).ok();
                    StepResult::ok("audit-wasm-breakdown")
                        .detail(format!("{} crate(s) attributed", report.crates.len()))
                }
                Err(error) => StepResult::fail("audit-wasm-breakdown").detail(format!("{error:#}")),
            };
            result.push(step);
            let _ = site_path;
        }
        Command::Report { site_path, .. } => {
            result = CommandResult::new("audit-wasm");
            let step = match audit_wasm::run(site_path.as_deref()) {
                Ok(report) => {
                    result.report = Some(audit_wasm::render_table(&report));
                    result.evidence = serde_json::to_value(&report).ok();
                    StepResult::ok("audit-wasm")
                        .detail(format!("{} artifact(s)", report.artifacts.len()))
                }
                Err(error) => StepResult::fail("audit-wasm").detail(format!("{error:#}")),
            };
            result.push(step);
        }
        Command::Budget => {
            result = CommandResult::new("wasm-budget");
            host_proof::steps::wasm_budget::run(&mut result);
        }
    }
    if let Some(step) = result.steps.last_mut() {
        step.duration_ms = start.elapsed().as_millis();
    }
    result.duration_ms = start.elapsed().as_millis();
    std::process::exit(result.emit());
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command};
    use clap::Parser;

    #[test]
    fn wasm_file_requires_breakdown_and_budget_parses() {
        assert!(Cli::try_parse_from(["wasm-audit", "report", "--wasm", "x.wasm"]).is_err());
        assert!(matches!(
            Cli::try_parse_from(["wasm-audit", "report", "--breakdown", "--wasm", "x.wasm"])
                .unwrap()
                .command,
            Command::Report {
                breakdown: true,
                ..
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["wasm-audit", "budget"])
                .unwrap()
                .command,
            Command::Budget
        ));
    }
}
