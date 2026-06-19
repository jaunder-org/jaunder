use clap::Parser;
use xtask::{run, Cli};

fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    let command = cli.command_name();
    match run(cli) {
        Ok(result) => {
            result.report(json);
            eprintln!(
                "xtask-done: command={} ok={} exit={} duration_ms={}",
                result.command,
                result.ok,
                result.exit_code(),
                result.duration_ms
            );
            std::process::exit(result.exit_code());
        }
        Err(err) => {
            eprintln!("xtask: {err:#}");
            eprintln!(
                "xtask-done: command={command} ok=false exit=2 error={:?}",
                err.to_string()
            );
            std::process::exit(2);
        }
    }
}
