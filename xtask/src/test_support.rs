//! Shared fixtures for xtask's unit tests — the throwaway-git-repo idiom three
//! modules need (`git`, `adr`, `doc_links`), plus the retired-spelling fixture the
//! server-fn gates need.
//!
//! Every git call here goes through [`crate::git::at`], which scrubs `GIT_DIR` and
//! friends. That is load-bearing, not defensive: these tests run under the
//! pre-commit hook, which exports those vars, and a bare `git` call would retarget
//! the fixture at the real repository — corrupting the shared config and committing
//! fixture files to the branch.

use std::path::{Path, PathBuf};
use std::process::ExitStatus;

/// A fn wearing the **retired** leptos `#[server]` spelling (#714) — the fixture
/// every server-fn gate uses to pin that the old spelling no longer enumerates.
/// `attr_args` is what follows `server` inside the brackets (`""` for the bare
/// form); `item` is the fn itself.
///
/// Assembled from pieces rather than written as one string literal on purpose: a
/// grep for the retired attribute at the start of a fixture literal is what proves
/// no gate fixture was left unconverted (#714), and these deliberate negative
/// fixtures must not read as leftovers. One helper rather than six copies, so that
/// sweep stays a clean zero and the single exception lives here, named.
pub fn retired_server_fn(attr_args: &str, item: &str) -> String {
    format!("#[{}{attr_args}]\n{item}\n", "server")
}

/// Run git against `dir` with the repo-redirecting env scrubbed.
pub fn git(dir: &Path, args: &[&str]) -> ExitStatus {
    crate::git::at(dir).args(args).status().unwrap()
}

/// Run git against `dir`, asserting it succeeded.
pub fn git_ok(dir: &Path, args: &[&str]) {
    assert!(git(dir, args).success(), "git {args:?} failed");
}

/// A fresh git repo under a temp dir, identity configured.
///
/// The directory name is `jaunder-<prefix>-<tag>-<pid>`. `tag` must be unique among
/// the callers sharing a `prefix`: tests in one binary share a process, so the pid
/// alone does not separate them and a collision means two tests racing on one repo.
pub fn temp_repo(prefix: &str, tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jaunder-{prefix}-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "t@t"],
        &["config", "user.name", "t"],
    ] {
        git_ok(&dir, args);
    }
    dir
}

/// Write `rel` under `dir`, creating its parent.
pub fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// [`write`], then `git add` + `git commit` it — the file ends up tracked.
pub fn commit(dir: &Path, rel: &str, body: &str) {
    write(dir, rel, body);
    git_ok(dir, &["add", rel]);
    git_ok(dir, &["commit", "-qm", "c"]);
}
