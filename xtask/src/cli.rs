use std::path::PathBuf;

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
    /// Apply source-aware coverage/CRAP policy to realized reports; never runs Nix.
    CoveragePolicy {
        #[arg(long)]
        out: PathBuf,
    },
    /// Coverage tooling — the source-filter drift probe (#241).
    #[command(subcommand)]
    Coverage(CoverageCommand),
    /// Reconcile the independent Chromium and Firefox Playwright/WASM coverage
    /// evidence. Both producers run before a verdict; raw profiles are merged only
    /// when both retained status documents pass and identify the same module/toolchain.
    #[command(subcommand)]
    WasmCoverage(WasmCoverageCommand),
    /// Nix maintenance commands that evaluate repository derivation boundaries.
    #[command(subcommand)]
    Nix(NixCommand),
    /// Prove the cache policy admits only support closures and excludes every
    /// coverage/e2e verdict, aggregate, and lifted equivalent.
    #[command(subcommand)]
    CacheSafety(CacheSafetyCommand),
    /// Validate completed mutation producer evidence without running mutations.
    #[command(subcommand)]
    Mutants(MutantsCommand),
}

#[derive(Subcommand)]
pub enum MutantsCommand {
    /// Report counts only for completed, applicable and consistent evidence.
    VerifyEvidence {
        #[arg(long, value_parser = nonempty)]
        package: String,
        #[arg(long, value_parser = nonempty)]
        file: String,
        /// Producer output root containing mutants.out/.
        #[arg(long)]
        output: PathBuf,
        #[arg(long, allow_hyphen_values = true)]
        producer_status: i32,
    },
}

/// `coverage` subcommands.
#[derive(Subcommand)]
pub enum CoverageCommand {
    /// Guard the Nix coverage derivation's source filter against silent drift:
    /// assert that staging an excluded file leaves `coverage.drvPath` unchanged and
    /// staging an instrumented `.rs` changes it. Eval-only (no build); runs in CI and
    /// on request, NOT in per-commit `check`/`validate` (#241, #37).
    #[command(after_help = "EXAMPLES:\n  cargo xtask coverage probe-source")]
    ProbeSource,
    /// Measure the existing coverage producer under a declared unloaded host window.
    /// This is an opt-in local experiment; it does not alter production coverage.
    #[command(
        after_help = "EXAMPLES:\n  cargo xtask coverage benchmark-local --unloaded-system 'operator confirms no competing host load'"
    )]
    BenchmarkLocal {
        /// Operator assertion that the host has no competing workload.
        #[arg(long, value_parser = nonempty)]
        unloaded_system: String,
    },
}
/// `wasm-coverage` subcommands.
#[derive(Subcommand)]
pub enum WasmCoverageCommand {
    /// Realize each browser evidence producer, retain and validate its archive, then
    /// conditionally count-sum their profiles with the matching pinned LLVM tools.
    /// The aggregate is always written to `.xtask/wasm-coverage/status.json`.
    #[command(after_help = "EXAMPLES:\n  cargo xtask wasm-coverage probe")]
    Probe,
    /// Measure the separately built baseline and instrumented diagnostic bundles during
    /// a coordinated quiet host window. The acknowledgement is deliberately required:
    /// these numbers are invalid when unrelated host work is competing for resources.
    #[command(
        after_help = "EXAMPLES:\n  cargo xtask wasm-coverage measure --quiescent-window '2026-09-06 coordinated window'"
    )]
    Measure {
        #[arg(long, value_parser = nonempty)]
        quiescent_window: String,
    },
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
            Command::CoveragePolicy { .. } => "coverage-policy",
            Command::Coverage(CoverageCommand::ProbeSource) => "coverage-probe-source",
            Command::Coverage(CoverageCommand::BenchmarkLocal { .. }) => "coverage-benchmark-local",
            Command::WasmCoverage(WasmCoverageCommand::Probe) => "wasm-coverage-probe",
            Command::WasmCoverage(WasmCoverageCommand::Measure { .. }) => "wasm-coverage-measure",
            Command::Nix(NixCommand::ProbeSource) => "nix-probe-source",
            Command::CacheSafety(CacheSafetyCommand::Probe) => "cache-safety-probe",
            Command::Mutants(MutantsCommand::VerifyEvidence { .. }) => "mutants-verify-evidence",
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

    #[test]
    fn coverage_benchmark_local_requires_an_unloaded_system_assertion() {
        assert!(Cli::try_parse_from(["xtask", "coverage", "benchmark-local"]).is_err());
        let cli = Cli::try_parse_from([
            "xtask",
            "coverage",
            "benchmark-local",
            "--unloaded-system",
            "operator confirms no competing host load",
        ])
        .unwrap();
        assert_eq!(cli.command_name(), "coverage-benchmark-local");
        assert!(matches!(
            cli.command,
            Command::Coverage(CoverageCommand::BenchmarkLocal { unloaded_system })
                if unloaded_system == "operator confirms no competing host load"
        ));
    }

    #[test]
    fn wasm_coverage_probe_parses() {
        let cli = Cli::try_parse_from(["xtask", "wasm-coverage", "probe"]).unwrap();
        assert_eq!(cli.command_name(), "wasm-coverage-probe");
        assert!(matches!(
            cli.command,
            Command::WasmCoverage(WasmCoverageCommand::Probe)
        ));
    }
}
