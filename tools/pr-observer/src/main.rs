use clap::{Parser, Subcommand, ValueEnum};
use pr_observer::pr::observe::observe;
use pr_observer::pr::snapshot::GhSource;
use pr_observer::pr::watch::{SystemClock, WatchConfig};
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
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum WatchUntil {
    Merged,
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
    let report = match observe(&source, &clock, number, config, &mut sink) {
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
