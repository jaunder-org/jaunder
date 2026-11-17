//! `csr-prerequisite <root>`: validate a CSR bundle for startup-capable host
//! tests and print its resolved root, which `dev test-local` hands to nextest.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

/// Validate a CSR bundle and print its resolved absolute root on stdout.
#[derive(Parser)]
#[command(name = "csr-prerequisite")]
struct Cli {
    /// The bundle root, absolute or relative to the working directory.
    root: PathBuf,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let _run = host_proof::span_records::join_run();
    match host_proof::steps::csr_prerequisite::validate(&cli.root) {
        Ok(root) => {
            println!("{}", root.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("csr-prerequisite: {error:#}");
            ExitCode::FAILURE
        }
    }
}
