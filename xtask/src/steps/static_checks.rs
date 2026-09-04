use xshell::Shell;

use crate::compile_cache;
use crate::result::{Mode, StepResult};
use crate::sh;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// Fast source-shape checks that can auto-fix or report concrete files before
    /// any expensive compile/type work starts.
    SourceConsistency,
    /// Checks that invoke compilers, type checkers, or dependency analyzers.
    CompileAndType,
    /// Runtime test surfaces that depend on prior compile/type success.
    HostRuntime,
}
/// A single static-check step: a named command and its arguments, already
/// resolved for the active `Mode`.
pub struct StepSpec {
    pub name: &'static str,
    pub program: &'static str,
    pub args: Vec<&'static str>,
    pub cache_rustc: bool,
    pub markdown_eligible: bool,
}

/// The ordered static-check steps for `phase` and `mode`. Pure (no I/O) so the
/// step list and its mode-dependent arguments can be unit-tested without
/// shelling out.
///
/// Ordering policy: fixable/source-shape checks first, then compiler/type
/// surfaces. Repository-wide health checks that do not need a compiler are
/// interleaved by the host gate between these phases.
pub fn specs_for_phase(phase: Phase, mode: Mode) -> Vec<StepSpec> {
    // xtask/ workspace: a separate workspace *with* a root package, so a bare
    // `--manifest-path` covers it (no `--all`, unlike tools/).
    let xtask_fmt_args = match mode {
        Mode::Check => vec!["fmt", "--manifest-path", "xtask/Cargo.toml", "--check"],
        Mode::Fix => vec!["fmt", "--manifest-path", "xtask/Cargo.toml"],
    };

    match phase {
        Phase::SourceConsistency => vec![
            devtool_check("fmt", mode),
            devtool_check("leptosfmt", mode),
            devtool_check("prettier", mode),
            devtool_check("elisp-fmt", mode),
            devtool_check("tools-fmt", mode),
            devtool_check("ast-grep-tests", mode),
            devtool_check("no-full-reload", mode),
            StepSpec {
                name: "xtask-fmt",
                program: "cargo",
                args: xtask_fmt_args,
                cache_rustc: false,
                markdown_eligible: false,
            },
        ],
        Phase::CompileAndType => vec![
            // Byte-compilation is a compile/type precondition for the elisp runtime tests.
            devtool_check("byte-compile", mode),
            // tsc — `devtool check tsc` provisions end2end/node_modules first (the former
            // `tsc-deps` step, now folded in) then type-checks; verify-only.
            devtool_check("tsc", mode),
            devtool_check("cargo-deny", mode),
            devtool_compile_check("clippy", mode),
            devtool_compile_check("web-server-clippy", mode),
            devtool_compile_check("web-no-server-clippy", mode),
            devtool_compile_check("wasm-clippy", mode),
            devtool_compile_check("tools-clippy", mode),
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
        ],
        Phase::HostRuntime => vec![devtool_check("ert", mode)],
    }
}

#[cfg(test)]
pub fn specs(mode: Mode) -> Vec<StepSpec> {
    [
        Phase::SourceConsistency,
        Phase::CompileAndType,
        Phase::HostRuntime,
    ]
    .into_iter()
    .flat_map(|phase| specs_for_phase(phase, mode))
    .collect()
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
        markdown_eligible: name == "prettier",
    }
}

fn cargo_compile_check(name: &'static str, args: Vec<&'static str>) -> StepSpec {
    StepSpec {
        name,
        program: "cargo",
        args,
        cache_rustc: true,
        markdown_eligible: false,
    }
}

pub(crate) fn run_spec(sh: &Shell, spec: &StepSpec) -> StepResult {
    if spec.cache_rustc {
        let (env, cache_detail) = compile_cache::cargo_compile_env();
        let mut result = sh::step_with_env(sh, spec.name, spec.program, &spec.args, &env);
        if let Some(cache_detail) = cache_detail {
            result.detail = Some(match result.detail.take() {
                Some(command_detail) => format!("{command_detail}\n{cache_detail}"),
                None => cache_detail,
            });
        }
        result
    } else {
        sh::step(sh, spec.name, spec.program, &spec.args)
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
        let ast_grep_tests = find(&s, "ast-grep-tests");
        assert_eq!(ast_grep_tests.program, "cargo");
        assert_eq!(
            ast_grep_tests.args,
            [
                "run",
                "--quiet",
                "--manifest-path",
                "tools/Cargo.toml",
                "-p",
                "devtool",
                "--",
                "check",
                "ast-grep-tests"
            ]
        );
        assert!(!ast_grep_tests.cache_rustc);
        let no_full_reload = find(&s, "no-full-reload");
        assert_eq!(no_full_reload.program, "cargo");
        assert_eq!(
            no_full_reload.args,
            [
                "run",
                "--quiet",
                "--manifest-path",
                "tools/Cargo.toml",
                "-p",
                "devtool",
                "--",
                "check",
                "no-full-reload"
            ]
        );
        assert!(!no_full_reload.cache_rustc);
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
    fn prettier_is_the_only_markdown_eligible_static_check() {
        for mode in [Mode::Check, Mode::Fix] {
            let eligible: Vec<_> = specs(mode)
                .into_iter()
                .filter(|spec| spec.markdown_eligible)
                .map(|spec| spec.name)
                .collect();
            assert_eq!(eligible, ["prettier"]);
        }
    }

    #[test]
    fn compiling_project_checks_delegate_to_devtool_with_cacheability() {
        let s = specs(Mode::Check);

        for name in [
            "cargo-deny",
            "clippy",
            "web-server-clippy",
            "web-no-server-clippy",
            "wasm-clippy",
            "tools-clippy",
        ] {
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
        assert!(find(&s, "web-server-clippy").cache_rustc);
        assert!(find(&s, "web-no-server-clippy").cache_rustc);
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
            "elisp-fmt",
            "tools-fmt",
            "ast-grep-tests",
            "no-full-reload",
            "xtask-fmt",
            "byte-compile",
            "tsc",
            "cargo-deny",
            "clippy",
            "web-server-clippy",
            "web-no-server-clippy",
            "wasm-clippy",
            "tools-clippy",
            "xtask-clippy",
            "ert",
        ];
        for mode in [Mode::Check, Mode::Fix] {
            let names: Vec<&str> = specs(mode).iter().map(|s| s.name).collect();
            assert_eq!(names, expected);
        }
    }

    #[test]
    fn phase_order_is_policy_visible() {
        let source_names: Vec<&str> = specs_for_phase(Phase::SourceConsistency, Mode::Check)
            .iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(
            source_names,
            [
                "fmt",
                "leptosfmt",
                "prettier",
                "elisp-fmt",
                "tools-fmt",
                "ast-grep-tests",
                "no-full-reload",
                "xtask-fmt"
            ]
        );

        let compile_names: Vec<&str> = specs_for_phase(Phase::CompileAndType, Mode::Check)
            .iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(compile_names.first(), Some(&"byte-compile"));
        assert!(compile_names.contains(&"clippy"));
        assert!(compile_names.contains(&"web-no-server-clippy"));
        assert!(!compile_names.contains(&"ert"));

        let runtime_names: Vec<&str> = specs_for_phase(Phase::HostRuntime, Mode::Check)
            .iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(runtime_names, ["ert"]);
    }
}
