//! Read-only Git facts used to identify the current repository and branch.
//!
//! These queries intentionally scrub hook-exported `GIT_*` redirection variables:
//! `-C` alone does not override them. No mutating Git operation belongs in this
//! observer package.

use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result, anyhow};

const REPOSITORY_REDIRECTS: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

fn run(dir: &Path, args: &[&str]) -> Result<Output> {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir).args(args);
    for variable in REPOSITORY_REDIRECTS {
        command.env_remove(variable);
    }
    command
        .output()
        .with_context(|| format!("running read-only Git query in {}", dir.display()))
}

fn checked_output(dir: &Path, args: &[&str]) -> Result<String> {
    let output = run(dir, args)?;
    if !output.status.success() {
        return Err(anyhow!(
            "read-only Git query {:?} failed with {}: {}",
            args,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn current_branch(dir: &Path) -> Result<Option<String>> {
    let branch = checked_output(dir, &["branch", "--show-current"])?;
    Ok((!branch.is_empty()).then_some(branch))
}

pub fn remote_url(dir: &Path, name: &str) -> Result<Option<String>> {
    match run(dir, &["config", "--get", &format!("remote.{name}.url")])? {
        output if output.status.success() => {
            let url = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            Ok((!url.is_empty()).then_some(url))
        }
        output if output.status.code() == Some(1) => Ok(None),
        output => Err(anyhow!(
            "reading Git remote {name} in {} failed with {}: {}",
            dir.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}
