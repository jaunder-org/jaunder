//! `admit-inputs <source-admits>`: refuse the checkout when a Git flake would
//! omit an untracked input that the selected output declares, such as
//! `.#csrBundle.sourceAdmits` before the CSR bundle is built.

use std::path::Path;
use std::process::ExitCode;

use clap::Parser;

/// Refuse untracked inputs that the flake attribute SOURCE_ADMITS admits.
#[derive(Parser)]
#[command(name = "admit-inputs")]
struct Cli {
    /// The output's source predicate, e.g. `.#csrBundle.sourceAdmits`.
    source_admits: String,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match repository_policy::checkout::admit_untracked_inputs(Path::new("."), &cli.source_admits) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("admit-inputs: {error:#}");
            ExitCode::FAILURE
        }
    }
}
