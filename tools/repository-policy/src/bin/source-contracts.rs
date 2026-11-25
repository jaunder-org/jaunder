//! Native source-policy verdicts.
use repository_policy::source_contracts::Operation;
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let operation = match args.as_slice() {
        [name] => Operation::parse(name),
        _ => None,
    };
    let Some(operation) = operation else {
        eprintln!("usage: source-contracts <fixed-policy>");
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
