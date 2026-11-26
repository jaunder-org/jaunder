//! Native source-policy verdicts and the read-only server-fn inventory listing.
use repository_policy::{source_contracts::Operation, source_inventory};
use std::path::Path;
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.as_slice() == ["server-fn-inventory"] {
        // A read-only listing of `web/src`, so it skips the checkout admission
        // the verdicts need: a local coverage regeneration must never be
        // refused over an unrelated untracked file.
        let listing = source_inventory::inventory_listing(Path::new(source_inventory::WEB_SRC))
            .and_then(|listing| Ok(serde_json::to_string(&listing)?));
        match listing {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("server-fn-inventory: {error:#}");
                std::process::exit(2);
            }
        }
        return;
    }
    let operation = match args.as_slice() {
        [name] => Operation::parse(name),
        _ => None,
    };
    let Some(operation) = operation else {
        eprintln!("usage: source-contracts <fixed-policy> | server-fn-inventory");
        std::process::exit(2);
    };
    if let Err(error) = host_tool::checkout::prepare(
        "source-contracts",
        // Git-backed Nix outputs refuse omitted inputs; native Cargo reads live source.
        option_env!("JAUNDER_REPOSITORY_POLICY_SOURCE"),
    ) {
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
