//! `devtool seed-e2e` — the canonical e2e fixture seed (site-config + users +
//! mail-reset) applied by BOTH the host loop (`cargo xtask e2e-local`) and the
//! flake VM `seed_db()`. It used to be three literal copies kept in sync only by
//! comment; now there is one list, here. Shells each step out to its target
//! binary (devtool can't link the main-workspace crates): the `site_config`
//! steps go through the shipped `jaunder` binary (`jaunder site-config set`),
//! the rest through `test-support`. Every step is fatal: both callers guarantee
//! a fresh / truncated DB before seeding, so a failure is a real error, not an
//! expected re-run collision. See issues #249 and #8.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context};

/// Which fixture binary a seed step runs against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SeedBin {
    /// The out-of-process `test-support` helper (users + mail-reset).
    TestSupport,
    /// The shipped `jaunder` binary (`site-config set`).
    Jaunder,
}

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

/// Run the canonical seed by shelling each step out to its target binary
/// (`test_support_bin` or `jaunder_bin`) with `JAUNDER_DB=db`. Fatal on the first
/// non-zero exit; the bail message names the offending binary path.
pub fn run(db: &str, test_support_bin: &Path, jaunder_bin: &Path) -> anyhow::Result<()> {
    for (bin, args, _fatal) in seed_invocations() {
        let path = match bin {
            SeedBin::TestSupport => test_support_bin,
            SeedBin::Jaunder => jaunder_bin,
        };
        let status = Command::new(path)
            .args(&args)
            .env("JAUNDER_DB", db)
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
    use super::{seed_invocations, SeedBin};

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
}
