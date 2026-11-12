//! Independent read-only documentation verdicts.
use repository_policy::repository_docs::Operation;
use std::path::Path;
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let operation = match args.as_slice() {
        [name] => Operation::parse(name),
        _ => None,
    };
    let Some(operation) = operation else {
        eprintln!(
            "usage: repository-docs <adr-format|adr-readme-parity|adr-view-parity|doc-links|flow-docs|sequence-check>"
        );
        std::process::exit(2);
    };
    if let Err(error) = repository_policy::checkout::prepare_policy_checkout("repository-docs") {
        eprintln!("repository-docs: {error:#}");
        std::process::exit(2);
    }
    match operation.check(Path::new(".")) {
        Ok(detail) => {
            println!("[ ok ] {}", operation.name());
            if let Some(detail) = detail {
                println!("{detail}");
            }
        }
        Err(detail) => {
            eprintln!("[FAIL] {}\n{detail}", operation.name());
            std::process::exit(1);
        }
    }
}
