//! `trace-analysis` — manual OTel trace diagnostics; not part of
//! `check`/`validate`. Prints one JSON report on stdout whose `report` field
//! holds the rendered tables (`just trace-analysis …` shows them).

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use host_proof::e2e::E2eBrowser;
use host_proof::traces;

#[derive(Parser)]
#[command(name = "trace-analysis")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Analyze OpenTelemetry JSONL traces exported by the e2e VM collector:
    /// slowest spans, per-test/-project hotspots and trace totals.
    #[command(after_help = "EXAMPLES:\n  \
        # trace files extracted from a capture-<backend>-<browser>-unsplit.tar.gz bundle (capture/otel-traces.jsonl):\n  \
        just trace-analysis analyze sqlite-otel-traces.jsonl postgres-otel-traces.jsonl\n  \
        just trace-analysis analyze --top 40 --project firefox trace-a.jsonl trace-b.jsonl\n  \
        just trace-analysis analyze --trace 1111...1111 traces.jsonl")]
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
        /// Playwright `json` reporter output(s); supplies the per-test
        /// span-coverage section's wall-clock denominator. Omit and that one
        /// section is skipped with a note.
        #[arg(long = "playwright-report")]
        playwright_report: Vec<PathBuf>,
        /// One or more `otel-traces.jsonl` files.
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Build the `{sqlite,postgres}×{chromium,firefox}` e2e VM checks and analyze
    /// their exported OTel traces in one step.
    #[command(after_help = "EXAMPLES:\n  \
        just trace-analysis run\n  \
        just trace-analysis run --top 40\n  \
        just trace-analysis run --single-worker\n  \
        just trace-analysis run --browser firefox")]
    Run {
        /// Rows per ranked table (default 25), forwarded to the analysis.
        #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u64).range(1..))]
        top: u64,
        /// Restrict the analysis to one trace id.
        #[arg(long)]
        trace: Option<String>,
        /// Build the single-worker package variants instead of the gate checks,
        /// for per-navigation cost where worker contention corrupts attribution.
        #[arg(long)]
        single_worker: bool,
        /// Restrict to one browser (default: both). Both backends are always built.
        #[arg(long, value_enum)]
        browser: Option<E2eBrowser>,
    },
    /// Median boot-phase decomposition per `(trace file, project, cacheWarmth)`
    /// (#818). Every segment is document-relative and the six close on
    /// `mount_done.startTime` exactly; the Node-side `commitToMountMs` and the
    /// skew between the two frames are reported but never decomposed.
    #[command(after_help = "EXAMPLES:\n  \
        just trace-analysis boot-phases sqlite-chromium.jsonl sqlite-firefox.jsonl")]
    BootPhases {
        /// One or more `otel-traces.jsonl` files.
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
}

fn main() {
    let result = match Cli::parse().command {
        Command::Analyze {
            top,
            trace,
            project,
            playwright_report,
            files,
        } => traces::analyze_command(top, trace, project, &playwright_report, &files),
        Command::Run {
            top,
            trace,
            single_worker,
            browser,
        } => traces::run_command(top, trace, single_worker, browser),
        Command::BootPhases { files } => traces::boot_phases_command(&files),
    };
    let code = match result {
        Ok(result) => result.emit(),
        Err(error) => {
            eprintln!("trace-analysis: {error:#}");
            2
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traces_analyze_parses_flags_and_files() {
        let cli = Cli::try_parse_from([
            "trace-analysis",
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
            Command::Analyze {
                top,
                trace,
                project,
                playwright_report,
                files,
            } => {
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
    }

    #[test]
    fn traces_analyze_requires_a_file() {
        assert!(Cli::try_parse_from(["trace-analysis", "analyze"]).is_err());
    }

    #[test]
    fn traces_analyze_top_must_be_positive() {
        assert!(
            Cli::try_parse_from(["trace-analysis", "analyze", "--top", "0", "x.jsonl"]).is_err()
        );
    }

    #[test]
    fn traces_run_parses_flags() {
        let cli = Cli::try_parse_from([
            "trace-analysis",
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
        match cli.command {
            Command::Run {
                top,
                trace,
                single_worker,
                browser,
            } => {
                assert_eq!(top, 40);
                assert_eq!(trace.as_deref(), Some("aa"));
                assert!(single_worker);
                assert_eq!(browser, Some(E2eBrowser::Firefox));
            }
            _ => panic!("expected traces run"),
        }
    }

    #[test]
    fn traces_run_defaults() {
        let cli = Cli::try_parse_from(["trace-analysis", "run"]).unwrap();
        match cli.command {
            Command::Run {
                top,
                trace,
                single_worker,
                browser,
            } => {
                assert_eq!(top, 25);
                assert_eq!(trace, None);
                assert!(!single_worker);
                assert_eq!(browser, None);
            }
            _ => panic!("expected traces run"),
        }
    }
}
