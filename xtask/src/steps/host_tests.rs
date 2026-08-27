use xshell::Shell;

use crate::result::CommandResult;
use crate::sh::step;

const XTASK_TEST_ARGS: &[&str] = &[
    "test",
    "--manifest-path",
    "xtask/Cargo.toml",
    "--lib",
    "--bins",
    "--tests",
];
const TOOLS_TEST_ARGS: &[&str] = &[
    "test",
    "--manifest-path",
    "tools/Cargo.toml",
    "--lib",
    "--bins",
    "--tests",
];

fn auxiliary_test_commands() -> [(&'static str, &'static [&'static str]); 2] {
    [
        ("xtask-tests", XTASK_TEST_ARGS),
        ("tools-test", TOOLS_TEST_ARGS),
    ]
}

/// Run fast host-side unit tests for auxiliary workspaces whose unit suites are
/// not executed by root application coverage or Nix test gates. Runs in every
/// mode; it is NOT the heavy Nix instrumented suite that `--no-test` /
/// `--no-e2e` skip. No coverage here.
pub fn run(sh: &Shell, result: &mut CommandResult) {
    // Doctests run only in `doctest-fences`, where captured output can be
    // reconciled with the source population rather than discarded.
    for (name, args) in auxiliary_test_commands() {
        result.push(step(sh, name, "cargo", args));
    }
}

#[cfg(test)]
mod tests {
    use super::auxiliary_test_commands;

    #[test]
    fn auxiliary_host_tests_cover_non_doc_targets_without_running_doctests() {
        let commands = auxiliary_test_commands();

        assert_eq!(commands.len(), 2);
        for (_, args) in commands {
            assert!(!args.contains(&"--doc"), "{args:?}");
            assert!(args.contains(&"--lib"), "{args:?}");
            assert!(args.contains(&"--bins"), "{args:?}");
            assert!(args.contains(&"--tests"), "{args:?}");
        }
    }
}
