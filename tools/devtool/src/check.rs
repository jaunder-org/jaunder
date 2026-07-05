//! The 7 non-compiling static checks (#188). This is the single home of their tool +
//! args: the host verify ladder runs them via `cargo run -p devtool -- check <name>`
//! (so a local `tools/` edit is reflected) and the nix `static-checks` derivation runs
//! `devtool check --all` from the prebuilt `devtoolBin`. The *compiling* checks
//! (`clippy`/`deny`) stay on their crane derivations; `tools-clippy`/`xtask-*` stay
//! host-only — see the ADR.

use std::process::Command;

use anyhow::{bail, Context, Result};

/// The 7 non-compiling static checks devtool owns, in the host gate's order.
///
/// Kept in sync with the `devtool_check(<name>)` calls in
/// `xtask/src/steps/static_checks.rs::specs()` (the host mirror — it can't import this
/// list, being a separate host-only workspace that reaches devtool only over the CLI).
pub const ALL: &[&str] = &[
    "fmt",
    "leptosfmt",
    "prettier",
    "tsc",
    "elisp-fmt",
    "ert",
    "tools-fmt",
];

/// Pure: the `(program, args)` for `name` in the given mode. `fix` makes the five
/// formatters (`fmt`, `leptosfmt`, `prettier`, `elisp-fmt`, `tools-fmt`) mutate in place;
/// `ert`/`tsc` have no autofix and ignore it. Args are verbatim from the former
/// `xtask::steps::static_checks::specs` — this is now their single source of truth.
fn spec(name: &str, fix: bool) -> Result<(&'static str, Vec<String>)> {
    let owned = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    Ok(match name {
        "fmt" => (
            "cargo",
            if fix {
                owned(&["fmt"])
            } else {
                owned(&["fmt", "--check"])
            },
        ),
        "leptosfmt" => (
            "leptosfmt",
            if fix {
                owned(&["-x", ".direnv", "-x", ".git", "-x", "target", "**/*.rs"])
            } else {
                owned(&[
                    "-x", ".direnv", "-x", ".git", "-x", "target", "--check", "**/*.rs",
                ])
            },
        ),
        "prettier" => (
            "prettier",
            if fix {
                owned(&["-w", "end2end", "**/*.md"])
            } else {
                owned(&["--check", "end2end", "**/*.md"])
            },
        ),
        "tsc" => ("tsc", owned(&["--noEmit", "-p", "end2end/tsconfig.json"])),
        "elisp-fmt" => (
            "emacs",
            if fix {
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
        ),
        "ert" => (
            "emacs",
            owned(&["--batch", "-Q", "-l", "elisp/scripts/run-tests.el"]),
        ),
        "tools-fmt" => (
            "cargo",
            if fix {
                owned(&["fmt", "--manifest-path", "tools/Cargo.toml", "--all"])
            } else {
                owned(&[
                    "fmt",
                    "--manifest-path",
                    "tools/Cargo.toml",
                    "--all",
                    "--check",
                ])
            },
        ),
        other => bail!("unknown check '{other}' (known: {ALL:?})"),
    })
}

/// Run one check by name, or all of them (`--all`). `tsc` provisions
/// `end2end/node_modules` (the type-dep closure) first, via the shared script.
pub fn run(name: Option<&str>, all: bool, fix: bool) -> Result<()> {
    let names: Vec<&str> = match (name, all) {
        (Some(n), false) => vec![n],
        (None, true) => ALL.to_vec(),
        _ => bail!("pass exactly one of <name> or --all"),
    };
    for n in &names {
        if *n == "tsc" {
            let st = Command::new("bash")
                .arg("end2end/provision-node-modules.sh")
                .status()
                .context("provisioning end2end/node_modules for tsc")?;
            if !st.success() {
                bail!("tsc-deps (provision-node-modules.sh) failed ({st})");
            }
        }
        let (program, args) = spec(n, fix)?;
        let st = Command::new(program)
            .args(&args)
            .status()
            .with_context(|| format!("spawning `{program}` for check {n}"))?;
        if !st.success() {
            bail!("check {n} failed ({st})");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_check_vs_fix() {
        assert_eq!(
            spec("fmt", false).unwrap(),
            ("cargo", vec!["fmt".to_string(), "--check".into()])
        );
        assert_eq!(
            spec("fmt", true).unwrap(),
            ("cargo", vec!["fmt".to_string()])
        );
    }

    #[test]
    fn prettier_covers_end2end_and_markdown() {
        // The #185 fix: unified prettier checks end2end AND all markdown.
        let (_p, args) = spec("prettier", false).unwrap();
        assert!(args.contains(&"--check".to_string()));
        assert!(args.contains(&"end2end".to_string()));
        assert!(args.contains(&"**/*.md".to_string()));
    }

    #[test]
    fn ert_and_tsc_ignore_fix() {
        assert_eq!(spec("ert", true).unwrap(), spec("ert", false).unwrap());
        assert_eq!(spec("tsc", true).unwrap(), spec("tsc", false).unwrap());
    }

    #[test]
    fn tools_fmt_targets_tools_workspace() {
        let (_p, args) = spec("tools-fmt", false).unwrap();
        assert!(args
            .windows(2)
            .any(|w| w == ["--manifest-path", "tools/Cargo.toml"]));
        assert!(args.contains(&"--all".to_string()) && args.contains(&"--check".to_string()));
    }

    #[test]
    fn unknown_check_errors() {
        assert!(spec("nope", false).is_err());
    }

    #[test]
    fn all_names_have_specs() {
        for n in ALL {
            assert!(spec(n, false).is_ok(), "{n}");
        }
    }
}
