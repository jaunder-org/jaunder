//! Foreground owner for an empty disposable UX sandbox.
mod preparation;
mod session;

use anyhow::Context;
use clap::{CommandFactory, Parser};
use std::path::Path;
use tokio::process::Command;

#[derive(Parser)]
#[command(about = "Run an empty disposable Jaunder UX sandbox")]
struct Cli {}

#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() > 1 && !(args.len() == 2 && (args[1] == "--help" || args[1] == "-h")) {
        Cli::command()
            .error(
                clap::error::ErrorKind::UnknownArgument,
                format!(
                    "only disposable empty mode is supported in this tracer; received {:?}",
                    &args[1..]
                ),
            )
            .exit();
    }
    Cli::parse();
    let code = match run().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("sandbox: {error:#}");
            1
        }
    };
    std::process::exit(code);
}

async fn run() -> anyhow::Result<i32> {
    let mut signals = session::Signals::new()?;
    let (jaunder, code) = preparation::prepare(&mut signals).await?;
    if code != 0 {
        return Ok(code);
    }
    // Unresolved finalization retains this owner; RAII must not delete storage on unwind.
    let disposable = tempfile::Builder::new()
        .prefix("ux-sandbox-")
        .tempdir()?
        .keep();
    // init creates storage exclusively; the unique parent reserves our disposable leaf.
    let storage = disposable.join("storage");
    eprintln!("Disposable storage: {}", storage.display());
    let result = async {
        let code =
            session::run(app_command(&jaunder, &storage, "init"), None, &mut signals).await?;
        if code != 0 {
            return Ok(code);
        }
        session::run(
            app_command(&jaunder, &storage, "serve"),
            Some(&storage.join("runtime.json")),
            &mut signals,
        )
        .await
    }
    .await;
    // session returns only after explicit ownership finalization, including observed errors.
    let cleanup = std::fs::remove_dir_all(&disposable).with_context(|| {
        format!(
            "removing reaped disposable sandbox {}",
            disposable.display()
        )
    });
    match (result, cleanup) {
        (Err(error), cleanup) => {
            if let Err(cleanup) = cleanup {
                eprintln!("sandbox cleanup also failed: {cleanup:#}");
            }
            Err(error)
        }
        (Ok(code), cleanup) => {
            cleanup?;
            Ok(code)
        }
    }
}

fn app_command(jaunder: &Path, storage: &Path, mode: &str) -> Command {
    let mut command = Command::new(jaunder);
    command.arg(mode);
    // Ambient production/storage/telemetry settings must not leak into a disposable instance.
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("JAUNDER_")
            || name.to_string_lossy().starts_with("OTEL_")
        {
            command.env_remove(name);
        }
    }
    command
        .env("JAUNDER_STORAGE_PATH", storage)
        .env(
            "JAUNDER_DB",
            format!("sqlite:{}", storage.join("jaunder.db").display()),
        )
        .env("JAUNDER_ENV", "dev")
        .env("JAUNDER_BIND", "127.0.0.1:0")
        .env("JAUNDER_RUNTIME_FILE", storage.join("runtime.json"));
    command
}
