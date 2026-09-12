use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR")
            .unwrap_or_else(|| panic!("cargo sets CARGO_MANIFEST_DIR")),
    );
    let workspace = manifest_dir
        .parent()
        .unwrap_or_else(|| panic!("test-support has a workspace parent"));
    if !workspace.join(".git").exists() {
        println!("cargo:rerun-if-changed=src");
        println!("cargo:rerun-if-changed=Cargo.toml");
        println!("cargo:rustc-env=JAUNDER_BUILD_COMMIT=unavailable");
        println!("cargo:rustc-env=JAUNDER_BUILD_DIRTY=1");
        return;
    }
    let git_dir = git_dir(workspace);
    let common_dir = common_dir(&git_dir);
    let head = git_dir.join("HEAD");

    println!("cargo:rerun-if-changed={}", head.display());
    println!(
        "cargo:rerun-if-changed={}",
        common_dir.join("packed-refs").display()
    );

    let head_contents = fs::read_to_string(&head)
        .unwrap_or_else(|error| panic!("reading {}: {error}", head.display()));
    if let Some(reference) = head_contents.strip_prefix("ref: ") {
        let reference = reference.trim_end();
        assert!(
            !reference.is_empty(),
            "{} names an empty Git reference",
            head.display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            common_dir.join(reference).display()
        );
    }

    watch_tracked_inputs(workspace);
    let commit = git_commit(workspace);
    println!("cargo:rustc-env=JAUNDER_BUILD_COMMIT={commit}");
    println!(
        "cargo:rustc-env=JAUNDER_BUILD_DIRTY={}",
        if git_dirty(workspace) { "1" } else { "0" }
    );
}

fn git_dir(workspace: &Path) -> PathBuf {
    let dot_git = workspace.join(".git");
    if dot_git.is_dir() {
        return dot_git;
    }

    let contents = fs::read_to_string(&dot_git)
        .unwrap_or_else(|error| panic!("reading {}: {error}", dot_git.display()));
    let location = contents
        .strip_prefix("gitdir: ")
        .unwrap_or_else(|| panic!("{} is not a Git directory link", dot_git.display()))
        .trim_end();
    assert!(
        !location.is_empty(),
        "{} names an empty Git directory",
        dot_git.display()
    );
    let location = PathBuf::from(location);
    if location.is_absolute() {
        location
    } else {
        workspace.join(location)
    }
}

fn common_dir(git_dir: &Path) -> PathBuf {
    let commondir = git_dir.join("commondir");
    let Ok(contents) = fs::read_to_string(&commondir) else {
        return git_dir.to_path_buf();
    };
    let location = PathBuf::from(contents.trim_end());
    if location.is_absolute() {
        location
    } else {
        git_dir.join(location)
    }
}

fn git_commit(workspace: &Path) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(["rev-parse", "--verify", "HEAD^{commit}"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_NAMESPACE")
        .output()
        .unwrap_or_else(|error| panic!("reading Git HEAD: {error}"));
    assert!(
        output.status.success(),
        "reading Git HEAD failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let commit = String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("Git HEAD is not UTF-8: {error}"));
    let commit = commit.trim_end();
    assert!(
        commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Git HEAD is not a full SHA: {commit:?}"
    );
    commit.to_owned()
}

fn git_dirty(workspace: &Path) -> bool {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap_or_else(|error| panic!("reading Git status: {error}"));
    assert!(
        output.status.success(),
        "reading Git status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    !output.stdout.is_empty()
}

fn watch_tracked_inputs(workspace: &Path) {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(["ls-files", "-z"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap_or_else(|error| panic!("listing tracked build inputs: {error}"));
    assert!(
        output.status.success(),
        "listing tracked build inputs failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for relative in output.stdout.split(|byte| *byte == 0) {
        if relative.is_empty() {
            continue;
        }
        let relative = std::str::from_utf8(relative)
            .unwrap_or_else(|error| panic!("tracked path is not UTF-8: {error}"));
        println!(
            "cargo:rerun-if-changed={}",
            workspace.join(relative).display()
        );
    }
}
