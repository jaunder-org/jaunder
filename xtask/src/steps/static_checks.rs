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
            devtool_check("prettier-markdown", mode),
            devtool_check("prettier-end2end", mode),
            devtool_check("prettier-css", mode),
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
/// `xtask` itself being rebuilt each run. The Nix static derivations run the
/// corresponding devtool check groups from the prebuilt `devtoolBin`. Fix mode
/// appends `--fix`.
fn devtool_check(name: &'static str, mode: Mode) -> StepSpec {
    // Bootstrapping devtool compiles Rust even when the selected check does not.
    devtool_check_with_cache(name, mode, true)
}

fn devtool_compile_check(name: &'static str, mode: Mode) -> StepSpec {
    devtool_check_with_cache(name, mode, true)
}

fn devtool_check_with_cache(name: &'static str, mode: Mode, cache_rustc: bool) -> StepSpec {
    let mut args = vec![
        "run",
        "--locked",
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
        markdown_eligible: matches!(name, "prettier-markdown" | "prettier-end2end"),
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
    run_spec_with(
        spec,
        compile_cache::cargo_compile_env,
        |name, program, args, env| sh::step_with_env(sh, name, program, args, env),
    )
}

/// Keep cache policy and command execution observable without spawning test children.
fn run_spec_with(
    spec: &StepSpec,
    cache_env: impl FnOnce() -> (Vec<(String, String)>, Option<String>),
    run: impl FnOnce(&str, &str, &[&str], &[(String, String)]) -> StepResult,
) -> StepResult {
    let (mut env, cache_detail) = if spec.cache_rustc {
        cache_env()
    } else {
        (Vec::new(), None)
    };
    // Reduce newly generated native debug bytes, including the devtool launcher
    // and its compiling children. Keep the shared cache adapter and Cargo args
    // unchanged; format-only Cargo commands do not need a profile override.
    // This is a capacity experiment, not attribution of the unmeasured footprint.
    if spec.program == "cargo" && (spec.cache_rustc || spec.args.first() == Some(&"run")) {
        env.push(("CARGO_PROFILE_DEV_DEBUG".to_owned(), "0".to_owned()));
    }
    let mut result = run(spec.name, spec.program, &spec.args, &env);
    if let Some(cache_detail) = cache_detail {
        result.detail = Some(match result.detail.take() {
            Some(command_detail) => format!("{command_detail}\n{cache_detail}"),
            None => cache_detail,
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devtool_bootstrap_receives_compile_cache_environment() {
        let provider_env = vec![
            ("RUSTC_WRAPPER".to_string(), "sccache".to_string()),
            ("CARGO_INCREMENTAL".to_string(), "0".to_string()),
            ("SCCACHE_BASEDIRS".to_string(), "/worktree".to_string()),
        ];
        let mut expected_env = provider_env.clone();
        expected_env.push(("CARGO_PROFILE_DEV_DEBUG".to_string(), "0".to_string()));
        for mode in [Mode::Check, Mode::Fix] {
            for spec in specs(mode)
                .into_iter()
                .filter(|s| s.args.first() == Some(&"run"))
            {
                let mut provider_calls = 0;
                let mut recorded = None;
                let result = run_spec_with(
                    &spec,
                    || {
                        provider_calls += 1;
                        (provider_env.clone(), None)
                    },
                    |name, program, args, env| {
                        recorded = Some((
                            name.to_string(),
                            program.to_string(),
                            args.iter()
                                .map(|arg| (*arg).to_string())
                                .collect::<Vec<_>>(),
                            env.to_vec(),
                        ));
                        StepResult::ok(name)
                    },
                );
                let (name, program, args, env) = recorded.expect("runner called");
                assert_eq!(env, expected_env, "{} bootstrap environment", spec.name);
                assert_eq!(provider_calls, 1);
                assert_eq!(name, spec.name);
                assert_eq!(program, "cargo");
                let mut expected_args = vec![
                    "run",
                    "--locked",
                    "--quiet",
                    "--manifest-path",
                    "tools/Cargo.toml",
                    "-p",
                    "devtool",
                    "--",
                    "check",
                    spec.name,
                ];
                if matches!(mode, Mode::Fix) {
                    expected_args.push("--fix");
                }
                assert_eq!(args, expected_args);
                assert!(result.ok);
            }
        }
    }

    #[test]
    fn direct_formatting_does_not_request_cache_environment() {
        for mode in [Mode::Check, Mode::Fix] {
            let catalog = specs(mode);
            let spec = find(&catalog, "xtask-fmt");
            let result = run_spec_with(
                spec,
                || panic!("direct formatting needs no cache"),
                |name, program, args, env| {
                    assert_eq!(name, "xtask-fmt");
                    assert_eq!(program, "cargo");
                    assert_eq!(args, spec.args);
                    assert!(env.is_empty());
                    StepResult::fail(name).detail("formatter failure")
                },
            );
            assert!(!result.ok);
            assert_eq!(result.detail.as_deref(), Some("formatter failure"));
        }
    }

    #[test]
    fn cache_warning_preserves_runner_outcome_and_detail() {
        let catalog = specs(Mode::Check);
        let spec = find(&catalog, "clippy");
        for ok in [true, false] {
            for detail in [None, Some("command detail")] {
                for warning in [None, Some("cache warning")] {
                    let result = run_spec_with(
                        spec,
                        || (vec![], warning.map(str::to_string)),
                        |name, _, _, env| {
                            assert_eq!(
                                env,
                                &[("CARGO_PROFILE_DEV_DEBUG".to_string(), "0".to_string())]
                            );
                            let mut result = if ok {
                                StepResult::ok(name)
                            } else {
                                StepResult::fail(name)
                            };
                            result.detail = detail.map(str::to_string);
                            result
                        },
                    );
                    assert_eq!(result.ok, ok);
                    assert_eq!(result.name, spec.name);
                    let expected = match (detail, warning) {
                        (Some(d), Some(w)) => Some(format!("{d}\n{w}")),
                        (Some(d), None) => Some(d.to_string()),
                        (None, Some(w)) => Some(w.to_string()),
                        (None, None) => None,
                    };
                    assert_eq!(result.detail, expected);
                }
            }
        }
    }

    fn find<'a>(specs: &'a [StepSpec], name: &str) -> &'a StepSpec {
        specs.iter().find(|s| s.name == name).expect("step present")
    }

    #[cfg(unix)]
    #[test]
    fn run_spec_scopes_dev_debug_to_native_cargo_without_changing_cache_or_verdict() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let script = "#!/bin/sh\nset -eu\ntest \"$CARGO_PROFILE_DEV_DEBUG\" = \"$EXPECTED_DEBUG\"\ntest \"$CARGO_PROFILE_TEST_DEBUG\" = 2\ntest \"$CARGO_BUILD_JOBS\" = 7\nif [ \"$1\" = clippy ]; then\n test \"$RUSTC_WRAPPER\" = sccache\n test \"$CARGO_INCREMENTAL\" = 0\nelse\n test \"$RUSTC_WRAPPER\" = fixture-wrapper\n test \"$CARGO_INCREMENTAL\" = 1\nfi\nif [ \"${2:-}\" = --fail ]; then exit 17; fi\n";
        for program in ["cargo", "fixture-tool"] {
            let path = directory.path().join(program);
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let sh = Shell::new().unwrap();
        sh.change_dir(directory.path());
        let path = format!("{}:{}", directory.path().display(), sh.var("PATH").unwrap());
        let _path = sh.push_env("PATH", path);
        let _dev = sh.push_env("CARGO_PROFILE_DEV_DEBUG", "2");
        let _test = sh.push_env("CARGO_PROFILE_TEST_DEBUG", "2");
        let _jobs = sh.push_env("CARGO_BUILD_JOBS", "7");
        let _wrapper = sh.push_env("RUSTC_WRAPPER", "fixture-wrapper");
        let _incremental = sh.push_env("CARGO_INCREMENTAL", "1");
        for (program, command, cache_rustc, expected) in [
            ("cargo", "run", false, "0"),
            ("cargo", "clippy", true, "0"),
            ("cargo", "fmt", false, "2"),
            ("fixture-tool", "run", false, "2"),
        ] {
            let _expected = sh.push_env("EXPECTED_DEBUG", expected);
            let mut spec = StepSpec {
                name: "env-fixture",
                program,
                args: vec![command],
                cache_rustc,
                markdown_eligible: false,
            };
            let result = run_spec(&sh, &spec);
            assert!(result.ok, "{:?}", result.detail);
            spec.args.push("--fail");
            let failed = run_spec(&sh, &spec);
            assert!(!failed.ok);
            assert!(failed.detail.unwrap().contains("exited with status 17"));
        }
        assert_eq!(sh.var("CARGO_PROFILE_DEV_DEBUG").unwrap(), "2");
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
                "--locked",
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
        assert!(fmt.cache_rustc);
        let ast_grep_tests = find(&s, "ast-grep-tests");
        assert_eq!(ast_grep_tests.program, "cargo");
        assert_eq!(
            ast_grep_tests.args,
            [
                "run",
                "--locked",
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
        assert!(ast_grep_tests.cache_rustc);
        let no_full_reload = find(&s, "no-full-reload");
        assert_eq!(no_full_reload.program, "cargo");
        assert_eq!(
            no_full_reload.args,
            [
                "run",
                "--locked",
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
        assert!(no_full_reload.cache_rustc);
        let fix_specs = specs(Mode::Fix);
        let prettier_fix = find(&fix_specs, "prettier-markdown");
        assert_eq!(
            prettier_fix.args,
            [
                "run",
                "--locked",
                "--quiet",
                "--manifest-path",
                "tools/Cargo.toml",
                "-p",
                "devtool",
                "--",
                "check",
                "prettier-markdown",
                "--fix",
            ]
        );
        // tsc-deps is gone — folded into `devtool check tsc`.
        assert!(specs(Mode::Check).iter().all(|s| s.name != "tsc-deps"));
    }

    #[test]
    fn both_prettier_checks_are_markdown_eligible_in_catalog_order() {
        for mode in [Mode::Check, Mode::Fix] {
            let eligible: Vec<_> = specs(mode)
                .into_iter()
                .filter(|spec| spec.markdown_eligible)
                .map(|spec| spec.name)
                .collect();
            assert_eq!(eligible, ["prettier-markdown", "prettier-end2end"]);
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
                    "--locked",
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

        assert!(find(&s, "cargo-deny").cache_rustc);
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
            "prettier-markdown",
            "prettier-end2end",
            "prettier-css",
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
                "prettier-markdown",
                "prettier-end2end",
                "prettier-css",
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
