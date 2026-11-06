use std::{path::Path, process::Command};

use crate::{
    git,
    result::{CommandResult, StepResult},
};

/// Self-healing hook installation: point `core.hooksPath` at `.githooks` if it is not
/// already, so fresh clones and new worktrees wire up on first run. Best-effort — a
/// failure here must never block the actual command.
pub fn ensure_hooks_installed() {
    let phase =
        crate::invocation_trace::active().then(|| crate::invocation_trace::phase("xtask.hooks"));
    let ok = match git::ensure_hooks_path(Path::new(".")) {
        Ok(true) => {
            eprintln!("xtask: set core.hooksPath = {}", git::HOOKS_PATH);
            true
        }
        Ok(false) => true,
        Err(e) => {
            eprintln!("xtask: warning: could not set core.hooksPath: {e:#}");
            false
        }
    };
    if let Some(phase) = phase {
        phase.finish(ok);
    }
}

pub(crate) fn finalize(result: &mut CommandResult, start: std::time::Instant) {
    result.duration_ms = start.elapsed().as_millis();
    result.finished_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
}

/// Run Just's selected precommit host recipe inside the Git/index safety
/// boundary. Just owns the check population and ordering; this adapter owns only
/// admission, one reconciliation attempt, and the conditional docs verdict.
pub(crate) fn run_precommit_safety(dir: &Path) -> Result<(), String> {
    run_precommit_safety_with(
        dir,
        cfg!(all(target_os = "linux", target_arch = "x86_64")),
        |class| {
            let recipe = match class {
                git::PrecommitChangeClass::StagedMarkdownOnly => "precommit-markdown-host",
                git::PrecommitChangeClass::Broad(_) => "precommit-broad-host",
            };
            let status = Command::new("just")
                .arg(recipe)
                .current_dir(dir)
                .status()
                .map_err(|error| format!("could not run `just {recipe}`: {error}"))?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("`just {recipe}` exited {status}"))
            }
        },
        |_| crate::steps::nix::shared_static_docs_check(),
    )
}

fn run_precommit_safety_with(
    dir: &Path,
    shared_docs_enabled: bool,
    run_host: impl FnOnce(git::PrecommitChangeClass) -> Result<(), String>,
    request: impl FnOnce(&Path) -> StepResult,
) -> Result<(), String> {
    run_precommit_safety_with_snapshot(
        dir,
        shared_docs_enabled,
        git::status_snapshot,
        run_host,
        request,
    )
}

fn run_precommit_safety_with_snapshot(
    dir: &Path,
    shared_docs_enabled: bool,
    snapshot: impl FnMut(&Path) -> anyhow::Result<git::GitStatusSnapshot>,
    run_host: impl FnOnce(git::PrecommitChangeClass) -> Result<(), String>,
    request: impl FnOnce(&Path) -> StepResult,
) -> Result<(), String> {
    let result = run_precommit_with_shared_verdict_for_platform_and_snapshot(
        dir,
        shared_docs_enabled,
        snapshot,
        |class, result| {
            result.push(match run_host(class) {
                Ok(()) => StepResult::ok("precommit-host"),
                Err(error) => StepResult::fail("precommit-host").detail(error),
            });
        },
        request,
    )
    .map_err(|error| format!("Git/index safety operation failed: {error:#}"))?;

    // This process boundary returns only success or failure text. It does not
    // serialize CommandResult, write last-result.json, or append a CI summary.
    let failures = result
        .steps
        .iter()
        .filter(|step| StepResult::is_blocking_failure(step))
        .map(|step| match step.detail.as_deref() {
            Some(detail) => format!("{}: {detail}", step.name),
            None => step.name.clone(),
        })
        .collect::<Vec<_>>();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

#[cfg(test)]
fn run_precommit_with_host_gate(
    dir: &Path,
    run_gate: impl FnOnce(git::PrecommitChangeClass, &mut CommandResult),
) -> anyhow::Result<CommandResult> {
    run_precommit_with_shared_verdict(dir, run_gate, |_| {
        #[cfg(test)]
        {
            StepResult::ok("mock-nix")
        }
        #[cfg(not(test))]
        {
            crate::steps::nix::shared_static_docs_check()
        }
    })
}

#[cfg(test)]
fn run_precommit_with_shared_verdict(
    dir: &Path,
    run_gate: impl FnOnce(git::PrecommitChangeClass, &mut CommandResult),
    request: impl FnOnce(&Path) -> StepResult,
) -> anyhow::Result<CommandResult> {
    run_precommit_with_shared_verdict_for_platform(
        dir,
        cfg!(all(target_os = "linux", target_arch = "x86_64")),
        run_gate,
        request,
    )
}

#[cfg(test)]
fn run_precommit_with_shared_verdict_for_platform(
    dir: &Path,
    shared_docs_enabled: bool,
    run_gate: impl FnOnce(git::PrecommitChangeClass, &mut CommandResult),
    request: impl FnOnce(&Path) -> StepResult,
) -> anyhow::Result<CommandResult> {
    run_precommit_with_shared_verdict_for_platform_and_snapshot(
        dir,
        shared_docs_enabled,
        git::status_snapshot,
        run_gate,
        request,
    )
}

fn run_precommit_with_shared_verdict_for_platform_and_snapshot(
    dir: &Path,
    shared_docs_enabled: bool,
    mut snapshot: impl FnMut(&Path) -> anyhow::Result<git::GitStatusSnapshot>,
    run_gate: impl FnOnce(git::PrecommitChangeClass, &mut CommandResult),
    request: impl FnOnce(&Path) -> StepResult,
) -> anyhow::Result<CommandResult> {
    let before = snapshot(dir)?;
    let class = git::classify_precommit_change(&before);
    let mut result = CommandResult::new("precommit");
    result.push(StepResult::ok("precommit-routing").detail(class.detail()));
    run_gate(class, &mut result);
    let after = match snapshot(dir) {
        Ok(after) => after,
        Err(error) => {
            result.push(StepResult::fail("precommit-staging").detail(format!(
                "after-snapshot failed; refusing staging: {error:#}"
            )));
            return Ok(result);
        }
    };
    let plan = git::precommit_stage_plan(&before, &after);
    result.push(git::apply_precommit_stage_plan(dir, &plan));
    let host_and_stage_succeeded = !result.steps.iter().any(StepResult::is_blocking_failure);
    if matches!(class, git::PrecommitChangeClass::StagedMarkdownOnly)
        && shared_docs_enabled
        && host_and_stage_succeeded
    {
        run_shared_static_docs_request(dir, false, || request(dir), &mut result);
    }
    Ok(result)
}

/// Admit, request, and then verify the post-request Git state without staging
/// or reconciling a second time.
pub(crate) fn run_shared_static_docs_request(
    dir: &Path,
    require_clean: bool,
    request: impl FnOnce() -> StepResult,
    result: &mut CommandResult,
) {
    let before = if require_clean {
        git::shared_static_docs_clean_admission(dir)
    } else {
        git::shared_markdown_admission(dir)
    };
    let before = match before {
        Ok(state) => {
            result.push(StepResult::ok("shared-static-docs-admission"));
            state
        }
        Err(error) => {
            result.push(
                StepResult::fail("shared-static-docs-admission").detail(format!("{error:#}")),
            );
            return;
        }
    };
    result.push(request());
    let post_check = match git::shared_verdict_state_unchanged(dir, &before) {
        Ok(()) => StepResult::ok("shared-static-docs-state"),
        Err(error) => StepResult::fail("shared-static-docs-state").detail(format!("{error:#}")),
    };
    result.push(post_check);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precommit_safety_preserves_host_failure_and_skips_docs_request() {
        let dir = crate::test_support::temp_repo("precommit-safety", "host-failure");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);
        let host_calls = std::cell::Cell::new(0);
        let request_calls = std::cell::Cell::new(0);

        let result = run_precommit_safety_with(
            &dir,
            true,
            |class| {
                host_calls.set(host_calls.get() + 1);
                assert_eq!(class, git::PrecommitChangeClass::StagedMarkdownOnly);
                Err("original host leaf failure".to_owned())
            },
            |_| {
                request_calls.set(request_calls.get() + 1);
                StepResult::ok("unexpected-static-docs")
            },
        )
        .expect_err("host failure must remain the process failure");

        assert_eq!(host_calls.get(), 1);
        assert_eq!(request_calls.get(), 0);
        assert!(result.contains("original host leaf failure"));
        assert_eq!(
            git::output(&dir, &["diff", "--cached", "--name-only"]).unwrap(),
            "guide.md"
        );
    }

    #[test]
    fn precommit_safety_retains_host_failure_when_after_snapshot_fails() {
        let dir = crate::test_support::temp_repo("precommit-safety", "after-snapshot-failure");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);
        let snapshot_calls = std::cell::Cell::new(0);
        let request_calls = std::cell::Cell::new(0);

        let error = run_precommit_safety_with_snapshot(
            &dir,
            true,
            |dir| {
                let call = snapshot_calls.get();
                snapshot_calls.set(call + 1);
                if call == 0 {
                    git::status_snapshot(dir)
                } else {
                    Err(anyhow::anyhow!("injected after-snapshot failure"))
                }
            },
            |_| Err("original host failure".to_owned()),
            |_| {
                request_calls.set(request_calls.get() + 1);
                StepResult::ok("unexpected-static-docs")
            },
        )
        .expect_err("snapshot uncertainty must fail closed");

        assert_eq!(snapshot_calls.get(), 2, "one before and one after snapshot");
        assert_eq!(request_calls.get(), 0);
        assert!(error.contains("original host failure"));
        assert!(error.contains("injected after-snapshot failure"));
    }

    #[test]
    fn precommit_safety_routes_unsupported_existing_state_broad_without_admission() {
        let dir = crate::test_support::temp_repo("precommit-safety", "broad-preserves-state");
        crate::test_support::commit(&dir, "tracked.md", "tracked\n");
        crate::test_support::commit(&dir, "delete.md", "delete\n");
        std::fs::remove_file(dir.join("delete.md")).expect("remove fixture file");
        crate::test_support::git_ok(&dir, &["add", "-u", "delete.md"]);
        crate::test_support::write(&dir, "note.txt", "pre-existing untracked\n");
        let request_calls = std::cell::Cell::new(0);

        run_precommit_safety_with(
            &dir,
            true,
            |class| {
                assert!(matches!(class, git::PrecommitChangeClass::Broad(_)));
                Ok(())
            },
            |_| {
                request_calls.set(request_calls.get() + 1);
                StepResult::ok("unexpected-static-docs")
            },
        )
        .expect("safe pre-existing state is tolerated");
        assert_eq!(request_calls.get(), 0);
        assert_eq!(
            git::output(&dir, &["diff", "--cached", "--name-only"]).unwrap(),
            "delete.md"
        );
        assert_eq!(
            git::output(&dir, &["status", "--porcelain"]).unwrap(),
            "D  delete.md\n?? note.txt"
        );
    }

    #[test]
    fn precommit_safety_checks_after_failed_docs_request_and_keeps_primary_failure() {
        let dir = crate::test_support::temp_repo("precommit-safety", "docs-postcheck");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::commit(&dir, "README.md", "# Readme\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);
        let request_calls = std::cell::Cell::new(0);

        let result = run_precommit_safety_with(
            &dir,
            true,
            |class| {
                assert_eq!(class, git::PrecommitChangeClass::StagedMarkdownOnly);
                Ok(())
            },
            |dir| {
                request_calls.set(request_calls.get() + 1);
                crate::test_support::write(dir, "README.md", "mutated during request\n");
                StepResult::fail("nix-static-docs").detail("original Nix failure")
            },
        )
        .expect_err("failed request remains the process failure");

        assert_eq!(request_calls.get(), 1);
        let detail = result.as_str();
        assert!(detail.contains("original Nix failure"));
        assert!(detail.contains("shared-static-docs-state"));
    }

    #[test]
    fn precommit_reconciles_once_after_an_early_static_failure() {
        let dir = crate::test_support::temp_repo("precommit", "early-failure-reconcile");
        crate::test_support::commit(&dir, "a.rs", "fn a(){}\n");
        crate::test_support::write(&dir, "a.rs", "fn a() { }\n");
        crate::test_support::git_ok(&dir, &["add", "a.rs"]);
        let first_static_spec = "fake-host";
        let mut invoked = Vec::new();

        let result = run_precommit_with_host_gate(&dir, |_, result| {
            invoked.push(first_static_spec);
            crate::test_support::write(&dir, "a.rs", "fn a() { }\n// formatted\n");
            result.push(
                StepResult::fail(first_static_spec)
                    .detail("static diagnostic survives reconciliation"),
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
            assert!(matches!(class, git::PrecommitChangeClass::Broad(_)));
            crate::test_support::write(&dir, "a.rs", "fn a() { }\n// formatted\n");
            result.push(StepResult::ok("fake-complete-fix-host-gate"));
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
    }
    #[test]
    fn precommit_routes_staged_markdown_before_restaging_safe_formatter_output() {
        let dir = crate::test_support::temp_repo("precommit", "markdown-routing");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);

        let result = run_precommit_with_host_gate(&dir, |class, result| {
            assert_eq!(class, git::PrecommitChangeClass::StagedMarkdownOnly);
            crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\nFormatted\n");
            result.push(StepResult::ok("fake-markdown-host-gate"));
        })
        .expect("precommit orchestration");

        assert_eq!(result.steps[0].name, "precommit-routing");
        assert_eq!(
            result.steps[0].detail.as_deref(),
            Some("class=staged-markdown-only reason=isolated-staged-markdown")
        );
        assert!(result.ok);
        let mut expected = vec![
            "precommit-routing",
            "fake-markdown-host-gate",
            "precommit-staging",
        ];
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            expected.extend([
                "shared-static-docs-admission",
                "mock-nix",
                "shared-static-docs-state",
            ]);
        }
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            git::output(&dir, &["diff", "--cached", "--name-only"]).expect("cached paths"),
            "guide.md"
        );
    }

    #[test]
    fn narrow_dispatch_failure_still_reconciles_exactly_once() {
        let dir = crate::test_support::temp_repo("precommit", "markdown-failure");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);

        let result = run_precommit_with_host_gate(&dir, |class, result| {
            assert_eq!(class, git::PrecommitChangeClass::StagedMarkdownOnly);
            result.push(StepResult::fail("prettier-markdown").detail("synthetic failure"));
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
    }

    #[test]
    fn staging_failure_skips_shared_request_after_one_reconciliation_attempt() {
        let dir = crate::test_support::temp_repo("precommit", "stage-admission-failure");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);
        let requested = std::cell::Cell::new(false);
        let result = run_precommit_with_shared_verdict(
            &dir,
            |_, result| {
                crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\nFormatted\n");
                crate::test_support::write(&dir, "unexpected.md", "not admitted\n");
                result.push(StepResult::ok("host-checks"));
            },
            |_| {
                requested.set(true);
                StepResult::ok("unexpected-nix")
            },
        )
        .unwrap();
        assert!(!requested.get());
        assert_eq!(
            result
                .steps
                .iter()
                .filter(|step| step.name == "precommit-staging")
                .count(),
            1
        );
        assert!(
            result
                .steps
                .iter()
                .any(|step| step.name == "precommit-staging" && !step.ok)
        );
    }

    #[test]
    fn conversion_mismatch_refuses_shared_request_after_reconciliation() {
        let dir = crate::test_support::temp_repo("precommit", "eol-admission");
        crate::test_support::write(&dir, ".gitattributes", "*.md text eol=crlf\n");
        crate::test_support::commit(&dir, ".gitattributes", "*.md text eol=crlf\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\r\nChanged\r\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);
        let requested = std::cell::Cell::new(false);
        let result = run_precommit_with_shared_verdict_for_platform(
            &dir,
            true,
            |_, result| result.push(StepResult::ok("host-checks")),
            |_| {
                requested.set(true);
                StepResult::ok("unexpected-nix")
            },
        )
        .unwrap();
        assert!(!requested.get());
        assert!(
            result
                .steps
                .iter()
                .any(|step| step.name == "shared-static-docs-admission" && !step.ok)
        );
        assert_eq!(
            result
                .steps
                .iter()
                .filter(|step| step.name == "precommit-staging")
                .count(),
            1
        );
    }

    #[test]
    fn shared_leaf_failure_occurs_after_one_reconciliation_without_retry() {
        let dir = crate::test_support::temp_repo("precommit", "leaf-failure");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);
        let result = run_precommit_with_shared_verdict_for_platform(
            &dir,
            true,
            |_, result| result.push(StepResult::ok("host-checks")),
            |_| StepResult::fail("nix-static-docs").detail("leaf failure"),
        )
        .unwrap();
        assert!(!result.ok);
        assert_eq!(
            result
                .steps
                .iter()
                .filter(|step| step.name == "precommit-staging")
                .count(),
            1
        );
        assert!(result.steps.iter().any(|step| {
            step.name == "nix-static-docs" && step.detail.as_deref() == Some("leaf failure")
        }));
        assert!(result.steps.last().unwrap().ok);
    }

    #[test]
    fn clean_pre_push_uses_shared_admission_and_post_request_state_check() {
        let dir = crate::test_support::temp_repo("prepush", "clean-shared-docs");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        let requested = std::cell::Cell::new(false);
        let mut result = CommandResult::new("prepush");
        run_shared_static_docs_request(
            &dir,
            true,
            || {
                requested.set(true);
                StepResult::ok("nix-static-docs")
            },
            &mut result,
        );
        assert!(requested.get());
        assert!(result.ok);
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>(),
            [
                "shared-static-docs-admission",
                "nix-static-docs",
                "shared-static-docs-state"
            ]
        );
    }

    #[test]
    fn non_x86_precommit_keeps_host_fallback_without_shared_request() {
        let dir = crate::test_support::temp_repo("precommit", "non-x86-fallback");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);
        let requested = std::cell::Cell::new(false);
        let result = run_precommit_with_shared_verdict_for_platform(
            &dir,
            false,
            |_, result| result.push(StepResult::ok("host-markdown-fix-and-checks")),
            |_| {
                requested.set(true);
                StepResult::ok("unexpected-nix")
            },
        )
        .unwrap();
        assert!(!requested.get());
        assert!(result.ok);
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>(),
            [
                "precommit-routing",
                "host-markdown-fix-and-checks",
                "precommit-staging"
            ]
        );
    }

    fn assert_request_mutation_fails(path: &str, replacement: &str) {
        let dir = crate::test_support::temp_repo("precommit", "post-request-mutation");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::commit(&dir, ".prettierrc.json", "{}\n");
        crate::test_support::commit(&dir, "README.md", "# Readme\n");
        crate::test_support::commit(&dir, "flake.lock", "lock\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);
        let mut result = CommandResult::new("precommit");
        run_shared_static_docs_request(
            &dir,
            false,
            || {
                crate::test_support::write(&dir, path, replacement);
                StepResult::fail("nix-static-docs").detail("original Nix failure")
            },
            &mut result,
        );
        assert!(result.steps.iter().any(|step| {
            step.name == "nix-static-docs" && step.detail.as_deref() == Some("original Nix failure")
        }));
        assert!(
            result
                .steps
                .iter()
                .any(|step| step.name == "shared-static-docs-state" && !step.ok)
        );
        assert!(!result.ok);
    }

    #[test]
    fn post_request_check_detects_tracked_markdown_mutation() {
        assert_request_mutation_fails("README.md", "# Mutated\n");
    }

    #[test]
    fn post_request_check_detects_tracked_config_mutation() {
        assert_request_mutation_fails(".prettierrc.json", "{\"semi\":true}\n");
    }

    #[test]
    fn post_request_check_detects_lock_mutation() {
        assert_request_mutation_fails("flake.lock", "changed lock\n");
    }

    #[test]
    fn post_request_check_detects_new_staged_mutation() {
        let dir = crate::test_support::temp_repo("precommit", "post-request-staged-mutation");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);
        let mut result = CommandResult::new("precommit");
        run_shared_static_docs_request(
            &dir,
            false,
            || {
                crate::test_support::write(&dir, "README.md", "# New staged mutation\n");
                crate::test_support::git_ok(&dir, &["add", "README.md"]);
                StepResult::ok("nix-static-docs")
            },
            &mut result,
        );
        assert!(
            result
                .steps
                .iter()
                .any(|step| step.name == "shared-static-docs-state" && !step.ok)
        );
    }

    #[test]
    fn post_request_check_detects_nonignored_untracked_source_mutation() {
        let dir = crate::test_support::temp_repo("precommit", "post-request-untracked-mutation");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);
        let mut result = CommandResult::new("precommit");
        run_shared_static_docs_request(
            &dir,
            false,
            || {
                crate::test_support::write(&dir, "new-doc.md", "# New input\n");
                StepResult::ok("nix-static-docs")
            },
            &mut result,
        );
        assert!(
            result
                .steps
                .iter()
                .any(|step| step.name == "shared-static-docs-state" && !step.ok)
        );
    }

    #[test]
    fn post_request_query_failure_preserves_original_leaf_failure() {
        let dir = crate::test_support::temp_repo("precommit", "post-request-query-failure");
        crate::test_support::commit(&dir, "guide.md", "# Guide\n");
        crate::test_support::write(&dir, "guide.md", "# Guide\nChanged\n");
        crate::test_support::git_ok(&dir, &["add", "guide.md"]);
        let mut result = CommandResult::new("precommit");
        run_shared_static_docs_request(
            &dir,
            false,
            || {
                std::fs::rename(dir.join(".git"), dir.join(".git-hidden"))
                    .expect("hide repository metadata to force a Git query error");
                StepResult::fail("nix-static-docs").detail("original leaf failure")
            },
            &mut result,
        );
        assert!(
            result
                .steps
                .iter()
                .any(|step| step.name == "nix-static-docs" && !step.ok)
        );
        assert!(
            result
                .steps
                .iter()
                .any(|step| step.name == "shared-static-docs-state" && !step.ok)
        );
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
    }
}
