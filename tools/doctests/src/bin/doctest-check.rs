//! `doctest-check fences|workspace` — the host doctest leaves. `fences` checks
//! the fence policy over tracked sources; `workspace` runs and reconciles the
//! auxiliary workspaces' doctests. Prints one JSON report on stdout.

use clap::{Parser, Subcommand};
use repository_policy::report::{CommandResult, parse_args};

#[derive(Parser)]
#[command(name = "doctest-check")]
struct Cli {
    #[command(subcommand)]
    leaf: Leaf,
}

#[derive(Subcommand)]
enum Leaf {
    /// Enforce the closed fence vocabulary and companion rule on tracked sources.
    Fences,
    /// Run and reconcile the xtask and tools workspaces' doctests.
    Workspace,
}

fn main() {
    let cli = parse_args::<Cli>();
    let start = std::time::Instant::now();
    let mut result = CommandResult::new("doctest-check");
    if let Err(error) = repository_policy::checkout::prepare_checkout("tools/Cargo.toml", None) {
        eprintln!("doctest-check: {error:#}");
        std::process::exit(2);
    }
    repository_policy::checkout::ensure_hooks_installed("doctest-check");
    match cli.leaf {
        Leaf::Fences => doctests::host::run(&mut result),
        Leaf::Workspace => match xshell::Shell::new() {
            Ok(sh) => doctests::host::run_workspace(&sh, &mut result),
            Err(error) => {
                eprintln!("doctest-check: {error:#}");
                std::process::exit(2);
            }
        },
    }
    result.duration_ms = start.elapsed().as_millis();
    std::process::exit(result.emit());
}
