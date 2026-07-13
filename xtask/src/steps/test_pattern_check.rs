//! The `test-backend-pattern` static check: scans every Rust file under
//! `server/tests/` and `storage/src/` for any `#[tokio::test]` (including
//! parameterized `#[tokio::test(...)]` forms) that is not declared
//! backend-explicit.
//!
//! A test is backend-explicit when its attribute block carries one of the
//! backend-selecting rstest templates — `#[apply(backends)]`,
//! `#[apply(backends_matrix)]` (the `#[values]`-based variant for tests with a
//! local `#[case]` matrix), `#[apply(sqlite_only)]`, or
//! `#[apply(postgres_only)]`. A genuinely non-DB integration test instead
//! carries a `// guard:no-backend — <reason>` marker and is exempt. Pure
//! synchronous `#[test]` unit tests have no `#[tokio::test]` attribute and are
//! never flagged.
//!
//! This guard (introduced in #54 for `storage.rs`) is widened here (#127) to
//! the whole `server/tests` tree; #135 widened it again to the `storage/src`
//! crate so the storage crate's own in-file tests are policed by the same
//! scanner.

use std::path::{Path, PathBuf};

use crate::result::{CommandResult, StepResult};

/// Root directories this guard polices, each scanned recursively for `.rs` files.
const TEST_ROOTS: &[&str] = &["server/tests", "storage/src"];

/// True when a trimmed line applies one of the accepted backend templates.
/// `backends_matrix` is listed explicitly — it is NOT a substring of
/// `#[apply(backends)]`.
fn is_backend_apply(trimmed: &str) -> bool {
    trimmed.contains("#[apply(backends)]")
        || trimmed.contains("#[apply(backends_matrix)]")
        || trimmed.contains("#[apply(sqlite_only)]")
        || trimmed.contains("#[apply(postgres_only)]")
}

/// True for the bare `#[tokio::test]` and any parameterized
/// `#[tokio::test(flavor = …)]` form.
fn is_tokio_test(trimmed: &str) -> bool {
    trimmed == "#[tokio::test]" || trimmed.starts_with("#[tokio::test(")
}

/// True when a line in the attribute block satisfies the guard: an accepted
/// backend template, or an exemption marker. Two markers exempt a bare
/// `#[tokio::test]`: `// guard:no-backend` (touches no database) and
/// `// guard:low-level-db` (does low-level DB work — bootstrap admin,
/// `unique_postgres_url`, or both engines at once — that cannot go through the
/// `backend` fixture, so it wears no backend template).
fn is_exempt_or_tagged(trimmed: &str) -> bool {
    is_backend_apply(trimmed)
        || trimmed.starts_with("// guard:no-backend")
        || trimmed.starts_with("// guard:low-level-db")
}

/// Bounds the upward attribute-cluster scan: a blank line (rustfmt always
/// separates items with one) or a bare `}` ending the previous item.
fn is_cluster_boundary(trimmed: &str) -> bool {
    trimmed.is_empty() || trimmed == "}"
}

/// 1-based line numbers of every tokio test in `source` whose attribute cluster
/// carries neither a backend template nor the `// guard:no-backend` marker.
///
/// The cluster is the run of lines immediately above the `#[tokio::test…]` line
/// up to the preceding blank line / `}` boundary. Scanning to the blank boundary
/// (rather than requiring every line to start with `#[`) is what lets a
/// **multi-line** attribute — `#[case::x(\n  "arg",\n  "arg",\n)]`, whose
/// continuation lines and closing `)]` are not `#[`-prefixed — and an
/// interleaved doc-comment / two-line `// guard:no-backend` marker stay inside
/// the cluster. Pure given the source, so it is unit-tested directly.
fn violations(source: &str) -> Vec<usize> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !is_tokio_test(line.trim()) {
            continue;
        }
        let mut ok = false;
        let mut j = i;
        while j > 0 && !is_cluster_boundary(lines[j - 1].trim()) {
            j -= 1;
            if is_exempt_or_tagged(lines[j].trim()) {
                ok = true;
                break;
            }
        }
        if !ok {
            out.push(i + 1);
        }
    }
    out
}

/// The failure detail for all offending tests across the scanned files, or
/// `None` when every tokio test is tagged/exempt. Pure given the
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
            "  recovery: tag the test #[apply(backends|backends_matrix|sqlite_only|postgres_only)] \
             (a deliberately single-backend test must carry a // reason: comment; a genuinely \
             non-DB integration test may carry a // guard:no-backend — <reason> marker instead)"
                .to_string(),
        );
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Collect every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Scan every Rust file under each of [`TEST_ROOTS`] and push the result step. A
/// missing test root is a hard failure (not a silent pass), so a moved/renamed
/// tree can never quietly disable the guard.
pub fn run(result: &mut CommandResult) {
    let mut files = Vec::new();
    for root in TEST_ROOTS {
        if let Err(e) = rust_files(Path::new(root), &mut files) {
            result.push(
                StepResult::fail("test-backend-pattern").detail(format!("cannot scan {root}: {e}")),
            );
            return;
        }
    }
    let scanned: Vec<(String, String)> = files
        .iter()
        .filter_map(|p| {
            std::fs::read_to_string(p)
                .ok()
                .map(|s| (p.display().to_string(), s))
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
    const PARAM_BARE: &str = "\
#[tokio::test(flavor = \"multi_thread\")]
async fn bad_param() {}
";
    const PARAM_TAGGED: &str = "\
#[apply(sqlite_only)]
#[tokio::test(flavor = \"multi_thread\")]
async fn good_param(#[case] backend: Backend) {}
";
    const EXEMPT: &str = "\
// guard:no-backend — drives the asset router; no DB.
// (a second comment line of reason)
#[tokio::test]
async fn no_db() {}
";
    const LOW_LEVEL_DB: &str = "\
// guard:low-level-db — provisions a Postgres role/database via bootstrap admin.
#[tokio::test]
async fn provisions() {}
";
    const DOC_GAP: &str = "\
#[apply(backends)]
/// doc comment between the template and the test
#[tokio::test]
async fn good_with_doc(#[case] backend: Backend) {}
";
    const MATRIX_TAGGED: &str = "\
#[apply(backends_matrix)]
#[case::a(1)]
#[tokio::test]
async fn good_matrix(backend: Backend, #[case] n: i32) {}
";
    // A multi-line `#[case(...)]` whose continuation lines / closing `)]` are not
    // `#[`-prefixed — the case that exposed the contiguity bug: the `#[apply]` is
    // separated from `#[tokio::test]` by these non-attribute-prefixed lines.
    const MATRIX_MULTILINE_CASE: &str = "\
#[apply(backends_matrix)]
#[case::x(
    \"arg-one\",
    \"arg-two\"
)]
#[tokio::test]
async fn good_multiline(backend: Backend, #[case] a: &str, #[case] b: &str) {}
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
    fn parameterized_bare_is_flagged() {
        assert_eq!(violations(PARAM_BARE), vec![1]);
    }

    #[test]
    fn parameterized_tagged_is_clean() {
        assert!(violations(PARAM_TAGGED).is_empty());
    }

    #[test]
    fn no_backend_marker_exempts() {
        assert!(violations(EXEMPT).is_empty());
    }

    #[test]
    fn low_level_db_marker_exempts() {
        assert!(violations(LOW_LEVEL_DB).is_empty());
    }

    #[test]
    fn doc_comment_between_template_and_test_is_clean() {
        assert!(violations(DOC_GAP).is_empty());
    }

    #[test]
    fn backends_matrix_apply_is_clean() {
        assert!(violations(MATRIX_TAGGED).is_empty());
    }

    #[test]
    fn multiline_case_attribute_does_not_break_the_cluster() {
        assert!(violations(MATRIX_MULTILINE_CASE).is_empty());
    }

    #[test]
    fn problem_detail_names_file_line_and_recovery() {
        let detail = problems(&[("storage.rs".to_string(), BARE.to_string())]).expect("a problem");
        assert!(detail.contains("storage.rs:1"));
        assert!(detail.contains("#[apply(backends|backends_matrix|sqlite_only|postgres_only)]"));
    }

    #[test]
    fn clean_set_reports_no_problems() {
        assert_eq!(
            problems(&[("f.rs".to_string(), ANNOTATED.to_string())]),
            None
        );
    }

    #[test]
    fn storage_dialect_bare_tokio_test_is_flagged() {
        assert!(problems(&[("storage/src/sqlite/foo.rs".to_string(), BARE.to_string())]).is_some());
    }

    #[test]
    fn test_roots_includes_storage_src() {
        assert!(TEST_ROOTS.contains(&"storage/src"));
    }
}
