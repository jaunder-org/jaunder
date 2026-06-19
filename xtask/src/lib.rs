use clap::{Parser, Subcommand};

mod coverage;
mod result;
mod sh;
mod steps {
    pub mod nix;
    pub mod static_checks;
}
pub use result::{CommandResult, Mode, StepResult};

#[derive(Parser)]
#[command(name = "xtask", about = "Jaunder dev orchestration")]
pub struct Cli {
    /// Emit the structured result envelope as JSON to stdout.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Inner loop (auto-fixes formatting): host static checks + clippy, then the
    /// Nix coverage check (instrumented test suite + coverage). `--no-test` runs
    /// static + clippy only.
    Check {
        /// Skip the Nix coverage check — static checks + clippy only.
        #[arg(long)]
        no_test: bool,
    },
    /// Full gate (never mutates the tree): static + clippy (verify-only) + the Nix
    /// coverage check + the e2e VMs. `--no-e2e` skips the e2e VMs.
    Validate {
        /// Skip the e2e VM checks — static + coverage only.
        #[arg(long)]
        no_e2e: bool,
    },
}

impl Cli {
    pub fn command_name(&self) -> &'static str {
        match self.command {
            Command::Check { .. } => "check",
            Command::Validate { .. } => "validate",
        }
    }
}

pub fn run(cli: Cli) -> anyhow::Result<CommandResult> {
    match cli.command {
        Command::Check { no_test } => {
            let sh = xshell::Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("check");
            steps::static_checks::run(&sh, Mode::Fix, &mut result);
            if !no_test {
                steps::nix::coverage(&mut result);
            }
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Validate { no_e2e } => {
            let sh = xshell::Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("validate");
            steps::static_checks::run(&sh, Mode::Check, &mut result);
            steps::nix::coverage(&mut result);
            if !no_e2e {
                steps::nix::e2e(&mut result);
            }
            finalize(&mut result, start);
            Ok(result)
        }
    }
}

fn finalize(result: &mut CommandResult, start: std::time::Instant) {
    result.duration_ms = start.elapsed().as_millis();
    result.finished_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
}
