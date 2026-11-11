//! Independent read-only ADR format verdict.
use std::path::Path;
fn main() {
    if std::env::args_os().skip(1).collect::<Vec<_>>() != ["adr-format"] {
        eprintln!("usage: repository-docs adr-format");
        std::process::exit(2);
    }
    if let Err(error) = repository_policy::checkout::prepare_policy_checkout("repository-docs") {
        eprintln!("repository-docs: {error:#}");
        std::process::exit(2);
    }
    let problems = repository_policy::adr::format_problems(Path::new("."));
    if problems.is_empty() {
        println!("[ ok ] adr-format");
    } else {
        eprintln!("[FAIL] adr-format\n{}", problems.join("\n"));
        std::process::exit(1);
    }
}
