//! `devtool seed-e2e` — the canonical e2e fixture seed (users + site-config +
//! mail-reset) applied by BOTH the host loop (`cargo xtask e2e-local`) and the
//! flake VM `seed_db()`. It used to be three literal copies kept in sync only by
//! comment; now there is one list, here. Shells out to the `test-support` binary
//! (devtool can't link the main-workspace crate). Every step is fatal: both
//! callers guarantee a fresh / truncated DB before seeding, so a failure is a
//! real error, not an expected re-run collision. See issue #249.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context};

/// The canonical fixture invocations as `(args, fatal)`. `fatal` is currently
/// always true — the tuple shape keeps a future non-fatal step a data change
/// rather than a control-flow change. Pure, so it is unit-tested directly.
fn seed_invocations(mail_file: &str) -> Vec<(Vec<String>, bool)> {
    let step = |xs: &[&str]| -> (Vec<String>, bool) {
        (xs.iter().map(|x| (*x).to_owned()).collect(), true)
    };
    vec![
        step(&[
            "create-user",
            "--username",
            "testlogin",
            "--password",
            "testpassword123",
        ]),
        step(&[
            "create-user",
            "--username",
            "testnoemail",
            "--password",
            "testpassword123",
        ]),
        step(&[
            "create-user",
            "--username",
            "testoperator",
            "--password",
            "testpassword123",
            "--operator",
        ]),
        step(&[
            "set-site-config",
            "--key",
            "site.registration_policy",
            "--value",
            "open",
        ]),
        step(&[
            "set-site-config",
            "--key",
            "feeds.websub_hub_url",
            "--value",
            "https://hub.test.local/",
        ]),
        step(&["reset-mail", "--path", mail_file]),
    ]
}

/// Run the canonical seed by shelling each invocation out to `test_support_bin`
/// with `JAUNDER_DB=db`. Fatal on the first non-zero exit.
pub fn run(db: &str, mail_file: &str, test_support_bin: &Path) -> anyhow::Result<()> {
    for (args, _fatal) in seed_invocations(mail_file) {
        let status = Command::new(test_support_bin)
            .args(&args)
            .env("JAUNDER_DB", db)
            .status()
            .with_context(|| format!("spawning {} {}", test_support_bin.display(), args[0]))?;
        if !status.success() {
            bail!("test-support {} failed ({status})", args[0]);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::seed_invocations;

    #[test]
    fn canonical_fixture_invocations() {
        let inv = seed_invocations("/tmp/mail.jsonl");
        let as_vecs: Vec<Vec<&str>> = inv
            .iter()
            .map(|(args, fatal)| {
                assert!(*fatal, "all e2e seed steps are fatal against a fresh DB");
                args.iter().map(String::as_str).collect()
            })
            .collect();
        assert_eq!(
            as_vecs,
            vec![
                vec![
                    "create-user",
                    "--username",
                    "testlogin",
                    "--password",
                    "testpassword123"
                ],
                vec![
                    "create-user",
                    "--username",
                    "testnoemail",
                    "--password",
                    "testpassword123"
                ],
                vec![
                    "create-user",
                    "--username",
                    "testoperator",
                    "--password",
                    "testpassword123",
                    "--operator"
                ],
                vec![
                    "set-site-config",
                    "--key",
                    "site.registration_policy",
                    "--value",
                    "open"
                ],
                vec![
                    "set-site-config",
                    "--key",
                    "feeds.websub_hub_url",
                    "--value",
                    "https://hub.test.local/"
                ],
                vec!["reset-mail", "--path", "/tmp/mail.jsonl"],
            ]
        );
    }
}
