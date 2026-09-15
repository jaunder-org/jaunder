//! On-demand drift guard for the Nix `coverage` derivation's source filter (#241).
//!
//! #231 bounded the `coverage` derivation's `src` to cargo sources (+ an explicit
//! `csr/index.html`), closing the #37 impurity. This module guards that filter
//! against *silent* drift: [`probe_verdict`] asserts the two contract invariants over
//! three measured `coverage.drvPath` values —
//!
//! - adding a filter-**excluded** file must NOT change the drvPath (else the filter
//!   re-admits junk → the #37 impurity returns), and
//! - adding an **instrumented** `.rs` MUST change the drvPath (else the filter drops
//!   source → a coverage hole the stateless gate can never see).
//!
//! The pure verdict lives here; the impure orchestration (an ephemeral worktree that
//! stages each probe file and evaluates its drvPath) is [`probe_source`]. See the
//! spec for the load-bearing subtlety: nix ignores *untracked* new files even on a
//! dirty tree, so probe files must be `git add`-ed to be measured.

use std::fmt;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};

use crate::coverage::FileCoverage;
use crate::git;
use crate::result::StepResult;
use crate::steps::nix;

/// The ways the coverage source closure or producer boundary can drift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftError {
    AdmitsJunk { base: String, junk: String },
    DropsSource { base: String },
    DropsRequiredSource { path: &'static str, base: String },
    AdmitsXtaskSource { base: String, xtask_source: String },
    CoverageDoesNotDependOnRequiredSource { path: &'static str, base: String },
    BuildDependencyInReport { path: String, line: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProbeArm {
    path: &'static str,
    marker: &'static str,
    required: bool,
    requires_coverage_derivation: bool,
}

const PROBE_ARMS: [ProbeArm; 8] = [
    ProbeArm {
        path: "tools/csr_bundle/Cargo.toml",
        marker: "# coverage source-drift probe; never committed.",
        required: true,
        requires_coverage_derivation: true,
    },
    ProbeArm {
        path: "tools/csr_bundle/build.rs",
        marker: "// coverage source-drift probe; never committed.",
        required: true,
        requires_coverage_derivation: false,
    },
    ProbeArm {
        path: "tools/csr_bundle/src/lib.rs",
        marker: "// coverage source-drift probe; never committed.",
        required: true,
        requires_coverage_derivation: false,
    },
    ProbeArm {
        path: "server/src/lib.rs",
        marker: "// coverage source-drift probe; never committed.",
        required: true,
        requires_coverage_derivation: false,
    },
    ProbeArm {
        path: "server/tests/main.rs",
        marker: "// coverage source-drift probe; never committed.",
        required: true,
        requires_coverage_derivation: false,
    },
    ProbeArm {
        path: "server/tests/misc/backup_corpus/index.json",
        // A space plus stage_probe_arm's trailing newline stays JSON whitespace.
        marker: " ",
        required: true,
        requires_coverage_derivation: false,
    },
    ProbeArm {
        path: ".config/nextest.toml",
        marker: "# coverage source-drift probe; never committed.",
        required: true,
        requires_coverage_derivation: false,
    },
    ProbeArm {
        path: "xtask/src/main.rs",
        marker: "// coverage source-drift probe; never committed.",
        required: false,
        requires_coverage_derivation: false,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedProbeArm {
    arm: ProbeArm,
    source_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceProbeMatrix {
    source_base: String,
    arms: Vec<ObservedProbeArm>,
    coverage_drvpath_base: String,
    coverage_drvpath_required_source: String,
}

impl fmt::Display for DriftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AdmitsJunk { base, junk } => write!(
                f,
                "coverage src filter admits junk: staging an excluded file changed \
                 coverage.drvPath ({base} -> {junk}) — the #37 impurity regressed"
            ),
            Self::DropsSource { base } => write!(
                f,
                "coverage src filter drops source: staging an instrumented .rs left \
                 coverage.drvPath unchanged ({base}) — those lines would never be measured"
            ),
            Self::DropsRequiredSource { path, base } => {
                write!(f, "coverage source filter drops required {path} ({base})")
            }
            Self::AdmitsXtaskSource { base, xtask_source } => write!(
                f,
                "coverage source filter admits host-only xtask source ({base} -> {xtask_source})"
            ),
            Self::CoverageDoesNotDependOnRequiredSource { path, base } => write!(
                f,
                "coverage derivation does not depend on required {path} ({base})"
            ),
            Self::BuildDependencyInReport { path, line } => write!(
                f,
                "coverage producer reports executable build-dependency line {path}:{line}"
            ),
        }
    }
}

impl std::error::Error for DriftError {}

/// Assert the legacy excluded-junk/instrumented-source contract.
pub fn probe_verdict(base: &str, junk: &str, rs: &str) -> Result<(), DriftError> {
    if junk != base {
        return Err(DriftError::AdmitsJunk {
            base: base.to_owned(),
            junk: junk.to_owned(),
        });
    }
    if rs == base {
        return Err(DriftError::DropsSource {
            base: base.to_owned(),
        });
    }
    Ok(())
}

fn source_probe_verdict(matrix: &SourceProbeMatrix) -> Result<(), DriftError> {
    for observed in &matrix.arms {
        if observed.arm.required && observed.source_identity == matrix.source_base {
            return Err(DriftError::DropsRequiredSource {
                path: observed.arm.path,
                base: matrix.source_base.clone(),
            });
        }
    }
    let xtask = matrix
        .arms
        .iter()
        .find(|observed| !observed.arm.required)
        .expect("one excluded probe arm");
    if xtask.source_identity != matrix.source_base {
        return Err(DriftError::AdmitsXtaskSource {
            base: matrix.source_base.clone(),
            xtask_source: xtask.source_identity.clone(),
        });
    }
    let required = matrix
        .arms
        .iter()
        .find(|observed| observed.arm.requires_coverage_derivation)
        .expect("one end-to-end required probe arm");
    if matrix.coverage_drvpath_required_source == matrix.coverage_drvpath_base {
        return Err(DriftError::CoverageDoesNotDependOnRequiredSource {
            path: required.arm.path,
            base: matrix.coverage_drvpath_base.clone(),
        });
    }
    Ok(())
}

fn realized_report_verdict(files: &[FileCoverage]) -> Result<(), DriftError> {
    const EXTERNAL_PACKAGE: &str = "tools/csr_bundle";

    for file in files {
        if (file.path == EXTERNAL_PACKAGE
            || file.path.starts_with("tools/csr_bundle/")
            || file.path.contains("/tools/csr_bundle/"))
            && let Some(line) = file.lines.first()
        {
            return Err(DriftError::BuildDependencyInReport {
                path: file.path.clone(),
                line: line.line,
            });
        }
    }
    Ok(())
}

/// Removes the ephemeral probe worktree on every exit path (return, error, panic).
/// The whole point of an RAII guard here is the panic path: a bare cleanup call at
/// the end of `run_probe` would leak the worktree if any `?` bailed or a panic
/// unwound through it.
type WorktreeRemover<'a> =
    Box<dyn Fn(&Path, &Path) -> std::io::Result<std::process::ExitStatus> + 'a>;

struct WorktreeGuard<'a> {
    repo_root: PathBuf,
    path: PathBuf,
    remove: WorktreeRemover<'a>,
    stderr: Box<dyn Write + 'a>,
}

impl Drop for WorktreeGuard<'_> {
    fn drop(&mut self) {
        let status = (self.remove)(&self.repo_root, &self.path);
        report_worktree_cleanup(status, &mut self.stderr);
    }
}

fn report_worktree_cleanup(
    status: std::io::Result<std::process::ExitStatus>,
    stderr: &mut impl Write,
) {
    let failed = match status {
        Ok(status) => !status.success(),
        Err(_) => true,
    };
    if failed {
        let _ = writeln!(
            stderr,
            "xtask: warning: xtask.coverage.probe_worktree_cleanup: ignored failure while removing probe worktree"
        );
    }
}

/// Run a git subcommand in `dir` with hooks disabled; bail on a non-zero exit.
/// Hooks are disabled defensively — `worktree add` can fire a `post-checkout`
/// hook, and we never want the repo's gate hooks running inside the probe. The
/// `-c core.hooksPath=` prefix is the only probe-specific bit; the run-and-check
/// plumbing lives in [`git::run`].
fn git_run(dir: &Path, args: &[&str]) -> Result<()> {
    let mut full = vec!["-c", "core.hooksPath="];
    full.extend_from_slice(args);
    git::run(dir, &full)
}

/// The user-facing step: evaluate the bounded source matrix, prove the required
/// build dependency reaches coverage, then inspect the realized producer report.
pub fn probe_source() -> StepResult {
    match run_probe() {
        Ok(()) => StepResult::ok("coverage-probe-source").detail(
            "coverage source filter contract holds (required Cargo inputs admitted, xtask excluded, build dependency absent from report)",
        ),
        Err(e) => StepResult::fail("coverage-probe-source").detail(format!("{e:#}")),
    }
}

fn stage_probe_arm(tmp: &Path, arm: ProbeArm) -> Result<()> {
    git_run(tmp, &["reset", "--hard", "HEAD"])?;
    dirty_probe_tree(tmp)?;
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(tmp.join(arm.path))
        .with_context(|| format!("opening {} for source probe", arm.path))?;
    writeln!(file, "{}", arm.marker)
        .with_context(|| format!("writing {} for source probe", arm.path))?;
    git_run(tmp, &["add", arm.path])
}
fn worktree_registered_with(tmp: &Path, query: impl FnOnce() -> Result<Vec<u8>>) -> Result<bool> {
    let tmp = tmp.to_str().context("worktree path is not UTF-8")?;
    let fields = query()?;
    Ok(fields.split(|byte| *byte == 0).any(|field| {
        std::str::from_utf8(field)
            .ok()
            .and_then(|field| field.strip_prefix("worktree "))
            == Some(tmp)
    }))
}

fn remove_registered_worktree_with(
    registered: bool,
    remove: impl FnOnce() -> std::io::Result<std::process::ExitStatus>,
) -> Result<()> {
    if !registered {
        return Ok(());
    }
    let status = remove().context("removing registered stale coverage-probe worktree")?;
    if !status.success() {
        anyhow::bail!("removing registered stale coverage-probe worktree failed with {status}");
    }
    Ok(())
}

fn dirty_probe_tree(tmp: &Path) -> Result<()> {
    // A clean tree makes Nix's flake fetcher walk grafted-away history on CI's
    // shallow checkout (docs/adr/0116-coverage-probe-dirty-tree-workaround.md).
    let readme = tmp.join("README.md");
    let mut bytes = fs::read(&readme).context("reading README.md to dirty it")?;
    bytes.push(b'\n');
    fs::write(&readme, bytes).context("dirtying README.md")
}

fn run_probe() -> Result<()> {
    let repo_root = std::env::current_dir().context("resolving cwd")?;
    let tmp = repo_root.join(".xtask/coverage-probe.worktree");
    fs::create_dir_all(repo_root.join(".xtask")).context("creating .xtask")?;
    let registered = worktree_registered_with(&tmp, || {
        let output = git::at(&repo_root)
            .args(["worktree", "list", "--porcelain", "-z"])
            .output()
            .context("querying registered Git worktrees")?;
        if !output.status.success() {
            anyhow::bail!(
                "querying registered Git worktrees failed with {}",
                output.status
            );
        }
        Ok(output.stdout)
    })?;
    remove_registered_worktree_with(registered, || {
        git::at(&repo_root)
            .args(["-c", "core.hooksPath=", "worktree", "remove", "--force"])
            .arg(&tmp)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    })?;

    let tmp_str = tmp.to_str().context("worktree path is not UTF-8")?;
    git_run(
        &repo_root,
        &["worktree", "add", "--detach", tmp_str, "HEAD"],
    )?;
    let _guard = WorktreeGuard {
        repo_root: repo_root.clone(),
        path: tmp.clone(),
        remove: Box::new(|repo_root, path| {
            git::at(repo_root)
                .args(["-c", "core.hooksPath="])
                .args(["worktree", "remove", "--force"])
                .arg(path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
        }),
        stderr: Box::new(std::io::stderr()),
    };

    dirty_probe_tree(&tmp)?;
    let source_base = nix::eval_coverage_source_probe_drvpath(&tmp)?;
    let coverage_drvpath_base = nix::eval_coverage_drvpath(&tmp)?;
    let mut arms = Vec::with_capacity(PROBE_ARMS.len());
    let mut coverage_drvpath_required_source = None;

    for arm in PROBE_ARMS {
        stage_probe_arm(&tmp, arm)?;
        let source_identity = nix::eval_coverage_source_probe_drvpath(&tmp)?;
        if arm.requires_coverage_derivation {
            coverage_drvpath_required_source = Some(nix::eval_coverage_drvpath(&tmp)?);
        }
        arms.push(ObservedProbeArm {
            arm,
            source_identity,
        });
    }

    let matrix = SourceProbeMatrix {
        source_base,
        arms,
        coverage_drvpath_base,
        coverage_drvpath_required_source: coverage_drvpath_required_source
            .context("missing end-to-end required source probe")?,
    };
    source_probe_verdict(&matrix)?;

    git_run(&tmp, &["reset", "--hard", "HEAD"])?;
    dirty_probe_tree(&tmp)?;
    let out = nix::build_coverage_out_path(&tmp)?;
    let report_path = Path::new(&out).join("coverage-report.txt");
    let report = fs::read_to_string(&report_path)
        .with_context(|| format!("reading realized coverage report {}", report_path.display()))?;
    let repo_root = tmp.to_str().context("worktree path is not UTF-8")?;
    let files = crate::coverage::report::parse_text_report(&report, repo_root)
        .context("parsing realized coverage report")?;
    realized_report_verdict(&files)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_run_succeeds_and_fails() {
        let dir = std::env::temp_dir().join(format!("jaunder-probe-gitrun-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(
            git::at(&dir)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
        assert!(git_run(&dir, &["status", "--porcelain"]).is_ok());
        assert!(git_run(&dir, &["mv", "nope", "nowhere"]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn holds_when_junk_excluded_and_source_measured() {
        assert_eq!(probe_verdict("d-base", "d-base", "d-rs"), Ok(()));
    }

    #[test]
    fn admits_junk_when_junk_moves_drvpath() {
        assert_eq!(
            probe_verdict("d-base", "d-JUNKMOVED", "d-rs"),
            Err(DriftError::AdmitsJunk {
                base: "d-base".into(),
                junk: "d-JUNKMOVED".into()
            })
        );
    }

    #[test]
    fn drops_source_when_rs_does_not_move_drvpath() {
        assert_eq!(
            probe_verdict("d-base", "d-base", "d-base"),
            Err(DriftError::DropsSource {
                base: "d-base".into()
            })
        );
    }

    #[test]
    fn admits_junk_takes_precedence_over_drops_source() {
        // Both broken: junk moved AND rs == base. Junk (impurity) is checked first.
        assert_eq!(
            probe_verdict("d-base", "d-JUNKMOVED", "d-base"),
            Err(DriftError::AdmitsJunk {
                base: "d-base".into(),
                junk: "d-JUNKMOVED".into()
            })
        );
    }

    #[test]
    fn stale_cleanup_uses_git_registration_not_directory_presence() {
        use std::os::unix::process::ExitStatusExt;

        let tmp = Path::new("/repo/.xtask/coverage-probe.worktree");
        let registered = worktree_registered_with(tmp, || {
            Ok(b"worktree /repo/.xtask/coverage-probe.worktree\0HEAD abc\0".to_vec())
        })
        .unwrap();
        assert!(
            registered,
            "a vanished directory can remain registered by Git"
        );

        let removed = std::cell::Cell::new(false);
        remove_registered_worktree_with(registered, || {
            removed.set(true);
            Ok(std::process::ExitStatus::from_raw(0))
        })
        .unwrap();
        assert!(removed.get());

        let absent = worktree_registered_with(tmp, || {
            Ok(b"worktree /repo/another-worktree\0HEAD def\0".to_vec())
        })
        .unwrap();
        remove_registered_worktree_with(absent, || {
            unreachable!("confirmed unregistered paths require no removal")
        })
        .unwrap();
    }

    #[test]
    fn stale_worktree_registry_query_failure_is_typed() {
        let error =
            worktree_registered_with(Path::new("/repo/.xtask/coverage-probe.worktree"), || {
                Err(anyhow::Error::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected registry failure",
                )))
            })
            .unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied)
        );
    }

    #[test]
    fn display_names_the_broken_invariant() {
        let j = DriftError::AdmitsJunk {
            base: "b".into(),
            junk: "j".into(),
        };
        assert!(j.to_string().contains("admits") && j.to_string().contains("junk"));
        let s = DriftError::DropsSource { base: "b".into() };
        assert!(s.to_string().contains("drops") && s.to_string().contains("source"));
    }

    #[test]
    fn ancillary_warning_probe_worktree_cleanup_preserves_verdict() {
        let verdict = probe_verdict("d-base", "d-base", "d-rs");
        let mut stderr = Vec::new();
        {
            let guard = WorktreeGuard {
                repo_root: PathBuf::from("/repo"),
                path: PathBuf::from("/repo/worktree"),
                remove: Box::new(|_, _| {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "sensitive injected path",
                    ))
                }),
                stderr: Box::new(&mut stderr),
            };
            drop(guard);
        }
        assert_eq!(verdict, Ok(()));
        let warning = String::from_utf8(stderr).unwrap();
        assert_eq!(
            warning
                .matches("xtask.coverage.probe_worktree_cleanup")
                .count(),
            1
        );
        assert_eq!(warning.lines().count(), 1);
        assert!(!warning.contains("sensitive"));
    }
    fn complete_matrix() -> SourceProbeMatrix {
        SourceProbeMatrix {
            source_base: "source-base".into(),
            arms: PROBE_ARMS
                .into_iter()
                .map(|arm| ObservedProbeArm {
                    arm,
                    source_identity: if arm.required {
                        format!("source-{}", arm.path)
                    } else {
                        "source-base".into()
                    },
                })
                .collect(),
            coverage_drvpath_base: "coverage-base".into(),
            coverage_drvpath_required_source: "coverage-manifest".into(),
        }
    }

    #[test]
    fn source_probe_accepts_the_complete_bounded_matrix() {
        assert_eq!(source_probe_verdict(&complete_matrix()), Ok(()));
    }

    #[test]
    fn source_probe_catalog_pins_runtime_fixture_as_non_end_to_end_source() {
        let fixture = PROBE_ARMS
            .iter()
            .find(|arm| arm.path == "server/tests/misc/backup_corpus/index.json")
            .expect("runtime fixture probe arm");

        assert_eq!(fixture.marker, " ");
        assert!(fixture.required);
        assert!(!fixture.requires_coverage_derivation);
        let matrix = complete_matrix();
        let observed = matrix
            .arms
            .iter()
            .find(|observed| observed.arm == *fixture)
            .expect("catalog fixture arm");
        assert_ne!(
            observed.source_identity.as_str(),
            matrix.source_base.as_str()
        );
    }

    #[test]
    fn source_probe_rejects_each_unchanged_required_filtered_source_identity() {
        for arm in PROBE_ARMS.into_iter().filter(|arm| arm.required) {
            let mut matrix = complete_matrix();
            matrix
                .arms
                .iter_mut()
                .find(|observed| observed.arm == arm)
                .expect("catalog arm")
                .source_identity = "source-base".into();

            assert_eq!(
                source_probe_verdict(&matrix),
                Err(DriftError::DropsRequiredSource {
                    path: arm.path,
                    base: "source-base".into(),
                })
            );
        }
    }

    #[test]
    fn source_probe_rejects_xtask_source_admission() {
        let mut matrix = complete_matrix();
        matrix
            .arms
            .iter_mut()
            .find(|observed| !observed.arm.required)
            .expect("excluded catalog arm")
            .source_identity = "source-xtask".into();

        assert_eq!(
            source_probe_verdict(&matrix),
            Err(DriftError::AdmitsXtaskSource {
                base: "source-base".into(),
                xtask_source: "source-xtask".into(),
            })
        );
    }

    #[test]
    fn source_probe_prioritizes_required_catalog_arms_before_later_failures() {
        let mut matrix = complete_matrix();
        matrix.arms[0].source_identity = "source-base".into();
        matrix
            .arms
            .iter_mut()
            .find(|observed| !observed.arm.required)
            .expect("excluded catalog arm")
            .source_identity = "source-xtask".into();

        assert_eq!(
            source_probe_verdict(&matrix),
            Err(DriftError::DropsRequiredSource {
                path: "tools/csr_bundle/Cargo.toml",
                base: "source-base".into(),
            })
        );
    }

    #[test]
    fn source_probe_rejects_coverage_drvpath_unchanged_for_csr_bundle_manifest() {
        let mut matrix = complete_matrix();
        matrix.coverage_drvpath_required_source = "coverage-base".into();
        assert_eq!(
            source_probe_verdict(&matrix),
            Err(DriftError::CoverageDoesNotDependOnRequiredSource {
                path: "tools/csr_bundle/Cargo.toml",
                base: "coverage-base".into(),
            })
        );
    }

    #[test]
    fn realized_report_rejects_every_csr_bundle_source_boundary() {
        for path in [
            "/repo/tools/csr_bundle/build.rs",
            "/repo/tools/csr_bundle/src/other.rs",
            "/repo/tools/csr_bundle/src/lib.rs",
        ] {
            let files = crate::coverage::report::parse_text_report(
                &format!("{path}:\n    7|     1|pub fn build_only() {{}}"),
                "/repo",
            )
            .unwrap();
            assert_eq!(
                realized_report_verdict(&files),
                Err(DriftError::BuildDependencyInReport {
                    path: path.strip_prefix("/repo/").unwrap().into(),
                    line: 7,
                })
            );
        }
    }

    #[test]
    fn realized_report_accepts_root_workspace_and_near_name_lines() {
        let files = crate::coverage::report::parse_text_report(
            "\
/repo/server/src/lib.rs:
    1|     1|pub fn root() {}
/repo/tools/csr_bundle_extra/src/lib.rs:
    2|     1|pub fn unrelated() {}
",
            "/repo",
        )
        .unwrap();

        assert_eq!(realized_report_verdict(&files), Ok(()));
    }
}
