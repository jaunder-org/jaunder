//! Focused foreground owner for disposable and named UX sandboxes.
mod cli;
mod lifecycle;
mod preparation;
mod sandbox_name;
mod session;
mod workspace;

use clap::{CommandFactory, Parser};

#[tokio::main]
async fn main() {
    let cli = cli::Cli::parse();
    let empty_command = cli.command.is_empty() && std::env::args_os().any(|arg| arg == "--");
    if let Err(error) = cli.validate().and_then(|()| {
        if empty_command {
            Err("-- requires an operational command".into())
        } else {
            Ok(())
        }
    }) {
        cli::Cli::command()
            .error(clap::error::ErrorKind::ValueValidation, error)
            .exit();
    }
    let mut signals = match session::Signals::new() {
        Ok(signals) => signals,
        Err(error) => {
            eprintln!("sandbox signals: {error:#}");
            std::process::exit(1);
        }
    };
    let code = match lifecycle::run(cli, &mut signals).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("sandbox: {error:#}");
            signals.status().unwrap_or(1)
        }
    };
    std::process::exit(code);
}
