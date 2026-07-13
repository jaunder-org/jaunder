//! The `test-backend-pattern` static check: scans every Rust file under
//! `server/tests/` and `storage/src/` for any `#[tokio::test]` (including
//! parameterized `#[tokio::test(...)]` forms) that is not declared
//! backend-explicit.
//!
//! A test is backend-explicit when its attribute block carries one of the
//! backend-selecting rstest templates — `#[apply(backends)]`,
//! `#[apply(backends_matrix)]` (the `#[values]`-based variant for tests with a
//! local `#[case]` matrix), `#[apply(sqlite_only)]`, or
//! `#[apply(postgres_only)]`. A bare `#[tokio::test]` is instead exempt when it
//! carries `// guard:no-backend — <reason>` (touches no database) or
//! `// guard:low-level-db — <reason>` (low-level DB work that can't use the
//! `backend` fixture). Pure synchronous `#[test]` unit tests have no
//! `#[tokio::test]` attribute and are never flagged.
//!
//! Beyond that template-or-marker rule (`violations`), #418 adds two more,
//! enforcing ADR-0053: **homing** (`homing_violations`) — a `*_only` template
//! must live in its `sqlite/`/`postgres/` dialect dir and a dual template must
//! not; and **param-honesty** (`param_honesty_violations`) — a templated test
//! must use its injected `backend`, never discard it with `let _ = backend;`.
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

/// True when a line applies a *dual*-backend template (`backends`, or the
/// `#[values]`-based `backends_matrix`) — the templates that prove the generic
/// `XStore<DB>` contract and so belong in a generic module, never a dialect dir.
fn is_apply_dual(trimmed: &str) -> bool {
    trimmed.contains("#[apply(backends)]") || trimmed.contains("#[apply(backends_matrix)]")
}

/// True when a line applies the `sqlite_only` single-backend template.
fn is_apply_sqlite_only(trimmed: &str) -> bool {
    trimmed.contains("#[apply(sqlite_only)]")
}

/// True when a line applies the `postgres_only` single-backend template.
fn is_apply_postgres_only(trimmed: &str) -> bool {
    trimmed.contains("#[apply(postgres_only)]")
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

/// 1-based line numbers, each with a rule message, of every `#[apply(...)]` that
/// is mis-homed for the file it lives in (ADR-0053 §1 "home by what it proves").
/// Keyed on a `/sqlite/` or `/postgres/` path component:
/// - under `sqlite/`: only `sqlite_only` is allowed; a dual template or
///   `postgres_only` is a violation.
/// - under `postgres/`: only `postgres_only` is allowed; a dual template or
///   `sqlite_only` is a violation.
/// - a generic file (neither dialect dir): only the dual templates are allowed;
///   any `*_only` belongs in its dialect dir.
///
/// The `/dir/` form (leading + trailing slash) matches only a path segment named
/// exactly `sqlite`/`postgres`, never a longer name like `postgres_helpers`. Pure
/// given `(path, source)`, so it is unit-tested directly.
fn homing_violations(path: &str, source: &str) -> Vec<(usize, &'static str)> {
    let in_sqlite = path.contains("/sqlite/");
    let in_postgres = path.contains("/postgres/");
    let mut out = Vec::new();
    for (i, line) in source.lines().enumerate() {
        let t = line.trim();
        let rule = if in_sqlite {
            if is_apply_dual(t) {
                Some("dual-backend template in a sqlite/ dialect dir — proves the generic contract, so move it to a generic module (ADR-0053 §1)")
            } else if is_apply_postgres_only(t) {
                Some("postgres_only in a sqlite/ dialect dir — mismatched backend (ADR-0053 §1)")
            } else {
                None
            }
        } else if in_postgres {
            if is_apply_dual(t) {
                Some("dual-backend template in a postgres/ dialect dir — proves the generic contract, so move it to a generic module (ADR-0053 §1)")
            } else if is_apply_sqlite_only(t) {
                Some("sqlite_only in a postgres/ dialect dir — mismatched backend (ADR-0053 §1)")
            } else {
                None
            }
        } else if is_apply_sqlite_only(t) || is_apply_postgres_only(t) {
            Some("single-backend template in a generic file — a *_only test must live in its sqlite/ or postgres/ dialect dir (ADR-0053 §1/§2)")
        } else {
            None
        };
        if let Some(rule) = rule {
            out.push((i + 1, rule));
        }
    }
    out
}

/// 1-based line numbers of every test that wears a backend template but discards
/// the injected `#[case] backend` — the `let _ = backend;` idiom (or a
/// `#[case] _backend` param). Such a test either can use the backend (→ use it)
/// or self-fixtures low-level DB work (→ drop the template, become a bare
/// `#[tokio::test]` + `// guard:low-level-db`). `backend` only exists as a
/// template-injected binding, so the discard can appear only inside a templated
/// cluster. Pure given the source, so it is unit-tested directly.
fn param_honesty_violations(source: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, line) in source.lines().enumerate() {
        let t = line.trim();
        if t == "let _ = backend;" || t.contains("#[case] _backend") {
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
                "{path}:{ln}: #[tokio::test] without a backend template or guard marker"
            ));
        }
        for (ln, rule) in homing_violations(path, source) {
            lines.push(format!("{path}:{ln}: {rule}"));
        }
        for ln in param_honesty_violations(source) {
            lines.push(format!(
                "{path}:{ln}: backend template with a discarded backend (`let _ = backend;`) — \
                 use the injected backend, or drop the template for a bare #[tokio::test] + \
                 // guard:low-level-db (ADR-0053 §2)"
            ));
        }
    }
    if !lines.is_empty() {
        lines.push(
            "  recovery: templates are #[apply(backends|backends_matrix|sqlite_only|postgres_only)] \
             and must use their injected `backend`; a *_only test lives in its sqlite/ or postgres/ \
             dialect dir and carries a // reason: comment; a bare #[tokio::test] carries either \
             // guard:no-backend — <reason> (no DB) or // guard:low-level-db — <reason> (low-level \
             DB work that can't use the backend fixture)"
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

    // ── Homing (ADR-0053 §1) ────────────────────────────────────────────────

    #[test]
    fn dual_template_in_a_dialect_dir_is_flagged() {
        let sqlite = homing_violations("storage/src/sqlite/foo.rs", ANNOTATED);
        assert_eq!(sqlite.len(), 1);
        assert_eq!(sqlite[0].0, 1, "flags the #[apply(backends)] line");
        assert_eq!(
            homing_violations("storage/src/postgres/foo.rs", ANNOTATED).len(),
            1
        );
    }

    #[test]
    fn matching_single_backend_in_its_dialect_dir_is_clean() {
        assert!(homing_violations("storage/src/sqlite/foo.rs", PARAM_TAGGED).is_empty());
        assert!(homing_violations("storage/src/postgres/foo.rs", POSTGRES_ONLY).is_empty());
    }

    #[test]
    fn mismatched_single_backend_in_a_dialect_dir_is_flagged() {
        assert_eq!(
            homing_violations("storage/src/postgres/foo.rs", PARAM_TAGGED).len(),
            1
        );
        assert_eq!(
            homing_violations("storage/src/sqlite/foo.rs", POSTGRES_ONLY).len(),
            1
        );
    }

    #[test]
    fn single_backend_in_a_generic_file_is_flagged() {
        assert_eq!(
            homing_violations("server/tests/storage/storage.rs", POSTGRES_ONLY).len(),
            1
        );
    }

    #[test]
    fn dual_template_in_a_generic_file_is_clean() {
        assert!(homing_violations("server/tests/storage/storage.rs", ANNOTATED).is_empty());
    }

    #[test]
    fn low_level_db_bare_test_is_never_a_homing_violation() {
        assert!(homing_violations("storage/src/postgres/foo.rs", LOW_LEVEL_DB).is_empty());
        assert!(homing_violations("server/tests/misc/backup_interop.rs", LOW_LEVEL_DB).is_empty());
    }

    #[test]
    fn dialect_match_requires_a_full_path_segment() {
        // "/postgres_helpers/" is not the "/postgres/" dialect dir → treated as a
        // generic file, so a dual template there is clean (not a dialect violation).
        assert!(homing_violations("storage/src/postgres_helpers/foo.rs", ANNOTATED).is_empty());
    }

    // ── Param-honesty (ADR-0053 §2) ─────────────────────────────────────────

    #[test]
    fn discarded_backend_param_is_flagged() {
        let src = "\
#[apply(postgres_only)]
#[tokio::test]
async fn discards(#[case] backend: Backend) {
    let _ = backend;
}
";
        assert_eq!(param_honesty_violations(src), vec![4]);
    }

    #[test]
    fn underscore_case_param_is_flagged() {
        assert_eq!(
            param_honesty_violations("async fn f(#[case] _backend: Backend) {}\n"),
            vec![1]
        );
    }

    #[test]
    fn using_the_injected_backend_is_clean() {
        assert!(param_honesty_violations(ANNOTATED).is_empty());
        assert!(param_honesty_violations(
            "async fn f(#[case] backend: Backend) { let _e = backend.setup(); }\n"
        )
        .is_empty());
    }
}
