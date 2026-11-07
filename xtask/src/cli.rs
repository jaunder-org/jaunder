use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

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
pub enum PerformanceBackend {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum PerformanceBrowser {
    Chromium,
    Firefox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum PerformanceProfile {
    Small,
    Medium,
    Large,
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

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum SandboxProfile {
    Empty,
    Standard,
    Demo,
}

impl SandboxProfile {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Standard => "standard",
            Self::Demo => "demo",
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
    /// Run Just's selected precommit host recipe inside the Git/index safety boundary.
    #[command(hide = true)]
    PrecommitSafety,
    /// Produce a host-side repository census for manual maintenance audits. The
    /// report is informational: candidates are neither findings nor gate failures.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask census\n  \
        cargo xtask census --json")]
    Census,
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
    /// env + a per-run temp DB, seed via the shared `devtool seed-e2e`, run
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
    /// ADR maintenance.
    #[command(subcommand)]
    Adr(AdrCommand),
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
    /// Observe a pull request until the next caller-actionable outcome (#729).
    /// Host-only manual command; needs `gh`.
    #[command(subcommand)]
    Pr(PrCommand),
    /// Run fresh release-mode performance producers, compare a retained run, or
    /// import the verified canonical GitHub Actions baseline.
    #[command(name = "perf", subcommand)]
    Performance(PerformanceCommand),
    /// Internal Just adapter: no inputs means mutable smoke; harness alone
    /// selects local styling; product inputs select discovery or acceptance.
    #[command(hide = true)]
    QualificationLeaf {
        #[arg(long)]
        expected_harness: Option<String>,
        #[arg(long, requires = "expected_harness")]
        source: Option<String>,
        #[arg(long, requires = "source")]
        target: Option<String>,
    },
    /// Start an interactive disposable or named UX sandbox, or run an admitted
    /// operational Jaunder command against a named sandbox after `--`.
    Sandbox {
        /// Persistent workspace name. Omit for a disposable sandbox.
        name: Option<String>,
        #[arg(long, value_enum)]
        profile: Option<SandboxProfile>,
        #[arg(long)]
        reset: bool,
        /// A Jaunder command run against an existing named workspace.
        #[arg(last = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
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

/// Host-side performance operations. A run is canonical only when every
/// selector uses its default matrix and provenance comes from the trusted CI env.
#[derive(Subcommand)]
pub enum PerformanceCommand {
    Small(PerformanceRunArgs),
    Medium(PerformanceRunArgs),
    Large(PerformanceRunArgs),
    Compare {
        #[arg(long)]
        result: PathBuf,
    },
    ImportBaseline {
        #[arg(value_parser = clap::value_parser!(u64).range(1..))]
        run_id: u64,
    },
}

#[derive(clap::Args)]
pub struct PerformanceRunArgs {
    #[arg(long, value_enum)]
    pub backend: Vec<PerformanceBackend>,
    #[arg(long, value_enum)]
    pub browser: Vec<PerformanceBrowser>,
    #[arg(long, conflicts_with = "browser_only")]
    pub storage_only: bool,
    #[arg(long, conflicts_with = "storage_only")]
    pub browser_only: bool,
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub posts: Option<u64>,
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub authors: Option<u64>,
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub revisions: Option<u64>,
}

/// An explicit passive-observation target for `pr watch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PrWatchUntil {
    Merged,
}

/// `pr` subcommands (#729).
///
/// The split is the point: `watch` cannot merge anything, so running `land` **is**
/// the per-PR human merge approval. Routine technical diagnosis belongs to the
/// assigned Lead; recovery after ejection requires explicit authorization from
/// the authority assigned by owner policy. `land --reenqueue` remains explicit.
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
    /// Running this command is the per-PR human merge approval. Recovery after an
    /// ejection requires separate explicit authorization from the authority assigned
    /// by owner policy; diagnosis alone does not authorize it. It refuses when invoked
    /// from the PR's own branch with unpushed local commits. An ejected PR is refused
    /// unless --reenqueue explicitly permits one new enqueue; it never retries that arm.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask pr land\n  \
        cargo xtask pr land 731\n  \
        cargo xtask pr land 731 --reenqueue")]
    Land {
        /// Permit one enqueue after an observed ejection. Refuses queued or armed PRs.
        #[arg(long)]
        reenqueue: bool,
        /// PR number. Omitted: the open PR whose head is the current branch.
        number: Option<u64>,
        /// Seconds between polls.
        #[arg(long, value_name = "SECONDS", default_value_t = 30, value_parser = clap::value_parser!(u64).range(5..))]
        interval: u64,
        /// Minutes to watch before reporting `timed-out`.
        #[arg(long, value_name = "MINUTES", default_value_t = 90, value_parser = clap::value_parser!(u64).range(1..))]
        timeout: u64,
    },
    /// Clean an unmanaged checkout after a PR has merged.
    ///
    /// Manager-assigned work: leave the semantic branch checked out and the checkout
    /// intact; skip this command and equivalent detach/delete operations while the
    /// assignment is active. Lead accepts merge, validation, and preservation evidence
    /// for Manager handoff; Manager retains required branch selectors and owns
    /// authorized retirement through supported runtime routes.
    ///
    /// Unmanaged cleanup is local-only: it proves the exact merged head, main target,
    /// clean tree, and ancestry before detaching, safely deleting the branch, and
    /// running `cargo clean`. It does not authorize checkout removal, which separately
    /// requires an explicit removal request and interactive runtime confirmation.
    #[command(after_help = "UNMANAGED EXAMPLES:\n  \
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
            Command::PrecommitSafety => "precommit-safety",
            Command::Census => "census",
            Command::AuditWasm { .. } => "audit-wasm",
            Command::E2eVerify { .. } => "e2e-verify",
            Command::E2eReconcile { .. } => "e2e-reconcile",
            Command::E2eLocal { .. } => "e2e-local",
            Command::TestLocal { .. } => "test-local",
            Command::Adr(AdrCommand::SyncReadme) => "adr-sync-readme",
            Command::Adr(AdrCommand::Promote) => "adr-promote",
            Command::Adr(AdrCommand::Promoter) => "adr-promoter",
            Command::Performance(
                PerformanceCommand::Small(_)
                | PerformanceCommand::Medium(_)
                | PerformanceCommand::Large(_),
            ) => "perf",
            Command::Performance(PerformanceCommand::Compare { .. }) => "perf-compare",
            Command::Performance(PerformanceCommand::ImportBaseline { .. }) => {
                "perf-import-baseline"
            }
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
            Command::QualificationLeaf { .. } => "qualification-leaf",
            Command::Sandbox { .. } => "sandbox",
            Command::Mutants(MutantsCommand::VerifyEvidence { .. }) => "mutants-verify-evidence",
            Command::Pr(PrCommand::Watch { .. }) => "pr-watch",
            Command::Pr(PrCommand::Cleanup { .. }) => "pr-cleanup",
            Command::Pr(PrCommand::Land { .. }) => "pr-land",
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
            Command::QualificationLeaf { .. }
                | Command::CheckLeaf { .. }
                | Command::PrecommitSafety
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
    fn just_leaf_and_precommit_safety_commands_are_hidden_process_boundaries() {
        let leaf = Cli::try_parse_from(["xtask", "check-leaf", "doc-links"]).unwrap();
        assert_eq!(leaf.command_name(), "check-leaf");
        assert!(!leaf.command.produces_json_payload());
        assert!(matches!(
            leaf.command,
            Command::CheckLeaf { name } if name == "doc-links"
        ));

        let safety = Cli::try_parse_from(["xtask", "precommit-safety"]).unwrap();
        assert_eq!(safety.command_name(), "precommit-safety");
        assert!(!safety.command.produces_json_payload());
        assert!(matches!(safety.command, Command::PrecommitSafety));
    }

    #[test]
    fn qualification_leaf_requires_explicit_harness_for_product_inputs() {
        assert!(Cli::try_parse_from(["xtask", "qualification-leaf", "--source", "HEAD"]).is_err());
        assert!(Cli::try_parse_from(["xtask", "qualification-leaf", "--target", "HEAD"]).is_err());
        assert!(Cli::try_parse_from(["xtask", "qualification-leaf"]).is_ok());
    }

    #[test]
    fn census_parses_with_json_and_has_a_stable_command_name() {
        let cli = Cli::try_parse_from(["xtask", "census", "--json"]).unwrap();
        assert!(cli.json);
        assert_eq!(cli.command_name(), "census");
        assert!(matches!(cli.command, Command::Census));
    }

    #[test]
    fn perf_profile_and_selectors_parse_without_a_run_subcommand() {
        let cli = Cli::try_parse_from([
            "xtask",
            "perf",
            "small",
            "--backend",
            "sqlite",
            "--browser",
            "firefox",
            "--browser-only",
            "--posts",
            "120",
            "--authors",
            "12",
            "--revisions",
            "777",
        ])
        .unwrap();
        assert_eq!(cli.command_name(), "perf");
        let Command::Performance(PerformanceCommand::Small(args)) = cli.command else {
            panic!("expected small performance run");
        };
        assert_eq!(args.backend, [PerformanceBackend::Sqlite]);
        assert_eq!(args.browser, [PerformanceBrowser::Firefox]);
        assert!(args.browser_only);
        assert!(!args.storage_only);
        assert_eq!(args.posts, Some(120));
        assert_eq!(args.authors, Some(12));
        assert_eq!(args.revisions, Some(777));
    }

    #[test]
    fn perf_count_overrides_reject_zero() {
        assert!(Cli::try_parse_from(["xtask", "perf", "small", "--posts", "0"]).is_err());
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
    fn pr_cleanup_help_distinguishes_assignment_ownership() {
        for args in [
            vec!["xtask", "pr", "--help"],
            vec!["xtask", "pr", "cleanup", "--help"],
        ] {
            let cleanup_help = args.len() == 4;
            let help = Cli::try_parse_from(args).err().expect("help is displayed");
            assert_eq!(help.kind(), clap::error::ErrorKind::DisplayHelp);
            let text = help.to_string();
            assert!(text.contains("unmanaged"), "{text}");
            if cleanup_help {
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                assert!(text.contains("Manager-assigned work"), "{text}");
                assert!(text.contains("semantic branch checked out"), "{text}");
                assert!(text.contains("checkout intact"), "{text}");
                assert!(text.contains("skip this command"), "{text}");
            }
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
    fn pr_land_accepts_explicit_reenqueue_only_for_land() {
        let explicit = Cli::try_parse_from(["xtask", "pr", "land", "731", "--reenqueue"]).unwrap();
        assert!(matches!(
            explicit.command,
            Command::Pr(PrCommand::Land {
                number: Some(731),
                reenqueue: true,
                ..
            })
        ));
        let default = Cli::try_parse_from(["xtask", "pr", "land", "731"]).unwrap();
        assert!(matches!(
            default.command,
            Command::Pr(PrCommand::Land {
                reenqueue: false,
                ..
            })
        ));
        assert!(Cli::try_parse_from(["xtask", "pr", "watch", "731", "--reenqueue"]).is_err());
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
    fn sandbox_parses_disposable_named_reset_and_command_modes() {
        let disposable = Cli::try_parse_from(["xtask", "sandbox"]).unwrap();
        assert!(matches!(
            disposable.command,
            Command::Sandbox {
                name: None,
                profile: None,
                reset: false,
                command,
            } if command.is_empty()
        ));

        let named = Cli::try_parse_from([
            "xtask",
            "sandbox",
            "demo",
            "--profile",
            "standard",
            "--reset",
        ])
        .unwrap();
        assert!(matches!(
            named.command,
            Command::Sandbox {
                name: Some(name),
                profile: Some(SandboxProfile::Standard),
                reset: true,
                command,
            } if name == "demo" && command.is_empty()
        ));

        let operational = Cli::try_parse_from([
            "xtask",
            "sandbox",
            "demo",
            "--",
            "site-config",
            "get",
            "site.title",
        ])
        .unwrap();
        assert!(matches!(
            operational.command,
            Command::Sandbox {
                name: Some(name),
                profile: None,
                reset: false,
                command,
            } if name == "demo"
                && command == ["site-config", "get", "site.title"]
        ));
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
