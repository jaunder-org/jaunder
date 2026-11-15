//! `source-diagnostics` — eval-only Nix cache boundary guard; run on request,
//! not per commit. Prints one JSON report on stdout.

use clap::{Parser, Subcommand};
use host_proof::report::{CommandResult, parse_args};

#[derive(Parser)]
#[command(name = "source-diagnostics")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Prove the cache policy admits only support closures and excludes every
    /// coverage/e2e verdict, aggregate, and lifted equivalent.
    #[command(after_help = "EXAMPLES:\n  traced-run -- just source-diagnostics cache-safety-probe")]
    CacheSafetyProbe,
}

fn main() {
    let cli = parse_args::<Cli>();
    if let Err(error) = host_proof::checkout::prepare() {
        std::process::exit(
            CommandResult::refused("source-diagnostics", format!("{error:#}")).emit(),
        );
    }
    host_proof::checkout::ensure_hooks_installed();
    let start = std::time::Instant::now();
    let (name, step) = match cli.command {
        Command::CacheSafetyProbe => ("cache-safety-probe", host_proof::cache_safety::probe()),
    };
    let mut result = CommandResult::new(name);
    result.push(step.with_duration(start.elapsed()));
    result.duration_ms = start.elapsed().as_millis();
    std::process::exit(result.emit());
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command};
    use clap::Parser;

    #[test]
    fn probes_parse_as_closed_subcommands() {
        assert!(matches!(
            Cli::try_parse_from(["source-diagnostics", "cache-safety-probe"])
                .unwrap()
                .command,
            Command::CacheSafetyProbe
        ));
        // The derivation-identity matrix probe is retired (#1946).
        assert!(Cli::try_parse_from(["source-diagnostics", "nix-probe-source"]).is_err());
        assert!(Cli::try_parse_from(["source-diagnostics", "probe"]).is_err());
    }
}
