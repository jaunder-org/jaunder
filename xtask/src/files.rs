//! One recursive source-tree walk, shared by every static gate.
//!
//! Each `steps/*_check.rs` gate opens by listing a tree's source files, and that walk
//! had been copy-pasted per gate — eight byte-identical `rust_files`/`spec_files`
//! bodies. It is the same argument [`crate::server_fns`] makes for the `#[server]`
//! inventory: independent copies of one rule rot apart, and here the rule is
//! *which files a gate is even allowed to conclude anything about*, so a copy that
//! drifts silently narrows a guard.
//!
//! Exemptions stay the caller's business: this returns *every* matching file, so a
//! gate that spares one (`steps::traced_context_check`'s `fixtures.ts`) filters the
//! result rather than teaching the walk one gate's policy.

use std::path::{Path, PathBuf};

/// Every file under `root`, recursively, whose extension is exactly `ext` (no leading
/// dot). Sorted, so a gate's report order does not inherit `read_dir`'s arbitrary
/// order.
///
/// An unlistable directory is an `Err`, never a short list: every caller is a
/// fail-closed gate, and a tree we cannot enumerate could hide the very thing being
/// policed.
pub fn with_extension(root: &Path, ext: &str) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    collect(root, ext, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, ext, out)?;
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `root/a.rs`, `root/skip.txt`, `root/nested/b.rs`.
    fn tree(root: &Path) {
        std::fs::write(root.join("a.rs"), "").expect("write a.rs");
        std::fs::write(root.join("skip.txt"), "").expect("write skip.txt");
        let nested = root.join("nested");
        std::fs::create_dir(&nested).expect("mkdir nested");
        std::fs::write(nested.join("b.rs"), "").expect("write b.rs");
    }

    #[test]
    fn collects_nested_matches_and_ignores_other_extensions() {
        let tmp = tempfile::tempdir().expect("tempdir");
        tree(tmp.path());
        let found = with_extension(tmp.path(), "rs").expect("walks");
        assert_eq!(
            found,
            vec![tmp.path().join("a.rs"), tmp.path().join("nested/b.rs")],
            "nested match included, sorted, and `.txt` excluded"
        );
    }

    #[test]
    fn extension_is_matched_exactly_not_as_a_suffix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("spec.mts"), "").expect("write spec.mts");
        assert!(
            with_extension(tmp.path(), "ts").expect("walks").is_empty(),
            "`.mts` is not `.ts` — a suffix match would widen every gate's scope"
        );
    }

    #[test]
    fn a_missing_root_is_an_error_not_an_empty_list() {
        // The fail-closed contract: a moved tree must redden its gate, not read as
        // "nothing to police".
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(with_extension(&tmp.path().join("absent"), "rs").is_err());
    }
}
