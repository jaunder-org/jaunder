use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::{issue, steps};

#[derive(Parser)]
#[command(name = "xtask", about = "Jaunder dev orchestration")]
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

#[derive(Subcommand)]
pub enum Command {
    /// Inner loop (auto-fixes formatting): host static checks + clippy + the host
    /// xtask/tools unit suites, then the host-native root-workspace nextest suite
    /// under an ephemeral PostgreSQL and the Nix-only wasm/doctest checks. `--no-test`
    /// skips the root-workspace and Nix-only test checks; static, clippy, and the
    /// xtask/tools unit tests still run — as does the host-side `doctest-fences`
    /// step, which gates the `xtask`/`tools` fence population that no Nix check can
    /// see (#763).
    Check {
        /// Skip the root-workspace and Nix-only test checks — static + clippy +
        /// host xtask/tools unit tests only.
        #[arg(long)]
        no_test: bool,
    },
    /// Fast commit-time gate. Runs the same host surface as `check --no-test`,
    /// then re-stages only formatter/check mutations that are provably safe for the
    /// paths the author had already staged.
    Precommit,
    /// Fast push-time gate. Refuses a dirty working tree, then runs the
    /// verify-only host surface plus the host-native product Rust test lane. This
    /// is the local hook path; hermetic Nix coverage, wasm, and doctest checks stay
    /// in `validate --no-e2e` and CI.
    Prepush,
    /// Full gate (never mutates the tree): static + clippy + the host xtask unit
    /// suite (verify-only) + Nix coverage + the e2e VMs and authoritative
    /// server-function coverage verification. `--no-e2e` skips the e2e VMs.
    /// Refuses a dirty working tree unless `--allow-dirty`.
    Validate {
        /// Skip the e2e VM checks — static + clippy + xtask tests + coverage only.
        #[arg(long)]
        no_e2e: bool,
        /// Run even when the working tree is dirty (skip the clean-tree precheck).
        #[arg(long)]
        allow_dirty: bool,
    },
    /// Produce a host-side repository census for manual maintenance audits. The
    /// report is informational: candidates are neither findings nor gate failures.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask census\n  \
        cargo xtask census --json")]
    Census,
    /// Measure the frontend WASM/JS bundle size — raw, gzip, and brotli.
    ///
    /// Reports the download weight of the deterministic `nix build .#site`
    /// output (`pkg/jaunder.wasm`, `pkg/jaunder.js`) so you can catch
    /// bundle-size bloat before it ships and compare a change's effect on what
    /// users download. Run it after a change you expect to move the bundle (a new
    /// dependency, a feature touching the client), or periodically to watch the
    /// trend.
    ///
    /// The totals also back `validate`'s `wasm-budget` step, which fails when raw
    /// `pkg/jaunder.wasm` exceeds a committed ceiling (#836) — so the gate and
    /// this tool can never disagree about what the bundle weighs. `--breakdown`
    /// remains manual; it is not part of `check`/`validate`.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask audit-wasm\n  \
        cargo xtask audit-wasm --site-path /nix/store/...-jaunder-site\n  \
        cargo xtask audit-wasm --breakdown\n  \
        cargo xtask --json audit-wasm")]
    AuditWasm {
        /// Audit a prebuilt `.#site` store path instead of running `nix build`.
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
    /// Build ONE e2e VM check (a {backend}×{browser} combo) through the same
    /// diagnostic-preserving wrapper `validate` uses. For CI matrix fan-out;
    /// not part of `check`/`validate`. Runs on the host only.
    E2e {
        #[arg(value_enum)]
        backend: E2eBackend,
        #[arg(value_enum)]
        browser: E2eBrowser,
    },
    /// Run the host e2e loop, owning each lifecycle: build the CSR bundle +
    /// server, start `jaunder serve` on an ephemeral port with the VM's capture
    /// env + a per-run temp DB, seed via the shared `devtool seed-e2e`, run
    /// Playwright against the discovered URL, and tear the server down on every
    /// exit path. Normal mode runs Chromium ordinary/admin tests; visual update
    /// mode builds release CSR and updates Chromium and Firefox baselines in
    /// separate fresh lifecycles. Self-contained — no pre-existing server and no
    /// `:3000` conflict. Loads the same `playwright.config.ts` the CI VM loads.
    /// Host only.
    E2eLocal {
        /// A spec path or `file:line` filter passed through to Playwright as a
        /// positional arg (single-test runs).
        test: Option<String>,
        /// Update every Chromium and Firefox visual baseline using release CSR.
        #[arg(long, conflicts_with = "test")]
        update_visual_snapshots: bool,
    },
    /// Run host-native Rust tests through nextest with the shared sccache setup and
    /// an isolated PostgreSQL lifecycle. Pass nextest filters after `--`.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask test-local\n  \
        cargo xtask test-local -- -p storage site_config_primitives_round_trip")]
    TestLocal {
        /// Arguments forwarded to `cargo nextest run`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        nextest_args: Vec<String>,
    },
    /// Build the CSR wasm bundle on the host (`cargo build -p csr` + the shared
    /// `devtool csr-bundle` post-processing) — the cargo-leptos-free bundle build
    /// (#236). Output to `target/site/pkg/`. Debug by default; `--release` matches
    /// CI's optimized wasm. Host only.
    BuildCsr {
        /// Build optimized (release) wasm instead of debug.
        #[arg(long)]
        release: bool,
    },
    /// ADR maintenance.
    #[command(subcommand)]
    Adr(AdrCommand),
    /// OpenTelemetry trace analysis (host-side; ADR-0028).
    #[command(subcommand)]
    Traces(TracesCommand),
    /// Coverage tooling — the source-filter drift probe (#241).
    #[command(subcommand)]
    Coverage(CoverageCommand),
    /// Nix maintenance commands that evaluate repository derivation boundaries.
    #[command(subcommand)]
    Nix(NixCommand),
    /// Trace-derived `#[server]` fn flow coverage (#681): which server entry
    /// points the e2e suite actually drives.
    #[command(subcommand)]
    ServerFnCoverage(ServerFnCoverageCommand),
    /// Observe a pull request until the next caller-actionable outcome (#729).
    /// Host-only manual command; needs `gh`.
    #[command(subcommand)]
    Pr(PrCommand),
    /// Gather or apply Jaunder issue tracker metadata (#1090/#1091).
    /// Host-only manual command; needs `gh`.
    #[command(subcommand)]
    Issue(issue::IssueCommand),
}

/// An explicit passive-observation target for `pr watch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PrWatchUntil {
    Merged,
}

/// `pr` subcommands (#729).
///
/// The split is the point: `watch` cannot merge anything, so running `land` **is**
/// the merge approval. Every other action in the sequence — re-running a red job,
/// rebasing, re-enqueueing after an ejection — needs a judgement call and stays with
/// the human; these two only turn the crank.
#[derive(Subcommand)]
pub enum PrCommand {
    /// Observe a PR until the next caller-actionable outcome. Never merges,
    /// re-runs a job, pushes, or re-enqueues. A green, landable PR reports
    /// `ready-to-land`; an already armed or queued PR continues to its outcome.
    ///
    /// Exit is 0 for `ready-to-land` or `merged`. Distinguish every outcome through
    /// `pr.outcome` in `--json` or `.xtask/last-result.json`.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask pr watch\n  \
        cargo xtask pr watch 731\n  \
        cargo xtask --json pr watch 731 --once\n  \
        cargo xtask pr watch 731 --until merged\n  \
        cargo xtask pr watch 731 --interval 60 --timeout 30")]
    Watch {
        /// PR number. Omitted: the open PR whose head is the current branch.
        number: Option<u64>,
        /// Seconds between polls.
        #[arg(long, value_name = "SECONDS", default_value_t = 30, value_parser = clap::value_parser!(u64).range(5..))]
        interval: u64,
        /// Minutes to watch before reporting `timed-out`.
        #[arg(long, value_name = "MINUTES", default_value_t = 90, value_parser = clap::value_parser!(u64).range(1..))]
        timeout: u64,
        /// Take one snapshot and report, instead of looping. Can report `pending`.
        #[arg(long)]
        once: bool,
        /// Continue across `ready-to-land` while another actor may arm the merge.
        #[arg(long, value_name = "OUTCOME", conflicts_with = "once")]
        until: Option<PrWatchUntil>,
    },
    /// Arm auto-merge, verify the arm actually took, then watch it to `merged`.
    ///
    /// Running this command is the merge approval. It refuses when invoked from the
    /// PR's own branch with unpushed local commits, and never re-enqueues after an
    /// ejection — that needs a human to decide the ejection was noise.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask pr land\n  \
        cargo xtask pr land 731")]
    Land {
        /// PR number. Omitted: the open PR whose head is the current branch.
        number: Option<u64>,
        /// Seconds between polls.
        #[arg(long, value_name = "SECONDS", default_value_t = 30, value_parser = clap::value_parser!(u64).range(5..))]
        interval: u64,
        /// Minutes to watch before reporting `timed-out`.
        #[arg(long, value_name = "MINUTES", default_value_t = 90, value_parser = clap::value_parser!(u64).range(1..))]
        timeout: u64,
    },
    /// Clean this checkout after a PR has merged. This is local-only: it proves the
    /// exact merged head before fetching, detaching, safely deleting the branch, and
    /// running `cargo clean`.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask pr cleanup\n  \
        cargo xtask pr cleanup 1155")]
    Cleanup {
        /// Merged PR number. Omitted: exhaustively find the merged PR for this exact
        /// checked-out branch and HEAD.
        number: Option<u64>,
    },
}

/// `server-fn-coverage` subcommands (#681).
#[derive(Subcommand)]
pub enum ServerFnCoverageCommand {
    /// Re-derive the coverage snapshot from the `sqlite × chromium` e2e capture
    /// and write it to `docs/coverage/server-fns.json`.
    ///
    /// Run after `cargo xtask e2e sqlite chromium`, which lifts the capture this
    /// reads. That one combo is authoritative (spec D6): neither backend nor
    /// browser changes which server fns the UI invokes, and running it per-combo
    /// avoids the aggregate `checks.e2e` join, where both sqlite combos' captures
    /// collide under the same file name.
    #[command(after_help = "EXAMPLES:\n  cargo xtask e2e sqlite chromium\n  \
        cargo xtask server-fn-coverage regenerate")]
    Regenerate,
    /// Re-derive the snapshot and fail if it differs from the committed copy.
    /// The e2e-lane half of the gate.
    #[command(after_help = "EXAMPLES:\n  cargo xtask server-fn-coverage verify")]
    Verify,
}

/// `adr` subcommands.
#[derive(Subcommand)]
pub enum AdrCommand {
    /// Regenerate the ADR index table in `docs/README.md` from `docs/adr/`: the
    /// number, link target, and status cells. Hand-curated titles are preserved
    /// (a new row seeds its title from the ADR heading). Idempotent; touches only
    /// the marked table block. The `adr-readme-parity` gate fails on drift.
    #[command(after_help = "EXAMPLES:\n  cargo xtask adr sync-readme")]
    SyncReadme,
    /// Deterministic promotion mutation used by the serialized ADR promoter
    /// after feature merge: number tracked drafts, stage each complete rename,
    /// rewrite path citations and proposed status, and regenerate the index.
    /// Feature authors and shipping flows do not invoke this directly.
    #[command(
        after_help = "AUTOMATION PRIMITIVE: invoked by `cargo xtask adr promoter` after feature merge.\n\nEXAMPLES:\n  cargo xtask adr promote"
    )]
    Promote,
    /// Run the serialized ADR promoter for a GitHub Actions event. Generates from
    /// fresh `main`, owns the stable promoter PR, and fail-closed re-arms an exact
    /// dequeued promoter only after both required context sets are green.
    #[command(after_help = "EXAMPLES:\n  cargo xtask adr promoter")]
    Promoter,
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
        # trace files extracted from an e2e capture-<backend>.tar.gz bundle (capture/otel-traces.jsonl):\n  \
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
        /// `.xtask/diagnostics/e2e-sqlite-chromium/playwright-report-sqlite.json`.
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
            Command::Check { .. } => "check",
            Command::Precommit => "precommit",
            Command::Prepush => "prepush",
            Command::Validate { .. } => "validate",
            Command::Census => "census",
            Command::AuditWasm { .. } => "audit-wasm",
            Command::E2e { .. } => "e2e",
            Command::E2eLocal { .. } => "e2e-local",
            Command::TestLocal { .. } => "test-local",
            Command::BuildCsr { .. } => "build-csr",
            Command::Adr(AdrCommand::SyncReadme) => "adr-sync-readme",
            Command::Adr(AdrCommand::Promote) => "adr-promote",
            Command::Adr(AdrCommand::Promoter) => "adr-promoter",
            Command::Traces(TracesCommand::Analyze { .. }) => "traces-analyze",
            Command::Traces(TracesCommand::Run { .. }) => "traces-run",
            Command::Traces(TracesCommand::BootPhases { .. }) => "traces-boot-phases",
            Command::Coverage(CoverageCommand::ProbeSource) => "coverage-probe-source",
            Command::Nix(NixCommand::ProbeSource) => "nix-probe-source",
            Command::ServerFnCoverage(ServerFnCoverageCommand::Regenerate) => {
                steps::server_fn_coverage_check::REGENERATE_STEP
            }
            Command::ServerFnCoverage(ServerFnCoverageCommand::Verify) => {
                steps::server_fn_coverage_check::VERIFY_STEP
            }
            Command::Pr(PrCommand::Watch { .. }) => "pr-watch",
            Command::Pr(PrCommand::Cleanup { .. }) => "pr-cleanup",
            Command::Pr(PrCommand::Land { .. }) => "pr-land",
            Command::Issue(issue::IssueCommand::Candidates { .. }) => "issue-candidates",
            Command::Issue(issue::IssueCommand::Create { .. }) => "issue-create",
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
            Command::Traces(
                TracesCommand::Analyze { .. }
                    | TracesCommand::Run { .. }
                    | TracesCommand::BootPhases { .. }
            )
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn precommit_parses_as_first_class_subcommand() {
        let cli = Cli::try_parse_from(["xtask", "precommit"]).unwrap();
        match cli.command {
            Command::Precommit => {}
            _ => panic!("expected precommit"),
        }
    }

    #[test]
    fn census_parses_with_json_and_has_a_stable_command_name() {
        let cli = Cli::try_parse_from(["xtask", "census", "--json"]).unwrap();
        assert!(cli.json);
        assert_eq!(cli.command_name(), "census");
        assert!(matches!(cli.command, Command::Census));
    }

    #[test]
    fn prepush_parses_as_first_class_subcommand() {
        let cli = Cli::try_parse_from(["xtask", "prepush"]).unwrap();
        match cli.command {
            Command::Prepush => {}
            _ => panic!("expected prepush"),
        }
        assert_eq!(cli.command_name(), "prepush");
    }

    #[test]
    fn issue_candidates_parses_milestone_and_command_name() {
        let cli = Cli::try_parse_from([
            "xtask",
            "issue",
            "candidates",
            "--milestone",
            "Developer tooling & DX",
        ])
        .unwrap();
        assert_eq!(cli.command_name(), "issue-candidates");
        match cli.command {
            Command::Issue(issue::IssueCommand::Candidates { milestone }) => {
                assert_eq!(milestone, "Developer tooling & DX");
            }
            _ => panic!("expected issue candidates"),
        }
    }

    #[test]
    fn issue_create_parses_required_metadata() {
        let cli = Cli::try_parse_from([
            "xtask",
            "issue",
            "create",
            "--title",
            "feat(xtask): add issue helpers",
            "--type",
            "Task",
            "--milestone",
            "Developer tooling & DX",
            "--priority",
            "p2",
            "--label",
            "tooling",
            "--label",
            "dx",
            "--body-file",
            "/tmp/body.md",
        ])
        .unwrap();
        assert_eq!(cli.command_name(), "issue-create");
        match cli.command {
            Command::Issue(issue::IssueCommand::Create {
                title,
                issue_type,
                milestone,
                priority,
                labels,
                body_file,
            }) => {
                assert_eq!(title, "feat(xtask): add issue helpers");
                assert_eq!(issue_type, "Task");
                assert_eq!(milestone, "Developer tooling & DX");
                assert_eq!(priority, issue::Priority::P2);
                assert_eq!(labels, ["tooling", "dx"]);
                assert_eq!(body_file, PathBuf::from("/tmp/body.md"));
            }
            _ => panic!("expected issue create"),
        }
    }

    #[test]
    fn precommit_does_not_replace_check_no_test_parse() {
        let cli = Cli::try_parse_from(["xtask", "check", "--no-test"]).unwrap();
        match cli.command {
            Command::Check { no_test } => assert!(no_test),
            _ => panic!("expected check"),
        }
    }

    #[test]
    fn validate_allow_dirty_parses() {
        let cli = Cli::try_parse_from(["xtask", "validate", "--allow-dirty"]).unwrap();
        match cli.command {
            Command::Validate {
                no_e2e,
                allow_dirty,
            } => {
                assert!(!no_e2e);
                assert!(allow_dirty);
            }
            _ => panic!("expected validate"),
        }
    }

    #[test]
    fn validate_defaults_reject_dirty() {
        let cli = Cli::try_parse_from(["xtask", "validate"]).unwrap();
        match cli.command {
            Command::Validate { allow_dirty, .. } => assert!(!allow_dirty),
            _ => panic!("expected validate"),
        }
    }
    #[test]
    fn e2e_combo_parses_backend_and_browser() {
        let cli = Cli::try_parse_from(["xtask", "e2e", "postgres", "firefox"]).unwrap();
        match cli.command {
            Command::E2e { backend, browser } => {
                assert_eq!(backend, E2eBackend::Postgres);
                assert_eq!(browser, E2eBrowser::Firefox);
            }
            _ => panic!("expected e2e"),
        }
    }

    #[test]
    fn e2e_local_parses_filter_or_visual_update_mode() {
        let cli = Cli::try_parse_from(["xtask", "e2e-local"]).unwrap();
        match cli.command {
            Command::E2eLocal {
                test,
                update_visual_snapshots,
            } => {
                assert_eq!(test, None);
                assert!(!update_visual_snapshots);
            }
            _ => panic!("expected e2e-local"),
        }

        let cli = Cli::try_parse_from(["xtask", "e2e-local", "auth-flow.spec.ts"]).unwrap();
        match cli.command {
            Command::E2eLocal {
                test,
                update_visual_snapshots,
            } => {
                assert_eq!(test.as_deref(), Some("auth-flow.spec.ts"));
                assert!(!update_visual_snapshots);
            }
            _ => panic!("expected e2e-local with filter"),
        }

        let cli = Cli::try_parse_from(["xtask", "e2e-local", "--update-visual-snapshots"]).unwrap();
        match cli.command {
            Command::E2eLocal {
                test,
                update_visual_snapshots,
            } => {
                assert_eq!(test, None);
                assert!(update_visual_snapshots);
            }
            _ => panic!("expected e2e-local visual update mode"),
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
    fn adr_sync_readme_parses() {
        let cli = Cli::try_parse_from(["xtask", "adr", "sync-readme"]).unwrap();
        assert_eq!(cli.command_name(), "adr-sync-readme");
    }

    #[test]
    fn adr_promote_parses() {
        let cli = Cli::try_parse_from(["xtask", "adr", "promote"]).unwrap();
        assert_eq!(cli.command_name(), "adr-promote");
    }

    #[test]
    fn adr_promoter_parses() {
        let cli = Cli::try_parse_from(["xtask", "adr", "promoter"]).unwrap();
        assert_eq!(cli.command_name(), "adr-promoter");
    }

    #[test]
    fn nix_probe_source_parses_as_closed_subcommand() {
        let cli = Cli::try_parse_from(["xtask", "nix", "probe-source"]).unwrap();
        assert_eq!(cli.command_name(), "nix-probe-source");
        assert!(matches!(cli.command, Command::Nix(NixCommand::ProbeSource)));
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
    fn pr_watch_parses_with_defaults_and_optional_number() {
        let cli = Cli::try_parse_from(["xtask", "pr", "watch"]).unwrap();
        assert_eq!(cli.command_name(), "pr-watch");
        match cli.command {
            Command::Pr(PrCommand::Watch {
                number,
                interval,
                timeout,
                once,
                until,
            }) => {
                assert_eq!(number, None);
                assert_eq!(interval, 30);
                assert_eq!(timeout, 90);
                assert!(!once);
                assert_eq!(until, None);
            }
            _ => panic!("expected pr watch"),
        }
    }

    #[test]
    fn pr_cleanup_parses_optional_number_and_names_itself() {
        let cli = Cli::try_parse_from(["xtask", "pr", "cleanup"]).unwrap();
        assert_eq!(cli.command_name(), "pr-cleanup");
        assert!(matches!(
            cli.command,
            Command::Pr(PrCommand::Cleanup { number: None })
        ));
        assert!(matches!(
            Cli::try_parse_from(["xtask", "pr", "cleanup", "1155"])
                .unwrap()
                .command,
            Command::Pr(PrCommand::Cleanup { number: Some(1155) })
        ));
        assert!(Cli::try_parse_from(["xtask", "pr", "cleanup", "not-a-number"]).is_err());
    }

    #[test]
    fn pr_watch_parses_explicit_number_and_flags() {
        let cli = Cli::try_parse_from([
            "xtask",
            "pr",
            "watch",
            "731",
            "--interval",
            "60",
            "--timeout",
            "10",
            "--once",
        ])
        .unwrap();
        match cli.command {
            Command::Pr(PrCommand::Watch {
                number,
                interval,
                timeout,
                once,
                until,
            }) => {
                assert_eq!(number, Some(731));
                assert_eq!(interval, 60);
                assert_eq!(timeout, 10);
                assert!(once);
                assert_eq!(until, None);
            }
            _ => panic!("expected pr watch"),
        }
    }

    #[test]
    fn pr_watch_parses_until_merged() {
        let cli =
            Cli::try_parse_from(["xtask", "pr", "watch", "731", "--until", "merged"]).unwrap();
        match cli.command {
            Command::Pr(PrCommand::Watch { number, until, .. }) => {
                assert_eq!(number, Some(731));
                assert_eq!(until, Some(PrWatchUntil::Merged));
            }
            _ => panic!("expected pr watch"),
        }
    }

    #[test]
    fn pr_watch_rejects_once_with_until_merged() {
        assert!(
            Cli::try_parse_from(["xtask", "pr", "watch", "--once", "--until", "merged"]).is_err()
        );
    }

    #[test]
    fn pr_watch_rejects_unknown_until_value() {
        assert!(Cli::try_parse_from(["xtask", "pr", "watch", "--until", "ready"]).is_err());
    }

    #[test]
    fn pr_land_rejects_once() {
        // Arming a merge and then immediately not watching it is never intended, and
        // leaving it legal means someone walks away believing they watched it.
        assert!(Cli::try_parse_from(["xtask", "pr", "land", "--once"]).is_err());
    }

    #[test]
    fn pr_interval_below_the_floor_is_rejected() {
        assert!(Cli::try_parse_from(["xtask", "pr", "watch", "--interval", "1"]).is_err());
    }

    #[test]
    fn pr_land_names_itself() {
        assert_eq!(
            Cli::try_parse_from(["xtask", "pr", "land"])
                .unwrap()
                .command_name(),
            "pr-land"
        );
    }

    #[test]
    fn pr_requires_a_subcommand() {
        assert!(Cli::try_parse_from(["xtask", "pr"]).is_err());
    }

    #[test]
    fn produces_json_payload_false_for_traces_commands() {
        let analyze = Cli::try_parse_from(["xtask", "traces", "analyze", "x.jsonl"]).unwrap();
        assert!(!analyze.command.produces_json_payload());
        let run = Cli::try_parse_from(["xtask", "traces", "run"]).unwrap();
        assert!(!run.command.produces_json_payload());
        let check = Cli::try_parse_from(["xtask", "check"]).unwrap();
        assert!(check.command.produces_json_payload());
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
        let cli = Cli::try_parse_from(["xtask", "server-fn-coverage", "regenerate"]).unwrap();
        assert_eq!(cli.command_name(), "server-fn-coverage-regenerate");
        assert!(matches!(
            cli.command,
            Command::ServerFnCoverage(ServerFnCoverageCommand::Regenerate)
        ));

        let cli = Cli::try_parse_from(["xtask", "server-fn-coverage", "verify"]).unwrap();
        assert_eq!(cli.command_name(), "server-fn-coverage-verify");
        assert!(matches!(
            cli.command,
            Command::ServerFnCoverage(ServerFnCoverageCommand::Verify)
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
}
