//! Shared `nix build … --print-out-paths` → store-path helper for the host tools
//! (`audit-wasm`, `traces run`). One tested implementation of the store-path
//! selection, so the #224 stderr-leak bug (a `…-user-environment` line on stderr
//! being parsed as the result) cannot recur: selection reads **stdout only**.

use std::process::Command;

use anyhow::{bail, Context, Result};

/// Pull the built store path out of `nix build --print-out-paths` output. Nix may
/// print warnings interleaved, so we take the *last* `/nix/store/` line (the
/// realized output), matching the old script's `.at(-1)` selection.
pub fn parse_store_path(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|l| l.starts_with("/nix/store/"))
        .map(str::to_string)
}

/// Select the store path from a completed `nix build`'s streams. Parses **stdout
/// only** — `stderr` is used solely for the error message, never parsed — so a
/// `…-user-environment` (or any other) line nix writes to stderr can never be
/// selected as the result. This is the #224 fix (the Node script joined stderr
/// into the parsed text).
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
        // stdout carries no store line; stderr carries a `-user-environment` store
        // path (the exact #224 shape). Selection reads stdout only, so this is an
        // Err — the stderr store line is never chosen as the result.
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
