use clap::{Parser, Subcommand};

mod adr;
mod adr_readme;
mod audit_wasm;
pub mod coverage;
pub mod git;
mod ids;
mod nix_build;
mod result;
mod sh;
mod traces;
mod steps {
    pub mod adr_check;
    pub mod host_tests;
    pub mod nix;
    pub mod sequence_check;
    pub mod static_checks;
    pub mod test_pattern_check;
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
    /// xtask unit suite, then the Nix coverage check (instrumented test suite +
    /// coverage). `--no-test` skips only the Nix coverage check; static, clippy,
    /// and the xtask unit tests still run.
    Check {
        /// Skip the Nix coverage check — static + clippy + host xtask unit tests only.
        #[arg(long)]
        no_test: bool,
    },
    /// Full gate (never mutates the tree): static + clippy + the host xtask unit
    /// suite (verify-only) + the Nix coverage check + the e2e VMs. `--no-e2e` skips
    /// the e2e VMs. Refuses a dirty working tree unless `--allow-dirty`.
    Validate {
        /// Skip the e2e VM checks — static + clippy + xtask tests + coverage only.
        #[arg(long)]
        no_e2e: bool,
        /// Run even when the working tree is dirty (skip the clean-tree precheck).
        #[arg(long)]
        allow_dirty: bool,
    },
    /// Measure the frontend WASM/JS bundle size — raw, gzip, and brotli.
    ///
    /// Reports the download weight of the deterministic `nix build .#site`
    /// output (`pkg/jaunder.wasm`, `pkg/jaunder.js`) so you can catch
    /// bundle-size bloat before it ships and compare a change's effect on what
    /// users download. Run it after a change you expect to move the bundle (a new
    /// dependency, a feature touching the client), or periodically to watch the
    /// trend. This is a manual tool — it is not part of `check`/`validate`.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask audit-wasm\n  \
        cargo xtask audit-wasm --site-path /nix/store/...-jaunder-site\n  \
        cargo xtask --json audit-wasm")]
    AuditWasm {
        /// Audit a prebuilt `.#site` store path instead of running `nix build`.
        #[arg(long)]
        site_path: Option<String>,
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
    /// ADR maintenance.
    #[command(subcommand)]
    Adr(AdrCommand),
    /// OpenTelemetry trace analysis (host-side; ADR-0028).
    #[command(subcommand)]
    Traces(TracesCommand),
    /// Coverage tooling — the source-filter drift probe (#241).
    #[command(subcommand)]
    Coverage(CoverageCommand),
    /// Build the hermetic elisp live-integration VM check (ADR-0035) through the
    /// same diagnostic-preserving wrapper. For CI's parallel `elisp-integration`
    /// job; local `validate` realizes it via the `e2e` aggregate. Host only.
    ElispIntegration,
}

/// `adr` subcommands.
#[derive(Subcommand)]
pub enum AdrCommand {
    /// Renumber this branch's colliding ADR to the next free number and rewrite
    /// references. The ADR already on `origin/main` keeps its number; path-form
    /// references are rewritten repo-wide and bare `ADR-NNNN` references in
    /// branch-touched files. Run after rebasing onto the latest `origin/main`.
    #[command(after_help = "EXAMPLES:\n  cargo xtask adr renumber")]
    Renumber,
    /// Regenerate the ADR index table in `docs/README.md` from `docs/adr/`: the
    /// number, link target, and status cells. Hand-curated titles are preserved
    /// (a new row seeds its title from the ADR heading). Idempotent; touches only
    /// the marked table block. The `adr-readme-parity` gate fails on drift.
    #[command(after_help = "EXAMPLES:\n  cargo xtask adr sync-readme")]
    SyncReadme,
    /// Number the numberless drafts in `docs/adr/drafts/`: assign each the next
    /// free number, move it to `docs/adr/NNNN-<slug>.md`, rewrite its path-form
    /// references, sync the README table, and stage the result. Run at ship,
    /// after the final rebase, so the number is collision-free on first commit.
    #[command(after_help = "EXAMPLES:\n  cargo xtask adr promote")]
    Promote,
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
    /// totals). Faithful Rust port of `scripts/analyze-otel-traces`. A manual
    /// tool — not part of `check`/`validate`. Prints human tables only;
    /// `--json` is rejected.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask traces analyze /nix/store/...-e2e-sqlite-chromium/otel-traces-sqlite.jsonl/otel-traces.jsonl\n  \
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
        /// One or more `otel-traces.jsonl` files.
        #[arg(required = true)]
        files: Vec<std::path::PathBuf>,
    },
    /// Build the `{sqlite,postgres}×{chromium,firefox}` e2e VM checks and analyze
    /// their exported OTel traces in one step — the `nix build` orchestration that
    /// feeds `traces analyze`. Faithful Rust port of `scripts/run-e2e-trace-analysis`.
    /// A manual tool — not part of `check`/`validate`. Prints human tables only;
    /// `--json` is rejected.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask traces run\n  \
        cargo xtask traces run --top 40\n  \
        cargo xtask traces run --cold\n  \
        cargo xtask traces run --browser firefox")]
    Run {
        /// Rows per ranked table (default 25), forwarded to the analysis.
        #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u64).range(1..))]
        top: u64,
        /// Restrict the analysis to one trace id.
        #[arg(long)]
        trace: Option<String>,
        /// Build the cold-cache package variants instead of the warm check variants.
        #[arg(long)]
        cold: bool,
        /// Restrict to one browser (default: both). Both backends are always built.
        #[arg(long, value_enum)]
        browser: Option<E2eBrowser>,
    },
}

impl Cli {
    pub fn command_name(&self) -> &'static str {
        match self.command {
            Command::Check { .. } => "check",
            Command::Validate { .. } => "validate",
            Command::AuditWasm { .. } => "audit-wasm",
            Command::E2e { .. } => "e2e",
            Command::Adr(AdrCommand::Renumber) => "adr-renumber",
            Command::Adr(AdrCommand::SyncReadme) => "adr-sync-readme",
            Command::Adr(AdrCommand::Promote) => "adr-promote",
            Command::Traces(TracesCommand::Analyze { .. }) => "traces-analyze",
            Command::Traces(TracesCommand::Run { .. }) => "traces-run",
            Command::Coverage(CoverageCommand::ProbeSource) => "coverage-probe-source",
            Command::ElispIntegration => "elisp-integration",
        }
    }
}

impl Command {
    /// Whether `--json` yields a substantial structured payload for this command.
    /// Commands that answer `false` reject `--json` (there is nothing meaningful to
    /// serialize beyond the bare envelope). Defaults `true`; the `traces` reporting
    /// commands (`analyze`/`run`) print human tables only, so they opt out.
    pub fn produces_json_payload(&self) -> bool {
        !matches!(
            self,
            Command::Traces(TracesCommand::Analyze { .. } | TracesCommand::Run { .. })
        )
    }
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
            let sh = xshell::Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("check");
            steps::static_checks::run(&sh, Mode::Fix, &mut result);
            steps::sequence_check::run(&mut result);
            steps::adr_check::run(&mut result);
            steps::test_pattern_check::run(&mut result);
            steps::host_tests::run(&sh, &mut result);
            if !no_test {
                steps::nix::coverage(&mut result);
            }
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Validate {
            no_e2e,
            allow_dirty,
        } => {
            let sh = xshell::Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("validate");
            // Clean-tree backstop: refuse a dirty tree so what is measured equals the
            // committed tip (== what CI sees). Fail fast before the expensive steps.
            let precheck = clean_tree_precheck(&sh, allow_dirty);
            let blocked = !precheck.ok && !precheck.skipped;
            result.push(precheck);
            if blocked {
                finalize(&mut result, start);
                return Ok(result);
            }
            steps::static_checks::run(&sh, Mode::Check, &mut result);
            steps::sequence_check::run(&mut result);
            steps::adr_check::run(&mut result);
            steps::test_pattern_check::run(&mut result);
            steps::host_tests::run(&sh, &mut result);
            steps::nix::coverage(&mut result);
            if !no_e2e {
                // `e2e` builds the `e2e-checks` aggregate, which now includes the
                // `e2e-elisp-integration` check — so it runs in parallel with the
                // browser combos; no separate step needed (ADR-0035).
                steps::nix::e2e(&mut result);
            }
            finalize(&mut result, start);
            Ok(result)
        }
        Command::AuditWasm { site_path } => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("audit-wasm");
            match audit_wasm::run(site_path.as_deref()) {
                Ok(report) => {
                    let n = report.artifacts.len();
                    result.audit = Some(report);
                    result.push(StepResult::ok("audit-wasm").detail(format!("{n} artifact(s)")));
                }
                Err(e) => {
                    result.push(StepResult::fail("audit-wasm").detail(format!("{e:#}")));
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
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Coverage(CoverageCommand::ProbeSource) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("coverage-probe-source");
            result.push(coverage::probe::probe_source());
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Adr(AdrCommand::Renumber) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("adr-renumber");
            result.push(adr::renumber());
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Adr(AdrCommand::SyncReadme) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("adr-sync-readme");
            result.push(adr_readme::sync_readme());
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Adr(AdrCommand::Promote) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("adr-promote");
            result.push(adr::promote());
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Traces(TracesCommand::Analyze {
            top,
            trace,
            project,
            files,
        }) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("traces-analyze");
            let filters = traces::parse::Filters { trace, project };
            // A read/parse failure (missing file, malformed JSONL line) propagates
            // as Err → the exit-2 path in main.rs (spec §6), not a fail step.
            let analysis = traces::analyze::analyze(&files, filters)?;
            let n = analysis.span_count;
            result.traces = Some(traces::render::render(&analysis, top as usize));
            result.push(StepResult::ok("traces-analyze").detail(format!("{n} span(s)")));
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Traces(TracesCommand::Run {
            top,
            trace,
            cold,
            browser,
        }) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("traces-run");
            // A nix-build failure or a missing trace file propagates as Err → the
            // exit-2 path in main.rs (spec §5), not a fail step.
            let files = traces::run::collect_trace_files(cold, browser)?;
            let n = files.len();
            let filters = traces::parse::Filters {
                trace,
                project: None,
            };
            let analysis = traces::analyze::analyze(&files, filters)?;
            result.traces = Some(traces::render::render(&analysis, top as usize));
            result.push(StepResult::ok("traces-run").detail(format!("{n} trace file(s)")));
            finalize(&mut result, start);
            Ok(result)
        }
        Command::ElispIntegration => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("elisp-integration");
            steps::nix::elisp_integration(&mut result);
            finalize(&mut result, start);
            Ok(result)
        }
    }
}

/// Self-healing hook installation: point `core.hooksPath` at `.githooks` if it is not
/// already, so fresh clones and new worktrees wire up on first run. Best-effort — a
/// failure here must never block the actual command.
pub fn ensure_hooks_installed() {
    let Ok(sh) = xshell::Shell::new() else {
        return;
    };
    match git::ensure_hooks_path(&sh) {
        Ok(true) => eprintln!("xtask: set core.hooksPath = {}", git::HOOKS_PATH),
        Ok(false) => {}
        Err(e) => eprintln!("xtask: warning: could not set core.hooksPath: {e:#}"),
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
fn clean_tree_precheck(sh: &xshell::Shell, allow_dirty: bool) -> StepResult {
    if allow_dirty {
        return StepResult::skip("clean-tree").detail("--allow-dirty");
    }
    match git::working_tree_status(sh) {
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
    fn adr_renumber_parses() {
        let cli = Cli::try_parse_from(["xtask", "adr", "renumber"]).unwrap();
        assert_eq!(cli.command_name(), "adr-renumber");
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
                files,
            }) => {
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
        // A read failure propagates as Err → the exit-2 path (spec §6).
        let cli = Cli {
            json: false,
            command: Command::Traces(TracesCommand::Analyze {
                top: 25,
                trace: None,
                project: None,
                files: vec![PathBuf::from("/no/such/trace.jsonl")],
            }),
        };
        assert!(run(cli).is_err(), "missing file must propagate as Err");
    }

    #[test]
    fn traces_run_parses_flags() {
        let cli = Cli::try_parse_from([
            "xtask",
            "traces",
            "run",
            "--top",
            "40",
            "--cold",
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
                cold,
                browser,
            }) => {
                assert_eq!(top, 40);
                assert_eq!(trace.as_deref(), Some("aa"));
                assert!(cold);
                assert_eq!(browser, Some(E2eBrowser::Firefox));
            }
            _ => panic!("expected traces run"),
        }
    }

    #[test]
    fn traces_run_defaults() {
        let cli = Cli::try_parse_from(["xtask", "traces", "run"]).unwrap();
        match cli.command {
            Command::Traces(TracesCommand::Run {
                top,
                trace,
                cold,
                browser,
            }) => {
                assert_eq!(top, 25);
                assert_eq!(trace, None);
                assert!(!cold);
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
                cold: false,
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
}

#[cfg(test)]
mod git_env_tests {
    use crate::git::at as git_at;

    #[test]
    fn git_at_scrubs_repo_redirecting_env() {
        // Regression guard: without scrubbing these, a git op meant for `dir`
        // (a throwaway test repo) would be redirected at the hook's repo when run
        // inside a git hook, corrupting it. `get_envs()` yields `(key, None)` for a
        // removed var.
        let cmd = git_at(std::path::Path::new("/tmp/x"));
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
