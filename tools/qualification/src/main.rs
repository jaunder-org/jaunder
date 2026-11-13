//! `qualification-host`: the single host-run qualification executable. No
//! arguments selects the mutable local smoke; `--expected-harness` alone selects
//! local styling; product inputs select discovery or acceptance.

use clap::Parser;

#[derive(Parser)]
#[command(name = "qualification-host")]
struct Cli {
    #[arg(long)]
    expected_harness: Option<String>,
    #[arg(long, requires = "expected_harness")]
    source: Option<String>,
    #[arg(long, requires = "source")]
    target: Option<String>,
}

fn main() {
    let cli = Cli::parse();
    match qualification::run_leaf(
        cli.expected_harness.as_deref(),
        cli.source.as_deref(),
        cli.target.as_deref(),
    ) {
        Ok(detail) => println!("{detail}"),
        Err(error) => {
            eprintln!("qualification: {error:#}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn product_inputs_require_explicit_harness() {
        let parse = |args: &[&str]| Cli::try_parse_from(args).is_ok();
        assert!(!parse(&["qualification-host", "--source", "HEAD"]));
        assert!(!parse(&["qualification-host", "--target", "HEAD"]));
        assert!(!parse(&[
            "qualification-host",
            "--expected-harness",
            "a",
            "--target",
            "HEAD"
        ]));
        assert!(parse(&["qualification-host"]));
        assert!(parse(&[
            "qualification-host",
            "--expected-harness",
            "a",
            "--source",
            "b",
            "--target",
            "c"
        ]));
    }
}
