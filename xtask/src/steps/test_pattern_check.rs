//! The `test-backend-pattern` static check: scans the storage integration test
//! file(s) for any `#[tokio::test]` that is not tagged with one of the
//! backend-selecting rstest templates (`#[apply(backends)]` /
//! `#[apply(sqlite_only)]` / `#[apply(postgres_only)]`).
//!
//! Every async storage test must declare its backend coverage explicitly so a
//! new SQLite-only test cannot silently reintroduce the Postgres coverage hole
//! that issue #54 closed. Pure synchronous `#[test]` unit tests have no
//! `#[tokio::test]` attribute and are therefore never flagged. The scanned-path
//! set is deliberately a constant so #127 can widen it suite-wide and #135 can
//! add the storage crate without touching the scanning logic.

use std::path::Path;

use crate::result::{CommandResult, StepResult};

/// Files this guard polices. #54 scopes it to the storage integration suite;
/// later issues extend this list.
const SCANNED: &[&str] = &["server/tests/storage/storage.rs"];

/// True when a source line is one of the three accepted backend-template
/// applications. Matched on the trimmed line so indentation is irrelevant.
fn is_backend_apply(trimmed: &str) -> bool {
    trimmed.contains("#[apply(backends)]")
        || trimmed.contains("#[apply(sqlite_only)]")
        || trimmed.contains("#[apply(postgres_only)]")
}

/// 1-based line numbers of every `#[tokio::test]` in `source` that lacks a
/// backend template. A test's attributes form a contiguous block of `#[...]`
/// lines around the `#[tokio::test]` line; the template may sit either above it
/// (the convention) or below, so the block is scanned in both directions until
/// the first non-attribute line. Pure given the source, so it is unit-tested
/// directly.
fn violations(source: &str) -> Vec<usize> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.trim() != "#[tokio::test]" {
            continue;
        }
        let mut tagged = false;
        // Walk upward across the contiguous attribute block.
        let mut j = i;
        while j > 0 && lines[j - 1].trim().starts_with("#[") {
            j -= 1;
            if is_backend_apply(lines[j].trim()) {
                tagged = true;
                break;
            }
        }
        // Walk downward across the contiguous attribute block.
        if !tagged {
            let mut k = i + 1;
            while k < lines.len() && lines[k].trim().starts_with("#[") {
                if is_backend_apply(lines[k].trim()) {
                    tagged = true;
                    break;
                }
                k += 1;
            }
        }
        if !tagged {
            out.push(i + 1);
        }
    }
    out
}

/// The failure detail for all offending tests across the scanned files, or
/// `None` when every `#[tokio::test]` is tagged. Pure given the
/// `(path, source)` pairs, so it is unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    for (path, source) in scanned {
        for ln in violations(source) {
            lines.push(format!(
                "{path}:{ln}: #[tokio::test] without a backend template"
            ));
        }
    }
    if !lines.is_empty() {
        lines.push(
            "  recovery: tag the test #[apply(backends|sqlite_only|postgres_only)] \
             (a deliberately single-backend test must carry a // reason: comment)"
                .to_string(),
        );
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Read the scanned files and push the result step. A missing scanned file is
/// skipped (no-op) rather than erroring, matching the other static checks.
pub fn run(result: &mut CommandResult) {
    let scanned: Vec<(String, String)> = SCANNED
        .iter()
        .filter_map(|p| {
            std::fs::read_to_string(Path::new(p))
                .ok()
                .map(|s| ((*p).to_string(), s))
        })
        .collect();
    let step = match problems(&scanned) {
        None => StepResult::ok("test-backend-pattern"),
        Some(detail) => StepResult::fail("test-backend-pattern").detail(detail),
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANNOTATED: &str = "\
#[apply(backends)]
#[tokio::test]
async fn good(#[case] backend: Backend) {}
";
    const BARE: &str = "\
#[tokio::test]
async fn bad() {}
";
    const SYNC_UNIT: &str = "\
#[test]
fn pure_logic() {}
";
    const POSTGRES_ONLY: &str = "\
#[apply(postgres_only)]
#[tokio::test]
async fn pg(#[case] backend: Backend) {}
";

    #[test]
    fn annotated_tokio_test_is_clean() {
        assert!(violations(ANNOTATED).is_empty());
    }

    #[test]
    fn postgres_only_tokio_test_is_clean() {
        assert!(violations(POSTGRES_ONLY).is_empty());
    }

    #[test]
    fn bare_tokio_test_is_flagged_at_its_line() {
        assert_eq!(violations(BARE), vec![1]);
    }

    #[test]
    fn sync_unit_test_is_exempt() {
        assert!(violations(SYNC_UNIT).is_empty());
    }

    #[test]
    fn problem_detail_names_file_line_and_recovery() {
        let detail = problems(&[("storage.rs".to_string(), BARE.to_string())]).expect("a problem");
        assert!(detail.contains("storage.rs:1"));
        assert!(detail.contains("#[apply(backends|sqlite_only|postgres_only)]"));
    }

    #[test]
    fn clean_set_reports_no_problems() {
        assert_eq!(
            problems(&[("f.rs".to_string(), ANNOTATED.to_string())]),
            None
        );
    }
}
