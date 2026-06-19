use clap::Parser;
use xtask::{run, Cli};

fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    match run(cli) {
        Ok(result) => {
            result.report(json);
            std::process::exit(result.exit_code());
        }
        Err(err) => {
            eprintln!("xtask: {err:#}");
            std::process::exit(2);
        }
    }
}
