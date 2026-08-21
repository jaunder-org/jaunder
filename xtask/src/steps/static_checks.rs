use xshell::Shell;

use crate::compile_cache;
use crate::result::{CommandResult, Mode};
use crate::sh::{step, step_with_env};

/// A single static-check step: a named command and its arguments, already
/// resolved for the active `Mode`.
pub struct StepSpec {
    pub name: &'static str,
    pub program: &'static str,
    pub args: Vec<&'static str>,
    pub cache_rustc: bool,
}

/// The ordered static-check steps for `mode`. Pure (no I/O) so the step list
/// and its mode-dependent arguments can be unit-tested without shelling out.
///
/// Devtool-owned checks run through `devtool check <name>` — devtool owns their
/// tool + args (the single source of truth; #188/#276), and the nix
/// `static-checks` derivation runs the same definitions. Product/tool
/// compiling checks still opt into host `sccache` through `cache_rustc`; the
/// `xtask` self-lint stays native because `xtask/` is out of the flake source.
/// `xtask/` has a root package (no `--all`).
pub fn specs(mode: Mode) -> Vec<StepSpec> {
    // xtask/ workspace: a separate workspace *with* a root package, so a bare
    // `--manifest-path` covers it (no `--all`, unlike tools/).
    let xtask_fmt_args = match mode {
        Mode::Check => vec!["fmt", "--manifest-path", "xtask/Cargo.toml", "--check"],
        Mode::Fix => vec!["fmt", "--manifest-path", "xtask/Cargo.toml"],
    };

    vec![
        // The migrated checks — keep this set in sync with
        // `devtool::check::ALL` (tools/devtool/src/check.rs), which drives the nix
        // `static-checks` derivation's `devtool check --all`. They are interleaved with
        // the xtask-only native checks below in the host gate's order.
        devtool_check("fmt", mode),
        devtool_check("leptosfmt", mode),
        devtool_check("prettier", mode),
        // tsc — `devtool check tsc` provisions end2end/node_modules first (the former
        // `tsc-deps` step, now folded in) then type-checks; verify-only.
        devtool_check("tsc", mode),
        devtool_check("elisp-fmt", mode),
        devtool_check("ert", mode),
        devtool_check("byte-compile", mode),
        devtool_check("cargo-deny", mode),
        devtool_compile_check("clippy", mode),
        devtool_compile_check("wasm-clippy", mode),
        devtool_check("tools-fmt", mode),
        devtool_compile_check("tools-clippy", mode),
        StepSpec {
            name: "xtask-fmt",
            program: "cargo",
            args: xtask_fmt_args,
            cache_rustc: false,
        },
        cargo_compile_check(
            "xtask-clippy",
            vec![
                "clippy",
                "--manifest-path",
                "xtask/Cargo.toml",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
    ]
}

/// A migrated static check: run it through `devtool check <name>` so devtool is
/// the single source of truth for its tool+args, launched via `cargo run` from
/// the `tools/` workspace so a local edit is reflected — consistent with
/// `xtask` itself being rebuilt each run. The nix `static-checks` derivation
/// runs the same `devtool check` from the prebuilt `devtoolBin`. Fix mode
/// appends `--fix`.
fn devtool_check(name: &'static str, mode: Mode) -> StepSpec {
    devtool_check_with_cache(name, mode, false)
}

fn devtool_compile_check(name: &'static str, mode: Mode) -> StepSpec {
    devtool_check_with_cache(name, mode, true)
}

fn devtool_check_with_cache(name: &'static str, mode: Mode, cache_rustc: bool) -> StepSpec {
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
        cache_rustc,
    }
}

fn cargo_compile_check(name: &'static str, args: Vec<&'static str>) -> StepSpec {
    StepSpec {
        name,
        program: "cargo",
        args,
        cache_rustc: true,
    }
}

/// Run the static check suite. In `Mode::Fix`, formatting commands auto-fix in
/// place; in `Mode::Check`, every command is read-only — safe for CI.
pub fn run(sh: &Shell, mode: Mode, result: &mut CommandResult) {
    for spec in specs(mode) {
        if spec.cache_rustc {
            let (env, cache_detail) = compile_cache::cargo_compile_env();
            let mut step = step_with_env(sh, spec.name, spec.program, &spec.args, &env);
            if let Some(cache_detail) = cache_detail {
                step.detail = Some(match step.detail.take() {
                    Some(command_detail) => format!("{command_detail}\n{cache_detail}"),
                    None => cache_detail,
                });
            }
            result.push(step);
        } else {
            result.push(step(sh, spec.name, spec.program, &spec.args));
        }
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
        // The static checks devtool owns run via `cargo run -p devtool -- check <name>`
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
        assert!(!fmt.cache_rustc);
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
    fn compiling_project_checks_delegate_to_devtool_with_cacheability() {
        let s = specs(Mode::Check);

        for name in ["cargo-deny", "clippy", "wasm-clippy", "tools-clippy"] {
            let step = find(&s, name);
            assert_eq!(step.program, "cargo");
            assert_eq!(
                step.args,
                [
                    "run",
                    "--quiet",
                    "--manifest-path",
                    "tools/Cargo.toml",
                    "-p",
                    "devtool",
                    "--",
                    "check",
                    name,
                ]
            );
        }

        assert!(!find(&s, "cargo-deny").cache_rustc);
        assert!(find(&s, "clippy").cache_rustc);
        assert!(find(&s, "wasm-clippy").cache_rustc);
        assert!(find(&s, "tools-clippy").cache_rustc);
        assert_eq!(find(&s, "xtask-clippy").program, "cargo");
        assert!(find(&s, "xtask-clippy").cache_rustc);
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
            "byte-compile",
            "cargo-deny",
            "clippy",
            "wasm-clippy",
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
