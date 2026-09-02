use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use xshell::Shell;

mod adr;
mod adr_readme;
mod audit_wasm;
mod census;
mod compile_cache;
pub mod coverage;
mod doc_links;
pub mod elisp_coverage;
mod files;
pub mod git;
mod ids;
pub mod issue;
pub mod markers;
mod nix_build;
mod playwright_report;
pub mod pr;
mod result;
mod server_fn_coverage;
mod server_fns;
mod sh;
#[cfg(test)]
mod test_support;
mod traces;
mod wasm_budget;
mod wasm_sections;
mod wasm_symbols;
mod web_server_fns;
mod steps {
    pub mod adr_check;
    pub mod boot_decomposition_coverage;
    pub mod build_csr;
    pub mod common_host_target_closure;
    pub mod doc_links;
    pub mod doctest_fences;
    pub mod duration_budget;
    pub mod e2e_goto_wrapper_check;
    pub mod e2e_local;
    pub mod e2e_scaffold_check;
    pub mod e2e_server_fn_endpoint_check;
    pub mod e2e_telemetry_boundary_check;
    pub mod error_swallowing_inventory_check;
    pub mod flaky;
    pub mod flow_docs;
    pub mod host_tests;
    pub mod html_sink_check;
    pub mod ident_gate;
    pub mod lint_suppression_check;
    pub mod nix;
    pub mod proffered_secret_check;
    pub mod raw_html_door_check;
    pub mod rendered_html_compiler_boundary;
    pub mod scan;
    pub mod sequence_check;
    pub mod server_fn_coverage_check;
    pub mod server_fn_registrar_check;
    pub mod server_fn_tracing_check;
    pub mod server_fn_wire_arg_error_check;
    pub mod sqlx_newtype_bind_check;
    pub mod sqlx_newtype_decode_check;
    pub mod static_checks;
    pub mod target_arch_placement_check;
    pub mod test_local;
    pub mod test_pattern_check;
    pub mod thin_components;
    pub mod traced_context_check;
    pub mod wasm_budget;
    pub mod write_transaction_contract_check;
    pub mod xlang_literal_check;
}
pub use result::{CommandResult, Mode, StepResult};

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
    fn as_str(self) -> &'static str {
        match self {
            E2eBackend::Sqlite => "sqlite",
            E2eBackend::Postgres => "postgres",
        }
    }
}

impl E2eBrowser {
    fn as_str(self) -> &'static str {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionPolicy {
    Exhaustive,
    FailFast,
}

impl Command {
    fn execution_policy(&self) -> ExecutionPolicy {
        match self {
            Self::Precommit | Self::Prepush => ExecutionPolicy::FailFast,
            Self::Check { .. } | Self::Validate { .. } => ExecutionPolicy::Exhaustive,
            _ => ExecutionPolicy::Exhaustive,
        }
    }
}

fn run_with_policy<T>(
    policy: ExecutionPolicy,
    result: &mut CommandResult,
    runners: impl IntoIterator<Item = T>,
    mut run: impl FnMut(T, &mut CommandResult),
) {
    for runner in runners {
        let before = result.steps.len();
        run(runner, result);
        if matches!(policy, ExecutionPolicy::FailFast)
            && result.steps[before..]
                .iter()
                .any(StepResult::is_blocking_failure)
        {
            break;
        }
    }
}

enum HostGateStep {
    StaticChecks(steps::static_checks::Phase),
    ResultOnly {
        name: &'static str,
        run: fn(&mut CommandResult),
        markdown_eligible: bool,
    },
    HostTests {
        markdown_eligible: bool,
    },
}

impl HostGateStep {
    fn run(&self, sh: &Shell, mode: Mode, policy: ExecutionPolicy, result: &mut CommandResult) {
        match self {
            Self::StaticChecks(phase) => {
                steps::static_checks::run_phase_with(
                    sh,
                    mode,
                    *phase,
                    policy,
                    result,
                    steps::static_checks::run_spec,
                );
            }
            Self::ResultOnly { name, run, .. } => {
                debug_assert!(!name.is_empty());
                let before = result.steps.len();
                let start = std::time::Instant::now();
                run(result);
                let duration = start.elapsed().as_millis();
                for step in &mut result.steps[before..] {
                    if step.duration_ms == 0 {
                        step.duration_ms = duration;
                    }
                }
            }
            Self::HostTests { .. } => steps::host_tests::run(sh, result),
        }
    }

    fn run_markdown(
        &self,
        sh: &xshell::Shell,
        mode: Mode,
        policy: ExecutionPolicy,
        result: &mut CommandResult,
    ) {
        match self {
            Self::StaticChecks(phase) => steps::static_checks::run_markdown_phase_with(
                sh,
                mode,
                *phase,
                policy,
                result,
                steps::static_checks::run_spec,
            ),
            Self::ResultOnly { run, .. } => {
                let before = result.steps.len();
                let start = std::time::Instant::now();
                run(result);
                let duration = start.elapsed().as_millis();
                for step in &mut result.steps[before..] {
                    if step.duration_ms == 0 {
                        step.duration_ms = duration;
                    }
                }
            }
            Self::HostTests { .. } => unreachable!("host tests are not Markdown-route eligible"),
        }
    }

    fn markdown_eligible(&self, _mode: Mode) -> bool {
        match self {
            // The phase is a catalog container; `run_markdown_phase_with` applies
            // each static spec's own eligibility metadata without a parallel list.
            Self::StaticChecks(_) => true,
            Self::ResultOnly {
                markdown_eligible, ..
            } => *markdown_eligible,
            Self::HostTests { markdown_eligible } => *markdown_eligible,
        }
    }

    #[cfg(test)]
    fn push_names(&self, mode: Mode, names: &mut Vec<&'static str>) {
        match self {
            Self::StaticChecks(phase) => names.extend(
                steps::static_checks::specs_for_phase(*phase, mode)
                    .into_iter()
                    .map(|spec| spec.name),
            ),
            Self::ResultOnly { name, .. } => names.push(name),
            Self::HostTests { .. } => names.push("host-tests"),
        }
    }
}

fn run_flow_docs(result: &mut CommandResult) {
    result.push(steps::flow_docs::run());
}

const HOST_GATE_NON_TEST_STEPS: &[HostGateStep] = &[
    // Source consistency first: these are fixable or concrete file-shape errors,
    // and they must run before expensive compile/type work.
    HostGateStep::StaticChecks(steps::static_checks::Phase::SourceConsistency),
    HostGateStep::ResultOnly {
        name: "sequence-check",
        run: steps::sequence_check::run,
        markdown_eligible: true,
    },
    HostGateStep::ResultOnly {
        name: "adr-filenames",
        run: steps::adr_check::run,
        markdown_eligible: true,
    },
    HostGateStep::ResultOnly {
        name: "doc-links",
        run: steps::doc_links::run,
        markdown_eligible: true,
    },
    HostGateStep::ResultOnly {
        name: "flow-docs",
        run: run_flow_docs,
        markdown_eligible: true,
    },
    HostGateStep::ResultOnly {
        name: "error-swallowing-inventory",
        run: steps::error_swallowing_inventory_check::run,
        markdown_eligible: true,
    },
    HostGateStep::ResultOnly {
        name: "test-patterns",
        run: steps::test_pattern_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "server-fn-registrar",
        run: steps::server_fn_registrar_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "server-fn-tracing",
        run: steps::server_fn_tracing_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "server-fn-coverage",
        run: steps::server_fn_coverage_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "server-fn-wire-arg-error",
        run: steps::server_fn_wire_arg_error_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "traced-context",
        run: steps::traced_context_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "e2e-telemetry-boundary",
        run: steps::e2e_telemetry_boundary_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "proffered-secret",
        run: steps::proffered_secret_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "e2e-goto-wrapper",
        run: steps::e2e_goto_wrapper_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "e2e-server-fn-endpoints",
        run: steps::e2e_server_fn_endpoint_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "target-arch-placement",
        run: steps::target_arch_placement_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "write-transaction-contract",
        run: steps::write_transaction_contract_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "common-host-target-closure",
        run: steps::common_host_target_closure::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "lint-suppression",
        run: steps::lint_suppression_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "thin-components",
        run: steps::thin_components::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "sqlx-newtype-bind",
        run: steps::sqlx_newtype_bind_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "sqlx-newtype-decode",
        run: steps::sqlx_newtype_decode_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "doctest-fences",
        run: steps::doctest_fences::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "rendered-html-compiler-boundary",
        run: steps::rendered_html_compiler_boundary::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "raw-html-door",
        run: steps::raw_html_door_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "html-sink",
        run: steps::html_sink_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "e2e-scaffold",
        run: steps::e2e_scaffold_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "xlang-literal",
        run: steps::xlang_literal_check::run,
        markdown_eligible: false,
    },
    // Compile/type surfaces after cheap repository-shape checks. They are still
    // before host runtime tests, which cannot pass if compilation is broken.
    HostGateStep::StaticChecks(steps::static_checks::Phase::CompileAndType),
    HostGateStep::StaticChecks(steps::static_checks::Phase::HostRuntime),
];

const HOST_TESTS_STEP: HostGateStep = HostGateStep::HostTests {
    markdown_eligible: false,
};

fn run_host_steps_with<'a>(
    sh: &Shell,
    mode: Mode,
    policy: ExecutionPolicy,
    result: &mut CommandResult,
    steps: impl IntoIterator<Item = &'a HostGateStep>,
    mut run_step: impl FnMut(&HostGateStep, &Shell, Mode, ExecutionPolicy, &mut CommandResult),
) {
    run_with_policy(policy, result, steps, |step, result| {
        run_step(step, sh, mode, policy, result);
    });
}

fn run_host_gate_without_tests(
    sh: &Shell,
    mode: Mode,
    policy: ExecutionPolicy,
    result: &mut CommandResult,
) {
    run_host_steps_with(
        sh,
        mode,
        policy,
        result,
        HOST_GATE_NON_TEST_STEPS,
        HostGateStep::run,
    );
}

fn run_markdown_host_gate(
    sh: &Shell,
    mode: Mode,
    policy: ExecutionPolicy,
    result: &mut CommandResult,
) {
    run_host_steps_with(
        sh,
        mode,
        policy,
        result,
        HOST_GATE_NON_TEST_STEPS
            .iter()
            .filter(|step| step.markdown_eligible(mode)),
        HostGateStep::run_markdown,
    );
}

fn run_host_gate(sh: &Shell, mode: Mode, policy: ExecutionPolicy, result: &mut CommandResult) {
    run_host_steps_with(
        sh,
        mode,
        policy,
        result,
        HOST_GATE_NON_TEST_STEPS
            .iter()
            .chain(std::iter::once(&HOST_TESTS_STEP)),
        HostGateStep::run,
    );
}

fn dispatch_precommit_host_gate_with(
    class: git::PrecommitChangeClass,
    result: &mut CommandResult,
    run_markdown: impl FnOnce(&mut CommandResult),
    run_broad: impl FnOnce(&mut CommandResult),
) {
    match class {
        git::PrecommitChangeClass::StagedMarkdownOnly => run_markdown(result),
        git::PrecommitChangeClass::Broad(_) => run_broad(result),
    }
}

fn dispatch_precommit_host_gate(
    class: git::PrecommitChangeClass,
    sh: &xshell::Shell,
    policy: ExecutionPolicy,
    result: &mut CommandResult,
) {
    dispatch_precommit_host_gate_with(
        class,
        result,
        |result| run_markdown_host_gate(sh, Mode::Fix, policy, result),
        |result| run_host_gate(sh, Mode::Fix, policy, result),
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrepushPhase {
    HostGate,
    LocalTests,
    WorkspaceDoctests,
}

impl PrepushPhase {
    fn run(self, sh: &Shell, policy: ExecutionPolicy, result: &mut CommandResult) {
        match self {
            Self::HostGate => run_host_gate(sh, Mode::Check, policy, result),
            Self::LocalTests => steps::test_local::run(sh, result, &[]),
            Self::WorkspaceDoctests => steps::doctest_fences::run_workspace(sh, result),
        }
    }

    #[cfg(test)]
    const fn name(self) -> &'static str {
        match self {
            Self::HostGate => "host-gate",
            Self::LocalTests => "test-local",
            Self::WorkspaceDoctests => "workspace-doctests",
        }
    }
}

const PREPUSH_PHASES: &[PrepushPhase] = &[
    PrepushPhase::HostGate,
    PrepushPhase::LocalTests,
    PrepushPhase::WorkspaceDoctests,
];

fn run_local_push_gate(sh: &Shell, policy: ExecutionPolicy, result: &mut CommandResult) {
    run_with_policy(policy, result, PREPUSH_PHASES, |phase, result| {
        phase.run(sh, policy, result);
    });
}

fn run_prepush_with(
    sh: &Shell,
    policy: ExecutionPolicy,
    result: &mut CommandResult,
    precheck: impl FnOnce() -> StepResult,
    run_gate: impl FnOnce(&Shell, ExecutionPolicy, &mut CommandResult),
) {
    let precheck_start = std::time::Instant::now();
    let precheck = precheck().with_duration(precheck_start.elapsed());
    let blocked = precheck.is_blocking_failure();
    result.push(precheck);
    if !blocked {
        run_gate(sh, policy, result);
    }
}

fn run_precommit_with_host_gate(
    dir: &Path,
    run_gate: impl FnOnce(git::PrecommitChangeClass, &mut CommandResult),
) -> anyhow::Result<CommandResult> {
    let start = std::time::Instant::now();
    let before = git::status_snapshot(dir)?;
    let class = git::classify_precommit_change(&before);
    let mut result = CommandResult::new("precommit");
    result.push(StepResult::ok("precommit-routing").detail(class.detail()));
    run_gate(class, &mut result);
    let after = git::status_snapshot(dir)?;
    let plan = git::precommit_stage_plan(&before, &after);
    result.push(git::apply_precommit_stage_plan(dir, &plan));
    finalize(&mut result, start);
    Ok(result)
}

#[cfg(test)]
fn host_gate_step_names_for_test(mode: Mode) -> Vec<&'static str> {
    let mut names = host_gate_without_tests_step_names_for_test(mode);
    HOST_TESTS_STEP.push_names(mode, &mut names);
    names
}

#[cfg(test)]
fn host_gate_without_tests_step_names_for_test(mode: Mode) -> Vec<&'static str> {
    let mut names = Vec::new();
    for step in HOST_GATE_NON_TEST_STEPS {
        step.push_names(mode, &mut names);
    }
    names
}

#[cfg(test)]
fn precommit_host_step_names_for_test() -> Vec<&'static str> {
    host_gate_step_names_for_test(Mode::Fix)
}

#[cfg(test)]
fn markdown_precommit_step_names_for_test() -> Vec<&'static str> {
    let mut names = Vec::new();
    for step in HOST_GATE_NON_TEST_STEPS {
        match step {
            HostGateStep::StaticChecks(phase) => names.extend(
                steps::static_checks::specs_for_phase(*phase, Mode::Fix)
                    .into_iter()
                    .filter(|spec| spec.markdown_eligible)
                    .map(|spec| spec.name),
            ),
            HostGateStep::ResultOnly {
                name,
                markdown_eligible: true,
                ..
            } => names.push(name),
            HostGateStep::ResultOnly { .. } | HostGateStep::HostTests { .. } => {}
        }
    }
    names
}

pub fn run(cli: Cli) -> anyhow::Result<CommandResult> {
    // Reject --json for commands with no structured payload (the `traces` reporting
    // commands) before doing any work — a hollow envelope is worse than an error.
    if cli.json && !cli.command.produces_json_payload() {
        anyhow::bail!(
            "--json is not supported for `{}` (produces no structured output)",
            cli.command_name()
        );
    }
    match cli.command {
        Command::Check { no_test } => {
            let policy = Command::Check { no_test }.execution_policy();
            let sh = Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("check");
            run_host_gate(&sh, Mode::Fix, policy, &mut result);
            if !no_test {
                steps::test_local::run(&sh, &mut result, &[]);
            }
            steps::nix::check_supporting_test_checks(&mut result, no_test);
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Precommit => {
            let policy = Command::Precommit.execution_policy();
            let sh = Shell::new()?;
            run_precommit_with_host_gate(Path::new("."), |class, result| {
                dispatch_precommit_host_gate(class, &sh, policy, result);
            })
        }
        Command::Prepush => {
            let policy = Command::Prepush.execution_policy();
            let sh = Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("prepush");
            run_prepush_with(
                &sh,
                policy,
                &mut result,
                || clean_tree_precheck(false),
                run_local_push_gate,
            );
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Validate {
            no_e2e,
            allow_dirty,
        } => {
            let policy = Command::Validate {
                no_e2e,
                allow_dirty,
            }
            .execution_policy();
            let sh = Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("validate");
            // Clean-tree backstop: refuse a dirty tree so what is measured equals the
            // committed tip (== what CI sees). Fail fast before the expensive steps.
            let precheck_start = std::time::Instant::now();
            let precheck = clean_tree_precheck(allow_dirty).with_duration(precheck_start.elapsed());
            let blocked = precheck.is_blocking_failure();
            result.push(precheck);
            if blocked {
                finalize(&mut result, start);
                return Ok(result);
            }
            run_host_gate_without_tests(&sh, Mode::Check, policy, &mut result);
            steps::nix::static_checks(&mut result);
            // Deliberately in `validate` and not `check`: it costs a
            // `nix build .#site`, which the pre-commit gate should not pay (#836).
            steps::wasm_budget::run(&mut result);
            HOST_TESTS_STEP.run(&sh, Mode::Check, policy, &mut result);
            steps::nix::test_checks(&mut result, false);
            if !no_e2e {
                // Each browser/backend combo is realized, lifted, and reconciled
                // separately; their same-named per-backend inputs cannot safely
                // survive the aggregate `e2e-checks` symlink join. The coverage
                // verifier therefore resolves the already-realized authoritative
                // combo's individual output rather than reading the join.
                let e2e = steps::nix::e2e(&mut result);
                steps::server_fn_coverage_check::verify_after_validate(
                    &mut result,
                    e2e.combinations_ok,
                );
            }
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Census => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("census");
            let report = census::collect(Path::new("."), census::catalog())?;
            let failed = report.has_failed_cells();
            let cells = report.cell_count();
            result.census = Some(report);
            result.push(
                if failed {
                    StepResult::fail("census")
                        .detail(format!("{cells} cell(s); one or more collectors failed"))
                } else {
                    StepResult::ok("census").detail(format!("{cells} cell(s)"))
                }
                .with_duration(start.elapsed()),
            );
            finalize(&mut result, start);
            Ok(result)
        }
        Command::AuditWasm {
            site_path,
            breakdown,
            wasm,
        } => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("audit-wasm");
            let step_start = std::time::Instant::now();
            if breakdown {
                match audit_wasm::breakdown(wasm.as_deref()) {
                    Ok(report) => {
                        let n = report.crates.len();
                        result.breakdown = Some(report);
                        result.push(
                            StepResult::ok("audit-wasm-breakdown")
                                .detail(format!("{n} crate(s) attributed"))
                                .with_duration(step_start.elapsed()),
                        );
                    }
                    Err(e) => {
                        result.push(
                            StepResult::fail("audit-wasm-breakdown")
                                .detail(format!("{e:#}"))
                                .with_duration(step_start.elapsed()),
                        );
                    }
                }
            } else {
                match audit_wasm::run(site_path.as_deref()) {
                    Ok(report) => {
                        let n = report.artifacts.len();
                        result.audit = Some(report);
                        result.push(
                            StepResult::ok("audit-wasm")
                                .detail(format!("{n} artifact(s)"))
                                .with_duration(step_start.elapsed()),
                        );
                    }
                    Err(e) => {
                        result.push(
                            StepResult::fail("audit-wasm")
                                .detail(format!("{e:#}"))
                                .with_duration(step_start.elapsed()),
                        );
                    }
                }
            }
            finalize(&mut result, start);
            Ok(result)
        }
        Command::E2e { backend, browser } => {
            let start = std::time::Instant::now();
            let label = format!("e2e-{}-{}", backend.as_str(), browser.as_str());
            let mut result = CommandResult::new(&label);
            steps::nix::e2e_combo(&mut result, backend.as_str(), browser.as_str());
            // Surface retried-but-passed tests from the report `e2e_combo` just
            // lifted out of the VM (see steps::flaky). Informational — never fails
            // the combo.
            steps::flaky::collect(&mut result, backend.as_str(), browser.as_str());
            // #681: the e2e half of the flow-coverage gate. Only this per-combo path
            // has an uncollided capture (spec D8), and only the authoritative combo's
            // traces are used (D6) — `verify_after_combo` enforces both.
            steps::server_fn_coverage_check::verify_after_combo(
                &mut result,
                backend.as_str(),
                browser.as_str(),
            );
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Coverage(CoverageCommand::ProbeSource) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("coverage-probe-source");
            let step_start = std::time::Instant::now();
            result.push(coverage::probe::probe_source().with_duration(step_start.elapsed()));
            finalize(&mut result, start);
            Ok(result)
        }
        Command::E2eLocal {
            test,
            update_visual_snapshots,
        } => {
            let sh = Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("e2e-local");
            steps::e2e_local::run(&sh, &mut result, test.as_deref(), update_visual_snapshots);
            finalize(&mut result, start);
            Ok(result)
        }
        Command::TestLocal { nextest_args } => {
            let sh = Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("test-local");
            steps::test_local::run(&sh, &mut result, &nextest_args);
            finalize(&mut result, start);
            Ok(result)
        }
        Command::BuildCsr { release } => {
            let sh = Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("build-csr");
            steps::build_csr::run(&sh, &mut result, release);
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Adr(AdrCommand::SyncReadme) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("adr-sync-readme");
            let step_start = std::time::Instant::now();
            result.push(adr_readme::sync_readme().with_duration(step_start.elapsed()));
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Adr(AdrCommand::Promote) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("adr-promote");
            let step_start = std::time::Instant::now();
            result.push(adr::promote().with_duration(step_start.elapsed()));
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Adr(AdrCommand::Promoter) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("adr-promoter");
            let step_start = std::time::Instant::now();
            result.push(pr::promoter::execute().with_duration(step_start.elapsed()));
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Traces(TracesCommand::Analyze {
            top,
            trace,
            project,
            playwright_report,
            files,
        }) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("traces-analyze");
            let filters = traces::parse::Filters { trace, project };
            let reported = traces::report::ReportedDurations::from_paths(&playwright_report)?;
            let Some(analysis) = trace_attribute_owner_result(
                &mut result,
                "traces-analyze",
                traces::analyze::analyze(&files, filters, &reported),
            )?
            else {
                finalize(&mut result, start);
                return Ok(result);
            };
            let n = analysis.span_count;
            result.traces = Some(traces::render::render(&analysis, top as usize));
            result.push(
                StepResult::ok("traces-analyze")
                    .detail(format!("{n} span(s)"))
                    .with_duration(start.elapsed()),
            );
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Traces(TracesCommand::Run {
            top,
            trace,
            single_worker,
            browser,
        }) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("traces-run");
            // `_tmp` guards extracted traces until analysis ends. Collection and
            // Nix failures retain the trace command's top-level error contract.
            let (_tmp, files, reports) = traces::run::collect_trace_files(single_worker, browser)?;
            let n = files.len();
            let filters = traces::parse::Filters {
                trace,
                project: None,
            };
            // Paired per combo — see `ReportedDurations`: sqlite and postgres
            // share test+project+retry keys, so an unpaired merge would let one
            // backend's durations overwrite the other's.
            let reported = traces::report::ReportedDurations::from_labeled(&reports)?;
            let Some(analysis) = trace_attribute_owner_result(
                &mut result,
                "traces-run",
                traces::analyze::analyze(&files, filters, &reported),
            )?
            else {
                finalize(&mut result, start);
                return Ok(result);
            };
            result.traces = Some(traces::render::render(&analysis, top as usize));
            result.push(
                StepResult::ok("traces-run")
                    .detail(format!("{n} trace file(s)"))
                    .with_duration(start.elapsed()),
            );
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Traces(TracesCommand::BootPhases { files }) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("traces-boot-phases");
            let Some(rows) = trace_attribute_owner_result(
                &mut result,
                "traces-boot-phases",
                traces::boot_phases::boot_phases(&files),
            )?
            else {
                finalize(&mut result, start);
                return Ok(result);
            };
            let n = rows.len();
            result.traces = Some(traces::boot_phases::render(&rows));
            result.push(
                StepResult::ok("traces-boot-phases")
                    .detail(format!("{n} population(s)"))
                    .with_duration(start.elapsed()),
            );
            finalize(&mut result, start);
            Ok(result)
        }
        Command::ServerFnCoverage(sub) => {
            let start = std::time::Instant::now();
            use steps::server_fn_coverage_check::{REGENERATE_STEP, VERIFY_STEP};
            let regenerate = matches!(sub, ServerFnCoverageCommand::Regenerate);
            let mut result = CommandResult::new(if regenerate {
                REGENERATE_STEP
            } else {
                VERIFY_STEP
            });
            // A missing/empty/unparseable capture propagates as Err → the exit-2
            // path, never a green run: treating a broken capture as "nothing
            // uncovered" would make the whole gate dishonest.
            let step = steps::server_fn_coverage_check::from_capture(
                Path::new(server_fn_coverage::io::CAPTURE_PATH),
                regenerate,
            )?;
            result.push(step);
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Issue(sub) => issue::execute(sub),
        Command::Pr(PrCommand::Cleanup { number }) => {
            let start = std::time::Instant::now();
            let mut result = pr::cleanup::execute(number);
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Pr(sub) => {
            let start = std::time::Instant::now();
            let (operation, number, cfg) = match sub {
                PrCommand::Watch {
                    number,
                    interval,
                    timeout,
                    once,
                    until,
                } => (
                    pr::PrOperation::Watch,
                    number,
                    pr::watch::WatchConfig {
                        interval_secs: interval,
                        timeout_mins: timeout,
                        once,
                        stop_at_ready: until.is_none(),
                        ..Default::default()
                    },
                ),
                PrCommand::Land {
                    number,
                    interval,
                    timeout,
                } => (
                    pr::PrOperation::Land,
                    number,
                    pr::watch::WatchConfig {
                        interval_secs: interval,
                        timeout_mins: timeout,
                        once: false,
                        stop_at_ready: false,
                        ..Default::default()
                    },
                ),
                PrCommand::Cleanup { .. } => unreachable!("cleanup dispatches separately"),
            };
            // An `Err` here means the subject could not be established at all (exit
            // 2, no report). Every other failure — including `gh` being broken — is
            // a `watcher-error` report, so the outcome that most needs to be legible
            // always reaches the sidecar.
            let report = pr::execute(number, cfg, operation.is_landing())?;
            let mut result = pr::into_result(operation, report, start.elapsed());
            finalize(&mut result, start);
            Ok(result)
        }
    }
}

/// Self-healing hook installation: point `core.hooksPath` at `.githooks` if it is not
/// already, so fresh clones and new worktrees wire up on first run. Best-effort — a
/// failure here must never block the actual command.
pub fn ensure_hooks_installed() {
    match git::ensure_hooks_path(Path::new(".")) {
        Ok(true) => eprintln!("xtask: set core.hooksPath = {}", git::HOOKS_PATH),
        Ok(false) => {}
        Err(e) => eprintln!("xtask: warning: could not set core.hooksPath: {e:#}"),
    }
}

fn trace_attribute_owner_result<T>(
    result: &mut CommandResult,
    step: &'static str,
    value: anyhow::Result<T>,
) -> anyhow::Result<Option<T>> {
    match value {
        Ok(value) => Ok(Some(value)),
        Err(error)
            if error
                .downcast_ref::<traces::parse::MalformedJsonAttr>()
                .is_some() =>
        {
            result.push(
                StepResult::fail(step)
                    .detail(format!("{error:#}"))
                    .with_duration(std::time::Duration::ZERO),
            );
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn finalize(result: &mut CommandResult, start: std::time::Instant) {
    result.duration_ms = start.elapsed().as_millis();
    result.finished_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
}

/// The clean-tree precheck step for `validate`. With `--allow-dirty`, a skip.
/// Otherwise: `ok` when the tree is clean; `fail` when dirty (detail = the porcelain
/// status) or when git cannot be queried — the gate refuses to certify a tree it
/// cannot prove clean. `check` deliberately has no such precheck (Fix-mode runs on a
/// dirty tree by design).
fn clean_tree_precheck(allow_dirty: bool) -> StepResult {
    if allow_dirty {
        return StepResult::skip("clean-tree").detail("--allow-dirty");
    }
    match git::working_tree_status(Path::new(".")) {
        Ok(status) if git::porcelain_is_dirty(&status) => {
            StepResult::fail("clean-tree").detail(format!(
                "working tree is dirty — commit/stash or pass --allow-dirty:\n{}",
                status.trim()
            ))
        }
        Ok(_) => StepResult::ok("clean-tree"),
        Err(e) => {
            StepResult::fail("clean-tree").detail(format!("could not determine cleanliness: {e:#}"))
        }
    }
}

#[cfg(test)]
mod cli_tests {

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
    fn prepush_phase_plan_is_ordered_unique_and_nix_free() {
        let phases: Vec<_> = PREPUSH_PHASES.iter().map(|phase| phase.name()).collect();

        assert_eq!(
            phases,
            ["host-gate", "test-local", "workspace-doctests"],
            "the production-used prepush plan is the complete local gate"
        );
        assert_eq!(
            phases.len(),
            phases
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            "each prepush phase runs exactly once"
        );
        assert!(!phases.iter().any(|name| name.contains("nix")));

        let host_steps = host_gate_step_names_for_test(Mode::Check);
        assert_eq!(
            host_steps
                .iter()
                .filter(|name| **name == "host-tests")
                .count(),
            1
        );
        assert!(
            !host_steps.iter().any(|name| name.contains("nix")),
            "the host phase must remain Nix-free"
        );
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
    fn precommit_host_surface_is_check_no_test_surface() {
        let check = host_gate_step_names_for_test(Mode::Fix);
        let precommit = precommit_host_step_names_for_test();
        assert_eq!(precommit, check);
        assert!(!precommit.contains(&"nix-wasm-tests"));
        assert!(!precommit.contains(&"nix-coverage"));
        assert!(!precommit.contains(&"nix-doctests"));
        assert!(!precommit.contains(&"proffered-filename"));
    }

    #[test]
    fn staged_markdown_route_filters_the_production_host_catalog_in_order() {
        assert_eq!(
            markdown_precommit_step_names_for_test(),
            [
                "prettier",
                "sequence-check",
                "adr-filenames",
                "doc-links",
                "flow-docs",
                "error-swallowing-inventory",
            ]
        );
    }

    fn position(names: &[&str], name: &str) -> usize {
        names
            .iter()
            .position(|candidate| *candidate == name)
            .unwrap_or_else(|| panic!("{name} is present"))
    }

    #[test]
    fn host_gate_order_prioritizes_cheap_actionable_feedback() {
        let names = host_gate_step_names_for_test(Mode::Check);
        let write_transaction_contract = position(&names, "write-transaction-contract");
        let target_closure = position(&names, "common-host-target-closure");

        let fmt = position(&names, "fmt");
        let flow_docs = position(&names, "flow-docs");
        let clippy = position(&names, "clippy");
        let host_tests = position(&names, "host-tests");

        assert!(
            fmt < flow_docs,
            "source consistency runs before repo-shape checks"
        );
        assert!(
            target_closure < clippy,
            "target-resolved repository-shape checks run before compile checks"
        );
        assert!(
            write_transaction_contract < clippy,
            "write-transaction contract runs before compile checks"
        );
        assert!(
            flow_docs < clippy,
            "repo-shape checks run before expensive compile checks"
        );
        assert!(
            clippy < host_tests,
            "compile checks run before host runtime tests"
        );
    }

    #[test]
    fn prepush_clean_tree_short_circuits_the_local_gate() {
        let sh = Shell::new().expect("create shell");
        let mut result = CommandResult::new("prepush");
        let invoked = std::cell::Cell::new(false);

        run_prepush_with(
            &sh,
            Command::Prepush.execution_policy(),
            &mut result,
            || StepResult::fail("clean-tree").detail("dirty"),
            |_, _, _| invoked.set(true),
        );

        assert!(
            !invoked.get(),
            "the local gate must not run after a failed precheck"
        );
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>(),
            ["clean-tree"]
        );
    }

    #[test]
    fn fail_fast_detects_a_failure_when_one_phase_appends_multiple_results() {
        let mut result = CommandResult::new("prepush");
        let mut invoked = Vec::new();

        run_with_policy(
            ExecutionPolicy::FailFast,
            &mut result,
            PREPUSH_PHASES.iter().copied(),
            |phase, result| {
                invoked.push(phase);
                if matches!(phase, PrepushPhase::HostGate) {
                    result.push(StepResult::ok("host-gate-started"));
                    result.push(StepResult::fail("host-gate-failed").detail("preserved failure"));
                } else {
                    result.push(StepResult::ok(phase.name()));
                }
            },
        );

        assert_eq!(invoked, [PrepushPhase::HostGate]);
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>(),
            ["host-gate-started", "host-gate-failed"]
        );
        assert_eq!(result.steps[1].detail.as_deref(), Some("preserved failure"));
    }

    #[test]
    fn prepush_stops_before_product_and_doctest_phases_after_host_failure() {
        let mut result = CommandResult::new("prepush");
        let mut invoked = Vec::new();

        run_with_policy(
            Command::Prepush.execution_policy(),
            &mut result,
            PREPUSH_PHASES.iter().copied(),
            |phase, result| {
                invoked.push(phase);
                match phase {
                    PrepushPhase::HostGate => {
                        result.push(StepResult::fail("synthetic-host").detail("host failed"))
                    }
                    PrepushPhase::LocalTests => result.push(StepResult::ok("synthetic-product")),
                    PrepushPhase::WorkspaceDoctests => {
                        result.push(StepResult::ok("synthetic-doctests"))
                    }
                }
            },
        );

        assert_eq!(invoked, [PrepushPhase::HostGate]);
        assert_eq!(result.steps[0].detail.as_deref(), Some("host failed"));
    }

    #[test]
    fn prepush_stops_before_workspace_doctests_after_product_failure() {
        let mut result = CommandResult::new("prepush");
        let mut invoked = Vec::new();

        run_with_policy(
            Command::Prepush.execution_policy(),
            &mut result,
            PREPUSH_PHASES.iter().copied(),
            |phase, result| {
                invoked.push(phase);
                if matches!(phase, PrepushPhase::LocalTests) {
                    result.push(StepResult::fail("synthetic-product").detail("product failed"));
                } else {
                    result.push(StepResult::ok("synthetic-phase"));
                }
            },
        );

        assert_eq!(invoked, [PrepushPhase::HostGate, PrepushPhase::LocalTests]);
        assert_eq!(result.steps[1].detail.as_deref(), Some("product failed"));
    }

    #[test]
    fn precommit_reconciles_once_after_an_early_static_failure() {
        let dir = crate::test_support::temp_repo("precommit", "early-failure-reconcile");
        crate::test_support::commit(&dir, "a.rs", "fn a(){}\n");
        crate::test_support::write(&dir, "a.rs", "fn a() { }\n");
        crate::test_support::git_ok(&dir, &["add", "a.rs"]);
        let sh = xshell::Shell::new().expect("create shell");
        let first_static_spec = steps::static_checks::specs_for_phase(
            steps::static_checks::Phase::SourceConsistency,
            Mode::Fix,
        )
        .first()
        .expect("source-consistency phase has a spec")
        .name;
        let mut invoked = Vec::new();

        let result = run_precommit_with_host_gate(&dir, |_, result| {
            run_host_steps_with(
                &sh,
                Mode::Fix,
                Command::Precommit.execution_policy(),
                result,
                HOST_GATE_NON_TEST_STEPS,
                |step, sh, mode, policy, result| match step {
                    HostGateStep::StaticChecks(phase) => {
                        steps::static_checks::run_phase_with(
                            sh,
                            mode,
                            *phase,
                            policy,
                            result,
                            |_, spec| {
                                invoked.push(spec.name);
                                crate::test_support::write(
                                    &dir,
                                    "a.rs",
                                    "fn a() { }\n// formatted\n",
                                );
                                StepResult::fail(spec.name)
                                    .detail("static diagnostic survives reconciliation")
                            },
                        );
                    }
                    HostGateStep::ResultOnly { name, .. } => {
                        panic!("later host step {name} ran after static failure")
                    }
                    HostGateStep::HostTests { .. } => {
                        panic!("host tests ran after static failure")
                    }
                },
            );
        })
        .expect("precommit orchestration");

        assert_eq!(invoked, [first_static_spec]);
        assert_eq!(
            result
                .steps
                .iter()
                .filter(|step| step.name == "precommit-staging")
                .count(),
            1
        );
        assert_eq!(
            result
                .steps
                .iter()
                .find(|step| step.name == first_static_spec)
                .and_then(|step| step.detail.as_deref()),
            Some("static diagnostic survives reconciliation")
        );
        assert_eq!(
            git::output(&dir, &["diff", "--cached", "--name-only"]).expect("cached paths"),
            "a.rs"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_execution_policies_keep_explicit_gates_exhaustive_and_hooks_fail_fast() {
        assert_eq!(
            Command::Check { no_test: true }.execution_policy(),
            ExecutionPolicy::Exhaustive
        );
        assert_eq!(
            Command::Validate {
                no_e2e: true,
                allow_dirty: false,
            }
            .execution_policy(),
            ExecutionPolicy::Exhaustive
        );
        assert_eq!(
            Command::Precommit.execution_policy(),
            ExecutionPolicy::FailFast
        );
        assert_eq!(
            Command::Prepush.execution_policy(),
            ExecutionPolicy::FailFast
        );
    }

    #[test]
    fn explicit_host_gates_keep_running_production_steps_after_a_static_failure() {
        fn assert_exhaustive(command: &str, policy: ExecutionPolicy, include_host_tests: bool) {
            let sh = xshell::Shell::new().expect("create shell");
            let mut result = CommandResult::new(command);
            let expected = if include_host_tests {
                host_gate_step_names_for_test(Mode::Check)
            } else {
                host_gate_without_tests_step_names_for_test(Mode::Check)
            };
            let mut invoked = Vec::new();
            let mut failed = false;
            let mut run_step =
                |step: &HostGateStep,
                 sh: &xshell::Shell,
                 mode: Mode,
                 policy: ExecutionPolicy,
                 result: &mut CommandResult| match step {
                    HostGateStep::StaticChecks(phase) => {
                        steps::static_checks::run_phase_with(
                            sh,
                            mode,
                            *phase,
                            policy,
                            result,
                            |_, spec| {
                                invoked.push(spec.name);
                                if std::mem::replace(&mut failed, true) {
                                    StepResult::ok(spec.name)
                                } else {
                                    StepResult::fail(spec.name).detail("synthetic static failure")
                                }
                            },
                        );
                    }
                    HostGateStep::ResultOnly { name, .. } => {
                        invoked.push(name);
                        result.push(StepResult::ok(name));
                    }
                    HostGateStep::HostTests { .. } => {
                        invoked.push("host-tests");
                        result.push(StepResult::ok("host-tests"));
                    }
                };

            if include_host_tests {
                run_host_steps_with(
                    &sh,
                    Mode::Check,
                    policy,
                    &mut result,
                    HOST_GATE_NON_TEST_STEPS
                        .iter()
                        .chain(std::iter::once(&HOST_TESTS_STEP)),
                    &mut run_step,
                );
            } else {
                run_host_steps_with(
                    &sh,
                    Mode::Check,
                    policy,
                    &mut result,
                    HOST_GATE_NON_TEST_STEPS,
                    &mut run_step,
                );
            }

            assert_eq!(
                invoked, expected,
                "{command} must keep every later production host step after a static failure"
            );
            assert_eq!(result.steps.len(), expected.len());
            assert!(!result.ok);
        }

        assert_exhaustive(
            "check",
            Command::Check { no_test: true }.execution_policy(),
            true,
        );
        assert_exhaustive(
            "validate",
            Command::Validate {
                no_e2e: true,
                allow_dirty: false,
            }
            .execution_policy(),
            false,
        );
    }

    #[test]
    fn validate_host_surface_keeps_wasm_budget_before_host_tests() {
        let mut validate = host_gate_without_tests_step_names_for_test(Mode::Check);
        validate.push("wasm-budget");
        HOST_TESTS_STEP.push_names(Mode::Check, &mut validate);

        let wasm_budget = validate
            .iter()
            .position(|name| *name == "wasm-budget")
            .expect("validate includes wasm-budget");
        let host_tests = validate
            .iter()
            .position(|name| *name == "host-tests")
            .expect("validate includes host-tests");
        assert!(wasm_budget < host_tests);
    }

    #[test]
    fn check_uses_local_product_tests_before_nix_only_test_checks() {
        let mut names = host_gate_step_names_for_test(Mode::Fix);
        names.push("test-local");
        names.extend(steps::nix::check_supporting_test_check_names());

        let test_local = names
            .iter()
            .position(|name| *name == "test-local")
            .expect("check includes the host-native product test lane");
        let wasm_tests = names
            .iter()
            .position(|name| *name == "wasm-tests")
            .expect("check keeps the Nix-only wasm browser tests");

        assert!(test_local < wasm_tests);
        assert!(!names.contains(&"coverage"));
        assert!(names.contains(&"doctests"));
    }

    #[test]
    fn ci_shape_keeps_the_elisp_verdict_in_the_static_lane() {
        let workflow = include_str!("../../.github/workflows/ci.yml");

        assert!(workflow.contains("validate-no-e2e:"));
        assert!(workflow.contains("cargo xtask validate\n          --no-e2e"));
        assert!(workflow.contains(".xtask/gcroots/elisp-coverage-producer/elisp-coverage/"));
        assert!(workflow.contains("needs: [e2e]"));
        assert!(!workflow.contains("elisp-integration"));
    }

    #[test]
    fn precommit_orchestration_restages_safe_fixture() {
        let dir = crate::test_support::temp_repo("precommit", "orchestration-safe");
        crate::test_support::commit(&dir, "a.rs", "fn a(){}\n");
        crate::test_support::commit(&dir, "b.rs", "fn b(){}\n");
        crate::test_support::write(&dir, "a.rs", "fn a() { }\n");
        crate::test_support::git_ok(&dir, &["add", "a.rs"]);

        crate::test_support::write(&dir, "b.rs", "fn b() { }\n");
        let result = run_precommit_with_host_gate(&dir, |class, result| {
            dispatch_precommit_host_gate_with(
                class,
                result,
                |_| panic!("non-Markdown or mixed state must not dispatch to the narrow graph"),
                |result| {
                    crate::test_support::write(&dir, "a.rs", "fn a() { }\n// formatted\n");
                    result.push(StepResult::ok("fake-complete-fix-host-gate"));
                },
            );
        })
        .unwrap();

        assert!(result.ok);
        assert_eq!(result.steps[0].name, "precommit-routing");
        assert_eq!(
            result.steps[0].detail.as_deref(),
            Some("class=broad reason=unstaged-path")
        );
        assert_eq!(
            git::output(&dir, &["diff", "--cached", "--name-only"]).unwrap(),
            "a.rs"
        );
        assert_eq!(git::output(&dir, &["diff", "--name-only"]).unwrap(), "b.rs");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn precommit_routes_staged_markdown_before_restaging_safe_formatter_output() {
        let dir = crate::test_support::temp_repo("precommit", "markdown-routing");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);

        let result = run_precommit_with_host_gate(&dir, |class, result| {
            dispatch_precommit_host_gate_with(
                class,
                result,
                |result| {
                    crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\nFormatted\n");
                    result.push(StepResult::ok("fake-markdown-host-gate"));
                },
                |_| panic!("staged Markdown must not dispatch to the broad host graph"),
            );
        })
        .expect("precommit orchestration");

        assert_eq!(result.steps[0].name, "precommit-routing");
        assert_eq!(
            result.steps[0].detail.as_deref(),
            Some("class=staged-markdown-only reason=isolated-staged-markdown")
        );
        assert!(result.ok);
        assert_eq!(
            git::output(&dir, &["diff", "--cached", "--name-only"]).expect("cached paths"),
            "guide.md"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn narrow_dispatch_failure_still_reconciles_exactly_once() {
        let dir = crate::test_support::temp_repo("precommit", "markdown-failure");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);

        let result = run_precommit_with_host_gate(&dir, |class, result| {
            dispatch_precommit_host_gate_with(
                class,
                result,
                |result| result.push(StepResult::fail("prettier").detail("synthetic failure")),
                |_| panic!("staged Markdown must not dispatch to the broad host graph"),
            );
        })
        .expect("precommit orchestration");

        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>(),
            ["precommit-routing", "prettier", "precommit-staging"]
        );
        assert_eq!(
            result
                .steps
                .iter()
                .filter(|step| step.name == "precommit-staging")
                .count(),
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn precommit_orchestration_fails_clean_before_tracked_mutation() {
        let dir = crate::test_support::temp_repo("precommit", "orchestration-unsafe");
        crate::test_support::commit(&dir, "clean.rs", "one\n");

        let result = run_precommit_with_host_gate(&dir, |_, result| {
            crate::test_support::write(&dir, "clean.rs", "two\n");
            result.push(StepResult::ok("fake-host-gate"));
        })
        .unwrap();

        assert!(!result.ok);
        let staging = result
            .steps
            .iter()
            .find(|s| s.name == "precommit-staging")
            .unwrap();
        assert!(
            staging
                .detail
                .as_deref()
                .unwrap()
                .contains("will not add work the user did not stage")
        );
        assert!(
            git::output(&dir, &["diff", "--cached", "--name-only"])
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
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
    fn check_and_validate_include_flow_docs() {
        let with_tests = host_gate_step_names_for_test(Mode::Fix);
        let without_tests = host_gate_without_tests_step_names_for_test(Mode::Check);

        assert_eq!(
            with_tests
                .iter()
                .filter(|name| **name == "flow-docs")
                .count(),
            1,
            "check host gate must include flow-docs exactly once"
        );
        assert_eq!(
            without_tests
                .iter()
                .filter(|name| **name == "flow-docs")
                .count(),
            1,
            "validate host gate must include flow-docs exactly once before host-tests"
        );

        let doc_links = without_tests
            .iter()
            .position(|name| *name == "doc-links")
            .expect("doc-links is in host gate");
        let flow_docs = without_tests
            .iter()
            .position(|name| *name == "flow-docs")
            .expect("flow-docs is in host gate");
        assert_eq!(
            flow_docs,
            doc_links + 1,
            "flow-docs must run immediately after doc-links"
        );
        assert!(
            !without_tests.contains(&"host-tests"),
            "validate's early host gate excludes host tests"
        );
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
    fn run_rejects_json_for_traces_analyze() {
        let cli = Cli {
            json: true,
            command: Command::Traces(TracesCommand::Analyze {
                top: 25,
                trace: None,
                project: None,
                playwright_report: Vec::new(),
                files: vec![PathBuf::from("x.jsonl")],
            }),
        };
        let err = match run(cli) {
            Ok(_) => panic!("expected --json to be rejected for traces analyze"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("--json"),
            "error explains the --json rejection: {err}"
        );
    }

    #[test]
    fn run_errors_on_missing_trace_file() {
        let path = "/no/such/trace.jsonl";
        let cli = Cli {
            json: false,
            command: Command::Traces(TracesCommand::Analyze {
                top: 25,
                trace: None,
                project: None,
                playwright_report: Vec::new(),
                files: vec![PathBuf::from(path)],
            }),
        };
        let error = match run(cli) {
            Ok(_) => panic!("missing trace file must remain a top-level error"),
            Err(error) => error,
        };
        assert!(format!("{error:#}").contains(path));
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

    #[test]
    fn run_rejects_json_for_traces_run() {
        let cli = Cli {
            json: true,
            command: Command::Traces(TracesCommand::Run {
                top: 25,
                trace: None,
                single_worker: false,
                browser: None,
            }),
        };
        let err = match run(cli) {
            Ok(_) => panic!("expected --json to be rejected for traces run"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("--json"),
            "error explains the --json rejection: {err}"
        );
    }

    #[test]
    fn trace_json_attr_owner_returns_serializable_failed_command_result() {
        let span = serde_json::json!({
            "attributes": [{
                "key": "e2e.x",
                "value": { "stringValue": "{not json" }
            }]
        });
        let error = traces::parse::parse_json_attr(&span, "e2e.x", "source.jsonl").unwrap_err();
        let mut result = CommandResult::new("traces-analyze");
        let value: Option<()> =
            trace_attribute_owner_result(&mut result, "traces-analyze", Err(error)).unwrap();
        assert!(value.is_none());
        assert!(!result.ok);
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("source.jsonl"), "{json}");
        assert!(json.contains("e2e.x"), "{json}");
        assert!(json.contains("traces-analyze"), "{json}");
    }

    #[test]
    fn trace_non_attribute_failures_remain_top_level_errors() {
        let mut result = CommandResult::new("traces-run");
        let error = trace_attribute_owner_result::<()>(
            &mut result,
            "traces-run",
            Err(anyhow::anyhow!("nix build failed")),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("nix build failed"));
        assert!(result.steps.is_empty());
    }
}

#[cfg(test)]
mod git_env_tests {
    use crate::git;

    #[test]
    fn git_at_scrubs_repo_redirecting_env() {
        // Regression guard: without scrubbing these, a git op meant for `dir`
        // (a throwaway test repo) would be redirected at the hook's repo when run
        // inside a git hook, corrupting it. `get_envs()` yields `(key, None)` for a
        // removed var.
        let cmd = git::at(std::path::Path::new("/tmp/x"));
        let removed: std::collections::HashSet<std::ffi::OsString> = cmd
            .get_envs()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.to_owned())
            .collect();
        for var in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_COMMON_DIR",
            "GIT_NAMESPACE",
        ] {
            assert!(
                removed.contains(std::ffi::OsStr::new(var)),
                "{var} must be scrubbed so -C wins"
            );
        }
    }
}
