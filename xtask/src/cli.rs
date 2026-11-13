use clap::{Parser, Subcommand};

const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("JAUNDER_XTASK_BUILD_IDENTITY"),
    ")"
);

fn nonempty(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        Err("must not be empty".to_owned())
    } else {
        Ok(value.to_owned())
    }
}

#[derive(Parser)]
#[command(
    name = "xtask",
    about = "Jaunder dev orchestration",
    version = VERSION,
    long_version = VERSION
)]
pub struct Cli {
    /// Emit the structured result envelope as JSON to stdout.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

pub use host_proof::e2e::{E2eBackend, E2eBrowser};

#[derive(Subcommand)]
pub enum Command {
    /// Invoke one existing check/safety adapter from Just; hidden from the workflow list.
    #[command(hide = true)]
    CheckLeaf {
        #[arg(value_parser = nonempty)]
        name: String,
    },
    /// Measure the frontend WASM/JS bundle size — raw, gzip, and brotli.
    ///
    /// Reports the manifest-selected identity assets from deterministic
    /// `nix build .#csrBundle`, so bundle-size bloat is visible before it ships.
    /// Run it after a change expected to move the bundle (a new dependency or a
    /// client feature), or periodically to watch the trend.
    ///
    /// The totals also back `validate`'s `wasm-budget` step, which fails when the
    /// manifest-selected WASM identity artifact exceeds a committed ceiling
    /// (#836) — so the gate and this tool cannot disagree about what the bundle
    /// weighs. `--breakdown` remains manual; it is not part of `check`/`validate`.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask audit-wasm\n  \
        cargo xtask audit-wasm --site-path /nix/store/...-jaunder-csr-wasm-bundle\n  \
        cargo xtask audit-wasm --breakdown\n  \
        cargo xtask --json audit-wasm")]
    AuditWasm {
        /// Audit a prebuilt CSR bundle root instead of running `nix build`.
        #[arg(long)]
        site_path: Option<String>,
        /// Report per-section and per-crate byte attribution instead of totals.
        ///
        /// Measured on the pre-wasm-bindgen, unstripped `.#csrWasm` artifact,
        /// which still carries a name section — `wasm-opt` strips names from the
        /// shipped bundle, so the shipped file cannot be attributed (#836). Its
        /// total is NOT the shipped bundle size.
        #[arg(long)]
        breakdown: bool,
        /// Break down this wasm file instead of building `.#csrWasm`.
        #[arg(long, requires = "breakdown")]
        wasm: Option<String>,
    },
    /// Nix maintenance commands that evaluate repository derivation boundaries.
    #[command(subcommand)]
    Nix(NixCommand),
    /// Prove the cache policy admits only support closures and excludes every
    /// coverage/e2e verdict, aggregate, and lifted equivalent.
    #[command(subcommand)]
    CacheSafety(CacheSafetyCommand),
}

/// `cache-safety` subcommands.
#[derive(Subcommand)]
pub enum CacheSafetyCommand {
    /// Reconcile `nix/cache-policy.json` with the Nix-derived output inventory,
    /// then inspect admitted support closures without building final verdicts.
    #[command(after_help = "EXAMPLES:\n  traced-run -- cargo xtask cache-safety probe")]
    Probe,
}

/// `nix` subcommands.
#[derive(Subcommand)]
pub enum NixCommand {
    /// Guard static-docs, static-code, CSR/site, and client wasm-test source
    /// closures with isolated tracked perturbations. Eval-only (no realization);
    /// runs in CI and on request, not in per-commit `check`/`validate`.
    #[command(after_help = "EXAMPLES:\n  cargo xtask nix probe-source")]
    ProbeSource,
}

/// `traces` subcommands.
impl Cli {
    pub fn command_name(&self) -> &'static str {
        match self.command {
            Command::CheckLeaf { .. } => "check-leaf",
            Command::AuditWasm { .. } => "audit-wasm",
            Command::Nix(NixCommand::ProbeSource) => "nix-probe-source",
            Command::CacheSafety(CacheSafetyCommand::Probe) => "cache-safety-probe",
        }
    }
}

impl Command {
    /// Whether `--json` yields a substantial structured payload for this command.
    /// Commands that answer `false` reject `--json` (there is nothing meaningful to
    /// serialize beyond the bare envelope). Defaults `true`.
    pub fn produces_json_payload(&self) -> bool {
        !matches!(self, Command::CheckLeaf { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn standalone_csr_build_has_no_xtask_command_alias() {
        assert!(Cli::try_parse_from(["xtask", "build-csr"]).is_err());
    }

    #[test]
    fn retired_gate_commands_have_no_xtask_alias() {
        for arguments in [
            vec!["xtask", "check"],
            vec!["xtask", "check", "--no-test"],
            vec!["xtask", "precommit"],
            vec!["xtask", "prepush"],
            vec!["xtask", "validate"],
            vec!["xtask", "validate", "--no-e2e"],
            vec!["xtask", "--json", "validate", "--allow-dirty"],
            vec!["xtask", "ci-validate", "host"],
            vec!["xtask", "ci-validate", "hermetic"],
            vec!["xtask", "ci-validate", "test-checks"],
            vec!["xtask", "ci-validate", "coverage"],
        ] {
            assert!(
                Cli::try_parse_from(&arguments).is_err(),
                "{arguments:?} must use Just, not an xtask alias"
            );
        }
    }

    #[test]
    fn just_leaf_is_hidden_and_precommit_safety_alias_is_retired() {
        let leaf = Cli::try_parse_from(["xtask", "check-leaf", "doc-links"]).unwrap();
        assert_eq!(leaf.command_name(), "check-leaf");
        assert!(!leaf.command.produces_json_payload());
        assert!(matches!(
            leaf.command,
            Command::CheckLeaf { name } if name == "doc-links"
        ));

        assert!(Cli::try_parse_from(["xtask", "precommit-safety"]).is_err());
    }

    #[test]
    fn performance_workflow_is_not_an_xtask_command() {
        for arguments in [
            vec!["xtask", "perf", "small"],
            vec!["xtask", "perf", "compare", "--result", "result.json"],
            vec!["xtask", "perf", "import-baseline", "42"],
        ] {
            assert!(Cli::try_parse_from(arguments).is_err());
        }
    }

    #[test]
    fn ordinary_issue_commands_are_not_xtask_entrypoints() {
        for operation in ["candidates", "create", "edit"] {
            assert!(Cli::try_parse_from(["xtask", "issue", operation]).is_err());
        }
    }

    #[test]
    fn retired_schedulers_are_not_parallel_entrypoints() {
        for command in [
            "e2e",
            "e2e-lane",
            "e2e-control",
            "e2e-candidate",
            "e2e-experimental",
        ] {
            assert!(Cli::try_parse_from(["xtask", command, "sqlite"]).is_err());
        }
    }

    #[test]
    fn adr_renumber_is_rejected() {
        assert!(Cli::try_parse_from(["xtask", "adr", "renumber"]).is_err());
    }

    #[test]
    fn python_owned_adr_commands_are_retired() {
        for action in ["promote", "sync-readme", "promoter"] {
            assert!(Cli::try_parse_from(["xtask", "adr", action]).is_err());
        }
    }

    #[test]
    fn nix_probe_source_parses_as_closed_subcommand() {
        let cli = Cli::try_parse_from(["xtask", "nix", "probe-source"]).unwrap();
        assert_eq!(cli.command_name(), "nix-probe-source");
        assert!(matches!(cli.command, Command::Nix(NixCommand::ProbeSource)));
    }

    #[test]
    fn cache_safety_probe_parses_as_closed_subcommand() {
        let cli = Cli::try_parse_from(["xtask", "cache-safety", "probe"]).unwrap();
        assert_eq!(cli.command_name(), "cache-safety-probe");
        assert!(matches!(
            cli.command,
            Command::CacheSafety(CacheSafetyCommand::Probe)
        ));
    }

    #[test]
    fn ordinary_pr_commands_are_not_xtask_entrypoints() {
        for command in ["watch", "land", "cleanup"] {
            assert!(Cli::try_parse_from(["xtask", "pr", command, "731"]).is_err());
        }
    }

    #[test]
    fn retired_sandbox_route_is_rejected() {
        for args in [
            vec!["xtask", "sandbox"],
            vec![
                "xtask",
                "sandbox",
                "demo",
                "--profile",
                "standard",
                "--reset",
            ],
            vec![
                "xtask",
                "sandbox",
                "demo",
                "--",
                "site-config",
                "get",
                "site.title",
            ],
        ] {
            let error = Cli::try_parse_from(args).err().expect("retired route");
            assert_eq!(error.kind(), clap::error::ErrorKind::InvalidSubcommand);
        }
    }
}
