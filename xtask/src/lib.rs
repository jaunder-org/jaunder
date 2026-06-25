use clap::{Parser, Subcommand};

mod coverage;
mod result;
mod sh;
mod steps {
    pub mod host_tests;
    pub mod nix;
    pub mod static_checks;
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

#[derive(Subcommand)]
pub enum Command {
    /// Inner loop (auto-fixes formatting): host static checks + clippy, then the
    /// Nix coverage check (instrumented test suite + coverage). `--no-test` runs
    /// static + clippy only.
    Check {
        /// Skip the Nix coverage check — static checks + clippy only.
        #[arg(long)]
        no_test: bool,
    },
    /// Full gate (never mutates the tree): static + clippy (verify-only) + the Nix
    /// coverage check + the e2e VMs. `--no-e2e` skips the e2e VMs.
    Validate {
        /// Skip the e2e VM checks — static + coverage only.
        #[arg(long)]
        no_e2e: bool,
    },
    /// Regenerate the accepted-uncovered baseline (`coverage-baseline.json`)
    /// from a coverage check's `coverage-report.txt`. One-shot, hidden helper.
    #[command(name = "__regen-baseline", hide = true)]
    RegenBaseline {
        /// GC-root / out-link directory holding `coverage-report.txt`.
        #[arg(long, default_value = ".xtask/gcroots/coverage")]
        gcroot: String,
    },
}

impl Cli {
    pub fn command_name(&self) -> &'static str {
        match self.command {
            Command::Check { .. } => "check",
            Command::Validate { .. } => "validate",
            Command::RegenBaseline { .. } => "__regen-baseline",
        }
    }
}

pub fn run(cli: Cli) -> anyhow::Result<CommandResult> {
    match cli.command {
        Command::Check { no_test } => {
            let sh = xshell::Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("check");
            steps::static_checks::run(&sh, Mode::Fix, &mut result);
            steps::host_tests::run(&sh, &mut result);
            if !no_test {
                steps::nix::coverage(&mut result, Mode::Fix);
            }
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Validate { no_e2e } => {
            let sh = xshell::Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("validate");
            steps::static_checks::run(&sh, Mode::Check, &mut result);
            steps::host_tests::run(&sh, &mut result);
            steps::nix::coverage(&mut result, Mode::Check);
            if !no_e2e {
                steps::nix::e2e(&mut result);
            }
            finalize(&mut result, start);
            Ok(result)
        }
        Command::RegenBaseline { gcroot } => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("__regen-baseline");
            let step = regen_baseline(&gcroot);
            result.push(step);
            finalize(&mut result, start);
            Ok(result)
        }
    }
}

/// One-shot: parse `<gcroot>/coverage-report.txt`, build the accepted-uncovered
/// baseline from the currently-uncovered executable lines, and write it to
/// `coverage-baseline.json` (repo-relative paths via `git rev-parse`).
fn regen_baseline(gcroot: &str) -> StepResult {
    match regen_baseline_inner(gcroot) {
        Ok(n) => StepResult::ok("regen-baseline").detail(format!(
            "wrote coverage-baseline.json ({n} file(s) with gaps)"
        )),
        Err(e) => StepResult::fail("regen-baseline").detail(e.to_string()),
    }
}

fn regen_baseline_inner(gcroot: &str) -> anyhow::Result<usize> {
    use anyhow::Context as _;
    let report_path = format!("{gcroot}/coverage-report.txt");
    let report =
        std::fs::read_to_string(&report_path).with_context(|| format!("reading {report_path}"))?;
    let repo_root = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("running git rev-parse --show-toplevel")?
            .stdout,
    )?;
    let repo_root = repo_root.trim();
    let files = coverage::report::parse_text_report(&report, repo_root);
    let baseline = coverage::baseline::Baseline::from_files(&files);
    baseline.save("coverage-baseline.json")?;
    let with_gaps = files
        .iter()
        .filter(|f| f.lines.iter().any(|l| !l.covered))
        .count();
    Ok(with_gaps)
}

fn finalize(result: &mut CommandResult, start: std::time::Instant) {
    result.duration_ms = start.elapsed().as_millis();
    result.finished_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
}
