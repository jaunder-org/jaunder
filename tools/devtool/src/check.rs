//! The migrated static checks. This is the single home of their tool + args:
//! the host verify ladder runs each check via `cargo run -p devtool -- check
//! <name>` (so a local `tools/` edit is reflected), while Nix runs the `docs`,
//! `typescript`, `rust-format`, `code`, and `ert` groups from prebuilt devtool inputs.
//! Cargo-backed checks use
//! separate host and sandbox lanes from this shared definition.

use std::ffi::OsString;
use std::io::Write;
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail};

mod leptosfmt;

/// A stable, closed partition of devtool's static check inventory.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum CheckGroup {
    /// Markdown formatting only.
    Docs,
    /// The TypeScript type check.
    #[value(name = "typescript")]
    TypeScript,
    /// Product/tools Rust formatting and scoped Leptos formatting.
    RustFormat,
    /// Static checks outside documentation, TypeScript, Rust-format and ERT.
    Code,
    /// Standalone pure Elisp tests.
    Ert,
}

impl CheckGroup {
    fn names(self) -> &'static [&'static str] {
        match self {
            Self::Docs => &["prettier-markdown"],
            Self::TypeScript => &["tsc"],
            Self::Ert => &["ert"],
            Self::RustFormat => &["fmt", "leptosfmt", "tools-fmt"],
            Self::Code => &[
                "prettier-end2end",
                "prettier-css",
                "elisp-fmt",
                "ast-grep-tests",
                "no-full-reload",
                "byte-compile",
                "cargo-deny",
                "product-core-clippy",
                "web-server-clippy",
                "web-no-server-clippy",
                "client-host-clippy",
                "wasm-clippy",
                "tools-clippy",
            ],
        }
    }
}

/// The complete static inventory in its prior effective order, without aliases
/// or duplicates.
fn all_names() -> Vec<&'static str> {
    // Group boundaries must not reorder the established host --all contract.
    vec![
        "fmt",
        "leptosfmt",
        "prettier-markdown",
        "prettier-end2end",
        "prettier-css",
        "elisp-fmt",
        "tools-fmt",
        "ast-grep-tests",
        "no-full-reload",
        "byte-compile",
        "tsc",
        "cargo-deny",
        "clippy",
        "web-server-clippy",
        "web-no-server-clippy",
        "wasm-clippy",
        "tools-clippy",
        "ert",
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CargoWorkspace {
    Product,
    Tools,
}

impl CargoWorkspace {
    fn cargo_home_env(self) -> &'static str {
        match self {
            Self::Product => "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME",
            Self::Tools => "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Tools => "tools",
        }
    }

    fn manifest_args(self) -> &'static [&'static str] {
        match self {
            Self::Product => &[],
            Self::Tools => &["--manifest-path", "tools/Cargo.toml"],
        }
    }

    fn cargo_args(self, args: &[String]) -> Result<Vec<String>> {
        let (subcommand, rest) = args
            .split_first()
            .with_context(|| format!("{} workspace Cargo check has no subcommand", self.name()))?;
        if rest.iter().any(|arg| arg == "--manifest-path") {
            bail!(
                "{} workspace Cargo check must not supply --manifest-path; workspace selection owns manifest routing",
                self.name()
            );
        }
        let mut routed_args = Vec::with_capacity(1 + self.manifest_args().len() + rest.len());
        routed_args.push(subcommand.clone());
        routed_args.extend(self.manifest_args().iter().map(|arg| (*arg).to_string()));
        routed_args.extend(rest.iter().cloned());
        Ok(routed_args)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CargoMode {
    Host,
    Sandbox,
}

#[derive(Debug, Eq, PartialEq)]
struct CargoCheck {
    workspace: CargoWorkspace,
    host_args: Vec<String>,
    sandbox_args: Vec<String>,
}

#[derive(Debug, Eq, PartialEq)]
enum CheckSpec {
    External {
        program: &'static str,
        args: Vec<String>,
    },
    Cargo(CargoCheck),
}

#[derive(Debug, Eq, PartialEq)]
struct BuiltCommand {
    program: &'static str,
    args: Vec<String>,
    env: Vec<(&'static str, OsString)>,
}

impl CheckSpec {
    fn build_with_env<F>(&self, cargo_mode: CargoMode, env_lookup: F) -> Result<BuiltCommand>
    where
        F: Fn(&'static str) -> Option<OsString>,
    {
        match self {
            Self::External { program, args } => Ok(BuiltCommand {
                program,
                args: args.clone(),
                env: Vec::new(),
            }),
            Self::Cargo(CargoCheck {
                workspace,
                host_args,
                sandbox_args,
            }) => match cargo_mode {
                CargoMode::Host => Ok(BuiltCommand {
                    program: "cargo",
                    args: workspace.cargo_args(host_args)?,
                    env: Vec::new(),
                }),
                CargoMode::Sandbox => {
                    let home_var = workspace.cargo_home_env();
                    let cargo_home = env_lookup(home_var)
                        .filter(|value| !value.is_empty())
                        .with_context(|| {
                            format!(
                                "{home_var} must be set for sandboxed {} workspace Cargo checks",
                                workspace.name()
                            )
                        })?;
                    let routed_args = workspace.cargo_args(sandbox_args)?;
                    let mut sandbox_args = Vec::with_capacity(routed_args.len() + 1);
                    sandbox_args.push("--offline".to_string());
                    sandbox_args.extend(routed_args);
                    Ok(BuiltCommand {
                        program: "cargo",
                        args: sandbox_args,
                        env: vec![
                            ("CARGO_HOME", cargo_home),
                            ("CARGO_NET_OFFLINE", OsString::from("true")),
                        ],
                    })
                }
            },
        }
    }
}

/// Pure: the command spec for `name` in the given mode. `fix` makes the seven
/// formatters (`fmt`, `leptosfmt`, `prettier-markdown`, `prettier-end2end`,
/// `prettier-css`, `elisp-fmt`, `tools-fmt`) mutate in place;
/// `ert`/`tsc`/`byte-compile`/
/// `ast-grep-tests`/`no-full-reload` have no autofix and ignore it.
fn spec(name: &str, fix: bool) -> Result<CheckSpec> {
    let owned = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let cargo_check = |workspace, args: &[&str]| {
        let args = owned(args);
        CheckSpec::Cargo(CargoCheck {
            workspace,
            host_args: args.clone(),
            sandbox_args: args,
        })
    };
    Ok(match name {
        "fmt" => cargo_check(
            CargoWorkspace::Product,
            if fix { &["fmt"] } else { &["fmt", "--check"] },
        ),
        "leptosfmt" => CheckSpec::External {
            program: "leptosfmt",
            args: if fix {
                owned(&[
                    "-x", ".direnv", "-x", ".git", "-x", "target", "web/src", "csr/src",
                ])
            } else {
                owned(&[
                    "-x", ".direnv", "-x", ".git", "-x", "target", "--check", "web/src", "csr/src",
                ])
            },
        },
        "prettier-markdown" => CheckSpec::External {
            program: "prettier",
            args: if fix {
                owned(&["-w", "**/*.md"])
            } else {
                owned(&["--check", "**/*.md"])
            },
        },
        "prettier-end2end" => CheckSpec::External {
            program: "prettier",
            args: if fix {
                owned(&["-w", "end2end"])
            } else {
                owned(&["--check", "end2end"])
            },
        },
        "prettier-css" => CheckSpec::External {
            program: "prettier",
            args: if fix {
                owned(&["-w", "**/*.css"])
            } else {
                owned(&["--check", "**/*.css"])
            },
        },
        "tsc" => CheckSpec::External {
            program: "tsc",
            args: owned(&["--noEmit", "-p", "end2end/tsconfig.json"]),
        },
        "elisp-fmt" => CheckSpec::External {
            program: "emacs",
            args: if fix {
                owned(&[
                    "--batch",
                    "-Q",
                    "-l",
                    "elisp/scripts/format.el",
                    "-f",
                    "jaunder-fmt-fix",
                ])
            } else {
                owned(&[
                    "--batch",
                    "-Q",
                    "-l",
                    "elisp/scripts/format.el",
                    "-f",
                    "jaunder-fmt-check",
                ])
            },
        },
        "ert" => CheckSpec::External {
            program: "emacs",
            args: owned(&["--batch", "-Q", "-l", "elisp/scripts/run-tests.el"]),
        },
        "byte-compile" => CheckSpec::External {
            program: "emacs",
            args: owned(&["--batch", "-Q", "-l", "elisp/scripts/byte-compile.el"]),
        },
        "ast-grep-tests" => CheckSpec::External {
            program: "ast-grep",
            args: owned(&["test", "--config", "sgconfig.yml"]),
        },
        "no-full-reload" => CheckSpec::External {
            program: "ast-grep",
            args: owned(&[
                "scan",
                "--config",
                "sgconfig.yml",
                "--filter",
                "^no-full-reload$",
                "web/src",
                "client/src",
            ]),
        },
        "cargo-deny" => CheckSpec::Cargo(CargoCheck {
            workspace: CargoWorkspace::Product,
            host_args: owned(&["deny", "check"]),
            sandbox_args: owned(&["deny", "check", "bans", "licenses", "sources"]),
        }),
        "product-core-clippy" => cargo_check(
            CargoWorkspace::Product,
            &[
                "clippy",
                "--workspace",
                "--exclude",
                "web",
                "--exclude",
                "client",
                "--exclude",
                "csr",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        "web-server-clippy" => cargo_check(
            CargoWorkspace::Product,
            &[
                "clippy",
                "-p",
                "web",
                "--features",
                "server",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        "web-no-server-clippy" => cargo_check(
            CargoWorkspace::Product,
            &[
                "clippy",
                "-p",
                "web",
                "--no-default-features",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        "client-host-clippy" => cargo_check(
            CargoWorkspace::Product,
            &[
                "clippy",
                "-p",
                "client",
                "--features",
                "csr",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        "wasm-clippy" => cargo_check(
            CargoWorkspace::Product,
            &[
                "clippy",
                "-p",
                "web",
                "-p",
                "client",
                "-p",
                "csr",
                "--features",
                "csr",
                "--target",
                "wasm32-unknown-unknown",
                "--",
                "-D",
                "warnings",
            ],
        ),
        "tools-fmt" => cargo_check(
            CargoWorkspace::Tools,
            if fix {
                &["fmt", "--all"]
            } else {
                &["fmt", "--all", "--check"]
            },
        ),
        "tools-clippy" => cargo_check(
            CargoWorkspace::Tools,
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        other => bail!("unknown check '{other}' (known: {:?})", all_names()),
    })
}

fn build_selected_commands_with_env<F>(
    names: &[&str],
    fix: bool,
    cargo_mode: CargoMode,
    env_lookup: F,
) -> Result<Vec<BuiltCommand>>
where
    F: Fn(&'static str) -> Option<OsString> + Copy,
{
    names
        .iter()
        .map(|name| spec(name, fix)?.build_with_env(cargo_mode, env_lookup))
        .collect()
}

/// Whether a check needs `end2end/node_modules` provisioned before it runs. Only `tsc`
/// type-checks against that closure. Kept as a pure predicate so the rule is testable
/// without executing tsc (#229).
pub fn needs_provisioning(name: &str) -> bool {
    name == "tsc"
}

/// Resolve exactly one public check selector without executing commands.
fn selected_names(name: Option<&str>, group: Option<CheckGroup>, all: bool) -> Result<Vec<&str>> {
    match (name, group, all) {
        (Some(name), None, false) => Ok(vec![name]),
        (None, Some(group), false) => Ok(group.names().to_vec()),
        (None, None, true) => Ok(all_names()),
        _ => bail!("pass exactly one of <name>, --group, or --all"),
    }
}

/// Run one check by name, a group, or all checks. `tsc` provisions
/// `end2end/node_modules` (the type-dep closure) first, by calling
/// [`crate::provision::run`] in-process.
pub fn run(
    name: Option<&str>,
    group: Option<CheckGroup>,
    all: bool,
    fix: bool,
    sandbox_cargo: bool,
) -> Result<()> {
    let names = selected_names(name, group, all)?;
    let cargo_mode = if sandbox_cargo {
        CargoMode::Sandbox
    } else {
        CargoMode::Host
    };
    let commands = build_selected_commands_with_env(&names, fix, cargo_mode, std::env::var_os)?;

    for (n, cmd) in names.iter().zip(commands) {
        if needs_provisioning(n) {
            let paths = crate::provision::StorePaths::resolve(None, None)?;
            crate::provision::run(std::path::Path::new("."), &paths)
                .context("provisioning end2end/node_modules for tsc")?;
        }
        let mut command = Command::new(cmd.program);
        command.args(&cmd.args).envs(cmd.env);
        run_check(n, &mut command, &mut std::io::stderr().lock())?;
    }
    Ok(())
}

/// Measure only the tsc or ERT child, not provisioning or launcher setup.
/// Emit one bounded versioned event after completion, including unsuccessful exits.
fn run_check(name: &str, command: &mut Command, timing_output: &mut dyn Write) -> Result<()> {
    if name == "leptosfmt" {
        return leptosfmt::run(command, &std::env::current_dir()?);
    }
    if matches!(name, "tsc" | "ert") {
        return run_timed_check(name, command, timing_output);
    }
    let status = command.status().with_context(|| {
        format!(
            "spawning `{}` for check {name}",
            command.get_program().to_string_lossy()
        )
    })?;
    if !status.success() {
        bail!("check {name} failed ({status})");
    }
    Ok(())
}

fn run_timed_check(
    check: &str,
    command: &mut Command,
    timing_output: &mut dyn Write,
) -> Result<()> {
    let started = Instant::now();
    let status = command
        .status()
        .with_context(|| format!("spawning `{check}` for check {check}"))?;
    let event = serde_json::json!({
        "event": "devtool-check-timing",
        "schema_version": 1,
        "check": check,
        "duration_ms": started.elapsed().as_millis(),
        "outcome": if status.success() { "success" } else { "failure" },
    });
    writeln!(timing_output, "{event}").with_context(|| format!("writing {check} timing event"))?;
    if !status.success() {
        bail!("check {check} failed ({status})");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_host(name: &str, fix: bool) -> BuiltCommand {
        spec(name, fix)
            .and_then(|spec| spec.build_with_env(CargoMode::Host, |_| None))
            .unwrap_or_else(|err| panic!("building host check {name}: {err}"))
    }

    #[test]
    fn only_tsc_needs_provisioning() {
        assert!(needs_provisioning("tsc"));
        for name in all_names().iter().filter(|n| **n != "tsc") {
            assert!(
                !needs_provisioning(name),
                "{name} must not provision end2end/node_modules"
            );
        }
    }

    #[test]
    fn fmt_uses_product_workspace_cargo_in_host_mode() {
        let cmd = build_host("fmt", false);

        assert_eq!(cmd.program, "cargo");
        assert_eq!(cmd.args, vec!["fmt", "--check"]);
        assert!(cmd.env.is_empty());
    }

    #[test]
    fn tools_fmt_uses_tools_workspace_manifest_in_host_mode() {
        let cmd = build_host("tools-fmt", false);

        assert_eq!(cmd.program, "cargo");
        assert_eq!(
            cmd.args,
            vec![
                "fmt",
                "--manifest-path",
                "tools/Cargo.toml",
                "--all",
                "--check",
            ]
        );
        assert!(cmd.env.is_empty());
    }

    #[test]
    fn sandbox_product_cargo_forces_offline_and_uses_product_home() {
        let cmd = spec("fmt", false)
            .unwrap()
            .build_with_env(CargoMode::Sandbox, |name| match name {
                "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" => {
                    Some("/nix/store/product-cargo-home".into())
                }

                "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME" => Some("/nix/store/tools-cargo-home".into()),
                _ => None,
            })
            .unwrap();

        assert_eq!(cmd.program, "cargo");
        assert_eq!(cmd.args, vec!["--offline", "fmt", "--check"]);
        assert!(cmd.env.contains(&(
            "CARGO_HOME",
            OsString::from("/nix/store/product-cargo-home")
        )));
        assert!(
            cmd.env
                .contains(&("CARGO_NET_OFFLINE", OsString::from("true")))
        );
    }
    #[test]
    fn workspace_selection_owns_manifest_routing() {
        let err = CheckSpec::Cargo(CargoCheck {
            workspace: CargoWorkspace::Product,
            host_args: vec![
                "fmt".to_string(),
                "--manifest-path".to_string(),
                "tools/Cargo.toml".to_string(),
            ],
            sandbox_args: vec![
                "fmt".to_string(),
                "--manifest-path".to_string(),
                "tools/Cargo.toml".to_string(),
            ],
        })
        .build_with_env(CargoMode::Host, |_| None)
        .unwrap_err()
        .to_string();

        assert!(err.contains("must not supply --manifest-path"), "{err}");
    }

    #[test]
    fn sandbox_tools_cargo_forces_offline_and_uses_tools_home() {
        let cmd = spec("tools-fmt", false)
            .unwrap()
            .build_with_env(CargoMode::Sandbox, |name| match name {
                "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" => {
                    Some("/nix/store/product-cargo-home".into())
                }
                "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME" => Some("/nix/store/tools-cargo-home".into()),
                _ => None,
            })
            .unwrap();

        assert_eq!(cmd.program, "cargo");
        assert_eq!(cmd.args[0], "--offline");
        assert!(
            cmd.args
                .windows(2)
                .any(|w| w == ["--manifest-path", "tools/Cargo.toml"])
        );
        assert!(
            cmd.env
                .contains(&("CARGO_HOME", OsString::from("/nix/store/tools-cargo-home")))
        );
    }

    #[test]
    fn sandbox_cargo_errors_before_spawn_when_workspace_home_is_missing() {
        let err = spec("tools-fmt", false)
            .unwrap()
            .build_with_env(CargoMode::Sandbox, |_| None)
            .unwrap_err()
            .to_string();

        assert!(err.contains("JAUNDER_DEVTOOL_TOOLS_CARGO_HOME"), "{err}");
    }

    #[test]
    fn sandbox_all_checks_validates_every_cargo_home_before_running() {
        let err = build_selected_commands_with_env(
            &["fmt", "tools-fmt"],
            false,
            CargoMode::Sandbox,
            |name| match name {
                "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" => {
                    Some("/nix/store/product-cargo-home".into())
                }
                _ => None,
            },
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("JAUNDER_DEVTOOL_TOOLS_CARGO_HOME"), "{err}");
    }

    #[test]
    fn leptosfmt_uses_only_frontend_roots_in_both_modes() {
        for fix in [false, true] {
            let cmd = build_host("leptosfmt", fix);
            let mut expected = vec!["-x", ".direnv", "-x", ".git", "-x", "target"];
            if !fix {
                expected.push("--check");
            }
            expected.extend(["web/src", "csr/src"]);
            assert_eq!(cmd.program, "leptosfmt");
            assert_eq!(cmd.args, expected);
        }
    }

    #[test]
    fn fmt_check_vs_fix() {
        assert_eq!(build_host("fmt", false).args, vec!["fmt", "--check"]);
        assert_eq!(build_host("fmt", true).args, vec!["fmt"]);
    }

    #[test]
    fn prettier_commands_preserve_their_distinct_populations() {
        assert_eq!(
            build_host("prettier-markdown", false),
            BuiltCommand {
                program: "prettier",
                args: vec!["--check".into(), "**/*.md".into()],
                env: Vec::new(),
            }
        );
        assert_eq!(
            build_host("prettier-end2end", false),
            BuiltCommand {
                program: "prettier",
                args: vec!["--check".into(), "end2end".into()],
                env: Vec::new(),
            }
        );
        assert_eq!(
            build_host("prettier-markdown", true).args,
            vec!["-w", "**/*.md"]
        );
        assert_eq!(
            build_host("prettier-end2end", true).args,
            vec!["-w", "end2end"]
        );
        assert_eq!(
            build_host("prettier-css", false),
            BuiltCommand {
                program: "prettier",
                args: vec!["--check".into(), "**/*.css".into()],
                env: Vec::new(),
            }
        );
        assert_eq!(
            build_host("prettier-css", true).args,
            vec!["-w", "**/*.css"]
        );
    }

    #[test]
    fn ert_and_tsc_ignore_fix() {
        assert_eq!(build_host("ert", true), build_host("ert", false));
        assert_eq!(build_host("tsc", true), build_host("tsc", false));
    }

    #[test]
    fn byte_compile_runs_the_script_and_ignores_fix() {
        assert_eq!(
            build_host("byte-compile", false),
            BuiltCommand {
                program: "emacs",
                args: vec![
                    "--batch".to_string(),
                    "-Q".into(),
                    "-l".into(),
                    "elisp/scripts/byte-compile.el".into(),
                ],
                env: Vec::new(),
            }
        );
        // No autofix — a warning is fixed by hand, so --fix is ignored.
        assert_eq!(
            build_host("byte-compile", true),
            build_host("byte-compile", false)
        );
    }

    #[test]
    fn cargo_deny_uses_full_host_policy() {
        let cmd = build_host("cargo-deny", false);

        assert_eq!(cmd.program, "cargo");
        assert_eq!(cmd.args, vec!["deny", "check"]);
        assert!(cmd.env.is_empty());
        assert_eq!(build_host("cargo-deny", true), cmd);
    }

    #[test]
    fn product_core_clippy_matches_selected_workspace_args() {
        let cmd = build_host("product-core-clippy", false);

        assert_eq!(cmd.program, "cargo");
        assert_eq!(
            cmd.args,
            vec![
                "clippy",
                "--workspace",
                "--exclude",
                "web",
                "--exclude",
                "client",
                "--exclude",
                "csr",
                "--all-targets",
                "--",
                "-D",
                "warnings"
            ]
        );
        assert!(cmd.env.is_empty());
        assert_eq!(build_host("product-core-clippy", true), cmd);
    }

    #[test]
    fn web_server_clippy_lints_host_server_feature_paths() {
        let cmd = build_host("web-server-clippy", false);

        assert_eq!(cmd.program, "cargo");
        assert_eq!(
            cmd.args,
            vec![
                "clippy",
                "-p",
                "web",
                "--features",
                "server",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]
        );
        assert!(cmd.env.is_empty());
        assert_eq!(build_host("web-server-clippy", true), cmd);
    }

    #[test]
    fn web_no_server_clippy_lints_isolated_host_test_targets() {
        let cmd = build_host("web-no-server-clippy", false);

        assert_eq!(cmd.program, "cargo");
        assert_eq!(
            cmd.args,
            vec![
                "clippy",
                "-p",
                "web",
                "--no-default-features",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]
        );
        assert!(cmd.env.is_empty());
        assert_eq!(build_host("web-no-server-clippy", true), cmd);
    }

    #[test]
    fn client_host_clippy_selects_csr_all_targets() {
        let cmd = build_host("client-host-clippy", false);
        assert_eq!(
            cmd.args,
            vec![
                "clippy",
                "-p",
                "client",
                "--features",
                "csr",
                "--all-targets",
                "--",
                "-D",
                "warnings"
            ]
        );
        assert!(cmd.env.is_empty());
        assert_eq!(build_host("client-host-clippy", true), cmd);
    }

    #[test]
    fn wasm_clippy_matches_existing_host_ladder_args() {
        let cmd = build_host("wasm-clippy", false);

        assert_eq!(cmd.program, "cargo");
        assert_eq!(
            cmd.args,
            vec![
                "clippy",
                "-p",
                "web",
                "-p",
                "client",
                "-p",
                "csr",
                "--features",
                "csr",
                "--target",
                "wasm32-unknown-unknown",
                "--",
                "-D",
                "warnings",
            ]
        );
        assert!(cmd.env.is_empty());
        assert_eq!(build_host("wasm-clippy", true), cmd);
    }

    #[test]
    fn tools_clippy_targets_tools_workspace() {
        let cmd = build_host("tools-clippy", false);

        assert_eq!(cmd.program, "cargo");
        assert_eq!(
            cmd.args,
            vec![
                "clippy",
                "--manifest-path",
                "tools/Cargo.toml",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]
        );
        assert!(cmd.env.is_empty());
        assert_eq!(build_host("tools-clippy", true), cmd);
    }

    #[test]
    fn sandbox_cargo_deny_skips_advisories_and_uses_product_home() {
        let cmd = spec("cargo-deny", false)
            .unwrap()
            .build_with_env(CargoMode::Sandbox, |name| match name {
                "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" => {
                    Some("/nix/store/product-cargo-home".into())
                }
                "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME" => Some("/nix/store/tools-cargo-home".into()),
                _ => None,
            })
            .unwrap();

        assert_eq!(cmd.program, "cargo");
        assert_eq!(
            cmd.args,
            vec!["--offline", "deny", "check", "bans", "licenses", "sources"]
        );
        assert!(!cmd.args.iter().any(|arg| arg == "advisories"));
        assert!(cmd.env.contains(&(
            "CARGO_HOME",
            OsString::from("/nix/store/product-cargo-home")
        )));
        assert!(
            cmd.env
                .contains(&("CARGO_NET_OFFLINE", OsString::from("true")))
        );
    }

    #[test]
    fn sandbox_cargo_deny_requires_product_home() {
        let err = spec("cargo-deny", false)
            .unwrap()
            .build_with_env(CargoMode::Sandbox, |_| None)
            .unwrap_err()
            .to_string();

        assert!(err.contains("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME"), "{err}");
    }

    #[test]
    fn sandbox_clippy_uses_product_offline_home() {
        let cmd = spec("product-core-clippy", false)
            .unwrap()
            .build_with_env(CargoMode::Sandbox, |name| match name {
                "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" => {
                    Some("/nix/store/product-cargo-home".into())
                }
                "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME" => Some("/nix/store/tools-cargo-home".into()),
                _ => None,
            })
            .unwrap();

        assert_eq!(cmd.program, "cargo");
        assert_eq!(
            cmd.args,
            vec![
                "--offline",
                "clippy",
                "--workspace",
                "--exclude",
                "web",
                "--exclude",
                "client",
                "--exclude",
                "csr",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]
        );
        assert!(cmd.env.contains(&(
            "CARGO_HOME",
            OsString::from("/nix/store/product-cargo-home")
        )));
        assert!(
            cmd.env
                .contains(&("CARGO_NET_OFFLINE", OsString::from("true")))
        );
    }

    #[test]
    fn sandbox_wasm_clippy_uses_product_offline_home_and_target_args() {
        let cmd = spec("wasm-clippy", false)
            .unwrap()
            .build_with_env(CargoMode::Sandbox, |name| match name {
                "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" => {
                    Some("/nix/store/product-cargo-home".into())
                }
                "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME" => Some("/nix/store/tools-cargo-home".into()),
                _ => None,
            })
            .unwrap();

        assert_eq!(cmd.program, "cargo");
        assert_eq!(
            cmd.args,
            vec![
                "--offline",
                "clippy",
                "-p",
                "web",
                "-p",
                "client",
                "-p",
                "csr",
                "--features",
                "csr",
                "--target",
                "wasm32-unknown-unknown",
                "--",
                "-D",
                "warnings",
            ]
        );
        assert!(cmd.env.contains(&(
            "CARGO_HOME",
            OsString::from("/nix/store/product-cargo-home")
        )));
        assert!(
            cmd.env
                .contains(&("CARGO_NET_OFFLINE", OsString::from("true")))
        );
    }
    #[test]
    fn sandbox_web_no_server_clippy_uses_product_offline_home_and_exact_args() {
        let cmd = spec("web-no-server-clippy", false)
            .unwrap()
            .build_with_env(CargoMode::Sandbox, |name| match name {
                "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" => {
                    Some("/nix/store/product-cargo-home".into())
                }
                "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME" => Some("/nix/store/tools-cargo-home".into()),
                _ => None,
            })
            .unwrap();

        assert_eq!(cmd.program, "cargo");
        assert_eq!(
            cmd.args,
            vec![
                "--offline",
                "clippy",
                "-p",
                "web",
                "--no-default-features",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]
        );
        assert!(cmd.env.contains(&(
            "CARGO_HOME",
            OsString::from("/nix/store/product-cargo-home")
        )));
        assert!(
            cmd.env
                .contains(&("CARGO_NET_OFFLINE", OsString::from("true")))
        );
    }

    #[test]
    fn sandbox_tools_clippy_uses_tools_offline_home() {
        let cmd = spec("tools-clippy", false)
            .unwrap()
            .build_with_env(CargoMode::Sandbox, |name| match name {
                "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" => {
                    Some("/nix/store/product-cargo-home".into())
                }
                "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME" => Some("/nix/store/tools-cargo-home".into()),
                _ => None,
            })
            .unwrap();

        assert_eq!(cmd.program, "cargo");
        assert_eq!(
            cmd.args,
            vec![
                "--offline",
                "clippy",
                "--manifest-path",
                "tools/Cargo.toml",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]
        );
        assert!(
            cmd.env
                .contains(&("CARGO_HOME", OsString::from("/nix/store/tools-cargo-home")))
        );
        assert!(
            cmd.env
                .contains(&("CARGO_NET_OFFLINE", OsString::from("true")))
        );
    }

    #[test]
    fn tools_fmt_targets_tools_workspace() {
        let cmd = build_host("tools-fmt", false);
        assert!(
            cmd.args
                .windows(2)
                .any(|w| w == ["--manifest-path", "tools/Cargo.toml"])
        );
        assert!(
            cmd.args.contains(&"--all".to_string()) && cmd.args.contains(&"--check".to_string())
        );
    }

    #[test]
    fn no_full_reload_runs_root_ast_grep_policy_over_browser_roots() {
        assert_eq!(
            build_host("no-full-reload", false),
            BuiltCommand {
                program: "ast-grep",
                args: vec![
                    "scan".into(),
                    "--config".into(),
                    "sgconfig.yml".into(),
                    "--filter".into(),
                    "^no-full-reload$".into(),
                    "web/src".into(),
                    "client/src".into(),
                ],
                env: vec![],
            }
        );
    }

    #[test]
    fn ast_grep_tests_run_committed_rule_fixtures() {
        assert_eq!(
            build_host("ast-grep-tests", false),
            BuiltCommand {
                program: "ast-grep",
                args: vec!["test".into(), "--config".into(), "sgconfig.yml".into()],
                env: vec![],
            }
        );
        assert_eq!(
            build_host("ast-grep-tests", true),
            build_host("ast-grep-tests", false)
        );
    }

    #[test]
    fn ert_group_preserves_named_command() {
        assert_eq!(CheckGroup::Ert.names(), ["ert"]);
        assert_eq!(
            <CheckGroup as clap::ValueEnum>::from_str("ert", true).unwrap(),
            CheckGroup::Ert
        );
        assert!(!CheckGroup::Code.names().contains(&"ert"));
        assert_eq!(selected_names(Some("ert"), None, false).unwrap(), ["ert"]);
        assert_eq!(
            selected_names(None, Some(CheckGroup::Ert), false).unwrap(),
            ["ert"]
        );
        let command = build_host("ert", false);
        assert_eq!(command.program, "emacs");
        assert_eq!(
            command.args,
            ["--batch", "-Q", "-l", "elisp/scripts/run-tests.el"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn ert_emits_one_bounded_child_event_on_success_and_failure() {
        for (exit, outcome) in [(0, "success"), (23, "failure")] {
            let mut child = Command::new("sh");
            child.args(["-c", &format!("sleep 0.03; exit {exit}")]);
            let mut events = Vec::new();
            let outer = std::time::Instant::now();
            let result = run_check("ert", &mut child, &mut events);
            let outer_ms = outer.elapsed().as_millis();
            assert_eq!(result.is_ok(), exit == 0);
            assert!(events.len() < 256);
            assert_eq!(events.iter().filter(|b| **b == b'\n').count(), 1);
            let event: serde_json::Value = serde_json::from_slice(&events).unwrap();
            assert_eq!(event["event"], "devtool-check-timing");
            assert_eq!(event["schema_version"], 1);
            assert_eq!(event["check"], "ert");
            assert_eq!(event["outcome"], outcome);
            let duration = u128::from(event["duration_ms"].as_u64().unwrap());
            assert!(duration >= 20 && duration <= outer_ms);
        }
        let mut child = Command::new("sh");
        child.args(["-c", "exit 0"]);
        let mut events = Vec::new();
        run_check("byte-compile", &mut child, &mut events).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn named_tsc_check_keeps_its_existing_command() {
        assert_eq!(
            build_host("tsc", false),
            BuiltCommand {
                program: "tsc",
                args: vec![
                    "--noEmit".into(),
                    "-p".into(),
                    "end2end/tsconfig.json".into()
                ],
                env: Vec::new(),
            }
        );
        assert_eq!(selected_names(Some("tsc"), None, false).unwrap(), ["tsc"]);
        assert_eq!(
            <CheckGroup as clap::ValueEnum>::from_str("typescript", true).unwrap(),
            CheckGroup::TypeScript
        );
    }

    #[cfg(unix)]
    #[test]
    fn tsc_emits_one_timing_event_for_success_and_failure() {
        for (exit, outcome) in [(0, "success"), (23, "failure")] {
            let mut child = Command::new("sh");
            child.args(["-c", &format!("sleep 0.03; exit {exit}")]);
            let mut events = Vec::new();
            let result = run_check("tsc", &mut child, &mut events);
            assert_eq!(result.is_ok(), exit == 0);
            assert!(events.len() < 256);
            assert_eq!(events.iter().filter(|b| **b == b'\n').count(), 1);
            let event: serde_json::Value = serde_json::from_slice(&events).unwrap();
            assert_eq!(event["event"], "devtool-check-timing");
            assert_eq!(event["schema_version"], 1);
            assert_eq!(event["check"], "tsc");
            assert_eq!(event["outcome"], outcome);
            assert!(event["duration_ms"].as_u64().unwrap() >= 20);
        }
    }

    #[test]
    fn unknown_check_errors() {
        assert!(spec("nope", false).is_err());
    }

    #[test]
    fn groups_are_an_ordered_disjoint_complete_partition() {
        assert_eq!(CheckGroup::Docs.names(), ["prettier-markdown"]);
        assert_eq!(CheckGroup::TypeScript.names(), ["tsc"]);
        assert_eq!(
            selected_names(None, Some(CheckGroup::TypeScript), false).unwrap(),
            ["tsc"]
        );
        assert_eq!(
            CheckGroup::RustFormat.names(),
            ["fmt", "leptosfmt", "tools-fmt"]
        );
        let mut partition = Vec::new();
        for group in [
            CheckGroup::Docs,
            CheckGroup::TypeScript,
            CheckGroup::RustFormat,
            CheckGroup::Code,
            CheckGroup::Ert,
        ] {
            let names = selected_names(None, Some(group), false).unwrap();
            assert_eq!(names, group.names());
            partition.extend(names);
        }
        partition.sort();
        let mut all = all_names();
        all.sort();
        assert_eq!(partition, all);
        assert_eq!(
            CheckGroup::Code.names(),
            [
                "prettier-end2end",
                "prettier-css",
                "elisp-fmt",
                "ast-grep-tests",
                "no-full-reload",
                "byte-compile",
                "cargo-deny",
                "product-core-clippy",
                "web-server-clippy",
                "web-no-server-clippy",
                "client-host-clippy",
                "wasm-clippy",
                "tools-clippy",
            ]
        );
        assert_eq!(
            all_names(),
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
                "byte-compile",
                "tsc",
                "cargo-deny",
                "product-core-clippy",
                "web-server-clippy",
                "web-no-server-clippy",
                "client-host-clippy",
                "wasm-clippy",
                "tools-clippy",
                "ert",
            ]
        );
        assert_eq!(
            selected_names(None, Some(CheckGroup::Docs), false).unwrap(),
            CheckGroup::Docs.names().to_vec()
        );
        assert_eq!(selected_names(None, None, true).unwrap(), all_names());
        let selected = all_names();
        assert_eq!(
            selected.len(),
            selected
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
        );
    }

    #[test]
    fn inventory_includes_web_no_server_clippy_once_in_clippy_order() {
        let all = all_names();
        assert_eq!(
            all.iter()
                .filter(|name| **name == "web-no-server-clippy")
                .count(),
            1
        );
        let clippy_start = all
            .iter()
            .position(|name| *name == "product-core-clippy")
            .expect("generic clippy present");
        assert_eq!(
            &all[clippy_start..=clippy_start + 4],
            [
                "product-core-clippy",
                "web-server-clippy",
                "web-no-server-clippy",
                "client-host-clippy",
                "wasm-clippy",
            ]
        );
    }

    #[test]
    fn inventory_includes_no_full_reload_once() {
        assert_eq!(
            all_names()
                .iter()
                .filter(|name| **name == "no-full-reload")
                .count(),
            1
        );
    }

    #[test]
    fn inventory_includes_ast_grep_tests_once_before_repository_scan() {
        let all = all_names();
        let ast_grep_tests = all
            .iter()
            .position(|name| *name == "ast-grep-tests")
            .expect("ast-grep tests present");
        assert_eq!(
            all.iter().filter(|name| **name == "ast-grep-tests").count(),
            1
        );
        assert_eq!(all[ast_grep_tests + 1], "no-full-reload");
    }

    #[test]
    fn all_names_have_specs() {
        for n in all_names() {
            assert!(spec(n, false).is_ok(), "{n}");
        }
    }
}
