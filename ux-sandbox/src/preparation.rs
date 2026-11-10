//! Select one immutable executable set and retain its root through consumer teardown.
use crate::{
    session::{self, Signals},
    workspace,
};
use anyhow::{Context, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use tokio::process::Command;

pub struct Artifacts {
    pub jaunder: PathBuf,
    pub support: Option<PathBuf>,
    pub digest: String,
    pub(crate) _root: tempfile::TempDir,
}
pub enum Prepared {
    Ready(Artifacts),
    Exited(i32),
}

pub async fn prepare(server: bool, signals: &mut Signals) -> anyhow::Result<Prepared> {
    let root = std::env::current_dir()?.canonicalize()?;
    if let Some(code) = admit_untracked(&root, signals).await? {
        return Ok(Prepared::Exited(code));
    }
    workspace::directory(&root.join(".xtask"))?;
    let retained = tempfile::Builder::new()
        .prefix("sandbox-artifacts-")
        .tempdir_in(root.join(".xtask"))?;
    let link = retained.path().join("selected");
    let mut command = checkout_command("nix", &root);
    command
        .args(["build", "--no-update-lock-file", "--no-write-lock-file"])
        .arg("--out-link")
        .arg(&link)
        .arg(".#sandboxArtifacts");
    let code = session::run(command, None, signals).await?;
    if code != 0 {
        return Ok(Prepared::Exited(code));
    }
    // This invocation owns a fresh link: failure cannot select an older root.
    // Nix's bin entries are legitimate immutable symlinks, not workspace paths.
    let jaunder = executable(&link.join("bin/jaunder"))?;
    let support = server
        .then(|| executable(&link.join("bin/test-support")))
        .transpose()?;
    let digest = fingerprint(&jaunder)?;
    Ok(Prepared::Ready(Artifacts {
        jaunder,
        support,
        digest,
        _root: retained,
    }))
}

#[derive(Serialize)]
struct Candidate {
    path: String,
    kind: &'static str,
}

async fn admit_untracked(root: &Path, signals: &mut Signals) -> anyhow::Result<Option<i32>> {
    let mut git = checkout_command("git", root);
    git.args(["ls-files", "--others", "--exclude-standard", "-z"]);
    let (code, paths) = session::output(git, signals).await?;
    if code != 0 {
        return Ok(Some(code));
    }
    let mut candidates = Vec::new();
    for bytes in paths
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let path = std::str::from_utf8(bytes).context("untracked input path is not UTF-8")?;
        let metadata = fs::symlink_metadata(root.join(path))
            .with_context(|| format!("reading untracked input {path}"))?;
        let kind = if metadata.is_symlink() {
            "symlink"
        } else if metadata.is_dir() {
            "directory"
        } else {
            ensure!(
                metadata.is_file(),
                "unsupported untracked input type: {path}"
            );
            "regular"
        };
        candidates.push(Candidate {
            path: path.into(),
            kind,
        });
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    // Match the existing Git-flake admission policy. Paths are JSON data in an
    // environment variable, never interpolated into Nix expressions or shell text.
    let mut nix = checkout_command("nix", root);
    nix.args([
        "eval",
        "--impure",
        "--json",
        "--no-update-lock-file",
        "--no-write-lock-file",
        ".#sandboxArtifacts.sourceAdmits",
        "--apply",
        r#"sourceAdmits:
let candidates = builtins.fromJSON (builtins.getEnv "JAUNDER_SANDBOX_UNTRACKED_INPUTS");
in map (candidate: candidate.path)
  (builtins.filter (candidate: sourceAdmits candidate.path candidate.kind) candidates)"#,
    ])
    .env(
        "JAUNDER_SANDBOX_UNTRACKED_INPUTS",
        serde_json::to_string(&candidates)?,
    );
    let (code, stdout) = session::output(nix, signals).await?;
    if code != 0 {
        return Ok(Some(code));
    }
    let omitted: Vec<String> =
        serde_json::from_slice(&stdout).context("decoding Nix artifact source admission")?;
    ensure!(
        omitted.is_empty(),
        "Git flakes omit untracked declared inputs: {}. Track them with git add (or git add -N) before invoking sandbox",
        omitted.join(", ")
    );
    Ok(None)
}

fn checkout_command(program: &str, root: &Path) -> Command {
    let mut command = Command::new(program);
    command.current_dir(root);
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
    command
}

fn executable(path: &Path) -> anyhow::Result<PathBuf> {
    let path = path
        .canonicalize()
        .with_context(|| format!("selecting {}", path.display()))?;
    let metadata = fs::metadata(&path)?;
    ensure!(
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
        "not an executable: {}",
        path.display()
    );
    eprintln!("Selected {}", path.display());
    Ok(path)
}

fn fingerprint(path: &Path) -> anyhow::Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("fingerprinting {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let mut hex = String::with_capacity(64);
    for byte in digest.finalize() {
        use std::fmt::Write;
        write!(&mut hex, "{byte:02x}")?;
    }
    Ok(hex)
}
