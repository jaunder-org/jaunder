//! Fail-closed adapter for the pinned formatter's scoped directory inputs.

use std::io::Write;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Check exactly the directory/glob population used by the pin. Unlike its CLI,
/// do not filter iterator errors. Cooperative callers do not race source writers.
fn checked_inputs(root: &Path) -> Result<usize> {
    let base = glob::Pattern::escape(root.to_str().context("non-UTF-8 leptosfmt root")?);
    let excludes = [".direnv", ".git", "target"]
        .into_iter()
        .map(|path| {
            let pattern = if root.join(path).is_dir() {
                format!("{path}/**/*.rs")
            } else {
                path.to_string()
            };
            glob::Pattern::new(&pattern)
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut count = 0;
    for scope in ["web/src", "csr/src"] {
        let directory = root.join(scope);
        if !std::fs::metadata(&directory)
            .with_context(|| format!("reading leptosfmt scope {}", directory.display()))?
            .is_dir()
        {
            bail!("leptosfmt scope {} is not a directory", directory.display());
        }
        for entry in glob::glob(&format!("{base}/{scope}/**/*.rs"))? {
            let path = entry.context("enumerating leptosfmt inputs")?;
            let relative = path.strip_prefix(root)?;
            if excludes
                .iter()
                .any(|pattern| pattern.matches_path(relative))
            {
                continue;
            }
            // The pin reads UTF-8 before parsing; reject those errors before Fix
            // can mutate other files, and without adding a second Rust parser.
            std::fs::read_to_string(&path)
                .with_context(|| format!("reading leptosfmt input {}", path.display()))?;
            count += 1;
        }
    }
    Ok(count)
}

pub fn run(root: &Path, mode: &str) -> Result<()> {
    if !matches!(mode, "fix" | "verify") {
        bail!("expected fix or verify");
    }
    let mut command = Command::new("just");
    command.args([
        "--justfile",
        "recipes/static.just",
        "--working-directory",
        ".",
        "_leptosfmt-cli",
        mode,
    ]);
    if checked_inputs(root)? == 0 {
        // Never fall back to the CLI's implicit input when selection is empty.
        return Ok(());
    }
    // The pin prints per-file errors on stdout but returns success for them.
    // Retain both diagnostic streams and promote its error marker to a verdict.
    let output = command
        .current_dir(root)
        .output()
        .context("running leptosfmt")?;
    std::io::stdout().write_all(&output.stdout)?;
    std::io::stderr().write_all(&output.stderr)?;
    if !output.status.success()
        || String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.starts_with("❌ "))
    {
        bail!("check leptosfmt failed ({})", output.status);
    }
    Ok(())
}
