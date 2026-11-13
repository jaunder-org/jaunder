use std::{fs, io, path::Path, process::Command};

use anyhow::{Context, Result, bail};
use repository_policy::report::{CommandResult, StepResult};

/// The flake checks are Linux-only (`optionalAttrs isLinux` in `nix/checks.nix`);
/// the project's CI host is x86_64-linux.
const SYSTEM: &str = "x86_64-linux";

/// Lift and consume the existing producer's fixed artifact set, without building it.
pub fn consume_elisp_coverage(result: &mut CommandResult) {
    let artifacts = Path::new(".xtask/gcroots/elisp-coverage-producer/elisp-coverage");
    result.push(lift_elisp_coverage_artifacts(
        artifacts,
        Path::new(".xtask/diagnostics/elisp-coverage"),
    ));

    let step = match crate::host::elisp::consume(Path::new("."), artifacts) {
        Ok(report) => StepResult::ok("elisp-coverage").detail(format!(
            "covered {} point(s); ignored {} point(s)",
            report.covered_points, report.ignored_points
        )),
        Err(error) => StepResult::fail("elisp-coverage").detail(elisp_coverage_detail(error)),
    };
    result.push(step);
}

fn lift_elisp_coverage_artifacts(source: &Path, destination: &Path) -> StepResult {
    let names = ["lcov.info", "summary.txt", "status.json"];
    if let Err(error) = fs::create_dir_all(destination) {
        return StepResult::fail("elisp-coverage-artifacts")
            .detail(format!("creating {}: {error}", destination.display()));
    }
    let mut failures = Vec::new();
    for name in names {
        let target = destination.join(name);
        if let Err(error) = fs::remove_file(&target)
            && error.kind() != io::ErrorKind::NotFound
        {
            failures.push(format!("{name}: removing prior artifact: {error}"));
            continue;
        }
        if let Err(error) = fs::copy(source.join(name), target) {
            failures.push(format!("{name}: {error}"));
        }
    }
    if failures.is_empty() {
        StepResult::ok("elisp-coverage-artifacts")
            .detail("lifted lcov.info, summary.txt, status.json")
    } else {
        StepResult::fail("elisp-coverage-artifacts").detail(failures.join("; "))
    }
}

fn elisp_coverage_detail(error: crate::host::elisp::CoverageError) -> String {
    use crate::host::elisp::CoverageError;

    match error {
        CoverageError::Artifact { path, message } | CoverageError::Source { path, message } => {
            format!("{}: {message}", path.display())
        }
        CoverageError::Status { message }
        | CoverageError::Census { message }
        | CoverageError::Lcov { message } => message,
        CoverageError::Verdict { failures } => failures
            .into_iter()
            .map(|failure| format!("{}:{} {}", failure.path, failure.line, failure.message))
            .collect::<Vec<_>>()
            .join("; "),
    }
}

/// Run `nix eval --raw --accept-flake-config <installable>`, optionally in a
/// supplied flake directory.
fn nix_eval_raw(dir: Option<&Path>, installable: &str) -> Result<String> {
    let mut cmd = Command::new("nix");
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let out = cmd
        .args(["eval", "--raw", "--accept-flake-config", installable])
        .output()
        .with_context(|| format!("spawning `nix eval {installable}`"))?;
    if !out.status.success() {
        bail!(
            "`nix eval {installable}` failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let path = String::from_utf8(out.stdout)
        .with_context(|| format!("`nix eval {installable}` output was not UTF-8"))?
        .trim()
        .to_owned();
    if path.is_empty() {
        bail!("`nix eval {installable}` returned an empty path");
    }
    Ok(path)
}

/// Evaluate the probe-only identity of the filtered coverage source.
pub(crate) fn eval_coverage_source_probe_drvpath(flake_dir: &Path) -> Result<String> {
    nix_eval_raw(
        Some(flake_dir),
        &format!(".#checks.{SYSTEM}.coverage-source-probe.drvPath"),
    )
}

/// Realize the coverage producer and return its output directory.
pub(crate) fn build_coverage_out_path(flake_dir: &Path) -> Result<String> {
    let installable = format!(".#checks.{SYSTEM}.coverage");
    let out = Command::new("nix")
        .current_dir(flake_dir)
        .args([
            "build",
            "--no-link",
            "--print-out-paths",
            "--accept-flake-config",
            &installable,
        ])
        .output()
        .with_context(|| format!("spawning `nix build {installable}`"))?;
    if !out.status.success() {
        bail!(
            "`nix build {installable}` failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let path = String::from_utf8(out.stdout)
        .with_context(|| format!("`nix build {installable}` output was not UTF-8"))?
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    if path.is_empty() {
        bail!("`nix build {installable}` returned an empty output path");
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::lift_elisp_coverage_artifacts;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn elisp_coverage_artifact_lift_replaces_prior_read_only_outputs() {
        // Nix outputs are read-only; repeated validate runs must replace the
        // prior lifted diagnostics rather than failing on their mode bits.
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        for name in ["lcov.info", "summary.txt", "status.json"] {
            std::fs::write(source.path().join(name), format!("fresh-{name}")).unwrap();
            let existing = destination.path().join(name);
            std::fs::write(&existing, "stale").unwrap();
            let mut permissions = std::fs::metadata(&existing).unwrap().permissions();
            permissions.set_mode(0o444);
            std::fs::set_permissions(&existing, permissions).unwrap();
        }

        let step = lift_elisp_coverage_artifacts(source.path(), destination.path());

        assert!(step.ok, "{:?}", step.detail);
        for name in ["lcov.info", "summary.txt", "status.json"] {
            assert_eq!(
                std::fs::read_to_string(destination.path().join(name)).unwrap(),
                format!("fresh-{name}")
            );
        }
    }
}
