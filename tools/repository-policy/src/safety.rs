//! Single-process Git/index custody around Just's named host workflows.
//! There is no snapshot interchange or workflow result envelope: Git owns the
//! evidence, Just owns check ordering, and each required failure stays blocking.
use crate::git;
use anyhow::{Context, Result, bail};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
};

fn host(dir: &Path, class: git::PrecommitChangeClass) -> Result<()> {
    let recipe = match class {
        git::PrecommitChangeClass::StagedMarkdownOnly => "precommit-markdown-host",
        git::PrecommitChangeClass::Broad(_) => "precommit-broad-host",
    };
    let status = Command::new("just")
        .arg(recipe)
        .current_dir(dir)
        .status()
        .with_context(|| format!("could not run `just {recipe}`"))?;
    anyhow::ensure!(status.success(), "`just {recipe}` exited {status}");
    Ok(())
}

pub fn precommit(dir: &Path) -> Result<()> {
    let before = git::status_snapshot(dir).context("precommit before-snapshot")?;
    let class = git::classify_precommit_change(&before);
    println!("precommit-routing: {}", class.detail());
    let mut failures = Vec::new();
    if let Err(error) = host(dir, class) {
        failures.push(format!("precommit-host: {error:#}"));
    }
    // Reconcile once even after host failure, without broadening staged authority.
    let staging = git::status_snapshot(dir)
        .context("after-snapshot failed; refusing staging")
        .and_then(|after| {
            git::apply_precommit_stage_plan(dir, &git::precommit_stage_plan(&before, &after))
        });
    match staging {
        Ok(detail) => eprintln!("precommit-staging: {detail}"),
        Err(error) => {
            eprintln!("precommit-staging: {error:#}");
            failures.push(format!("{error:#}"));
        }
    }
    if !failures.is_empty() {
        bail!(failures.join("\n"));
    }
    if cfg!(all(target_os = "linux", target_arch = "x86_64"))
        && matches!(class, git::PrecommitChangeClass::StagedMarkdownOnly)
    {
        static_docs(dir, false)?;
    }
    Ok(())
}

/// The post-check is mandatory even if the ordinary Nix request fails.
pub fn static_docs(dir: &Path, require_clean: bool) -> Result<()> {
    let before = if require_clean {
        git::shared_static_docs_clean_admission(dir)
    } else {
        git::shared_markdown_admission(dir)
    }
    .context("shared-static-docs-admission")?;
    let request = request_static_docs(dir);
    let post_check =
        git::shared_verdict_state_unchanged(dir, &before).context("shared-static-docs-state");
    match (request, post_check) {
        (Ok(()), Ok(())) => {
            eprintln!("shared-static-docs-state: unchanged");
            Ok(())
        }
        (Err(primary), Ok(())) => {
            eprintln!("shared-static-docs-state: unchanged");
            Err(primary)
        }
        (Ok(()), Err(error)) => Err(error),
        (Err(primary), Err(post)) => bail!("{primary:#}\n{post:#}"),
    }
}

fn request_static_docs(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir.join(".xtask/gcroots"))
        .context("creating static-docs root directory")?;
    let mut command = Command::new("nix");
    command.current_dir(dir).args([
        "build",
        "-L",
        "--keep-failed",
        "--log-lines",
        "50",
        "--accept-flake-config",
        "--no-update-lock-file",
        "--no-write-lock-file",
        "--out-link",
        ".xtask/gcroots/static-docs",
        ".#checks.x86_64-linux.static-docs",
    ]);
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
    // The docs leaf keeps its live stderr and caller-owned diagnostic log without
    // the host workflow's generic Nix report or VM-artifact rescue machinery.
    let diagnostics = dir.join(".xtask/diagnostics/static-docs");
    let log_path = diagnostics.join("build.log");
    let mut log = std::fs::create_dir_all(&diagnostics).and_then(|()| File::create(&log_path));
    let mut diagnostic_failed = log.is_err();
    let mut child = command
        .stderr(Stdio::piped())
        .spawn()
        .context("nix-static-docs request")?;
    let mut stderr = child
        .stderr
        .take()
        .context("nix-static-docs stderr pipe unavailable")?;
    let mut primary_failed = false;
    let mut buffer = [0; 8192];
    loop {
        match stderr.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                if let Ok(file) = &mut log {
                    diagnostic_failed |= file.write_all(&buffer[..size]).is_err();
                }
                primary_failed |= std::io::stderr().write_all(&buffer[..size]).is_err();
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                primary_failed = true;
                break;
            }
        }
    }
    drop(stderr);
    let status = child.wait().context("waiting for nix-static-docs")?;
    drop(log);
    if !status.success() && !diagnostic_failed {
        match std::fs::read_to_string(&log_path) {
            Ok(body) => {
                let lines = body.lines().collect::<Vec<_>>();
                let excerpt = match lines.iter().position(|line| line.starts_with("error:")) {
                    Some(start) => lines[start..].join("\n"),
                    None => format!(
                        "=== no `error:` block in build log; last 50 lines: ===\n{}",
                        lines[lines.len().saturating_sub(50)..].join("\n")
                    ),
                };
                let excerpt_path = diagnostics.join("failure-excerpt.log");
                if std::fs::write(&excerpt_path, format!("{excerpt}\n")).is_ok() {
                    eprintln!(
                        "nix-static-docs: scoped excerpt (read first): {}; full build log: {}",
                        excerpt_path.display(),
                        log_path.display()
                    );
                } else {
                    diagnostic_failed = true;
                }
            }
            Err(_) => diagnostic_failed = true,
        }
    }
    if diagnostic_failed {
        eprintln!(
            "git-index-safety: warning: could not completely capture static-docs diagnostics"
        );
    }
    anyhow::ensure!(
        !primary_failed,
        "failed to stream nix-static-docs stderr (child exited {status})"
    );
    anyhow::ensure!(status.success(), "nix-static-docs exited {status}");
    Ok(())
}
