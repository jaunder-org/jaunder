//! Git-tracked, approved working-tree inputs for census collectors.
//!
//! The snapshot is the sole source-content seam: it excludes generated and
//! vendored paths while retaining approved text inputs for both language-specific
//! and repository-wide collectors. A tracked path deleted from the working tree is
//! omitted; other read failures stop collection rather than silently shrinking the
//! audit surface.

use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context, Result};

/// Current working-tree content for one Git-tracked repository-relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub path: String,
    pub content: String,
}

/// The fixed, Git-tracked source surface collectors may inspect.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceSnapshot {
    pub files: Vec<SourceFile>,
}

impl SourceSnapshot {
    /// Read approved tracked paths from the working tree, never Git blobs, so source collectors see
    /// local edits while history collectors can independently inspect `HEAD`. A tracked source
    /// deleted from the working tree is absent from this source snapshot.
    pub fn from_tracked(repo_root: &Path) -> Result<Self> {
        let paths = crate::git::tracked_files(repo_root, ".")?;
        let mut files = Vec::new();
        for path in paths.into_iter().filter(|path| is_approved_path(path)) {
            let path_on_disk = repo_root.join(&path);
            let content = match std::fs::read_to_string(&path_on_disk) {
                Ok(content) => content,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    Err(error).with_context(|| format!("reading census source {path}"))?
                }
            };
            files.push(SourceFile { path, content });
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Self { files })
    }
}

/// The root Rust workspace and the four explicitly approved non-root source trees.
pub(crate) fn is_approved_path(path: &str) -> bool {
    !is_excluded_path(path)
        && is_text_input(path)
        && (matches!(
            path.split('/').next(),
            Some("xtask" | "tools" | "end2end" | "elisp")
        ) || is_root_rust_workspace_path(path))
}

fn is_root_rust_workspace_path(path: &str) -> bool {
    matches!(
        path.split('/').next(),
        Some(
            "Cargo.toml"
                | "Cargo.lock"
                | "rust-toolchain.toml"
                | "common"
                | "client"
                | "csr"
                | "host"
                | "macros"
                | "server"
                | "storage"
                | "test-support"
                | "web"
        )
    )
}

fn is_text_input(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some(
            "rs" | "toml"
                | "lock"
                | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "mjs"
                | "cjs"
                | "json"
                | "yaml"
                | "yml"
                | "el"
                | "nix"
                | "html"
                | "css"
                | "md"
                | "txt"
                | "sh"
                | "sql"
        )
    )
}

fn is_excluded_path(path: &str) -> bool {
    path.split('/').any(|part| {
        matches!(
            part,
            "target"
                | "node_modules"
                | "vendor"
                | "dist"
                | "build"
                | "test-results"
                | "playwright-report"
                | ".xtask"
        ) || part.ends_with("-snapshots")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &Path, args: &[&str]) {
        let output = crate::git::at(root).args(args).output().expect("runs git");
        assert!(
            output.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn omits_tracked_sources_deleted_from_the_working_tree() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let root = temporary.path();
        git(root, &["init"]);
        git(root, &["config", "user.email", "census@example.test"]);
        git(root, &["config", "user.name", "Census Fixture"]);
        std::fs::create_dir_all(root.join("server/src")).expect("source dir");
        std::fs::write(root.join("server/src/present.rs"), "fn present() {}\n").expect("present");
        std::fs::write(root.join("server/src/deleted.rs"), "fn deleted() {}\n").expect("deleted");
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "tracked sources"]);
        std::fs::remove_file(root.join("server/src/deleted.rs")).expect("delete source");

        let snapshot = SourceSnapshot::from_tracked(root).expect("snapshot");

        assert_eq!(
            snapshot.files,
            vec![SourceFile {
                path: "server/src/present.rs".into(),
                content: "fn present() {}\n".into(),
            }]
        );
    }

    #[test]
    fn approved_paths_include_only_declared_source_roots() {
        assert!(is_approved_path("server/src/lib.rs"));
        assert!(is_approved_path("xtask/src/lib.rs"));
        assert!(is_approved_path("tools/devtool/src/main.rs"));
        assert!(is_approved_path("end2end/tests/a.spec.ts"));
        assert!(is_approved_path("elisp/jaunder.el"));
        assert!(is_approved_path("Cargo.toml"));
        assert!(!is_approved_path("docs/DESIGN.md"));
        assert!(!is_approved_path("scripts/git-add"));
    }

    #[test]
    fn approved_paths_exclude_generated_and_vendored_content() {
        assert!(!is_approved_path("server/target/debug/jaunder"));
        assert!(!is_approved_path("end2end/node_modules/pkg/index.js"));
        assert!(!is_approved_path("tools/vendor/lib.rs"));
        assert!(!is_approved_path("end2end/playwright-report/index.html"));
        assert!(!is_approved_path(
            "end2end/tests/auth.spec.ts-snapshots/login-page-chromium-visual-linux.png"
        ));
    }
}
