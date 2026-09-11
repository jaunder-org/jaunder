//! Eval-only drift guard for the Nix invalidation boundaries from #1289.
//!
//! The pure comparison is intentionally independent from Nix and Git. The command
//! measures each tracked perturbation in a disposable worktree, then compares only
//! derivation identities; it never builds or realizes an output.

use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::git;
use crate::result::StepResult;
use crate::steps::nix;
use crate::steps::nix::SourceProbeDrvPaths;

const WORKTREE_DIR: &str = ".xtask/nix-source-probe.worktree";

/// A derivation identity that source-probe guards. This catalog is the single
/// authority for the matrix labels used in policy and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Boundary {
    StaticDocs,
    StaticCode,
    Site,
    WasmTests,
}

impl Boundary {
    const ALL: [Self; 4] = [
        Self::StaticDocs,
        Self::StaticCode,
        Self::Site,
        Self::WasmTests,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::StaticDocs => "static-docs",
            Self::StaticCode => "static-code",
            Self::Site => "site",
            Self::WasmTests => "wasm-tests",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    Docs,
    DocsArchive,
    Server,
    Web,
    Common,
    Macros,
}

impl Arm {
    const ALL: [Self; 6] = [
        Self::Docs,
        Self::DocsArchive,
        Self::Server,
        Self::Web,
        Self::Common,
        Self::Macros,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Docs => "docs",
            Self::DocsArchive => "docs-archive",
            Self::Server => "server",
            Self::Web => "web",
            Self::Common => "common",
            Self::Macros => "macros",
        }
    }

    const fn marker_path(self) -> &'static str {
        match self {
            Self::Docs => "docs/__nix_source_probe.md",
            Self::DocsArchive => "docs/archive/__nix_source_probe.md",
            Self::Server => "server/src/__nix_source_probe.rs",
            Self::Web => "web/src/__nix_source_probe.rs",
            Self::Common => "common/src/__nix_source_probe.rs",
            Self::Macros => "macros/src/__nix_source_probe.rs",
        }
    }

    const fn expects_change(self, boundary: Boundary) -> bool {
        matches!(
            (self, boundary),
            (Self::Docs, Boundary::StaticDocs)
                | (Self::Server, Boundary::StaticCode)
                | (Self::Web, Boundary::StaticCode | Boundary::Site)
                | (
                    Self::Common | Self::Macros,
                    Boundary::StaticCode | Boundary::Site | Boundary::WasmTests,
                )
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrvPaths {
    static_docs: String,
    static_code: String,
    site: String,
    wasm_tests: String,
}

impl DrvPaths {
    fn get(&self, boundary: Boundary) -> &str {
        match boundary {
            Boundary::StaticDocs => &self.static_docs,
            Boundary::StaticCode => &self.static_code,
            Boundary::Site => &self.site,
            Boundary::WasmTests => &self.wasm_tests,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    MissingChange {
        arm: Arm,
        boundary: &'static str,
        drv: String,
    },
    UnexpectedChange {
        arm: Arm,
        boundary: &'static str,
        base: String,
        changed: String,
    },
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingChange { arm, boundary, drv } => write!(
                f,
                "{}/{}: expected derivation identity to change, but it remained {drv}",
                arm.name(),
                boundary
            ),
            Self::UnexpectedChange {
                arm,
                boundary,
                base,
                changed,
            } => write!(
                f,
                "{}/{}: expected derivation identity to remain {base}, but it changed to {changed}",
                arm.name(),
                boundary
            ),
        }
    }
}

impl std::error::Error for ProbeError {}

/// Compare one perturbation against the exact boundary matrix. Check required
/// fan-out before over-inclusion so a dropped source is reported first.
pub fn compare_arm(base: &DrvPaths, arm: Arm, changed: &DrvPaths) -> Result<(), ProbeError> {
    for boundary in Boundary::ALL {
        if arm.expects_change(boundary) && changed.get(boundary) == base.get(boundary) {
            return Err(ProbeError::MissingChange {
                arm,
                boundary: boundary.name(),
                drv: base.get(boundary).to_owned(),
            });
        }
    }
    for boundary in Boundary::ALL {
        if !arm.expects_change(boundary) && changed.get(boundary) != base.get(boundary) {
            return Err(ProbeError::UnexpectedChange {
                arm,
                boundary: boundary.name(),
                base: base.get(boundary).to_owned(),
                changed: changed.get(boundary).to_owned(),
            });
        }
    }
    Ok(())
}

pub fn probe_source() -> StepResult {
    match run_probe() {
        Ok(()) => StepResult::ok("nix-probe-source").detail(
            "Nix source-invalidation and Cachix direct-filter contracts hold (docs/docs-archive/server/web/common/macros; 7 final verdicts excluded, 6 supports eligible)",
        ),
        Err(error) => StepResult::fail("nix-probe-source").detail(format!("{error:#}")),
    }
}

fn parse_drv_path(boundary: &str, output: &str) -> Result<String> {
    let path = output.trim();
    if path.is_empty() || !path.starts_with("/nix/store/") || !path.ends_with(".drv") {
        bail!("{boundary}: malformed nix eval drvPath output: {output:?}");
    }
    Ok(path.to_owned())
}

fn eval_paths_with(evaluate: impl FnOnce() -> Result<SourceProbeDrvPaths>) -> Result<DrvPaths> {
    let paths = evaluate()?;
    Ok(DrvPaths {
        static_docs: parse_drv_path(Boundary::StaticDocs.name(), &paths.static_docs)?,
        static_code: parse_drv_path(Boundary::StaticCode.name(), &paths.static_code)?,
        site: parse_drv_path(Boundary::Site.name(), &paths.site)?,
        wasm_tests: parse_drv_path(Boundary::WasmTests.name(), &paths.wasm_tests)?,
    })
}

fn eval_paths(dir: &Path) -> Result<DrvPaths> {
    eval_paths_with(|| nix::eval_source_probe_drvpaths(dir))
}

/// Outputs governed by Cachix's direct `pushFilter` boundary. This is the one
/// authority for the final verdict and cacheable-support catalogs.
#[derive(Clone, Copy)]
enum CacheBoundaryOutput {
    Coverage,
    CoverageGate,
    E2eSqliteChromium,
    E2eSqliteFirefox,
    E2ePostgresChromium,
    E2ePostgresFirefox,
    E2eAggregate,
    CoverageSourceProbe,
    E2ePackage,
    E2eSqliteChromiumSingleWorker,
    E2eSqliteFirefoxSingleWorker,
    E2ePostgresChromiumSingleWorker,
    E2ePostgresFirefoxSingleWorker,
}

impl CacheBoundaryOutput {
    const FINALS: [Self; 7] = [
        Self::Coverage,
        Self::CoverageGate,
        Self::E2eSqliteChromium,
        Self::E2eSqliteFirefox,
        Self::E2ePostgresChromium,
        Self::E2ePostgresFirefox,
        Self::E2eAggregate,
    ];
    const SUPPORTS: [Self; 6] = [
        Self::CoverageSourceProbe,
        Self::E2ePackage,
        Self::E2eSqliteChromiumSingleWorker,
        Self::E2eSqliteFirefoxSingleWorker,
        Self::E2ePostgresChromiumSingleWorker,
        Self::E2ePostgresFirefoxSingleWorker,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Coverage => "coverage",
            Self::CoverageGate => "coverage-gate",
            Self::E2eSqliteChromium => "e2e-sqlite-chromium",
            Self::E2eSqliteFirefox => "e2e-sqlite-firefox",
            Self::E2ePostgresChromium => "e2e-postgres-chromium",
            Self::E2ePostgresFirefox => "e2e-postgres-firefox",
            Self::E2eAggregate => "e2e",
            Self::CoverageSourceProbe => "coverage-source-probe",
            Self::E2ePackage => "jaunder-e2e package",
            Self::E2eSqliteChromiumSingleWorker => "e2e-sqlite-chromium-single-worker",
            Self::E2eSqliteFirefoxSingleWorker => "e2e-sqlite-firefox-single-worker",
            Self::E2ePostgresChromiumSingleWorker => "e2e-postgres-chromium-single-worker",
            Self::E2ePostgresFirefoxSingleWorker => "e2e-postgres-firefox-single-worker",
        }
    }

    fn eval_out_path(self, flake_dir: &Path) -> Result<String> {
        match self {
            Self::E2ePackage => nix::eval_e2e_package_out_path(flake_dir),
            Self::Coverage => nix::eval_check_out_path(flake_dir, "coverage"),
            Self::CoverageGate => nix::eval_check_out_path(flake_dir, "coverage-gate"),
            Self::E2eSqliteChromium => nix::eval_check_out_path(flake_dir, "e2e-sqlite-chromium"),
            Self::E2eSqliteFirefox => nix::eval_check_out_path(flake_dir, "e2e-sqlite-firefox"),
            Self::E2ePostgresChromium => {
                nix::eval_check_out_path(flake_dir, "e2e-postgres-chromium")
            }
            Self::E2ePostgresFirefox => nix::eval_check_out_path(flake_dir, "e2e-postgres-firefox"),
            Self::E2eAggregate => nix::eval_check_out_path(flake_dir, "e2e"),
            Self::CoverageSourceProbe => {
                nix::eval_check_out_path(flake_dir, "coverage-source-probe")
            }
            Self::E2eSqliteChromiumSingleWorker => {
                nix::eval_package_out_path(flake_dir, "e2e-sqlite-chromium-single-worker")
            }
            Self::E2eSqliteFirefoxSingleWorker => {
                nix::eval_package_out_path(flake_dir, "e2e-sqlite-firefox-single-worker")
            }
            Self::E2ePostgresChromiumSingleWorker => {
                nix::eval_package_out_path(flake_dir, "e2e-postgres-chromium-single-worker")
            }
            Self::E2ePostgresFirefoxSingleWorker => {
                nix::eval_package_out_path(flake_dir, "e2e-postgres-firefox-single-worker")
            }
        }
    }
}

fn parse_push_filter(action: &str) -> Result<String> {
    let fields = action
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("pushFilter:"))
        .map(str::trim)
        .collect::<Vec<_>>();
    let [value] = fields.as_slice() else {
        bail!(
            "expected exactly one pushFilter field, found {}",
            fields.len()
        );
    };
    let Some(quote) = value
        .chars()
        .next()
        .filter(|quote| matches!(quote, '\'' | '"'))
    else {
        bail!("pushFilter must be a quoted scalar");
    };
    let Some(filter) = value
        .strip_prefix(quote)
        .and_then(|value| value.strip_suffix(quote))
    else {
        bail!("malformed quoted pushFilter");
    };
    if filter.is_empty() || filter.contains(quote) {
        bail!("malformed quoted pushFilter");
    }
    Ok(filter.to_owned())
}

fn grep_filter_matches(filter: &str, output_path: &str) -> Result<bool> {
    let mut child = Command::new("grep")
        .args(["-Eq", filter])
        .stdin(Stdio::piped())
        .spawn()
        .context("spawning grep for Cachix pushFilter")?;
    let mut stdin = child.stdin.take().context("grep stdin was unavailable")?;
    writeln!(stdin, "{output_path}").context("writing output path to grep")?;
    drop(stdin);
    match child
        .wait()
        .context("waiting for Cachix pushFilter grep")?
        .code()
    {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        status => bail!("Cachix pushFilter grep failed with status {status:?}"),
    }
}

fn verify_cache_boundary_with(
    filter: &str,
    finals: impl IntoIterator<Item = (&'static str, String)>,
    supports: impl IntoIterator<Item = (&'static str, String)>,
    mut matches: impl FnMut(&str, &str) -> Result<bool>,
) -> Result<()> {
    for (name, output_path) in finals {
        if !matches(filter, &output_path)
            .with_context(|| format!("applying pushFilter to final {name} ({output_path})"))?
        {
            bail!("final verdict {name} is reusable: {output_path}");
        }
    }
    for (name, output_path) in supports {
        if matches(filter, &output_path)
            .with_context(|| format!("applying pushFilter to support {name} ({output_path})"))?
        {
            bail!("cacheable support {name} is excluded: {output_path}");
        }
    }
    Ok(())
}

fn verify_cache_boundary(flake_dir: &Path) -> Result<()> {
    let action = fs::read_to_string(flake_dir.join(".github/actions/setup-ci/action.yml"))
        .context("reading checked-in Cachix setup action")?;
    let filter = parse_push_filter(&action).context("parsing checked-in Cachix pushFilter")?;
    let finals = CacheBoundaryOutput::FINALS
        .into_iter()
        .map(|output| {
            Ok((
                output.name(),
                output
                    .eval_out_path(flake_dir)
                    .with_context(|| format!("evaluating final {} output path", output.name()))?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let supports = CacheBoundaryOutput::SUPPORTS
        .into_iter()
        .map(|output| {
            Ok((
                output.name(),
                output
                    .eval_out_path(flake_dir)
                    .with_context(|| format!("evaluating support {} output path", output.name()))?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    verify_cache_boundary_with(&filter, finals, supports, grep_filter_matches)
}

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
        match (self.remove)(&self.repo_root, &self.path) {
            Ok(status) if status.success() => {}
            _ => {
                let _ = writeln!(
                    self.stderr,
                    "xtask: warning: xtask.nix.probe_worktree_cleanup: ignored failure while removing probe worktree"
                );
            }
        }
    }
}

fn git_run(dir: &Path, args: &[&str]) -> Result<()> {
    let mut full = vec!["-c", "core.hooksPath="];
    full.extend_from_slice(args);
    git::run(dir, &full)
}

fn worktree_registered(repo_root: &Path, path: &Path) -> Result<bool> {
    let output = git::at(repo_root)
        .args(["worktree", "list", "--porcelain", "-z"])
        .output()
        .context("querying registered Git worktrees")?;
    if !output.status.success() {
        bail!(
            "querying registered Git worktrees failed with {}",
            output.status
        );
    }
    let path = path.to_str().context("worktree path is not UTF-8")?;
    Ok(output.stdout.split(|byte| *byte == 0).any(|field| {
        std::str::from_utf8(field)
            .ok()
            .and_then(|field| field.strip_prefix("worktree "))
            == Some(path)
    }))
}

fn run_probe() -> Result<()> {
    let repo_root = std::env::current_dir().context("resolving cwd")?;
    let tmp = repo_root.join(WORKTREE_DIR);
    fs::create_dir_all(repo_root.join(".xtask")).context("creating .xtask")?;
    if worktree_registered(&repo_root, &tmp)? {
        let status = git::at(&repo_root)
            .args(["-c", "core.hooksPath=", "worktree", "remove", "--force"])
            .arg(&tmp)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("removing stale nix-source-probe worktree")?;
        if !status.success() {
            bail!("removing stale nix-source-probe worktree failed with {status}");
        }
    }

    let tmp_str = tmp.to_str().context("worktree path is not UTF-8")?;
    git_run(
        &repo_root,
        &["worktree", "add", "--detach", tmp_str, "HEAD"],
    )?;
    let _guard = WorktreeGuard {
        repo_root: repo_root.clone(),
        path: tmp.clone(),
        remove: Box::new(|root, path| {
            git::at(root)
                .args(["-c", "core.hooksPath=", "worktree", "remove", "--force"])
                .arg(path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
        }),
        stderr: Box::new(std::io::stderr()),
    };

    // Every state is dirty. This keeps Nix's flake fetcher off grafted-away
    // history on shallow CI checkouts, while the shared change cancels out.
    let readme = tmp.join("README.md");
    let mut readme_bytes = fs::read(&readme).context("reading README.md to dirty it")?;
    readme_bytes.push(b'\n');
    fs::write(&readme, readme_bytes).context("dirtying README.md")?;

    let baseline = eval_paths(&tmp).context("evaluating baseline derivation identities")?;
    for arm in Arm::ALL {
        let marker = tmp.join(arm.marker_path());
        fs::write(&marker, format!("// nix source probe: {}\n", arm.name()))
            .with_context(|| format!("writing {} marker", arm.name()))?;
        git_run(&tmp, &["add", arm.marker_path()])?;
        let changed = eval_paths(&tmp).with_context(|| {
            format!(
                "evaluating {} perturbation derivation identities",
                arm.name()
            )
        })?;
        compare_arm(&baseline, arm, &changed)
            .with_context(|| format!("checking {} perturbation", arm.name()))?;
        git_run(&tmp, &["rm", "--cached", "--quiet", arm.marker_path()])?;
        fs::remove_file(marker).with_context(|| format!("removing {} marker", arm.name()))?;
    }
    verify_cache_boundary(&tmp).context("checking Cachix direct-filter boundary")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(static_docs: &str, static_code: &str, site: &str, wasm_tests: &str) -> DrvPaths {
        DrvPaths {
            static_docs: static_docs.to_owned(),
            static_code: static_code.to_owned(),
            site: site.to_owned(),
            wasm_tests: wasm_tests.to_owned(),
        }
    }

    #[test]
    fn matrix_accepts_exact_expected_changes() {
        let base = paths("docs", "code", "site", "wasm");
        for (arm, changed) in [
            (Arm::Docs, paths("docs-2", "code", "site", "wasm")),
            (Arm::DocsArchive, paths("docs", "code", "site", "wasm")),
            (Arm::Server, paths("docs", "code-2", "site", "wasm")),
            (Arm::Web, paths("docs", "code-2", "site-2", "wasm")),
            (Arm::Common, paths("docs", "code-2", "site-2", "wasm-2")),
            (Arm::Macros, paths("docs", "code-2", "site-2", "wasm-2")),
        ] {
            assert_eq!(compare_arm(&base, arm, &changed), Ok(()));
        }
    }

    #[test]
    fn matrix_rejects_missing_required_fan_out() {
        let base = paths("docs", "code", "site", "wasm");
        assert!(matches!(
            compare_arm(
                &base,
                Arm::Common,
                &paths("docs", "code", "site-2", "wasm-2")
            ),
            Err(ProbeError::MissingChange {
                boundary: "static-code",
                ..
            })
        ));
    }

    #[test]
    fn matrix_rejects_over_inclusion() {
        let base = paths("docs", "code", "site", "wasm");
        assert!(matches!(
            compare_arm(
                &base,
                Arm::Server,
                &paths("docs", "code-2", "site-2", "wasm")
            ),
            Err(ProbeError::UnexpectedChange {
                boundary: "site",
                ..
            })
        ));
    }

    #[test]
    fn rejects_malformed_evaluation_output() {
        assert!(parse_drv_path("site", "not-a-derivation").is_err());
        assert!(parse_drv_path("site", "").is_err());
    }

    #[test]
    fn propagates_evaluation_failure() {
        let error = eval_paths_with(|| anyhow::bail!("nix eval failed")).unwrap_err();
        assert!(format!("{error:#}").contains("nix eval failed"));
    }

    const STORE: &str = "/nix/store/0123456789abcdefghijklmnopqrstuv";

    fn checked_filter() -> String {
        parse_push_filter(include_str!("../../.github/actions/setup-ci/action.yml")).unwrap()
    }
    fn output(name: &str) -> String {
        format!("{STORE}-{name}")
    }

    #[test]
    fn cache_filter_matches_only_exact_final_output_basenames() {
        let filter = checked_filter();
        for final_output in [
            "jaunder-coverage-0.1.0",
            "jaunder-coverage-gate",
            "vm-test-run-jaunder-e2e-sqlite-chromium",
            "vm-test-run-jaunder-e2e-sqlite-firefox",
            "vm-test-run-jaunder-e2e-postgres-chromium",
            "vm-test-run-jaunder-e2e-postgres-firefox",
            "jaunder-e2e-checks",
        ] {
            assert!(
                grep_filter_matches(&filter, &output(final_output)).unwrap(),
                "{final_output}"
            );
        }
        for support_or_lookalike in [
            "jaunder-coverage-source-probe",
            "jaunder-e2e",
            "vm-test-run-jaunder-e2e-sqlite-chromium-single-worker",
            "prefix-vm-test-run-jaunder-e2e-sqlite-chromium",
            "vm-test-run-jaunder-e2e-sqlite-chromium-suffix",
        ] {
            assert!(
                !grep_filter_matches(&filter, &output(support_or_lookalike)).unwrap(),
                "{support_or_lookalike}"
            );
        }
    }

    #[test]
    fn cache_boundary_rejects_a_reusable_final() {
        let error = verify_cache_boundary_with(
            &checked_filter(),
            [("coverage", output("jaunder-coverage-0.1.0"))],
            [],
            |_, _| Ok(false),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("final verdict coverage is reusable"));
    }

    #[test]
    fn cache_boundary_rejects_an_excluded_support() {
        let error = verify_cache_boundary_with(
            &checked_filter(),
            [],
            [("e2e package", output("jaunder-e2e"))],
            |_, _| Ok(true),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("cacheable support e2e package is excluded"));
    }

    #[test]
    fn cache_boundary_propagates_grep_indeterminate_error() {
        let error = verify_cache_boundary_with(
            &checked_filter(),
            [("coverage", output("jaunder-coverage-0.1.0"))],
            [],
            |_, _| anyhow::bail!("grep failed with status 2"),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("grep failed with status 2"));
    }

    #[test]
    fn push_filter_requires_one_quoted_scalar() {
        for action in [
            "",
            "pushFilter: unquoted",
            "pushFilter: 'unterminated",
            "pushFilter: 'one'\npushFilter: 'two'",
        ] {
            assert!(parse_push_filter(action).is_err(), "{action:?}");
        }
        assert_eq!(
            parse_push_filter("pushFilter: '^final$'").unwrap(),
            "^final$"
        );
    }
}
