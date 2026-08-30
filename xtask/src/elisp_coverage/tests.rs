use std::fs;

use tempfile::TempDir;

use super::{CoverageError, consume};

fn fixture(source: &str, status: &str, lcov: &str) -> TempDir {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join("elisp")).unwrap();
    fs::create_dir(temp.path().join("artifacts")).unwrap();
    fs::write(temp.path().join("elisp/client.el"), source).unwrap();
    fs::write(temp.path().join("artifacts/status.json"), status).unwrap();
    fs::write(
        temp.path().join("artifacts/summary.txt"),
        "coverage summary\n",
    )
    .unwrap();
    fs::write(temp.path().join("artifacts/lcov.info"), lcov).unwrap();
    temp
}

fn status(outcome: &str, forms: &str) -> String {
    format!(
        r#"{{"schema":"elisp-coverage-v1","outcome":"{outcome}","modules":[{{"path":"elisp/client.el","forms":{forms}}}]}}"#
    )
}

fn ordinary_status() -> String {
    status(
        "success",
        r#"[{"start_line":1,"kind":"defun","points":[{"line":2,"kind":"ordinary"}]}]"#,
    )
}

fn lcov(records: &str) -> String {
    format!("SF:elisp/client.el\n{records}end_of_record\n")
}

fn consume_fixture(temp: &TempDir) -> Result<super::CoverageReport, CoverageError> {
    consume(temp.path(), &temp.path().join("artifacts"))
}

fn assert_message(result: Result<super::CoverageReport, CoverageError>, needle: &str) {
    let error = result.unwrap_err();
    assert!(format!("{error:?}").contains(needle), "{error:?}");
}

#[test]
fn success_status_with_a_covered_point_passes() {
    // Intent: a producer success is only accepted after an ordinary point reconciles.
    let temp = fixture(
        "(defun client ()\n  (message \"covered\"))\n",
        &ordinary_status(),
        &lcov("DA:2,1\n"),
    );
    assert_eq!(consume_fixture(&temp).unwrap().covered_points, 1);
}

#[test]
fn every_controlled_failure_outcome_is_a_host_failure() {
    for outcome in ["ert-failure", "instrumentation-failure", "invalid-report"] {
        let temp = fixture("(defun client () nil)\n", &status(outcome, "[]"), "");
        assert_message(consume_fixture(&temp), outcome);
    }
}

#[test]
fn unknown_or_malformed_status_and_missing_artifacts_fail_closed() {
    let temp = fixture(
        "(defun client () nil)\n",
        &status("success", "[]").replace("elisp-coverage-v1", "other"),
        "",
    );
    assert_message(consume_fixture(&temp), "unknown status schema");
    fs::write(temp.path().join("artifacts/status.json"), "{").unwrap();
    assert_message(consume_fixture(&temp), "invalid status.json");
    fs::remove_file(temp.path().join("artifacts/summary.txt")).unwrap();
    assert_message(consume_fixture(&temp), "Artifact");
    let missing_lcov = fixture("(defun client () nil)\n", &status("success", "[]"), "");
    fs::remove_file(missing_lcov.path().join("artifacts/lcov.info")).unwrap();
    assert_message(consume_fixture(&missing_lcov), "Artifact");
}

#[test]
fn malformed_lcov_fails_closed() {
    let temp = fixture(
        "(defun client ()\n  (message \"x\"))\n",
        &ordinary_status(),
        "SF:elisp/client.el\nDA:not-a-line,1\nend_of_record\n",
    );
    assert_message(consume_fixture(&temp), "invalid DA line");
}

#[test]
fn source_population_and_forms_must_match_the_producer_census() {
    let missing_module = fixture(
        "(defun client () nil)\n",
        r#"{"schema":"elisp-coverage-v1","outcome":"success","modules":[]}"#,
        "",
    );
    assert_message(consume_fixture(&missing_module), "producer modules");
    let missing_form = fixture("(defun client () nil)\n", &status("success", "[]"), "");
    assert_message(consume_fixture(&missing_form), "census forms");
}

#[test]
fn ordinary_points_require_one_and_only_one_lcov_record() {
    let source = "(defun client ()\n  (message \"x\"))\n";
    let missing = fixture(source, &ordinary_status(), &lcov(""));
    assert_message(consume_fixture(&missing), "missing an LCOV record");
    let duplicate = fixture(source, &ordinary_status(), &lcov("DA:2,0\nDA:2,1\n"));
    assert_message(consume_fixture(&duplicate), "2 LCOV records");
}

#[test]
fn uncovered_points_fail_and_covered_points_pass() {
    let source = "(defun client ()\n  (message \"x\"))\n";
    let uncovered = fixture(source, &ordinary_status(), &lcov("DA:2,0\n"));
    assert_message(consume_fixture(&uncovered), "uncovered executable point");
    let covered = fixture(source, &ordinary_status(), &lcov("DA:2,2\n"));
    assert_eq!(consume_fixture(&covered).unwrap().covered_points, 1);
}

#[test]
fn zero_stop_and_macro_forms_remain_visible_as_synthetic_points() {
    // Intent: a production macro must not disappear merely because Edebug has no stop.
    let source = "(defmacro client-macro () nil)\n";
    let census = status(
        "success",
        r#"[{"start_line":1,"kind":"defmacro","points":[{"line":1,"kind":"synthetic"}]}]"#,
    );
    let unignored = fixture(source, &census, "");
    assert_message(
        consume_fixture(&unignored),
        "uninstrumented synthetic point",
    );
    let ignored = fixture(
        "(defmacro client-macro () nil) ;; cov:ignore: edebug cannot stop here\n",
        &census,
        "",
    );
    assert_eq!(consume_fixture(&ignored).unwrap().ignored_points, 1);
}

#[test]
fn only_a_reasoned_trailing_marker_ignores_an_uncovered_point() {
    let valid = fixture(
        "(defun client ()\n  (message \"x\")) ;; cov:ignore: exercised only by Emacs\n",
        &ordinary_status(),
        &lcov("DA:2,0\n"),
    );
    assert_eq!(consume_fixture(&valid).unwrap().ignored_points, 1);
    let empty = fixture(
        "(defun client ()\n  (message \"x\")) ;; cov:ignore:   \n",
        &ordinary_status(),
        &lcov("DA:2,0\n"),
    );
    assert_message(consume_fixture(&empty), "malformed cov:ignore");
    let malformed = fixture(
        "(defun client ()\n  (message \"x\")) ;; cov:ignore reason\n",
        &ordinary_status(),
        &lcov("DA:2,0\n"),
    );
    assert_message(consume_fixture(&malformed), "malformed cov:ignore");
    let in_string = fixture(
        "(defun client ()\n  (message \"cov:ignore\"))\n",
        &ordinary_status(),
        &lcov("DA:2,0\n"),
    );
    assert_message(consume_fixture(&in_string), "uncovered executable point");
}

#[test]
fn markers_on_covered_or_non_executable_lines_fail() {
    let covered = fixture(
        "(defun client ()\n  (message \"x\")) ;; cov:ignore: stale\n",
        &ordinary_status(),
        &lcov("DA:2,1\n"),
    );
    assert_message(consume_fixture(&covered), "covered point");
    let non_executable = fixture(
        "(defun client () ;; cov:ignore: stale\n  (message \"x\"))\n",
        &ordinary_status(),
        &lcov("DA:2,0\n"),
    );
    assert_message(consume_fixture(&non_executable), "not a census point");
}

#[test]
fn source_reader_handles_comments_strings_and_rejects_malformed_structure() {
    let source = "; (defun ignored ())\n(defun client ()\n  (message \"(not a form)\"))\n";
    let temp = fixture(
        source,
        &ordinary_status().replace("\"start_line\":1", "\"start_line\":2"),
        &lcov("DA:2,1\n"),
    );
    assert_eq!(consume_fixture(&temp).unwrap().covered_points, 1);
    let malformed = fixture("(defun client ()\n", &ordinary_status(), &lcov("DA:2,1\n"));
    assert_message(consume_fixture(&malformed), "unterminated list");
}
