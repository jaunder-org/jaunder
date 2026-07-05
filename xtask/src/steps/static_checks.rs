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
/// The 7 non-compiling checks (`fmt`, `leptosfmt`, `prettier`, `tsc`, `elisp-fmt`,
/// `ert`, `tools-fmt`) run through `devtool check <name>` — devtool owns their tool +
/// args (the single source of truth; #188), and the nix `static-checks` derivation runs
/// the same command. The *compiling* checks (`clippy`, `cargo-deny`, `tools-clippy`) and
/// the `xtask` self-lint stay native `cargo` invocations here — they need built deps, or
/// `xtask/` is out of the flake source. `tools/` is a virtual workspace (needs `--all`);
/// `xtask/` has a root package (no `--all`).
pub fn specs(mode: Mode) -> Vec<StepSpec> {
    // xtask/ workspace: a separate workspace *with* a root package, so a bare
    // `--manifest-path` covers it (no `--all`, unlike tools/).
    let xtask_fmt_args = match mode {
        Mode::Check => vec!["fmt", "--manifest-path", "xtask/Cargo.toml", "--check"],
        Mode::Fix => vec!["fmt", "--manifest-path", "xtask/Cargo.toml"],
    };

    vec![
        devtool_check("fmt", mode),
        devtool_check("leptosfmt", mode),
        devtool_check("prettier", mode),
        // tsc — `devtool check tsc` provisions end2end/node_modules first (the former
        // `tsc-deps` step, now folded in) then type-checks; verify-only.
        devtool_check("tsc", mode),
        devtool_check("elisp-fmt", mode),
        devtool_check("ert", mode),
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
        devtool_check("tools-fmt", mode),
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

/// A migrated (non-compiling) static check: run it through `devtool check <name>` so
/// devtool is the single source of truth for its tool+args, launched via `cargo run`
/// from the `tools/` workspace so a local edit is reflected — consistent with `xtask`
/// itself being rebuilt each run. The nix `static-checks` derivation runs the same
/// `devtool check` from the prebuilt `devtoolBin`. Fix mode appends `--fix`.
fn devtool_check(name: &'static str, mode: Mode) -> StepSpec {
    let mut args = vec![
        "run",
        "--quiet",
        "--manifest-path",
        "tools/Cargo.toml",
        "-p",
        "devtool",
        "--",
        "check",
        name,
    ];
    if matches!(mode, Mode::Fix) {
        args.push("--fix");
    }
    StepSpec {
        name,
        program: "cargo",
        args,
    }
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
    fn migrated_checks_delegate_to_devtool() {
        // The 7 non-compiling checks now run via `cargo run -p devtool -- check <name>`
        // (devtool owns their tool+args); fix mode appends --fix.
        let s = specs(Mode::Check);
        let fmt = find(&s, "fmt");
        assert_eq!(fmt.program, "cargo");
        assert_eq!(
            fmt.args,
            [
                "run",
                "--quiet",
                "--manifest-path",
                "tools/Cargo.toml",
                "-p",
                "devtool",
                "--",
                "check",
                "fmt"
            ]
        );
        let fix_specs = specs(Mode::Fix);
        let prettier_fix = find(&fix_specs, "prettier");
        assert!(
            prettier_fix.args.contains(&"--fix"),
            "fix mode passes --fix: {:?}",
            prettier_fix.args
        );
        // tsc-deps is gone — folded into `devtool check tsc`.
        assert!(specs(Mode::Check).iter().all(|s| s.name != "tsc-deps"));
    }

    #[test]
    fn native_checks_stay_native() {
        // The compiling checks + xtask self-lint still run cargo directly.
        let s = specs(Mode::Check);
        assert_eq!(
            find(&s, "clippy").args,
            ["clippy", "--all-targets", "--", "-D", "warnings"]
        );
        assert_eq!(find(&s, "cargo-deny").args, ["deny", "check"]);
        assert_eq!(find(&s, "xtask-clippy").program, "cargo");
    }

    #[test]
    fn step_order_is_locked() {
        let expected = [
            "fmt",
            "leptosfmt",
            "prettier",
            "tsc",
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
