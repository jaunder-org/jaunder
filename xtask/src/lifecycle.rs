use std::path::Path;

use crate::{
    git,
    result::{CommandResult, StepResult},
};

/// Self-healing hook installation: point `core.hooksPath` at `.githooks` if it is not
/// already, so fresh clones and new worktrees wire up on first run. Best-effort — a
/// failure here must never block the actual command.
pub fn ensure_hooks_installed() {
    match git::ensure_hooks_path(Path::new(".")) {
        Ok(true) => eprintln!("xtask: set core.hooksPath = {}", git::HOOKS_PATH),
        Ok(false) => {}
        Err(e) => eprintln!("xtask: warning: could not set core.hooksPath: {e:#}"),
    }
}

pub(crate) fn finalize(result: &mut CommandResult, start: std::time::Instant) {
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
pub(crate) fn clean_tree_precheck(allow_dirty: bool) -> StepResult {
    if allow_dirty {
        return StepResult::skip("clean-tree").detail("--allow-dirty");
    }
    match git::working_tree_status(Path::new(".")) {
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

pub(crate) fn run_precommit_with_host_gate(
    dir: &Path,
    run_gate: impl FnOnce(git::PrecommitChangeClass, &mut CommandResult),
) -> anyhow::Result<CommandResult> {
    let start = std::time::Instant::now();
    let before = git::status_snapshot(dir)?;
    let class = git::classify_precommit_change(&before);
    let mut result = CommandResult::new("precommit");
    result.push(StepResult::ok("precommit-routing").detail(class.detail()));
    run_gate(class, &mut result);
    let after = git::status_snapshot(dir)?;
    let plan = git::precommit_stage_plan(&before, &after);
    result.push(git::apply_precommit_stage_plan(dir, &plan));
    finalize(&mut result, start);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cli::Command, gate, result::Mode, steps};

    #[test]
    fn precommit_reconciles_once_after_an_early_static_failure() {
        let dir = crate::test_support::temp_repo("precommit", "early-failure-reconcile");
        crate::test_support::commit(&dir, "a.rs", "fn a(){}\n");
        crate::test_support::write(&dir, "a.rs", "fn a() { }\n");
        crate::test_support::git_ok(&dir, &["add", "a.rs"]);
        let sh = xshell::Shell::new().expect("create shell");
        let first_static_spec = steps::static_checks::specs_for_phase(
            steps::static_checks::Phase::SourceConsistency,
            Mode::Fix,
        )
        .first()
        .expect("source-consistency phase has a spec")
        .name;
        let mut invoked = Vec::new();

        let result = run_precommit_with_host_gate(&dir, |_, result| {
            gate::run_host_steps_with(
                &sh,
                Mode::Fix,
                gate::execution_policy(&Command::Precommit),
                result,
                gate::HOST_GATE_NON_TEST_STEPS,
                |step, sh, mode, policy, result| match step {
                    gate::HostGateStep::StaticChecks(phase) => {
                        gate::run_static_phase_with(sh, mode, *phase, policy, result, |_, spec| {
                            invoked.push(spec.name);
                            crate::test_support::write(&dir, "a.rs", "fn a() { }\n// formatted\n");
                            StepResult::fail(spec.name)
                                .detail("static diagnostic survives reconciliation")
                        });
                    }
                    gate::HostGateStep::ResultOnly { name, .. } => {
                        panic!("later host step {name} ran after static failure")
                    }
                    gate::HostGateStep::HostTests { .. } => {
                        panic!("host tests ran after static failure")
                    }
                },
            );
        })
        .expect("precommit orchestration");

        assert_eq!(invoked, [first_static_spec]);
        assert_eq!(
            result
                .steps
                .iter()
                .filter(|step| step.name == "precommit-staging")
                .count(),
            1
        );
        assert_eq!(
            result
                .steps
                .iter()
                .find(|step| step.name == first_static_spec)
                .and_then(|step| step.detail.as_deref()),
            Some("static diagnostic survives reconciliation")
        );
        assert_eq!(
            git::output(&dir, &["diff", "--cached", "--name-only"]).expect("cached paths"),
            "a.rs"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn precommit_orchestration_restages_safe_fixture() {
        let dir = crate::test_support::temp_repo("precommit", "orchestration-safe");
        crate::test_support::commit(&dir, "a.rs", "fn a(){}\n");
        crate::test_support::commit(&dir, "b.rs", "fn b(){}\n");
        crate::test_support::write(&dir, "a.rs", "fn a() { }\n");
        crate::test_support::git_ok(&dir, &["add", "a.rs"]);

        crate::test_support::write(&dir, "b.rs", "fn b() { }\n");
        let result = run_precommit_with_host_gate(&dir, |class, result| {
            gate::dispatch_precommit_host_gate_with(
                class,
                result,
                |_| panic!("non-Markdown or mixed state must not dispatch to the narrow graph"),
                |result| {
                    crate::test_support::write(&dir, "a.rs", "fn a() { }\n// formatted\n");
                    result.push(StepResult::ok("fake-complete-fix-host-gate"));
                },
            );
        })
        .unwrap();

        assert!(result.ok);
        assert_eq!(result.steps[0].name, "precommit-routing");
        assert_eq!(
            result.steps[0].detail.as_deref(),
            Some("class=broad reason=unstaged-path")
        );
        assert_eq!(
            git::output(&dir, &["diff", "--cached", "--name-only"]).unwrap(),
            "a.rs"
        );
        assert_eq!(git::output(&dir, &["diff", "--name-only"]).unwrap(), "b.rs");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn precommit_routes_staged_markdown_before_restaging_safe_formatter_output() {
        let dir = crate::test_support::temp_repo("precommit", "markdown-routing");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);

        let result = run_precommit_with_host_gate(&dir, |class, result| {
            gate::dispatch_precommit_host_gate_with(
                class,
                result,
                |result| {
                    crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\nFormatted\n");
                    result.push(StepResult::ok("fake-markdown-host-gate"));
                },
                |_| panic!("staged Markdown must not dispatch to the broad host graph"),
            );
        })
        .expect("precommit orchestration");

        assert_eq!(result.steps[0].name, "precommit-routing");
        assert_eq!(
            result.steps[0].detail.as_deref(),
            Some("class=staged-markdown-only reason=isolated-staged-markdown")
        );
        assert!(result.ok);
        assert_eq!(
            git::output(&dir, &["diff", "--cached", "--name-only"]).expect("cached paths"),
            "guide.md"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn narrow_dispatch_failure_still_reconciles_exactly_once() {
        let dir = crate::test_support::temp_repo("precommit", "markdown-failure");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);

        let result = run_precommit_with_host_gate(&dir, |class, result| {
            gate::dispatch_precommit_host_gate_with(
                class,
                result,
                |result| {
                    result.push(StepResult::fail("prettier-markdown").detail("synthetic failure"));
                },
                |_| panic!("staged Markdown must not dispatch to the broad host graph"),
            );
        })
        .expect("precommit orchestration");

        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>(),
            [
                "precommit-routing",
                "prettier-markdown",
                "precommit-staging"
            ]
        );
        assert_eq!(
            result
                .steps
                .iter()
                .filter(|step| step.name == "precommit-staging")
                .count(),
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn precommit_orchestration_fails_clean_before_tracked_mutation() {
        let dir = crate::test_support::temp_repo("precommit", "orchestration-unsafe");
        crate::test_support::commit(&dir, "clean.rs", "one\n");

        let result = run_precommit_with_host_gate(&dir, |_, result| {
            crate::test_support::write(&dir, "clean.rs", "two\n");
            result.push(StepResult::ok("fake-host-gate"));
        })
        .unwrap();

        assert!(!result.ok);
        let staging = result
            .steps
            .iter()
            .find(|s| s.name == "precommit-staging")
            .unwrap();
        assert!(
            staging
                .detail
                .as_deref()
                .unwrap()
                .contains("will not add work the user did not stage")
        );
        assert!(
            git::output(&dir, &["diff", "--cached", "--name-only"])
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
