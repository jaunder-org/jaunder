//! Completed-build output selection uses stdout, never diagnostic stderr.
use anyhow::{Context, Result, bail};
use std::process::Command;

/// Pull the built store path out of `nix build --print-out-paths` output. Nix may
/// print warnings interleaved, so we take the *last* `/nix/store/` line (the
/// realized output).
pub fn parse_store_path(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|l| l.starts_with("/nix/store/"))
        .map(str::to_string)
}

/// Select the store path from a completed `nix build`'s streams. Parses **stdout
/// only** — `stderr` is used solely for the error message, never parsed — so a
/// `…-user-environment` (or any other) line nix writes to stderr can never be
/// selected as the result (#224).
pub fn store_path_from_streams(stdout: &str, stderr: &str) -> Result<String> {
    parse_store_path(stdout).with_context(|| {
        format!("could not parse a /nix/store path from nix stdout; stderr:\n{stderr}")
    })
}

/// `nix build .#<attr> --no-link --print-out-paths`; captures both streams, bails
/// with the captured stderr on non-zero status, else selects the store path from
/// stdout via [`store_path_from_streams`].
pub fn build_out_path(attr: &str) -> Result<String> {
    let flake_ref = format!(".#{attr}");
    let out = Command::new("nix")
        .args(["build", &flake_ref, "--no-link", "--print-out-paths"])
        .output()
        .with_context(|| format!("spawning `nix build {flake_ref}`"))?;
    if !out.status.success() {
        bail!(
            "`nix build {flake_ref}` failed ({}):\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    store_path_from_streams(
        &String::from_utf8_lossy(&out.stdout),
        &String::from_utf8_lossy(&out.stderr),
    )
    .with_context(|| format!("`nix build {flake_ref}` produced no store path"))
}

/// Realize and root a native checkout package in one Nix operation. The caller
/// holds the host artifact lock until every consumer has finished with this root.
pub(crate) fn build_rooted(
    sh: &xshell::Shell,
    attr: &str,
    root: &std::path::Path,
) -> Result<String> {
    let installable = format!(".#{attr}");
    let out = xshell::cmd!(
        sh,
        "nix build {installable} --out-link {root} --print-out-paths"
    )
    .output()
    .with_context(|| format!("realizing {installable}"))?;
    store_path_from_streams(
        &String::from_utf8_lossy(&out.stdout),
        &String::from_utf8_lossy(&out.stderr),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_store_path_takes_last_store_line() {
        let out = "warning: ignoring\n/nix/store/aaa-x\n  /nix/store/bbb-jaunder-site  \n";
        assert_eq!(
            parse_store_path(out).as_deref(),
            Some("/nix/store/bbb-jaunder-site")
        );
    }

    #[test]
    fn parse_store_path_none_when_no_store_line() {
        assert_eq!(parse_store_path("no paths here\n"), None);
    }

    #[test]
    fn store_path_from_streams_ignores_stderr() {
        let result =
            store_path_from_streams("no result here\n", "/nix/store/zzz-user-environment\n");
        assert!(result.is_err(), "stderr store path must not be selected");
    }

    #[test]
    fn store_path_from_streams_takes_stdout() {
        assert_eq!(
            store_path_from_streams("/nix/store/aaa-e2e\n", "junk stderr").unwrap(),
            "/nix/store/aaa-e2e"
        );
    }
}
