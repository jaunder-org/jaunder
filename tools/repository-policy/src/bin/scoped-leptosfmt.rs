//! The pinned formatter needs fail-closed admission and per-file status promotion.
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.len() != 1 {
        eprintln!("usage: scoped-leptosfmt <verify|fix>");
        return ExitCode::from(2);
    }
    let result = std::env::current_dir()
        .map_err(anyhow::Error::from)
        .and_then(|root| repository_policy::scoped_leptosfmt::run(&root, &arguments[0]));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("scoped-leptosfmt: {error:#}");
            ExitCode::FAILURE
        }
    }
}
