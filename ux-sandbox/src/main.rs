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
    std::process::exit(execute(cli, session::Signals::new()).await);
}

async fn execute(cli: cli::Cli, signals: anyhow::Result<session::Signals>) -> i32 {
    let mut signals = match signals {
        Ok(signals) => signals,
        Err(error) => {
            eprintln!("sandbox signals: {error:#}");
            return 1;
        }
    };
    match lifecycle::run(cli, &mut signals).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("sandbox: {error:#}");
            signals.status().unwrap_or(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn signal_registration_failure_returns_failure_before_workspace_access() {
        let cli = cli::Cli::try_parse_from(["ux-sandbox"]).unwrap();
        let error = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "one-shot signal registration failure",
        );
        assert_eq!(execute(cli, Err(error.into())).await, 1);
    }
}
