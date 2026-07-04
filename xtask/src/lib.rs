use clap::{Parser, Subcommand};

mod adr;
mod adr_readme;
mod audit_wasm;
mod coverage;
pub mod git;
mod ids;
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
    /// output (`pkg/jaunder_bg.wasm`, `pkg/jaunder.js`) so you can catch
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
    /// Coverage-baseline maintenance.
    #[command(subcommand)]
    Coverage(CoverageCommand),
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
}

/// `coverage` subcommands.
#[derive(Subcommand)]
pub enum CoverageCommand {
    /// Re-anchor `coverage-baseline.json` to the current coverage report when the
    /// drift is a safe line-shift (ADR-0030); refuse and write a candidate to
    /// `.xtask/coverage-baseline.candidate.json` on a genuine coverage lowering.
    /// Consumes an existing report (run `check`/`validate` first); never rebuilds.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask coverage reanchor\n  \
        cargo xtask coverage reanchor --gcroot .xtask/gcroots/coverage")]
    Reanchor {
        /// GC-root / out-link directory holding `coverage-report.txt`.
        #[arg(long, default_value = ".xtask/gcroots/coverage")]
        gcroot: String,
    },
    /// Refresh `crap-manifest.json` from the current CRAP report. With no
    /// regressions it rewrites the committed manifest in place (a no-op when no
    /// CRAP-relevant field changed); on a regression it refuses and writes a
    /// candidate to `.xtask/crap-manifest.candidate.json` for a deliberate `cp`.
    /// Consumes an existing report (run `check`/`validate` first); never rebuilds.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask coverage refresh-crap\n  \
        cargo xtask coverage refresh-crap --gcroot .xtask/gcroots/coverage")]
    RefreshCrap {
        /// GC-root / out-link directory holding `crap-report.json`.
        #[arg(long, default_value = ".xtask/gcroots/coverage")]
        gcroot: String,
    },
}

impl Cli {
    pub fn command_name(&self) -> &'static str {
        match self.command {
            Command::Check { .. } => "check",
            Command::Validate { .. } => "validate",
            Command::AuditWasm { .. } => "audit-wasm",
            Command::Coverage(CoverageCommand::Reanchor { .. }) => "coverage-reanchor",
            Command::Coverage(CoverageCommand::RefreshCrap { .. }) => "coverage-refresh-crap",
            Command::E2e { .. } => "e2e",
            Command::Adr(AdrCommand::Renumber) => "adr-renumber",
            Command::Adr(AdrCommand::SyncReadme) => "adr-sync-readme",
            Command::Adr(AdrCommand::Promote) => "adr-promote",
            Command::Traces(TracesCommand::Analyze { .. }) => "traces-analyze",
            Command::ElispIntegration => "elisp-integration",
        }
    }
}

impl Command {
    /// Whether `--json` yields a substantial structured payload for this command.
    /// Commands that answer `false` reject `--json` (there is nothing meaningful to
    /// serialize beyond the bare envelope). Defaults `true`; only `traces analyze`
    /// (human tables, no structured output) opts out today.
    pub fn produces_json_payload(&self) -> bool {
        !matches!(self, Command::Traces(TracesCommand::Analyze { .. }))
    }
}

pub fn run(cli: Cli) -> anyhow::Result<CommandResult> {
    // Reject --json for commands with no structured payload (only `traces analyze`
    // today) before doing any work — a hollow envelope is worse than an error.
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
                steps::nix::coverage(&mut result, Mode::Fix);
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
            steps::nix::coverage(&mut result, Mode::Check);
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
        Command::Coverage(CoverageCommand::Reanchor { gcroot }) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("coverage-reanchor");
            result.push(coverage::reanchor(&gcroot));
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Coverage(CoverageCommand::RefreshCrap { gcroot }) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("coverage-refresh-crap");
            result.push(coverage::refresh_crap(&gcroot));
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
            match traces::analyze::analyze(&files, filters) {
                Ok(analysis) => {
                    let n = analysis.span_count;
                    result.traces = Some(traces::render::render(&analysis, top as usize));
                    result.push(StepResult::ok("traces-analyze").detail(format!("{n} span(s)")));
                }
                Err(e) => {
                    result.push(StepResult::fail("traces-analyze").detail(format!("{e:#}")));
                }
            }
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

/// Register the keep-ours merge driver in `repo_dir`'s local git config. The
/// driver command is `true`: it exits 0 without touching `%A` (ours), so a merge
/// of the generated coverage artifacts resolves to our side with no conflict
/// markers. The next `cargo xtask check` re-heals to the merged-tree state.
fn register_keepours(repo_dir: &std::path::Path) -> anyhow::Result<()> {
    use anyhow::ensure;
    let cfg = |args: &[&str]| -> anyhow::Result<()> {
        let status = git::at(repo_dir).args(args).status()?;
        ensure!(status.success(), "git {:?} failed", args);
        Ok(())
    };
    cfg(&[
        "config",
        "merge.coverage-keepours.name",
        "keep ours for generated coverage artifacts",
    ])?;
    cfg(&["config", "merge.coverage-keepours.driver", "true"])?;
    Ok(())
}

/// Whether the `coverage-keepours` merge driver needs (re)registering, given the
/// current `merge.coverage-keepours.driver` value (`None` = unset). The driver command
/// is the shell builtin `true`; any other value (or unset) means re-register.
fn needs_merge_driver(current: Option<&str>) -> bool {
    match current {
        Some(value) => value.trim() != "true",
        None => true,
    }
}

/// Current `merge.coverage-keepours.driver` in `repo_dir`, or `None` when unset/blank.
/// `git config --get` exits non-zero (empty stdout) when the key is missing, so a blank
/// read maps to `None`. Goes through `git::at` so ambient `GIT_DIR`/etc. (exported when
/// run inside a hook) cannot redirect the query at another repo.
fn merge_driver_value(repo_dir: &std::path::Path) -> Option<String> {
    let out = git::at(repo_dir)
        .args(["config", "--get", "merge.coverage-keepours.driver"])
        .output()
        .ok()?;
    let value = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Ensure the keep-ours merge driver is registered in `repo_dir`; register it when
/// unset/wrong. Returns `true` when it changed config. Mirrors [`git::ensure_hooks_path`].
fn ensure_merge_driver(repo_dir: &std::path::Path) -> anyhow::Result<bool> {
    if needs_merge_driver(merge_driver_value(repo_dir).as_deref()) {
        register_keepours(repo_dir)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Self-healing merge-driver registration: register the keep-ours driver for the
/// generated coverage artifacts if it is not already, so fresh clones wire up on first
/// run. Git config is shared per-clone, so this also covers every worktree. Best-effort —
/// a failure here must never block the actual command. Parallels [`ensure_hooks_installed`].
pub fn ensure_merge_driver_installed() {
    match ensure_merge_driver(std::path::Path::new(".")) {
        Ok(true) => eprintln!("xtask: registered merge.coverage-keepours (keep-ours)"),
        Ok(false) => {}
        Err(e) => {
            eprintln!("xtask: warning: could not register merge.coverage-keepours: {e:#}")
        }
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
    fn coverage_reanchor_parses_with_default_gcroot() {
        let cli = Cli::try_parse_from(["xtask", "coverage", "reanchor"]).unwrap();
        match cli.command {
            Command::Coverage(CoverageCommand::Reanchor { gcroot }) => {
                assert_eq!(gcroot, ".xtask/gcroots/coverage");
            }
            _ => panic!("expected coverage reanchor"),
        }
    }

    #[test]
    fn coverage_reanchor_accepts_gcroot() {
        let cli =
            Cli::try_parse_from(["xtask", "coverage", "reanchor", "--gcroot", "/tmp/x"]).unwrap();
        match cli.command {
            Command::Coverage(CoverageCommand::Reanchor { gcroot }) => assert_eq!(gcroot, "/tmp/x"),
            _ => panic!("expected coverage reanchor"),
        }
    }

    #[test]
    fn coverage_refresh_crap_parses_with_default_gcroot() {
        let cli = Cli::try_parse_from(["xtask", "coverage", "refresh-crap"]).unwrap();
        match cli.command {
            Command::Coverage(CoverageCommand::RefreshCrap { gcroot }) => {
                assert_eq!(gcroot, ".xtask/gcroots/coverage");
            }
            _ => panic!("expected coverage refresh-crap"),
        }
    }

    #[test]
    fn coverage_refresh_crap_accepts_gcroot() {
        let cli = Cli::try_parse_from(["xtask", "coverage", "refresh-crap", "--gcroot", "/tmp/x"])
            .unwrap();
        match cli.command {
            Command::Coverage(CoverageCommand::RefreshCrap { gcroot }) => {
                assert_eq!(gcroot, "/tmp/x")
            }
            _ => panic!("expected coverage refresh-crap"),
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
    fn produces_json_payload_false_only_for_traces_analyze() {
        let traces = Cli::try_parse_from(["xtask", "traces", "analyze", "x.jsonl"]).unwrap();
        assert!(!traces.command.produces_json_payload());
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
}

#[cfg(test)]
mod merge_driver_tests {
    use super::{ensure_merge_driver, needs_merge_driver, register_keepours};
    use crate::git::at as git_at;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = git_at(dir).args(args).status().unwrap().success();
        assert!(ok, "git {args:?} failed");
    }

    fn git_stdout(dir: &std::path::Path, args: &[&str]) -> String {
        let out = git_at(dir).args(args).output().unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    #[test]
    fn git_at_scrubs_repo_redirecting_env() {
        // Regression guard: without scrubbing these, a git op meant for `dir`
        // (a throwaway test repo, or the user's repo via the merge-driver self-heal)
        // would be redirected at the hook's repo when run inside a git hook,
        // corrupting it. `get_envs()` yields `(key, None)` for a removed var.
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

    #[test]
    fn keepours_driver_resolves_merge_to_ours_without_markers() {
        let tmp = std::env::temp_dir().join(format!("jaunder-mergetest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        git(&tmp, &["init", "-q"]);
        git(&tmp, &["config", "user.email", "t@t"]);
        git(&tmp, &["config", "user.name", "t"]);
        register_keepours(&tmp).unwrap();
        std::fs::write(
            tmp.join(".gitattributes"),
            "crap-manifest.json merge=coverage-keepours\n",
        )
        .unwrap();
        std::fs::write(tmp.join("crap-manifest.json"), "base\n").unwrap();
        git(&tmp, &["add", "."]);
        git(&tmp, &["commit", "-q", "-m", "base"]);
        // The default branch name varies (main vs master) — capture it.
        let base = git_stdout(&tmp, &["branch", "--show-current"]);

        git(&tmp, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(tmp.join("crap-manifest.json"), "theirs\n").unwrap();
        git(&tmp, &["commit", "-qam", "theirs"]);

        git(&tmp, &["checkout", "-q", &base]);
        std::fs::write(tmp.join("crap-manifest.json"), "ours\n").unwrap();
        git(&tmp, &["commit", "-qam", "ours"]);

        // Merge must succeed (exit 0) and keep "ours" with no conflict markers.
        let merged = git_at(&tmp)
            .args(["merge", "-q", "--no-edit", "feature"])
            .status()
            .unwrap();
        assert!(merged.success(), "keep-ours merge must not conflict");
        let content = std::fs::read_to_string(tmp.join("crap-manifest.json")).unwrap();
        assert_eq!(content, "ours\n", "keep-ours must retain our side");
        assert!(!content.contains("<<<<<<<"), "no conflict markers");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn needs_merge_driver_when_unset_or_wrong() {
        assert!(needs_merge_driver(None));
        assert!(needs_merge_driver(Some("")));
        assert!(needs_merge_driver(Some("false")));
    }

    #[test]
    fn no_need_when_merge_driver_already_true() {
        assert!(!needs_merge_driver(Some("true")));
        assert!(!needs_merge_driver(Some(" true \n")));
    }

    #[test]
    fn ensure_merge_driver_registers_then_is_idempotent() {
        let tmp = std::env::temp_dir().join(format!("jaunder-ensure-md-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        git(&tmp, &["init", "-q"]);
        // First call registers and reports a change.
        assert!(ensure_merge_driver(&tmp).unwrap(), "first call registers");
        assert_eq!(
            git_stdout(&tmp, &["config", "--get", "merge.coverage-keepours.driver"]),
            "true"
        );
        // Second call is a no-op (idempotent) — the value already matches.
        assert!(
            !ensure_merge_driver(&tmp).unwrap(),
            "second call is a no-op"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
