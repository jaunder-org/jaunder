//! Complete, sorted UTF-8 Rust-source populations for fixed source policies.
//! Traversal or read failure stops before analysis: partial input cannot pass.
use std::path::Path;

pub(super) fn run_source_scan(
    roots: &[&str],
    problems: impl FnOnce(&[(String, String)]) -> Option<String>,
) -> Result<Option<String>, String> {
    run_source_scan_with(roots, |path| std::fs::read_to_string(path), problems)
}

pub(super) fn run_source_scan_with(
    roots: &[&str],
    mut reader: impl FnMut(&Path) -> std::io::Result<String>,
    problems: impl FnOnce(&[(String, String)]) -> Option<String>,
) -> Result<Option<String>, String> {
    let mut paths = Vec::new();
    for root in roots {
        let mut found = crate::files::with_extension(Path::new(root), "rs")
            .map_err(|error| format!("cannot scan {root}: {error}"))?;
        paths.append(&mut found);
    }
    paths.sort();
    let mut sources = Vec::with_capacity(paths.len());
    for path in paths {
        let source =
            reader(&path).map_err(|error| format!("{}: cannot read — {error}", path.display()))?;
        sources.push((path.display().to_string(), source));
    }
    match problems(&sources) {
        Some(detail) => Err(detail),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, io};

    #[test]
    fn sources_across_roots_are_passed_to_the_analyzer_in_lexical_order() {
        let tmp = tempfile::tempdir().unwrap();
        let a_root = tmp.path().join("a-root");
        let z_root = tmp.path().join("z-root");
        std::fs::create_dir_all(a_root.join("nested")).unwrap();
        std::fs::create_dir(&z_root).unwrap();
        std::fs::write(a_root.join("nested/a.rs"), "a").unwrap();
        std::fs::write(z_root.join("b.rs"), "b").unwrap();
        let mut received = Vec::new();
        let result = run_source_scan(
            &[z_root.to_str().unwrap(), a_root.to_str().unwrap()],
            |sources| {
                received = sources.iter().map(|(path, _)| path.clone()).collect();
                None
            },
        );
        assert_eq!(
            received,
            vec![
                a_root.join("nested/a.rs").display().to_string(),
                z_root.join("b.rs").display().to_string()
            ]
        );
        assert!(result.is_ok());
    }
    #[test]
    fn injected_read_failure_fails_without_analyzing() {
        let tmp = tempfile::tempdir().unwrap();
        let blocked = tmp.path().join("blocked.rs");
        std::fs::write(&blocked, "blocked").unwrap();
        let analyzed = Cell::new(false);
        let result = run_source_scan_with(
            &[tmp.path().to_str().unwrap()],
            |_| Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied")),
            |_| {
                analyzed.set(true);
                None
            },
        );
        assert!(!analyzed.get());
        let detail = result.unwrap_err();
        assert!(detail.contains(&blocked.display().to_string()));
        assert!(detail.contains("denied"));
    }
    #[test]
    fn invalid_utf8_file_fails_without_analyzing() {
        let tmp = tempfile::tempdir().unwrap();
        let invalid = tmp.path().join("invalid.rs");
        std::fs::write(&invalid, [0xff]).unwrap();
        let analyzed = Cell::new(false);
        let result = run_source_scan(&[tmp.path().to_str().unwrap()], |_| {
            analyzed.set(true);
            None
        });
        assert!(!analyzed.get());
        assert!(result.unwrap_err().contains(&invalid.display().to_string()));
    }
    #[test]
    fn analyzer_problem_fails_after_a_complete_scan() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("source.rs"), "source").unwrap();
        let result = run_source_scan(&[tmp.path().to_str().unwrap()], |_| {
            Some("violation".into())
        });
        assert_eq!(result, Err("violation".into()));
    }
}
