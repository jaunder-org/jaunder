//! Shared fixtures for xtask's unit tests — the throwaway-git-repo idiom.
//!
//! Every git call here goes through [`crate::git::at`], which scrubs `GIT_DIR` and
//! friends. That is load-bearing, not defensive: these tests run under the
//! pre-commit hook, which exports those vars, and a bare `git` call would retarget
//! the fixture at the real repository — corrupting the shared config and committing
//! fixture files to the branch.

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn temp_repo_cleans_after_normal_return() {
        let path = {
            let owner = temp_repo("ownership", "normal");
            let path: &Path = owner.as_ref();
            git_ok(path, &["rev-parse", "--git-dir"]);
            path.to_path_buf()
        };
        assert!(!path.exists(), "fixture survived normal return: {path:?}");
    }

    #[test]
    fn temp_repo_cleans_after_error_return() {
        fn fail(path: &mut Option<PathBuf>) -> Result<(), &'static str> {
            let owner = temp_repo("ownership", "error");
            let root: &Path = owner.as_ref();
            git_ok(root, &["rev-parse", "--git-dir"]);
            *path = Some(root.to_path_buf());
            Err("fixture consumer failed")
        }

        let mut path = None;
        assert_eq!(fail(&mut path), Err("fixture consumer failed"));
        let path = path.expect("fixture created before error");
        assert!(!path.exists(), "fixture survived error return: {path:?}");
    }

    #[test]
    fn temp_repo_cleans_after_panic_unwinding() {
        let mut path = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let owner = temp_repo("ownership", "panic");
            let root: &Path = owner.as_ref();
            git_ok(root, &["rev-parse", "--git-dir"]);
            path = Some(root.to_path_buf());
            panic!("fixture consumer panicked");
        }));
        assert!(result.is_err());
        let path = path.expect("fixture created before panic");
        assert!(!path.exists(), "fixture survived panic unwinding: {path:?}");
    }
}
