//! Real Git fixtures; every command scrubs hook repository redirection.
use std::path::Path;
use std::process::ExitStatus;

/// Run git against `dir` with the repo-redirecting env scrubbed.
pub fn git(dir: &Path, args: &[&str]) -> ExitStatus {
    crate::git::at(dir).args(args).status().unwrap()
}

/// Run git against `dir`, asserting it succeeded.
pub fn git_ok(dir: &Path, args: &[&str]) {
    assert!(git(dir, args).success(), "git {args:?} failed");
}

/// An owning repository path: path access never separates callers from cleanup.
pub struct TestRepo(tempfile::TempDir);

impl std::ops::Deref for TestRepo {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        self.0.path()
    }
}

impl AsRef<Path> for TestRepo {
    fn as_ref(&self) -> &Path {
        self.0.path()
    }
}

/// A fresh git repo under a temp dir, identity configured.
///
/// Retain the returned owner through every Git/filesystem use. Randomized names
/// isolate callers, and ownership starts before Git initialization can unwind.
pub fn temp_repo(prefix: &str, tag: &str) -> TestRepo {
    let dir = TestRepo(
        tempfile::Builder::new()
            .prefix(&format!("jaunder-{prefix}-{tag}-"))
            .tempdir()
            .unwrap(),
    );
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
