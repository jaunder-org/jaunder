//! Focused browser proof: `e2e-local [--browser B] [<spec-or-file:line>]`
//! owns one fresh host lifecycle per browser (build, serve, seed, Playwright,
//! teardown). `--update-visual-snapshots` refreshes Chromium and Firefox
//! baselines with release CSR. Local proof does not replace CI.

use clap::Parser;
use host_proof::report::{CommandResult, StepResult};
use host_proof::steps::e2e_local::{self, E2eLocalBrowser};

/// Focused browser proof: use a spec or file:line selector while iterating.
///
/// Rerun the same focused command after a fix. Use representative hermetic
/// `just e2e-producer <identity>` proof when backend/browser parity, release-build
/// behavior or the hermetic environment is load-bearing; local proof does not
/// replace CI/full validation.
///
/// Owns each host lifecycle: build the CSR bundle + server, start `jaunder serve`
/// on an ephemeral port with the VM's capture env + a per-run temp DB, seed via
/// the shared `test-support seed-e2e`, run Playwright against the discovered URL,
/// and tear the server down on every exit path. Visual update mode builds release
/// CSR and updates Chromium and Firefox baselines in separate fresh lifecycles.
/// Prints one JSON report on stdout; child output goes to stderr.
#[derive(Parser)]
#[command(name = "e2e-local")]
struct Cli {
    /// Run the selected ordinary Playwright browser project (default Chromium).
    #[arg(long, value_enum, conflicts_with = "update_visual_snapshots")]
    browser: Option<E2eLocalBrowser>,
    /// A spec path or `file:line` filter passed through to Playwright.
    test: Option<String>,
    /// Update every Chromium and Firefox visual baseline using release CSR.
    #[arg(long, conflicts_with = "test")]
    update_visual_snapshots: bool,
}

fn main() {
    let cli = Cli::parse();
    let start = std::time::Instant::now();
    let mut result = CommandResult::new("e2e-local");
    match host_proof::checkout::prepare().and_then(|()| Ok(xshell::Shell::new()?)) {
        Ok(sh) => {
            host_proof::checkout::ensure_hooks_installed();
            e2e_local::run(
                &sh,
                &mut result,
                cli.browser,
                cli.test.as_deref(),
                cli.update_visual_snapshots,
            );
        }
        Err(error) => {
            result.push(StepResult::fail("e2e-local-checkout").detail(format!("{error:#}")))
        }
    }
    result.duration_ms = start.elapsed().as_millis();
    std::process::exit(result.emit());
}

#[cfg(test)]
mod tests {
    use super::{Cli, E2eLocalBrowser};
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("e2e-local").chain(args.iter().copied()))
    }

    #[test]
    fn parses_filter_browser_or_visual_update_mode() {
        let cli = parse(&[]).unwrap();
        assert_eq!(
            (cli.browser, cli.test, cli.update_visual_snapshots),
            (None, None, false)
        );
        let cli = parse(&["theme.spec.ts:63", "--browser", "firefox"]).unwrap();
        assert_eq!(cli.browser, Some(E2eLocalBrowser::Firefox));
        assert_eq!(cli.test.as_deref(), Some("theme.spec.ts:63"));
        for (argument, browser) in [
            ("chromium", E2eLocalBrowser::Chromium),
            ("firefox", E2eLocalBrowser::Firefox),
            ("webkit", E2eLocalBrowser::Webkit),
        ] {
            assert_eq!(
                parse(&["--browser", argument]).unwrap().browser,
                Some(browser)
            );
        }
        assert!(
            parse(&["--update-visual-snapshots"])
                .unwrap()
                .update_visual_snapshots
        );
    }

    #[test]
    fn help_redirects_iteration_without_replacing_gates() {
        let help = Cli::try_parse_from(["e2e-local", "--help"])
            .err()
            .unwrap()
            .to_string();
        let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
        for phrase in [
            "Focused browser proof",
            "same focused command",
            "hermetic",
            "parity",
            "just e2e-producer",
        ] {
            assert!(help.contains(phrase), "missing {phrase:?}: {help}");
        }
    }

    #[test]
    fn visual_update_rejects_filter_and_browser() {
        assert!(parse(&["--update-visual-snapshots", "auth-flow.spec.ts"]).is_err());
        let Err(error) = parse(&["--update-visual-snapshots", "--browser", "webkit"]) else {
            panic!("browser selection conflicts with visual snapshot updates");
        };
        assert!(error.to_string().contains("--browser"));
        assert!(error.to_string().contains("--update-visual-snapshots"));
    }
}
