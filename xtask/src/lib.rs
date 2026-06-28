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
    /// Coverage-baseline maintenance.
    #[command(subcommand)]
    Coverage(CoverageCommand),
}

/// `coverage` subcommands.
#[derive(Subcommand)]
pub enum CoverageCommand {
    /// Re-anchor `coverage-baseline.json` to the current coverage report when the
    /// drift is a safe line-shift (ADR-0030); refuse and write a candidate to
    /// `.xtask/coverage-baseline.candidate.json` on a genuine coverage lowering.
    /// Consumes an existing report (run `check`/`validate` first); never rebuilds.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask coverage reanchor\n  \
        cargo xtask coverage reanchor --gcroot .xtask/gcroots/coverage")]
    Reanchor {
        /// GC-root / out-link directory holding `coverage-report.txt`.
        #[arg(long, default_value = ".xtask/gcroots/coverage")]
        gcroot: String,
    },
}

impl Cli {
    pub fn command_name(&self) -> &'static str {
        match self.command {
            Command::Check { .. } => "check",
            Command::Validate { .. } => "validate",
            Command::AuditWasm { .. } => "audit-wasm",
            Command::Coverage(CoverageCommand::Reanchor { .. }) => "coverage-reanchor",
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
        Command::Coverage(CoverageCommand::Reanchor { gcroot }) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("coverage-reanchor");
            result.push(coverage::reanchor(&gcroot));
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

/// Whether the `coverage-keepours` merge driver needs (re)registering, given the
/// current `merge.coverage-keepours.driver` value (`None` = unset). The driver command
/// is the shell builtin `true`; any other value (or unset) means re-register.
fn needs_merge_driver(current: Option<&str>) -> bool {
    match current {
        Some(value) => value.trim() != "true",
        None => true,
    }
}

/// Current `merge.coverage-keepours.driver` in `repo_dir`, or `None` when unset/blank.
/// `git config --get` exits non-zero (empty stdout) when the key is missing, so a blank
/// read maps to `None`. Goes through `git_at` so ambient `GIT_DIR`/etc. (exported when
/// run inside a hook) cannot redirect the query at another repo.
fn merge_driver_value(repo_dir: &std::path::Path) -> Option<String> {
    let out = git_at(repo_dir)
        .args(["config", "--get", "merge.coverage-keepours.driver"])
        .output()
        .ok()?;
    let value = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Ensure the keep-ours merge driver is registered in `repo_dir`; register it when
/// unset/wrong. Returns `true` when it changed config. Mirrors [`git::ensure_hooks_path`].
fn ensure_merge_driver(repo_dir: &std::path::Path) -> anyhow::Result<bool> {
    if needs_merge_driver(merge_driver_value(repo_dir).as_deref()) {
        register_keepours(repo_dir)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Self-healing merge-driver registration: register the keep-ours driver for the
/// generated coverage artifacts if it is not already, so fresh clones wire up on first
/// run. Git config is shared per-clone, so this also covers every worktree. Best-effort —
/// a failure here must never block the actual command. Parallels [`ensure_hooks_installed`].
pub fn ensure_merge_driver_installed() {
    match ensure_merge_driver(std::path::Path::new(".")) {
        Ok(true) => eprintln!("xtask: registered merge.coverage-keepours (keep-ours)"),
        Ok(false) => {}
        Err(e) => {
            eprintln!("xtask: warning: could not register merge.coverage-keepours: {e:#}")
        }
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

    #[test]
    fn coverage_reanchor_parses_with_default_gcroot() {
        let cli = Cli::try_parse_from(["xtask", "coverage", "reanchor"]).unwrap();
        match cli.command {
            Command::Coverage(CoverageCommand::Reanchor { gcroot }) => {
                assert_eq!(gcroot, ".xtask/gcroots/coverage");
            }
            _ => panic!("expected coverage reanchor"),
        }
    }

    #[test]
    fn coverage_reanchor_accepts_gcroot() {
        let cli =
            Cli::try_parse_from(["xtask", "coverage", "reanchor", "--gcroot", "/tmp/x"]).unwrap();
        match cli.command {
            Command::Coverage(CoverageCommand::Reanchor { gcroot }) => assert_eq!(gcroot, "/tmp/x"),
            _ => panic!("expected coverage reanchor"),
        }
    }
}

#[cfg(test)]
mod merge_driver_tests {
    use super::{ensure_merge_driver, git_at, needs_merge_driver, register_keepours};

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
        // (a throwaway test repo, or the user's repo via the merge-driver self-heal)
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

    #[test]
    fn needs_merge_driver_when_unset_or_wrong() {
        assert!(needs_merge_driver(None));
        assert!(needs_merge_driver(Some("")));
        assert!(needs_merge_driver(Some("false")));
    }

    #[test]
    fn no_need_when_merge_driver_already_true() {
        assert!(!needs_merge_driver(Some("true")));
        assert!(!needs_merge_driver(Some(" true \n")));
    }

    #[test]
    fn ensure_merge_driver_registers_then_is_idempotent() {
        let tmp = std::env::temp_dir().join(format!("jaunder-ensure-md-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        git(&tmp, &["init", "-q"]);
        // First call registers and reports a change.
        assert!(ensure_merge_driver(&tmp).unwrap(), "first call registers");
        assert_eq!(
            git_stdout(&tmp, &["config", "--get", "merge.coverage-keepours.driver"]),
            "true"
        );
        // Second call is a no-op (idempotent) — the value already matches.
        assert!(
            !ensure_merge_driver(&tmp).unwrap(),
            "second call is a no-op"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
