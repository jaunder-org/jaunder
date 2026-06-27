use clap::{Parser, Subcommand};

mod audit_wasm;
mod coverage;
pub mod git;
mod result;
mod sh;
mod steps {
    pub mod host_tests;
    pub mod nix;
    pub mod static_checks;
}
pub use result::{CommandResult, Mode, StepResult};

#[derive(Parser)]
#[command(name = "xtask", about = "Jaunder dev orchestration")]
pub struct Cli {
    /// Emit the structured result envelope as JSON to stdout.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Inner loop (auto-fixes formatting): host static checks + clippy + the host
    /// xtask unit suite, then the Nix coverage check (instrumented test suite +
    /// coverage). `--no-test` skips only the Nix coverage check; static, clippy,
    /// and the xtask unit tests still run.
    Check {
        /// Skip the Nix coverage check — static + clippy + host xtask unit tests only.
        #[arg(long)]
        no_test: bool,
    },
    /// Full gate (never mutates the tree): static + clippy + the host xtask unit
    /// suite (verify-only) + the Nix coverage check + the e2e VMs. `--no-e2e` skips
    /// the e2e VMs. Refuses a dirty working tree unless `--allow-dirty`.
    Validate {
        /// Skip the e2e VM checks — static + clippy + xtask tests + coverage only.
        #[arg(long)]
        no_e2e: bool,
        /// Run even when the working tree is dirty (skip the clean-tree precheck).
        #[arg(long)]
        allow_dirty: bool,
    },
    /// Regenerate the accepted-uncovered baseline (`coverage-baseline.json`)
    /// from a coverage check's `coverage-report.txt`. One-shot, hidden helper.
    #[command(name = "__regen-baseline", hide = true)]
    RegenBaseline {
        /// GC-root / out-link directory holding `coverage-report.txt`.
        #[arg(long, default_value = ".xtask/gcroots/coverage")]
        gcroot: String,
    },
    /// Measure the frontend WASM/JS bundle size — raw, gzip, and brotli.
    ///
    /// Reports the download weight of the deterministic `nix build .#site`
    /// output (`pkg/jaunder_bg.wasm`, `pkg/jaunder.js`) so you can catch
    /// bundle-size bloat before it ships and compare a change's effect on what
    /// users download. Run it after a change you expect to move the bundle (a new
    /// dependency, a feature touching the client), or periodically to watch the
    /// trend. This is a manual tool — it is not part of `check`/`validate`.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask audit-wasm\n  \
        cargo xtask audit-wasm --site-path /nix/store/...-jaunder-site\n  \
        cargo xtask --json audit-wasm")]
    AuditWasm {
        /// Audit a prebuilt `.#site` store path instead of running `nix build`.
        #[arg(long)]
        site_path: Option<String>,
    },
    /// Register the keep-ours git merge driver for the generated coverage
    /// artifacts. `.gitattributes` maps `coverage-baseline.json` and
    /// `crap-manifest.json` to `merge=coverage-keepours`; git config is not
    /// version-controlled, so this one-shot wires the driver into the local
    /// clone (run once per clone/worktree).
    #[command(name = "install-merge-driver")]
    InstallMergeDriver,
}

impl Cli {
    pub fn command_name(&self) -> &'static str {
        match self.command {
            Command::Check { .. } => "check",
            Command::Validate { .. } => "validate",
            Command::RegenBaseline { .. } => "__regen-baseline",
            Command::AuditWasm { .. } => "audit-wasm",
            Command::InstallMergeDriver => "install-merge-driver",
        }
    }
}

pub fn run(cli: Cli) -> anyhow::Result<CommandResult> {
    match cli.command {
        Command::Check { no_test } => {
            let sh = xshell::Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("check");
            steps::static_checks::run(&sh, Mode::Fix, &mut result);
            steps::host_tests::run(&sh, &mut result);
            if !no_test {
                steps::nix::coverage(&mut result, Mode::Fix);
            }
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Validate {
            no_e2e,
            allow_dirty,
        } => {
            let sh = xshell::Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("validate");
            // Clean-tree backstop: refuse a dirty tree so what is measured equals the
            // committed tip (== what CI sees). Fail fast before the expensive steps.
            let precheck = clean_tree_precheck(&sh, allow_dirty);
            let blocked = !precheck.ok && !precheck.skipped;
            result.push(precheck);
            if blocked {
                finalize(&mut result, start);
                return Ok(result);
            }
            steps::static_checks::run(&sh, Mode::Check, &mut result);
            steps::host_tests::run(&sh, &mut result);
            steps::nix::coverage(&mut result, Mode::Check);
            if !no_e2e {
                steps::nix::e2e(&mut result);
            }
            finalize(&mut result, start);
            Ok(result)
        }
        Command::RegenBaseline { gcroot } => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("__regen-baseline");
            let step = regen_baseline(&gcroot);
            result.push(step);
            finalize(&mut result, start);
            Ok(result)
        }
        Command::InstallMergeDriver => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("install-merge-driver");
            result.push(install_merge_driver());
            finalize(&mut result, start);
            Ok(result)
        }
        Command::AuditWasm { site_path } => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("audit-wasm");
            match audit_wasm::run(site_path.as_deref()) {
                Ok(report) => {
                    let n = report.artifacts.len();
                    result.audit = Some(report);
                    result.push(StepResult::ok("audit-wasm").detail(format!("{n} artifact(s)")));
                }
                Err(e) => {
                    result.push(StepResult::fail("audit-wasm").detail(format!("{e:#}")));
                }
            }
            finalize(&mut result, start);
            Ok(result)
        }
    }
}

/// Self-healing hook installation: point `core.hooksPath` at `.githooks` if it is not
/// already, so fresh clones and new worktrees wire up on first run. Best-effort — a
/// failure here must never block the actual command.
pub fn ensure_hooks_installed() {
    let Ok(sh) = xshell::Shell::new() else {
        return;
    };
    match git::ensure_hooks_path(&sh) {
        Ok(true) => eprintln!("xtask: set core.hooksPath = {}", git::HOOKS_PATH),
        Ok(false) => {}
        Err(e) => eprintln!("xtask: warning: could not set core.hooksPath: {e:#}"),
    }
}

/// One-shot: parse `<gcroot>/coverage-report.txt`, build the accepted-uncovered
/// baseline from the currently-uncovered executable lines, and write it to
/// `coverage-baseline.json` (repo-relative paths via `git rev-parse`).
fn regen_baseline(gcroot: &str) -> StepResult {
    match regen_baseline_inner(gcroot) {
        Ok(n) => StepResult::ok("regen-baseline").detail(format!(
            "wrote coverage-baseline.json ({n} file(s) with gaps)"
        )),
        Err(e) => StepResult::fail("regen-baseline").detail(e.to_string()),
    }
}

fn regen_baseline_inner(gcroot: &str) -> anyhow::Result<usize> {
    use anyhow::Context as _;
    let report_path = format!("{gcroot}/coverage-report.txt");
    let report =
        std::fs::read_to_string(&report_path).with_context(|| format!("reading {report_path}"))?;
    let repo_root = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("running git rev-parse --show-toplevel")?
            .stdout,
    )?;
    let repo_root = repo_root.trim();
    let files = coverage::report::parse_text_report(&report, repo_root);
    let baseline = coverage::baseline::Baseline::from_files(&files);
    baseline.save("coverage-baseline.json")?;
    let with_gaps = files
        .iter()
        .filter(|f| f.lines.iter().any(|l| !l.covered))
        .count();
    Ok(with_gaps)
}

/// A `git -C <repo_dir>` command scrubbed of the ambient env vars that redirect
/// git at a different repository. A git hook (e.g. `.githooks/pre-push` running
/// `cargo xtask validate`) exports `GIT_DIR`/`GIT_INDEX_FILE`; those would make
/// `git -C <repo_dir>` operate on the HOOK's repo instead of `repo_dir`, so a
/// command meant for `repo_dir` (or a throwaway test repo) could corrupt the
/// surrounding worktree. Clearing them pins the target to `-C <repo_dir>`.
fn git_at(repo_dir: &std::path::Path) -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(repo_dir);
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

/// Register the keep-ours merge driver in `repo_dir`'s local git config. The
/// driver command is `true`: it exits 0 without touching `%A` (ours), so a merge
/// of the generated coverage artifacts resolves to our side with no conflict
/// markers. The next `cargo xtask check` re-heals to the merged-tree state.
fn register_keepours(repo_dir: &std::path::Path) -> anyhow::Result<()> {
    use anyhow::ensure;
    let cfg = |args: &[&str]| -> anyhow::Result<()> {
        let status = git_at(repo_dir).args(args).status()?;
        ensure!(status.success(), "git {:?} failed", args);
        Ok(())
    };
    cfg(&[
        "config",
        "merge.coverage-keepours.name",
        "keep ours for generated coverage artifacts",
    ])?;
    cfg(&["config", "merge.coverage-keepours.driver", "true"])?;
    Ok(())
}

fn install_merge_driver() -> StepResult {
    match register_keepours(std::path::Path::new(".")) {
        Ok(()) => StepResult::ok("install-merge-driver")
            .detail("registered merge.coverage-keepours (keep-ours)"),
        Err(e) => StepResult::fail("install-merge-driver").detail(e.to_string()),
    }
}

fn finalize(result: &mut CommandResult, start: std::time::Instant) {
    result.duration_ms = start.elapsed().as_millis();
    result.finished_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
}

/// The clean-tree precheck step for `validate`. With `--allow-dirty`, a skip.
/// Otherwise: `ok` when the tree is clean; `fail` when dirty (detail = the porcelain
/// status) or when git cannot be queried — the gate refuses to certify a tree it
/// cannot prove clean. `check` deliberately has no such precheck (Fix-mode runs on a
/// dirty tree by design).
fn clean_tree_precheck(sh: &xshell::Shell, allow_dirty: bool) -> StepResult {
    if allow_dirty {
        return StepResult::skip("clean-tree").detail("--allow-dirty");
    }
    match git::working_tree_status(sh) {
        Ok(status) if git::porcelain_is_dirty(&status) => {
            StepResult::fail("clean-tree").detail(format!(
                "working tree is dirty — commit/stash or pass --allow-dirty:\n{}",
                status.trim()
            ))
        }
        Ok(_) => StepResult::ok("clean-tree"),
        Err(e) => {
            StepResult::fail("clean-tree").detail(format!("could not determine cleanliness: {e:#}"))
        }
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn validate_allow_dirty_parses() {
        let cli = Cli::try_parse_from(["xtask", "validate", "--allow-dirty"]).unwrap();
        match cli.command {
            Command::Validate {
                no_e2e,
                allow_dirty,
            } => {
                assert!(!no_e2e);
                assert!(allow_dirty);
            }
            _ => panic!("expected validate"),
        }
    }

    #[test]
    fn validate_defaults_reject_dirty() {
        let cli = Cli::try_parse_from(["xtask", "validate"]).unwrap();
        match cli.command {
            Command::Validate { allow_dirty, .. } => assert!(!allow_dirty),
            _ => panic!("expected validate"),
        }
    }
}

#[cfg(test)]
mod merge_driver_tests {
    use super::{git_at, register_keepours};

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = git_at(dir).args(args).status().unwrap().success();
        assert!(ok, "git {args:?} failed");
    }

    fn git_stdout(dir: &std::path::Path, args: &[&str]) -> String {
        let out = git_at(dir).args(args).output().unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    #[test]
    fn git_at_scrubs_repo_redirecting_env() {
        // Regression guard: without scrubbing these, a git op meant for `dir`
        // (a throwaway test repo, or the user's repo via install-merge-driver)
        // would be redirected at the hook's repo when run inside a git hook,
        // corrupting it. `get_envs()` yields `(key, None)` for a removed var.
        let cmd = git_at(std::path::Path::new("/tmp/x"));
        let removed: std::collections::HashSet<std::ffi::OsString> = cmd
            .get_envs()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.to_owned())
            .collect();
        for var in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_COMMON_DIR",
            "GIT_NAMESPACE",
        ] {
            assert!(
                removed.contains(std::ffi::OsStr::new(var)),
                "{var} must be scrubbed so -C wins"
            );
        }
    }

    #[test]
    fn keepours_driver_resolves_merge_to_ours_without_markers() {
        let tmp = std::env::temp_dir().join(format!("jaunder-mergetest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        git(&tmp, &["init", "-q"]);
        git(&tmp, &["config", "user.email", "t@t"]);
        git(&tmp, &["config", "user.name", "t"]);
        register_keepours(&tmp).unwrap();
        std::fs::write(
            tmp.join(".gitattributes"),
            "crap-manifest.json merge=coverage-keepours\n",
        )
        .unwrap();
        std::fs::write(tmp.join("crap-manifest.json"), "base\n").unwrap();
        git(&tmp, &["add", "."]);
        git(&tmp, &["commit", "-q", "-m", "base"]);
        // The default branch name varies (main vs master) — capture it.
        let base = git_stdout(&tmp, &["branch", "--show-current"]);

        git(&tmp, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(tmp.join("crap-manifest.json"), "theirs\n").unwrap();
        git(&tmp, &["commit", "-qam", "theirs"]);

        git(&tmp, &["checkout", "-q", &base]);
        std::fs::write(tmp.join("crap-manifest.json"), "ours\n").unwrap();
        git(&tmp, &["commit", "-qam", "ours"]);

        // Merge must succeed (exit 0) and keep "ours" with no conflict markers.
        let merged = git_at(&tmp)
            .args(["merge", "-q", "--no-edit", "feature"])
            .status()
            .unwrap();
        assert!(merged.success(), "keep-ours merge must not conflict");
        let content = std::fs::read_to_string(tmp.join("crap-manifest.json")).unwrap();
        assert_eq!(content, "ours\n", "keep-ours must retain our side");
        assert!(!content.contains("<<<<<<<"), "no conflict markers");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
