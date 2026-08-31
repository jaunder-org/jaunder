//! Shared, fail-closed Rust-source scanning for one-input static checks.
//!
//! A caller supplies its policed roots, step name, and pure analyzer. This module
//! discovers every `.rs` file with [`crate::files::with_extension`], sorts the
//! combined population lexically, and decodes each file as UTF-8 before exposing
//! `(display_path, source)` pairs to that analyzer. A traversal, read, or decode
//! failure produces the caller's one failed [`StepResult`] and does not invoke the
//! analyzer; a partial source population must never pass a gate. A complete
//! population invokes the analyzer exactly once and maps its optional detail to
//! the caller's one result step.

use std::path::Path;

use crate::files;
use crate::result::{CommandResult, StepResult};

/// Run one static check over the complete Rust-source population under `roots`.
///
/// `problems` receives lexically sorted `(display_path, source)` pairs only after
/// every root has been walked and every file decoded. The result always receives
/// exactly one step named `step`: a traversal/read failure, analyzer detail, or
/// success.
pub(super) fn run_source_scan(
    result: &mut CommandResult,
    step: &'static str,
    roots: &[&str],
    problems: impl FnOnce(&[(String, String)]) -> Option<String>,
) {
    run_source_scan_with(
        result,
        step,
        roots,
        |path| std::fs::read_to_string(path),
        problems,
    );
}

/// Like [`run_source_scan`], but accepts the reader used for each discovered file.
///
/// Production supplies [`std::fs::read_to_string`]. This crate-visible test seam
/// lets a check prove that an unreadable source cannot silently produce a partial
/// passing census.
pub(super) fn run_source_scan_with(
    result: &mut CommandResult,
    step: &'static str,
    roots: &[&str],
    reader: impl FnMut(&Path) -> std::io::Result<String>,
    problems: impl FnOnce(&[(String, String)]) -> Option<String>,
) {
    let mut paths = Vec::new();
    for root in roots {
        match files::with_extension(Path::new(root), "rs") {
            Ok(mut found) => paths.append(&mut found),
            Err(error) => {
                result.push(StepResult::fail(step).detail(format!("cannot scan {root}: {error}")));
                return;
            }
        }
    }
    paths.sort();

    let mut reader = reader;
    let mut sources = Vec::with_capacity(paths.len());
    for path in paths {
        match reader(&path) {
            Ok(source) => sources.push((path.display().to_string(), source)),
            Err(error) => {
                result.push(
                    StepResult::fail(step)
                        .detail(format!("{}: cannot read — {error}", path.display())),
                );
                return;
            }
        }
    }

    match problems(&sources) {
        Some(detail) => result.push(StepResult::fail(step).detail(detail)),
        None => result.push(StepResult::ok(step)),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io;

    use super::{run_source_scan, run_source_scan_with};
    use crate::CommandResult;

    #[test]
    fn sources_across_roots_are_passed_to_the_analyzer_in_lexical_order() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let a_root = tmp.path().join("a-root");
        let z_root = tmp.path().join("z-root");
        std::fs::create_dir_all(a_root.join("nested")).expect("mkdir nested");
        std::fs::create_dir(&z_root).expect("mkdir z root");
        std::fs::write(a_root.join("nested/a.rs"), "a").expect("write a.rs");
        std::fs::write(z_root.join("b.rs"), "b").expect("write b.rs");

        let mut result = CommandResult::new("test");
        let mut received = Vec::new();
        run_source_scan(
            &mut result,
            "source-scan-test",
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
                z_root.join("b.rs").display().to_string(),
            ]
        );
        assert_eq!(result.steps.len(), 1);
        assert!(result.steps[0].ok);
    }

    #[test]
    fn injected_read_failure_fails_the_named_step_without_analyzing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let blocked = tmp.path().join("blocked.rs");
        std::fs::write(&blocked, "blocked").expect("write blocked.rs");

        let mut result = CommandResult::new("test");
        let analyzed = Cell::new(false);
        run_source_scan_with(
            &mut result,
            "source-scan-test",
            &[tmp.path().to_str().unwrap()],
            |path| {
                if path == blocked {
                    Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
                } else {
                    std::fs::read_to_string(path)
                }
            },
            |_| {
                analyzed.set(true);
                None
            },
        );

        assert!(!analyzed.get());
        assert_eq!(result.steps.len(), 1);
        assert!(!result.steps[0].ok);
        assert_eq!(result.steps[0].name, "source-scan-test");
        let detail = result.steps[0].detail.as_deref().unwrap();
        assert!(detail.contains(&blocked.display().to_string()));
        assert!(detail.contains("denied"));
    }

    #[test]
    fn invalid_utf8_file_fails_the_named_step_without_analyzing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let invalid = tmp.path().join("invalid.rs");
        std::fs::write(&invalid, [0xff]).expect("write invalid.rs");

        let mut result = CommandResult::new("test");
        let analyzed = Cell::new(false);
        run_source_scan(
            &mut result,
            "source-scan-test",
            &[tmp.path().to_str().unwrap()],
            |_| {
                analyzed.set(true);
                None
            },
        );

        assert!(!analyzed.get());
        assert_eq!(result.steps.len(), 1);
        assert!(!result.steps[0].ok);
        assert_eq!(result.steps[0].name, "source-scan-test");
        assert!(
            result.steps[0]
                .detail
                .as_deref()
                .unwrap()
                .contains(&invalid.display().to_string())
        );
    }

    #[test]
    fn analyzer_problem_fails_the_named_step_after_a_complete_scan() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("source.rs"), "source").expect("write source.rs");

        let mut result = CommandResult::new("test");
        run_source_scan(
            &mut result,
            "source-scan-test",
            &[tmp.path().to_str().unwrap()],
            |_| Some("violation".into()),
        );

        assert_eq!(result.steps.len(), 1);
        assert!(!result.steps[0].ok);
        assert_eq!(result.steps[0].name, "source-scan-test");
        assert_eq!(result.steps[0].detail.as_deref(), Some("violation"));
    }
}
