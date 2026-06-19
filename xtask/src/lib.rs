use clap::{Parser, Subcommand};

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
    /// Tight inner loop: static checks + clippy (host).
    Check,
    /// The hub: check + the Nix coverage check (tests+coverage). `--full` adds the Nix e2e + postgres-integration checks.
    Validate {
        #[arg(long)]
        full: bool,
    },
}

pub fn run(cli: Cli) -> anyhow::Result<CommandResult> {
    match cli.command {
        Command::Check => {
            let sh = xshell::Shell::new()?;
            let mut result = CommandResult::new("check");
            steps::static_checks::run(&sh, Mode::Fix, &mut result);
            Ok(result)
        }
        Command::Validate { full } => {
            let sh = xshell::Shell::new()?;
            let mut result = CommandResult::new("validate");
            steps::static_checks::run(&sh, Mode::Fix, &mut result);
            steps::nix::run(full, &mut result);
            Ok(result)
        }
    }
}
