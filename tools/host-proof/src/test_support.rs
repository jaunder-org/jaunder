//! Throwaway Git repositories for unit tests. Every call goes through
//! [`crate::git::at`], which scrubs inherited `GIT_DIR` and friends so a hook
//! environment cannot retarget a fixture at the real repository.

use std::path::Path;

pub fn git_ok(dir: &Path, args: &[&str]) {
    let status = crate::git::at(dir).args(args).status().unwrap();
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

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

pub fn temp_repo(prefix: &str, tag: &str) -> TestRepo {
    let dir = tempfile::Builder::new()
        .prefix(&format!("{prefix}-{tag}-"))
        .tempdir()
        .unwrap();
    git_ok(dir.path(), &["init", "-q", "-b", "main"]);
    git_ok(dir.path(), &["config", "user.email", "t@example.invalid"]);
    git_ok(dir.path(), &["config", "user.name", "t"]);
    git_ok(dir.path(), &["config", "commit.gpgsign", "false"]);
    TestRepo(dir)
}

pub fn commit(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, body).unwrap();
    git_ok(dir, &["add", rel]);
    git_ok(dir, &["commit", "-q", "-m", rel]);
}
