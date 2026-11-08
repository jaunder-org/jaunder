//! Fixed-action adapters to the active checkout's sole Python promoter owner.

use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

use anyhow::{Context, Result, bail};

use crate::cli::{AdrCommand, Command as CliCommand};
use crate::result::StepResult;

pub(super) enum Action {
    Promote,
    SyncReadme,
    Controller,
}

fn command(repo: &Path, action: Action) -> Result<Command> {
    // The Rust tool's own predicate excludes Python; admit this selected package too.
    crate::host_tooling::admit_untracked_inputs(repo, ".#adr-promoter.sourceAdmits")?;
    let mut command = Command::new("nix");
    command.current_dir(repo).args([
        "run",
        "--accept-flake-config",
        "--no-update-lock-file",
        "--no-write-lock-file",
        ".#adr-promoter",
        "--",
        match action {
            Action::Promote => "promote",
            Action::SyncReadme => "sync-readme",
            Action::Controller => "controller",
        },
    ]);
    // Hook variables may name the invoking index instead of this worktree.
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
    ] {
        command.env_remove(name);
    }
    Ok(command)
}

pub(super) fn output(repo: &Path, action: Action) -> Result<String> {
    let result = command(repo, action)?
        .stderr(Stdio::inherit())
        .output()
        .context("running Nix-selected ADR operation")?;
    let summary = String::from_utf8_lossy(&result.stdout).trim().to_string();
    if !result.status.success() {
        bail!("ADR operation failed ({}): {summary}", result.status);
    }
    Ok(summary)
}

pub(crate) fn sync_readme() -> StepResult {
    match output(Path::new("."), Action::SyncReadme) {
        Ok(summary) => StepResult::ok("adr-sync-readme").detail(summary),
        Err(error) => StepResult::fail("adr-sync-readme").detail(format!("{error:#}")),
    }
}

/// Temporary PR 1 compatibility facade; no Rust controller policy is executable.
pub(crate) fn controller() -> StepResult {
    match output(Path::new("."), Action::Controller) {
        Ok(summary) => StepResult::ok("adr-promoter").detail(summary),
        Err(error) => StepResult::fail("adr-promoter").detail(format!("{error:#}")),
    }
}

/// Public CLI delegates bypass lifecycle/StepResult mapping to preserve child status.
pub fn mutation_status(invocation: &CliCommand) -> Option<Result<ExitStatus>> {
    let action = match invocation {
        CliCommand::Adr(AdrCommand::Promote) => Action::Promote,
        CliCommand::Adr(AdrCommand::SyncReadme) => Action::SyncReadme,
        CliCommand::Adr(AdrCommand::Promoter) => Action::Controller,
        _ => return None,
    };
    Some(crate::host_tooling::prepare_checkout().and_then(|()| {
        command(Path::new("."), action)?
            .status()
            .context("running Nix-selected ADR operation")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_mutation_consumers_refuse_omitted_inputs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        assert!(
            crate::git::at(root)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
        let predicate = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("nix/adr-promoter-source.nix");
        let flake = format!(
            r#"{{ outputs = {{ self }}: {{ packages = builtins.listToAttrs (map (name: {{
          inherit name;
          value.adr-promoter.sourceAdmits = import (builtins.toPath {predicate:?});
        }}) [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ]); }}; }}"#
        );
        std::fs::write(root.join("flake.nix"), flake).unwrap();
        assert!(
            crate::git::at(root)
                .args(["add", "flake.nix"])
                .status()
                .unwrap()
                .success()
        );
        std::fs::create_dir_all(root.join("tools/adr-promoter/tests")).unwrap();
        std::fs::write(root.join("tools/adr-promoter/tests/new.py"), "source").unwrap();
        for action in [Action::Promote, Action::SyncReadme, Action::Controller] {
            let error = output(root, action).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("tools/adr-promoter/tests/new.py"),
                "{error:#}"
            );
            assert!(error.to_string().contains("git add -N"), "{error:#}");
        }
    }
}
