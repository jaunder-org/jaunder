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

pub(super) fn run(command: &mut Command, root: &Path) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for root in ["web/src", "csr/src"] {
            std::fs::create_dir_all(dir.path().join(root)).unwrap();
        }
        dir
    }

    fn formatter(fix: bool) -> Command {
        let spec = super::super::spec("leptosfmt", fix).unwrap();
        let built = spec
            .build_with_env(super::super::CargoMode::Host, |_| None)
            .unwrap();
        let mut cmd = Command::new(built.program);
        cmd.args(built.args);
        cmd
    }

    #[test]
    fn checked_population_matches_pinned_cli_and_keeps_literal_paths() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("checkout[1]");
        for directory in ["web/src/nested", "csr/src", "other", "target"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        for path in [
            "web/src/a[1].rs",
            "web/src/nested/b.rs",
            "csr/src/.hidden.rs",
        ] {
            std::fs::write(root.join(path), "fn valid() {}\n").unwrap();
        }
        // Neither an unrelated Rust source nor a non-Rust scoped source is read.
        std::fs::write(root.join("other/broken.rs"), "fn broken(").unwrap();
        std::fs::write(root.join("web/src/unreadable.txt"), [0xff]).unwrap();
        assert_eq!(checked_inputs(&root).unwrap(), 3);
        let output = formatter(false).current_dir(&root).output().unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("Checked 3 files"), "{stdout}");
        assert!(run(&mut formatter(true), &root).is_ok());
        assert_eq!(
            std::fs::read_to_string(root.join("other/broken.rs")).unwrap(),
            "fn broken("
        );
    }

    #[test]
    fn empty_scope_never_invokes_formatter_or_falls_back_to_repository_scan() {
        let dir = fixture();
        assert!(run(&mut Command::new("false"), dir.path()).is_ok());
    }

    #[test]
    fn directory_masquerading_as_source_is_an_io_failure() {
        let dir = fixture();
        std::fs::create_dir(dir.path().join("csr/src/unreadable.rs")).unwrap();
        assert!(run(&mut Command::new("true"), dir.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn inaccessible_scoped_directory_is_not_silently_filtered_by_glob() {
        use std::os::unix::fs::PermissionsExt;
        let dir = fixture();
        let nested = dir.path().join("web/src/private");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("source.rs"), "fn valid() {}\n").unwrap();
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o000)).unwrap();
        let result = checked_inputs(dir.path());
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            result.is_err(),
            "run this permission fixture as an unprivileged user"
        );
    }

    #[test]
    fn missing_root_fails_before_formatter_execution() {
        let dir = tempfile::tempdir().unwrap();
        assert!(run(&mut Command::new("true"), dir.path()).is_err());
    }

    #[test]
    fn unreadable_utf8_source_fails_before_formatter_execution() {
        let dir = fixture();
        std::fs::write(dir.path().join("csr/src/unreadable.rs"), [0xff]).unwrap();
        assert!(run(&mut Command::new("true"), dir.path()).is_err());
    }

    #[test]
    fn formatter_process_failures_and_success_status_error_markers_fail() {
        let dir = fixture();
        std::fs::write(dir.path().join("web/src/valid.rs"), "fn valid() {}\n").unwrap();
        for script in [
            "exit 7",
            "printf '❌ web/src/valid.rs\\n'; printf 'read failed\\n' >&2",
        ] {
            let mut command = Command::new("sh");
            command.args(["-c", script]);
            assert!(run(&mut command, dir.path()).is_err());
        }
        assert!(run(&mut Command::new("/nonexistent/leptosfmt"), dir.path()).is_err());
    }

    #[test]
    fn malformed_scoped_source_fails_even_when_pinned_cli_exits_zero() {
        let dir = fixture();
        std::fs::write(dir.path().join("web/src/broken.rs"), "fn broken(").unwrap();
        for fix in [false, true] {
            assert!(run(&mut formatter(fix), dir.path()).is_err());
        }
    }
}
