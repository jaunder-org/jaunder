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

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum E2eBackend {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum E2eBrowser {
    Chromium,
    Firefox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum E2eLocalBrowser {
    Chromium,
    Firefox,
    Webkit,
}

impl E2eBackend {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            E2eBackend::Sqlite => "sqlite",
            E2eBackend::Postgres => "postgres",
        }
    }
}

impl E2eBrowser {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            E2eBrowser::Chromium => "chromium",
            E2eBrowser::Firefox => "firefox",
        }
    }
}

impl E2eLocalBrowser {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Chromium => "chromium",
            Self::Firefox => "firefox",
            Self::Webkit => "webkit",
        }
    }
}

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
    /// Verify one lane's explicit artifacts without building a VM.
    E2eVerify {
        #[arg(long)]
        lane: String,
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        artifacts: PathBuf,
    },
    /// Reconcile one backend's already-lifted retained Firefox lane evidence
    /// without invoking Nix. `diagnostics_root` contains `e2e-<lane>` directories.
    E2eReconcile {
        #[arg(value_enum)]
        backend: E2eBackend,
        #[arg(long)]
        diagnostics_root: PathBuf,
        #[arg(long)]
        catalog: PathBuf,
    },
    /// Apply source-aware coverage/CRAP policy to realized reports; never runs Nix.
    CoveragePolicy {
        #[arg(long)]
        out: PathBuf,
    },
    /// Focused browser proof: use a spec or file:line selector while iterating.
    ///
    /// Rerun the same focused command after a fix. Use representative hermetic
    /// `just e2e-producer <identity>` proof when backend/browser parity, release-build
    /// behavior or the hermetic environment is load-bearing; local proof does not
    /// replace CI/full validation.
    ///
    /// Owns each host lifecycle: build the CSR bundle +
    /// server, start `jaunder serve` on an ephemeral port with the VM's capture
    /// env + a per-run temp DB, seed via the shared `test-support seed-e2e`, run
    /// Playwright against the discovered URL, and tear the server down on every
    /// exit path. Normal mode runs the selected ordinary browser project; visual
    /// update mode builds release CSR and updates Chromium and Firefox baselines in
    /// separate fresh lifecycles. Self-contained — no pre-existing server and no
    /// `:3000` conflict. Loads the same `playwright.config.ts` the CI VM loads.
    /// Host only.
    E2eLocal {
        /// Run the selected ordinary Playwright browser project. Omit to preserve
        /// the existing Chromium run.
        #[arg(long, value_enum, conflicts_with = "update_visual_snapshots")]
        browser: Option<E2eLocalBrowser>,
        /// A spec path or `file:line` filter passed through to Playwright as a
        /// positional arg (single-test runs).
        test: Option<String>,
        /// Update every Chromium and Firefox visual baseline using release CSR.
        #[arg(long, conflicts_with = "test")]
        update_visual_snapshots: bool,
    },
    /// Focused Rust proof: pass nextest selectors after `--` while iterating.
    ///
    /// Rerun the same focused command after a fix. This accelerates feedback;
    /// normal commit/push hooks and CI retain their boundary authority.
    /// Runs host-native tests with shared sccache and isolated PostgreSQL.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask test-local\n  \
        cargo xtask test-local -- -p storage site_config_primitives_round_trip")]
    TestLocal {
        /// Arguments forwarded to `cargo nextest run`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        nextest_args: Vec<String>,
    },
    /// OpenTelemetry trace analysis (host-side; ADR-0028).
    #[command(subcommand)]
    Traces(TracesCommand),
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
    /// Trace-derived `#[server]` fn flow coverage (#681): which server entry
    /// points the e2e suite actually drives.
    #[command(subcommand)]
    ServerFnCoverage(ServerFnCoverageCommand),
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

/// `server-fn-coverage` subcommands (#681).
#[derive(Subcommand)]
pub enum ServerFnCoverageCommand {
    /// Explicitly regenerate the committed snapshot from selected producer evidence.
    Regenerate(ProducerCapture),
    /// Compare selected producer evidence with the committed snapshot.
    Verify(ProducerCapture),
}

/// Evidence selected by a successful producer's nix build --json invocation.
#[derive(clap::Args)]
pub struct ProducerCapture {
    #[arg(long)]
    pub capture: PathBuf,
    #[arg(long)]
    pub producer_output: PathBuf,
    #[arg(long)]
    pub lane: String,
    #[arg(long)]
    pub catalog: PathBuf,
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
#[derive(Subcommand)]
pub enum TracesCommand {
    /// Analyze OpenTelemetry JSONL traces exported by the e2e VM collector and
    /// print the report tables (slowest spans, per-test/-project hotspots, trace
    /// totals). A manual tool — not part of `check`/`validate`. Prints human
    /// tables only; `--json` is rejected.
    #[command(after_help = "EXAMPLES:\n  \
        # trace files extracted from a capture-<backend>-<browser>-unsplit.tar.gz bundle (capture/otel-traces.jsonl):\n  \
        cargo xtask traces analyze sqlite-otel-traces.jsonl postgres-otel-traces.jsonl\n  \
        cargo xtask traces analyze --top 40 --project firefox trace-a.jsonl trace-b.jsonl\n  \
        cargo xtask traces analyze --trace 1111...1111 traces.jsonl")]
    Analyze {
        /// Rows per ranked table (default 25). The cache-warmth, per-project, and
        /// long-task-by-project tables always print every row.
        #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u64).range(1..))]
        top: u64,
        /// Restrict analysis to one trace id.
        #[arg(long)]
        trace: Option<String>,
        /// Restrict analysis to one e2e project (filters only `e2e.`-named spans).
        #[arg(long)]
        project: Option<String>,
        /// Playwright `json` reporter output(s), e.g.
        /// `.xtask/diagnostics/e2e-sqlite-chromium-unsplit/playwright-report-sqlite-chromium-unsplit.json`.
        /// Supplies the per-test span-coverage section's denominator — the traces
        /// alone cannot say how long a test took wall-clock. Omit and that one
        /// section is skipped with a note.
        #[arg(long = "playwright-report")]
        playwright_report: Vec<PathBuf>,
        /// One or more `otel-traces.jsonl` files.
        #[arg(required = true)]
        files: Vec<std::path::PathBuf>,
    },
    /// Build the `{sqlite,postgres}×{chromium,firefox}` e2e VM checks and analyze
    /// their exported OTel traces in one step — the `nix build` orchestration that
    /// feeds `traces analyze`. A manual tool — not part of `check`/`validate`.
    /// Prints human tables only; `--json` is rejected.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask traces run\n  \
        cargo xtask traces run --top 40\n  \
        cargo xtask traces run --single-worker\n  \
        cargo xtask traces run --browser firefox")]
    Run {
        /// Rows per ranked table (default 25), forwarded to the analysis.
        #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u64).range(1..))]
        top: u64,
        /// Restrict the analysis to one trace id.
        #[arg(long)]
        trace: Option<String>,
        /// Build the single-worker package variants instead of the gate checks.
        /// Use when the question is per-navigation cost, where worker contention
        /// would corrupt the attribution.
        #[arg(long)]
        single_worker: bool,
        /// Restrict to one browser (default: both). Both backends are always built.
        #[arg(long, value_enum)]
        browser: Option<E2eBrowser>,
    },
    /// Median boot-phase decomposition per `(trace file, project, cacheWarmth)`
    /// (#818). Every segment is document-relative and the six of them close on
    /// `mount_done.startTime` exactly; the Node-side `commitToMountMs` and the
    /// skew between the two frames are reported but never decomposed. A manual
    /// tool — not part of `check`/`validate`. Prints human tables only; `--json`
    /// is rejected.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask traces boot-phases sqlite-chromium.jsonl sqlite-firefox.jsonl")]
    BootPhases {
        /// One or more `otel-traces.jsonl` files.
        #[arg(required = true)]
        files: Vec<std::path::PathBuf>,
    },
}

impl Cli {
    pub fn command_name(&self) -> &'static str {
        match self.command {
            Command::CheckLeaf { .. } => "check-leaf",
            Command::AuditWasm { .. } => "audit-wasm",
            Command::E2eVerify { .. } => "e2e-verify",
            Command::E2eReconcile { .. } => "e2e-reconcile",
            Command::E2eLocal { .. } => "e2e-local",
            Command::TestLocal { .. } => "test-local",
            Command::Traces(TracesCommand::Analyze { .. }) => "traces-analyze",
            Command::Traces(TracesCommand::Run { .. }) => "traces-run",
            Command::Traces(TracesCommand::BootPhases { .. }) => "traces-boot-phases",
            Command::CoveragePolicy { .. } => "coverage-policy",
            Command::Coverage(CoverageCommand::ProbeSource) => "coverage-probe-source",
            Command::Coverage(CoverageCommand::BenchmarkLocal { .. }) => "coverage-benchmark-local",
            Command::WasmCoverage(WasmCoverageCommand::Probe) => "wasm-coverage-probe",
            Command::WasmCoverage(WasmCoverageCommand::Measure { .. }) => "wasm-coverage-measure",
            Command::Nix(NixCommand::ProbeSource) => "nix-probe-source",
            Command::CacheSafety(CacheSafetyCommand::Probe) => "cache-safety-probe",
            Command::ServerFnCoverage(ServerFnCoverageCommand::Regenerate(_)) => {
                "server-fn-coverage-regenerate"
            }
            Command::ServerFnCoverage(ServerFnCoverageCommand::Verify(_)) => {
                "server-fn-coverage-verify"
            }
            Command::Mutants(MutantsCommand::VerifyEvidence { .. }) => "mutants-verify-evidence",
        }
    }
}

impl Command {
    /// Whether `--json` yields a substantial structured payload for this command.
    /// Commands that answer `false` reject `--json` (there is nothing meaningful to
    /// serialize beyond the bare envelope). Defaults `true`; the `traces` reporting
    /// commands (`analyze`/`run`/`boot-phases`) print human tables only, so they
    /// opt out.
    pub fn produces_json_payload(&self) -> bool {
        !matches!(
            self,
            Command::CheckLeaf { .. }
                | Command::Traces(
                    TracesCommand::Analyze { .. }
                        | TracesCommand::Run { .. }
                        | TracesCommand::BootPhases { .. }
                )
                | Command::E2eVerify { .. }
                | Command::E2eReconcile { .. }
                | Command::ServerFnCoverage(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn standalone_csr_build_has_no_xtask_command_alias() {
        assert!(Cli::try_parse_from(["xtask", "build-csr"]).is_err());
    }

    #[test]
    fn feedback_help_distinguishes_focused_proof_from_boundary_gates() {
        // Exercise Clap's public help paths, including the summaries callers see
        // before choosing a command; source comments alone do not prove the guidance.
        for flag in ["-h", "--help"] {
            let error = Cli::try_parse_from(["xtask", flag]).err().unwrap();
            assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
            let help = error.to_string();
            for role in ["Focused Rust proof", "Focused browser proof"] {
                assert!(help.contains(role), "missing {role:?} in {flag}: {help}");
            }
            assert!(!help.contains("Inner loop"));
        }
    }

    #[test]
    fn feedback_command_help_redirects_iteration_without_replacing_gates() {
        for (command, required) in [
            (
                "test-local",
                &["Focused Rust proof", "same focused command", "hooks", "CI"][..],
            ),
            (
                "e2e-local",
                &[
                    "Focused browser proof",
                    "same focused command",
                    "hermetic",
                    "parity",
                    "just e2e-producer",
                ][..],
            ),
        ] {
            for flag in ["-h", "--help"] {
                let error = Cli::try_parse_from(["xtask", command, flag]).err().unwrap();
                assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
                let help = error.to_string();
                assert!(help.contains(required[0]), "{command} {flag}: {help}");
                if flag == "--help" {
                    // Whitespace is presentation, not the cadence contract.
                    let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
                    for phrase in required {
                        assert!(help.contains(phrase), "missing {phrase:?}: {help}");
                    }
                }
            }
        }
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
    fn artifact_verification_requires_explicit_runtime_inputs() {
        assert!(Cli::try_parse_from(["xtask", "e2e-verify"]).is_err());
        let cli = Cli::try_parse_from([
            "xtask",
            "e2e-verify",
            "--lane",
            "sqlite-chromium-unsplit",
            "--catalog",
            "lanes.json",
            "--artifacts",
            "output",
        ])
        .unwrap();
        assert_eq!(cli.command_name(), "e2e-verify");
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
    fn e2e_local_parses_filter_or_visual_update_mode() {
        let cli = Cli::try_parse_from(["xtask", "e2e-local"]).unwrap();
        match cli.command {
            Command::E2eLocal {
                browser,
                test,
                update_visual_snapshots,
            } => {
                assert_eq!(browser, None);
                assert_eq!(test, None);
                assert!(!update_visual_snapshots);
            }
            _ => panic!("expected e2e-local"),
        }

        let cli = Cli::try_parse_from(["xtask", "e2e-local", "auth-flow.spec.ts"]).unwrap();
        match cli.command {
            Command::E2eLocal {
                browser,
                test,
                update_visual_snapshots,
            } => {
                assert_eq!(browser, None);
                assert_eq!(test.as_deref(), Some("auth-flow.spec.ts"));
                assert!(!update_visual_snapshots);
            }
            _ => panic!("expected e2e-local with filter"),
        }

        let cli = Cli::try_parse_from(["xtask", "e2e-local", "--update-visual-snapshots"]).unwrap();
        match cli.command {
            Command::E2eLocal {
                browser,
                test,
                update_visual_snapshots,
            } => {
                assert_eq!(browser, None);
                assert_eq!(test, None);
                assert!(update_visual_snapshots);
            }
            _ => panic!("expected e2e-local visual update mode"),
        }
    }

    #[test]
    fn e2e_local_parses_each_browser_selection() {
        for (argument, browser) in [
            ("chromium", E2eLocalBrowser::Chromium),
            ("firefox", E2eLocalBrowser::Firefox),
            ("webkit", E2eLocalBrowser::Webkit),
        ] {
            let cli = Cli::try_parse_from(["xtask", "e2e-local", "--browser", argument]).unwrap();
            match cli.command {
                Command::E2eLocal {
                    browser: selected, ..
                } => assert_eq!(selected, Some(browser)),
                _ => panic!("expected e2e-local"),
            }
        }
    }

    #[test]
    fn e2e_local_parses_firefox_admin_site_filter() {
        let cli = Cli::try_parse_from([
            "xtask",
            "e2e-local",
            "theme.spec.ts:63",
            "--browser",
            "firefox",
        ])
        .unwrap();
        match cli.command {
            Command::E2eLocal {
                browser,
                test,
                update_visual_snapshots,
            } => {
                assert_eq!(browser, Some(E2eLocalBrowser::Firefox));
                assert_eq!(test.as_deref(), Some("theme.spec.ts:63"));
                assert!(!update_visual_snapshots);
            }
            _ => panic!("expected filtered Firefox e2e-local"),
        }
    }

    #[test]
    fn e2e_local_rejects_filter_with_visual_update_mode() {
        assert!(
            Cli::try_parse_from([
                "xtask",
                "e2e-local",
                "--update-visual-snapshots",
                "auth-flow.spec.ts",
            ])
            .is_err()
        );

        let Err(error) = Cli::try_parse_from([
            "xtask",
            "e2e-local",
            "--update-visual-snapshots",
            "--browser",
            "webkit",
        ]) else {
            panic!("browser selection conflicts with visual snapshot updates");
        };
        assert!(error.to_string().contains("--browser"));
        assert!(error.to_string().contains("--update-visual-snapshots"));
    }

    #[test]
    fn parses_test_local_defaults_and_forwarded_nextest_args() {
        let cli = Cli::try_parse_from(["xtask", "test-local"]).unwrap();
        match cli.command {
            Command::TestLocal { ref nextest_args } => {
                assert!(nextest_args.is_empty());
            }
            _ => panic!("expected test-local"),
        }
        assert_eq!(cli.command_name(), "test-local");

        let cli = Cli::try_parse_from([
            "xtask",
            "test-local",
            "--",
            "-p",
            "storage",
            "site_config_primitives_round_trip",
        ])
        .unwrap();
        match cli.command {
            Command::TestLocal { nextest_args } => {
                assert_eq!(
                    nextest_args,
                    ["-p", "storage", "site_config_primitives_round_trip"]
                );
            }
            _ => panic!("expected test-local with nextest args"),
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
    fn traces_analyze_parses_flags_and_files() {
        let cli = Cli::try_parse_from([
            "xtask",
            "traces",
            "analyze",
            "--top",
            "40",
            "--project",
            "firefox",
            "a.jsonl",
            "b.jsonl",
        ])
        .unwrap();
        match cli.command {
            Command::Traces(TracesCommand::Analyze {
                top,
                trace,
                project,
                playwright_report,
                files,
            }) => {
                assert!(
                    playwright_report.is_empty(),
                    "the flag is opt-in; omitting it must not conjure a report path",
                );
                assert_eq!(top, 40);
                assert_eq!(trace, None);
                assert_eq!(project.as_deref(), Some("firefox"));
                assert_eq!(
                    files,
                    vec![PathBuf::from("a.jsonl"), PathBuf::from("b.jsonl")]
                );
            }
            _ => panic!("expected traces analyze"),
        }
        assert_eq!(
            Cli::try_parse_from(["xtask", "traces", "analyze", "x.jsonl"])
                .unwrap()
                .command_name(),
            "traces-analyze"
        );
    }

    #[test]
    fn traces_analyze_requires_a_file() {
        assert!(Cli::try_parse_from(["xtask", "traces", "analyze"]).is_err());
    }

    #[test]
    fn traces_analyze_top_must_be_positive() {
        assert!(
            Cli::try_parse_from(["xtask", "traces", "analyze", "--top", "0", "x.jsonl"]).is_err()
        );
    }

    #[test]
    fn ordinary_pr_commands_are_not_xtask_entrypoints() {
        for command in ["watch", "land", "cleanup"] {
            assert!(Cli::try_parse_from(["xtask", "pr", command, "731"]).is_err());
        }
    }

    #[test]
    fn produces_json_payload_false_for_traces_commands() {
        let analyze = Cli::try_parse_from(["xtask", "traces", "analyze", "x.jsonl"]).unwrap();
        assert!(!analyze.command.produces_json_payload());
        let run = Cli::try_parse_from(["xtask", "traces", "run"]).unwrap();
        assert!(!run.command.produces_json_payload());
        let audit = Cli::try_parse_from(["xtask", "audit-wasm"]).unwrap();
        assert!(audit.command.produces_json_payload());
    }

    #[test]
    fn traces_run_parses_flags() {
        let cli = Cli::try_parse_from([
            "xtask",
            "traces",
            "run",
            "--top",
            "40",
            "--single-worker",
            "--browser",
            "firefox",
            "--trace",
            "aa",
        ])
        .unwrap();
        assert_eq!(cli.command_name(), "traces-run");
        match cli.command {
            Command::Traces(TracesCommand::Run {
                top,
                trace,
                single_worker,
                browser,
            }) => {
                assert_eq!(top, 40);
                assert_eq!(trace.as_deref(), Some("aa"));
                assert!(single_worker);
                assert_eq!(browser, Some(E2eBrowser::Firefox));
            }
            _ => panic!("expected traces run"),
        }
    }

    #[test]
    fn server_fn_coverage_parses_both_subcommands() {
        // Both spellings appear verbatim in the gate's failure messages and in
        // CONTRIBUTING, so a rename must break a test rather than a developer.
        assert!(Cli::try_parse_from(["xtask", "server-fn-coverage", "regenerate"]).is_err());
        let cli = Cli::try_parse_from([
            "xtask",
            "server-fn-coverage",
            "regenerate",
            "--capture",
            "capture.tar.gz",
            "--producer-output",
            "/nix/store/selected",
            "--lane",
            "sqlite-chromium-unsplit",
            "--catalog",
            "lanes.json",
        ])
        .unwrap();
        assert_eq!(cli.command_name(), "server-fn-coverage-regenerate");
        assert!(matches!(
            cli.command,
            Command::ServerFnCoverage(ServerFnCoverageCommand::Regenerate(_))
        ));

        let cli = Cli::try_parse_from([
            "xtask",
            "server-fn-coverage",
            "verify",
            "--capture",
            "capture.tar.gz",
            "--producer-output",
            "/nix/store/selected",
            "--lane",
            "sqlite-chromium-unsplit",
            "--catalog",
            "lanes.json",
        ])
        .unwrap();
        assert_eq!(cli.command_name(), "server-fn-coverage-verify");
        assert!(matches!(
            cli.command,
            Command::ServerFnCoverage(ServerFnCoverageCommand::Verify(_))
        ));
    }

    #[test]
    fn server_fn_coverage_requires_a_subcommand() {
        assert!(Cli::try_parse_from(["xtask", "server-fn-coverage"]).is_err());
    }

    #[test]
    fn traces_run_defaults() {
        let cli = Cli::try_parse_from(["xtask", "traces", "run"]).unwrap();
        match cli.command {
            Command::Traces(TracesCommand::Run {
                top,
                trace,
                single_worker,
                browser,
            }) => {
                assert_eq!(top, 25);
                assert_eq!(trace, None);
                assert!(!single_worker);
                assert_eq!(browser, None);
            }
            _ => panic!("expected traces run"),
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
