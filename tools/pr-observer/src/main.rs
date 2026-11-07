use clap::{Parser, Subcommand, ValueEnum};
use pr_observer::pr::decide::Progress;
use pr_observer::pr::observe::observe;
use pr_observer::pr::snapshot::GhSource;
use pr_observer::pr::watch::{SystemClock, WatchConfig, WatchContinuation};
use pr_observer::pr::{Event, Outcome};

#[derive(Debug, Parser)]
#[command(name = "pr-observer", about = "Read-only Jaunder PR observation")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Observe a PR until the next caller-actionable outcome. Never merges or mutates.
    Watch {
        /// PR number. Omit to resolve the open PR for the current branch.
        number: Option<u64>,
        /// Seconds between polls.
        #[arg(long, value_name = "SECONDS", default_value_t = 30, value_parser = clap::value_parser!(u64).range(5..))]
        interval: u64,
        /// Minutes to observe before reporting `timed-out`.
        #[arg(long, value_name = "MINUTES", default_value_t = 90, value_parser = clap::value_parser!(u64).range(1..))]
        timeout: u64,
        /// Observe one snapshot instead of polling.
        #[arg(long, conflicts_with = "until")]
        once: bool,
        /// Continue across `ready-to-land` while another actor may arm the merge.
        #[arg(long, value_name = "OUTCOME", conflicts_with = "once")]
        until: Option<WatchUntil>,
        /// Bind every observed verdict, including merged, to this approved head.
        #[arg(long, value_parser = full_sha)]
        expected_head: Option<String>,
        /// Historical failed merge-group URL captured by explicit one-shot recovery.
        /// This does not change its verdict or authorize any mutation.
        #[arg(long, requires = "expected_head", value_parser = run_url)]
        acknowledged_ejection: Option<String>,
        /// Carry positive same-head queue evidence across a separately owned action boundary.
        #[arg(long, requires = "expected_head", value_parser = full_sha)]
        observed_queued_head: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum WatchUntil {
    Merged,
}

fn full_sha(value: &str) -> Result<String, String> {
    if value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(value.into())
    } else {
        Err("expected a full lowercase 40-hex head SHA".into())
    }
}

fn run_url(value: &str) -> Result<String, String> {
    let parts = value
        .strip_prefix("https://github.com/")
        .unwrap_or("")
        .split('/')
        .collect::<Vec<_>>();
    if matches!(parts.as_slice(), [owner, repo, "actions", "runs", id]
        if !owner.is_empty() && !repo.is_empty() && !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
    {
        Ok(value.into())
    } else {
        Err("expected a GitHub Actions run URL".into())
    }
}

fn main() {
    let Cli {
        command:
            Command::Watch {
                number,
                interval,
                timeout,
                once,
                until,
                expected_head,
                acknowledged_ejection,
                observed_queued_head,
            },
    } = Cli::parse();

    let source = GhSource::default();
    let clock = SystemClock;
    let config = WatchConfig {
        interval_secs: interval,
        timeout_mins: timeout,
        once,
        stop_at_ready: until.is_none(),
        ..WatchConfig::default()
    };
    let mut sink = |event: &Event| {
        eprintln!("  {} [{}] {}", event.at, event.kind.as_str(), event.detail);
    };
    let report = match observe(
        &source,
        &clock,
        number,
        config,
        WatchContinuation {
            progress: Progress {
                queued_head_sha: observed_queued_head,
            },
            emitted_optional_failure_ids: Default::default(),
            expected_head: expected_head.as_deref(),
            acknowledged_ejection: acknowledged_ejection.as_deref(),
        },
        &mut sink,
    ) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("pr-observer: {error:#}");
            std::process::exit(2);
        }
    };

    match serde_json::to_string(&report) {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("pr-observer: serializing observer outcome: {error}");
            std::process::exit(2);
        }
    }
    if !matches!(report.outcome, Outcome::ReadyToLand | Outcome::Merged) {
        std::process::exit(1);
    }
}
