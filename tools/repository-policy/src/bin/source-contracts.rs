//! Native source-policy verdicts and nongating maintenance census.
use repository_policy::{census, source_contracts::Operation};
use std::path::Path;
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let census_json = args.as_slice() == ["census", "--json"];
    if args.as_slice() == ["census"] || census_json {
        if let Err(error) = repository_policy::checkout::prepare_policy_checkout("source-contracts")
        {
            eprintln!("source-contracts: {error:#}");
            std::process::exit(2);
        }
        let report = match census::collect(Path::new(".")) {
            Ok(report) => report,
            Err(error) => {
                eprintln!("census: {error:#}");
                std::process::exit(1);
            }
        };
        if census_json {
            match serde_json::to_string_pretty(&report) {
                Ok(json) => println!("{json}"),
                Err(error) => {
                    eprintln!("census: {error}");
                    std::process::exit(2);
                }
            }
        } else {
            print!("{}", census::render_human(&report));
        }
        if report.has_failed_cells() {
            std::process::exit(1);
        }
        return;
    }
    let operation = match args.as_slice() {
        [name] => Operation::parse(name),
        _ => None,
    };
    let Some(operation) = operation else {
        eprintln!("usage: source-contracts <fixed-policy> | census [--json]");
        std::process::exit(2);
    };
    if let Err(error) = repository_policy::checkout::prepare_policy_checkout("source-contracts") {
        eprintln!("source-contracts: {error:#}");
        std::process::exit(2);
    }
    match operation.check() {
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
