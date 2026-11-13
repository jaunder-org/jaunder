//! Focused Rust proof: `test-local [-- <nextest args>]` runs host-native tests
//! with shared sccache and isolated PostgreSQL. Rerun the same focused command
//! after a fix; normal hooks and CI keep their boundary authority.

use clap::Parser;

/// Focused Rust proof: pass nextest selectors after `--` while iterating.
///
/// Rerun the same focused command after a fix. This accelerates feedback;
/// normal commit/push hooks and CI retain their boundary authority. Runs
/// host-native tests with shared sccache and isolated PostgreSQL, and prints one
/// JSON report on stdout.
#[derive(Parser)]
#[command(name = "test-local")]
struct Cli {
    /// Arguments forwarded to `cargo nextest run`.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    nextest_args: Vec<String>,
}

fn main() {
    let cli = host_proof::report::parse_args::<Cli>();
    let code = match host_proof::invocation_trace::execute_test_local(&cli.nextest_args) {
        Ok(result) => result.emit(),
        Err(error) => {
            eprintln!("test-local: {error:#}");
            2
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn help_redirects_iteration_without_replacing_gates() {
        let help = Cli::try_parse_from(["test-local", "--help"])
            .err()
            .unwrap()
            .to_string();
        let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
        for phrase in ["Focused Rust proof", "same focused command", "hooks", "CI"] {
            assert!(help.contains(phrase), "missing {phrase:?}: {help}");
        }
    }

    #[test]
    fn parses_defaults_and_forwarded_nextest_args() {
        assert!(
            Cli::try_parse_from(["test-local"])
                .unwrap()
                .nextest_args
                .is_empty()
        );
        let cli = Cli::try_parse_from([
            "test-local",
            "--",
            "-p",
            "storage",
            "site_config_primitives_round_trip",
        ])
        .unwrap();
        assert_eq!(
            cli.nextest_args,
            ["-p", "storage", "site_config_primitives_round_trip"]
        );
    }
}
