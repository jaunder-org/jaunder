use xshell::Shell;

use crate::result::{CommandResult, Mode};
use crate::sh::step;

/// A single static-check step: a named command and its arguments, already
/// resolved for the active `Mode`.
pub struct StepSpec {
    pub name: &'static str,
    pub program: &'static str,
    pub args: Vec<&'static str>,
}

/// The ordered static-check steps for `mode`. Pure (no I/O) so the step list
/// and its mode-dependent arguments can be unit-tested without shelling out.
///
/// Command invocations are kept verbatim with `scripts/verify` Phase 1 + 2,
/// adjusted only for the Fix/Check switch on the formatting tools. `tools/` is a
/// virtual workspace (needs `--all`); `xtask/` has a root package (no `--all`).
pub fn specs(mode: Mode) -> Vec<StepSpec> {
    // cargo fmt — scripts/verify uses `cargo fmt --check` (no --all)
    let fmt_args = match mode {
        Mode::Check => vec!["fmt", "--check"],
        Mode::Fix => vec!["fmt"],
    };
    // leptosfmt — scripts/verify: leptosfmt -x .direnv -x .git -x target --check '**/*.rs'
    let leptos_args = match mode {
        Mode::Check => vec![
            "-x", ".direnv", "-x", ".git", "-x", "target", "--check", "**/*.rs",
        ],
        Mode::Fix => vec!["-x", ".direnv", "-x", ".git", "-x", "target", "**/*.rs"],
    };
    // prettier — scripts/verify: prettier --check end2end
    let prettier_args = match mode {
        Mode::Check => vec!["--check", "end2end"],
        Mode::Fix => vec!["-w", "end2end"],
    };
    // elisp-fmt — emacs-batch indentation; prettier cannot format Emacs Lisp, so
    // the elisp subproject is formatted with built-in emacs-lisp-mode indentation.
    let elisp_fmt_args = match mode {
        Mode::Check => vec![
            "--batch",
            "-Q",
            "-l",
            "elisp/scripts/format.el",
            "-f",
            "jaunder-fmt-check",
        ],
        Mode::Fix => vec![
            "--batch",
            "-Q",
            "-l",
            "elisp/scripts/format.el",
            "-f",
            "jaunder-fmt-fix",
        ],
    };
    // tools/ workspace (coverage + devtool): a separate *virtual* workspace, so
    // `--all` is required because the workspace root has no package targets.
    let tools_fmt_args = match mode {
        Mode::Check => vec![
            "fmt",
            "--manifest-path",
            "tools/Cargo.toml",
            "--all",
            "--check",
        ],
        Mode::Fix => vec!["fmt", "--manifest-path", "tools/Cargo.toml", "--all"],
    };
    // xtask/ workspace: a separate workspace *with* a root package, so a bare
    // `--manifest-path` covers it (no `--all`, unlike tools/).
    let xtask_fmt_args = match mode {
        Mode::Check => vec!["fmt", "--manifest-path", "xtask/Cargo.toml", "--check"],
        Mode::Fix => vec!["fmt", "--manifest-path", "xtask/Cargo.toml"],
    };

    vec![
        StepSpec {
            name: "fmt",
            program: "cargo",
            args: fmt_args,
        },
        StepSpec {
            name: "leptosfmt",
            program: "leptosfmt",
            args: leptos_args,
        },
        StepSpec {
            name: "prettier",
            program: "prettier",
            args: prettier_args,
        },
        StepSpec {
            name: "elisp-fmt",
            program: "emacs",
            args: elisp_fmt_args,
        },
        StepSpec {
            name: "ert",
            program: "emacs",
            args: vec!["--batch", "-Q", "-l", "elisp/scripts/run-tests.el"],
        },
        StepSpec {
            name: "cargo-deny",
            program: "cargo",
            args: vec!["deny", "check"],
        },
        // clippy — scripts/verify: cargo clippy --all-targets -- -D warnings (no --workspace)
        StepSpec {
            name: "clippy",
            program: "cargo",
            args: vec!["clippy", "--all-targets", "--", "-D", "warnings"],
        },
        StepSpec {
            name: "tools-fmt",
            program: "cargo",
            args: tools_fmt_args,
        },
        StepSpec {
            name: "tools-clippy",
            program: "cargo",
            args: vec![
                "clippy",
                "--manifest-path",
                "tools/Cargo.toml",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        },
        StepSpec {
            name: "xtask-fmt",
            program: "cargo",
            args: xtask_fmt_args,
        },
        StepSpec {
            name: "xtask-clippy",
            program: "cargo",
            args: vec![
                "clippy",
                "--manifest-path",
                "xtask/Cargo.toml",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        },
    ]
}

/// Run the static check suite. In `Mode::Fix`, formatting commands auto-fix in
/// place; in `Mode::Check`, every command is read-only — safe for CI.
pub fn run(sh: &Shell, mode: Mode, result: &mut CommandResult) {
    for spec in specs(mode) {
        result.push(step(sh, spec.name, spec.program, &spec.args));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(specs: &'a [StepSpec], name: &str) -> &'a StepSpec {
        specs.iter().find(|s| s.name == name).expect("step present")
    }

    #[test]
    fn xtask_fmt_checks_in_check_mode() {
        let s = specs(Mode::Check);
        let xtask_fmt = find(&s, "xtask-fmt");
        assert_eq!(xtask_fmt.program, "cargo");
        assert_eq!(
            xtask_fmt.args,
            ["fmt", "--manifest-path", "xtask/Cargo.toml", "--check"]
        );
    }

    #[test]
    fn xtask_fmt_writes_in_fix_mode() {
        let s = specs(Mode::Fix);
        let xtask_fmt = find(&s, "xtask-fmt");
        assert_eq!(
            xtask_fmt.args,
            ["fmt", "--manifest-path", "xtask/Cargo.toml"]
        );
    }

    #[test]
    fn xtask_clippy_denies_warnings_in_both_modes() {
        for mode in [Mode::Check, Mode::Fix] {
            let s = specs(mode);
            let xtask_clippy = find(&s, "xtask-clippy");
            assert_eq!(xtask_clippy.program, "cargo");
            assert_eq!(
                xtask_clippy.args,
                [
                    "clippy",
                    "--manifest-path",
                    "xtask/Cargo.toml",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings"
                ]
            );
        }
    }

    #[test]
    fn elisp_fmt_checks_in_check_writes_in_fix() {
        let check = find(&specs(Mode::Check), "elisp-fmt").args.clone();
        assert_eq!(
            check,
            [
                "--batch",
                "-Q",
                "-l",
                "elisp/scripts/format.el",
                "-f",
                "jaunder-fmt-check"
            ]
        );
        let fix = find(&specs(Mode::Fix), "elisp-fmt").args.clone();
        assert_eq!(
            fix,
            [
                "--batch",
                "-Q",
                "-l",
                "elisp/scripts/format.el",
                "-f",
                "jaunder-fmt-fix"
            ]
        );
    }

    #[test]
    fn ert_runs_the_batch_runner_in_both_modes() {
        for mode in [Mode::Check, Mode::Fix] {
            let s = specs(mode);
            let ert = find(&s, "ert");
            assert_eq!(ert.program, "emacs");
            assert_eq!(
                ert.args,
                ["--batch", "-Q", "-l", "elisp/scripts/run-tests.el"]
            );
        }
    }

    #[test]
    fn step_order_is_locked() {
        let expected = [
            "fmt",
            "leptosfmt",
            "prettier",
            "elisp-fmt",
            "ert",
            "cargo-deny",
            "clippy",
            "tools-fmt",
            "tools-clippy",
            "xtask-fmt",
            "xtask-clippy",
        ];
        for mode in [Mode::Check, Mode::Fix] {
            let names: Vec<&str> = specs(mode).iter().map(|s| s.name).collect();
            assert_eq!(names, expected);
        }
    }
}
