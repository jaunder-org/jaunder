//! `source-diagnostics` — eval-only Nix source and cache boundary guards; run in
//! CI and on request, not per commit. Prints one JSON report on stdout.

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
    /// Apply isolated tracked marker perturbations and assert each guarded
    /// derivation's identity changes exactly when its declared inputs change.
    #[command(after_help = "EXAMPLES:\n  just source-diagnostics nix-probe-source")]
    NixProbeSource,
    /// Prove the cache policy admits only support closures and excludes every
    /// coverage/e2e verdict, aggregate, and lifted equivalent.
    #[command(after_help = "EXAMPLES:\n  traced-run -- just source-diagnostics cache-safety-probe")]
    CacheSafetyProbe,
}

fn main() {
    let cli = parse_args::<Cli>();
    if let Err(error) = host_proof::checkout::prepare() {
        eprintln!("source-diagnostics: {error:#}");
        std::process::exit(2);
    }
    host_proof::checkout::ensure_hooks_installed();
    let start = std::time::Instant::now();
    let (name, step) = match cli.command {
        Command::NixProbeSource => ("nix-probe-source", host_proof::nix_probe::probe_source()),
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
            Cli::try_parse_from(["source-diagnostics", "nix-probe-source"])
                .unwrap()
                .command,
            Command::NixProbeSource
        ));
        assert!(matches!(
            Cli::try_parse_from(["source-diagnostics", "cache-safety-probe"])
                .unwrap()
                .command,
            Command::CacheSafetyProbe
        ));
        assert!(Cli::try_parse_from(["source-diagnostics", "probe"]).is_err());
    }
}
