//! Typed native boundaries for precommit reconciliation and clean prepush docs.
fn main() {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let operation = match args.as_slice() {
        [arg] if arg == "precommit" => "precommit",
        [arg] if arg == "prepush-docs" => "prepush-docs",
        _ => {
            eprintln!("usage: git-index-safety <precommit|prepush-docs>");
            std::process::exit(2);
        }
    };
    if let Err(error) = repository_policy::checkout::prepare_policy_checkout("git-index-safety") {
        eprintln!("git-index-safety: {error:#}");
        std::process::exit(if operation == "precommit" { 1 } else { 2 });
    }
    let root = std::path::Path::new(".");
    let result = match operation {
        "precommit" => repository_policy::safety::precommit(root),
        _ => repository_policy::safety::static_docs(root, true),
    };
    match result {
        Ok(()) => println!("[ ok ] {operation}"),
        Err(error) => {
            eprintln!("[FAIL] {operation} — {error:#}");
            std::process::exit(1);
        }
    }
}
