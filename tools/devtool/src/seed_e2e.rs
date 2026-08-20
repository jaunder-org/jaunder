//! `devtool seed-e2e` — the canonical e2e fixture seed (site-config + users +
//! mail-reset) applied by BOTH the host loop (`cargo xtask e2e-local`) and the
//! flake VM `seed_db()` — one list, applied by both callers. Shells each step
//! out to its target
//! binary (devtool can't link the main-workspace crates): the `site_config`
//! steps go through the shipped `jaunder` binary (`jaunder site-config set`),
//! the rest through `test-support`. Every step is fatal: both callers guarantee
//! a fresh / truncated DB before seeding, so a failure is a real error, not an
//! expected re-run collision. See issues #249 and #8.
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};

/// Which fixture binary a seed step runs against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SeedBin {
    /// The out-of-process `test-support` helper (users + mail-reset).
    TestSupport,
    /// The shipped `jaunder` binary (`site-config set`).
    Jaunder,
}

const OTLP_ENDPOINT_ENV: &str = "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT";
const SEED_OTLP_ENDPOINT: &str = "http://127.0.0.1:4317";
const SEED_PROCESS_ENV: &str = "JAUNDER_E2E_SEED_PROCESS";
const JAUNDER_SEED_PROCESS: &str = "e2e.seed.jaunder";
const TEST_SUPPORT_SEED_PROCESS: &str = "e2e.seed.test-support";
const SEED_RUST_LOG: &str =
    "jaunder=warn,host=warn,web=warn,common=warn,tower_http=warn,sqlx=warn,storage=info";

/// The canonical fixture invocations as `(bin, args, fatal)`. `fatal` is
/// currently always true — the tuple shape keeps a future non-fatal step a data
/// change rather than a control-flow change. The `site_config` steps run
/// **first**, through the shipped `jaunder` binary, so a wrong `--jaunder-bin`
/// (e.g. a cheap-kdf build that fail-closes) aborts on an empty DB rather than
/// after the users are created. Pure, so it is unit-tested directly.
fn seed_invocations() -> Vec<(SeedBin, Vec<String>, bool)> {
    let step = |bin: SeedBin, xs: &[&str]| -> (SeedBin, Vec<String>, bool) {
        (bin, xs.iter().map(|x| (*x).to_owned()).collect(), true)
    };
    let ts = |xs: &[&str]| step(SeedBin::TestSupport, xs);
    let jaunder = |xs: &[&str]| step(SeedBin::Jaunder, xs);
    vec![
        jaunder(&["site-config", "set", "site.registration_policy", "open"]),
        // #560: feeds/AtomPub require a base URL to emit absolute atom:ids, so the e2e
        // fixture configures one. It is deliberately NOT the address the test server
        // listens on — atompub.spec's `onServer` helper re-bases absolute URLs onto the
        // live server when it needs to fetch them.
        jaunder(&["site-config", "set", "site.base_url", "https://example.com"]),
        jaunder(&[
            "site-config",
            "set",
            "feeds.websub_hub_url",
            "https://hub.test.local/",
        ]),
        ts(&[
            "create-user",
            "--username",
            "testlogin",
            "--password",
            "testpassword123",
        ]),
        ts(&[
            "create-user",
            "--username",
            "testnoemail",
            "--password",
            "testpassword123",
        ]),
        ts(&[
            "create-user",
            "--username",
            "testoperator",
            "--password",
            "testpassword123",
            "--operator",
        ]),
        ts(&["reset-mail"]),
    ]
}

fn child_env(db: &str, bin: SeedBin) -> [(&'static str, &str); 4] {
    let process = match bin {
        SeedBin::Jaunder => JAUNDER_SEED_PROCESS,
        SeedBin::TestSupport => TEST_SUPPORT_SEED_PROCESS,
    };
    [
        ("JAUNDER_DB", db),
        (OTLP_ENDPOINT_ENV, SEED_OTLP_ENDPOINT),
        (SEED_PROCESS_ENV, process),
        ("RUST_LOG", SEED_RUST_LOG),
    ]
}

/// Run the canonical seed by shelling each step out to its target binary
/// (`test_support_bin` or `jaunder_bin`) with the DB and OTLP endpoint passed to
/// every child process. Fatal on the first non-zero exit; the bail message names
pub fn run(db: &str, test_support_bin: &Path, jaunder_bin: &Path) -> anyhow::Result<()> {
    for (bin, args, _fatal) in seed_invocations() {
        let path = match bin {
            SeedBin::TestSupport => test_support_bin,
            SeedBin::Jaunder => jaunder_bin,
        };
        let env = child_env(db, bin);
        let status = Command::new(path)
            .args(&args)
            .envs(env.iter().map(|(key, value)| (OsStr::new(key), value)))
            .status()
            .with_context(|| format!("spawning {} {}", path.display(), args.join(" ")))?;
        if !status.success() {
            // Full args, so a failing `site-config set <key>` names which write
            // failed (both share args[0] = "site-config"); aids CI-VM debugging.
            bail!("{} {} failed ({status})", path.display(), args.join(" "));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        JAUNDER_SEED_PROCESS, OTLP_ENDPOINT_ENV, SEED_OTLP_ENDPOINT, SEED_PROCESS_ENV,
        SEED_RUST_LOG, SeedBin, TEST_SUPPORT_SEED_PROCESS, child_env, seed_invocations,
    };

    #[test]
    fn canonical_fixture_invocations() {
        let inv = seed_invocations();
        let as_tagged: Vec<(SeedBin, Vec<&str>)> = inv
            .iter()
            .map(|(bin, args, fatal)| {
                assert!(*fatal, "all e2e seed steps are fatal against a fresh DB");
                (*bin, args.iter().map(String::as_str).collect())
            })
            .collect();
        assert_eq!(
            as_tagged,
            vec![
                // site_config first, through the shipped `jaunder` binary.
                (
                    SeedBin::Jaunder,
                    vec!["site-config", "set", "site.registration_policy", "open"],
                ),
                (
                    SeedBin::Jaunder,
                    vec!["site-config", "set", "site.base_url", "https://example.com"],
                ),
                (
                    SeedBin::Jaunder,
                    vec![
                        "site-config",
                        "set",
                        "feeds.websub_hub_url",
                        "https://hub.test.local/",
                    ],
                ),
                (
                    SeedBin::TestSupport,
                    vec![
                        "create-user",
                        "--username",
                        "testlogin",
                        "--password",
                        "testpassword123",
                    ],
                ),
                (
                    SeedBin::TestSupport,
                    vec![
                        "create-user",
                        "--username",
                        "testnoemail",
                        "--password",
                        "testpassword123",
                    ],
                ),
                (
                    SeedBin::TestSupport,
                    vec![
                        "create-user",
                        "--username",
                        "testoperator",
                        "--password",
                        "testpassword123",
                        "--operator",
                    ],
                ),
                (SeedBin::TestSupport, vec!["reset-mail"]),
            ]
        );
    }

    #[test]
    fn child_env_sets_db_endpoint_and_seed_process() {
        assert_eq!(
            child_env("sqlite:/tmp/jaunder.db", SeedBin::TestSupport),
            [
                ("JAUNDER_DB", "sqlite:/tmp/jaunder.db"),
                (OTLP_ENDPOINT_ENV, SEED_OTLP_ENDPOINT),
                (SEED_PROCESS_ENV, TEST_SUPPORT_SEED_PROCESS),
                ("RUST_LOG", SEED_RUST_LOG),
            ]
        );
        assert_eq!(
            child_env("postgres://jaunder@localhost/jaunder", SeedBin::Jaunder),
            [
                ("JAUNDER_DB", "postgres://jaunder@localhost/jaunder"),
                (OTLP_ENDPOINT_ENV, SEED_OTLP_ENDPOINT),
                (SEED_PROCESS_ENV, JAUNDER_SEED_PROCESS),
                ("RUST_LOG", SEED_RUST_LOG),
            ]
        );
    }
}
