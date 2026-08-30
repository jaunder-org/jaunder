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

fn synthetic_status(kind: &str) -> String {
    status(
        "success",
        &format!(
            r#"[{{"start_line":1,"kind":"{kind}","points":[{{"line":1,"kind":"synthetic"}}]}}]"#
        ),
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
        &lcov(""),
    );
    assert_message(consume_fixture(&missing_module), "producer modules");
    let missing_form = fixture(
        "(defun client () nil)\n",
        &status("success", "[]"),
        &lcov(""),
    );
    assert_message(consume_fixture(&missing_form), "census forms");
}

#[test]
fn lcov_module_population_must_match_the_source_census() {
    let temp = fixture(
        "(defun client ()\n  (message \"client\"))\n",
        &ordinary_status(),
        &lcov("DA:2,1\n"),
    );
    fs::write(
        temp.path().join("elisp/other.el"),
        "(defmacro other () nil)\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("artifacts/status.json"),
        r#"{"schema":"elisp-coverage-v1","outcome":"success","modules":[{"path":"elisp/client.el","forms":[{"start_line":1,"kind":"defun","points":[{"line":2,"kind":"ordinary"}]}]},{"path":"elisp/other.el","forms":[{"start_line":1,"kind":"defmacro","points":[{"line":1,"kind":"synthetic"}]}]}]}"#,
    )
    .unwrap();
    assert_message(consume_fixture(&temp), "LCOV modules");
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
fn census_points_must_belong_to_their_owning_top_level_form() {
    let source =
        "(defun first ()\n  (message \"first\"))\n(defun second ()\n  (message \"second\"))\n";
    let census = status(
        "success",
        r#"[{"start_line":1,"kind":"defun","points":[{"line":3,"kind":"ordinary"}]},{"start_line":3,"kind":"defun","points":[{"line":4,"kind":"ordinary"}]}]"#,
    );
    let temp = fixture(source, &census, &lcov("DA:3,1\nDA:4,1\n"));
    assert_message(consume_fixture(&temp), "outside its form");
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
    let unignored = fixture(source, &census, &lcov(""));
    assert_message(
        consume_fixture(&unignored),
        "uninstrumented synthetic point",
    );
    let ignored = fixture(
        "(defmacro client-macro () nil) ;; cov:ignore: edebug cannot stop here\n",
        &census,
        &lcov(""),
    );
    assert_eq!(consume_fixture(&ignored).unwrap().ignored_points, 1);
}

#[test]
fn declarative_zero_stop_forms_are_automatically_structural() {
    for (kind, source) in [
        ("require", "(require 'client)\n"),
        ("provide", "(provide 'client)\n"),
        (
            "declare-function",
            "(declare-function client-function \"client\")\n",
        ),
        ("defgroup", "(defgroup client nil \"Client.\")\n"),
        ("cl-defstruct", "(cl-defstruct client name)\n"),
    ] {
        let temp = fixture(source, &synthetic_status(kind), &lcov(""));
        assert_eq!(
            consume_fixture(&temp).unwrap().ignored_points,
            1,
            "{kind} should not require a source marker"
        );
    }
}

#[test]
fn inert_declaration_initializers_are_automatically_structural() {
    let initializers = [
        "",
        " nil",
        " t",
        " 42",
        " -42",
        " 1.5e-2",
        " #x2a",
        " \"client\"",
        " ?c",
        " :client",
        " 'client",
        " #'client-function",
        " [client value]",
        " [client (nested :data) [more client-data] 'client]",
    ];
    for kind in ["defvar", "defconst", "defcustom"] {
        for initializer in initializers {
            let temp = fixture(
                &format!("({kind} client{initializer})\n"),
                &synthetic_status(kind),
                &lcov(""),
            );
            assert_eq!(
                consume_fixture(&temp).unwrap().ignored_points,
                1,
                "{kind}{initializer:?} should be inert"
            );
        }
    }
}

#[test]
fn radix_integer_initializers_follow_emacs_reader_grammar() {
    for (initializer, inert) in [
        (" #b101", true),
        (" #b-101", true),
        (" #o+77", true),
        (" #x-2A", true),
        (" #2r101", true),
        (" #2r-101", true),
        (" #16r2a", true),
        (" #16r+2A", true),
        (" #36rz", true),
        (" #1r0", false),
        (" #37r10", false),
        (" #2r2", false),
        (" #16r2g", false),
        (" #b2", false),
        (" #o8", false),
        (" #xg", false),
        (" #16r2a.", false),
        (" -#16r2a", false),
        (" #16R2a", false),
        (" #16r", false),
    ] {
        let temp = fixture(
            &format!("(defvar client{initializer})\n"),
            &synthetic_status("defvar"),
            &lcov(""),
        );
        let result = consume_fixture(&temp);
        if inert {
            assert_eq!(
                result.unwrap().ignored_points,
                1,
                "{initializer:?} should be an inert integer literal"
            );
        } else {
            assert_message(result, "uninstrumented synthetic point");
        }
    }
}

#[test]
fn evaluated_declaration_initializers_remain_measurable() {
    for initializer in [
        " (client-compute)",
        " client-value",
        " [#.(client-compute)]",
        " [client [#.(client-compute)]]",
        " [#_(client-compute)]",
        " [#s(client value)]",
        " `(client ,client-value)",
    ] {
        let temp = fixture(
            &format!("(defvar client{initializer})\n"),
            &synthetic_status("defvar"),
            &lcov(""),
        );
        assert_message(consume_fixture(&temp), "uninstrumented synthetic point");
    }
}

#[test]
fn structurally_exempt_forms_reject_ordinary_census_observations() {
    let temp = fixture(
        "(require 'client)\n",
        &status(
            "success",
            r#"[{"start_line":1,"kind":"require","points":[{"line":1,"kind":"ordinary"}]}]"#,
        ),
        &lcov("DA:1,1\n"),
    );
    assert_message(
        consume_fixture(&temp),
        "unexpectedly has an ordinary census point",
    );
}

#[test]
fn markers_on_automatically_structural_forms_are_stale() {
    let temp = fixture(
        "(provide 'client) ;; cov:ignore: obsolete\n",
        &synthetic_status("provide"),
        &lcov(""),
    );
    assert_message(consume_fixture(&temp), "automatically structural form");
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
fn cov_ignore_in_any_real_comment_must_use_the_trailing_marker_grammar() {
    let temp = fixture(
        "(defun client ()\n  (message \"x\")) ; cov:ignore: stale\n",
        &ordinary_status(),
        &lcov("DA:2,0\n"),
    );
    assert_message(consume_fixture(&temp), "malformed cov:ignore");
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

#[test]
fn source_reader_accepts_character_literals_as_list_heads() {
    // Character literals are reader atoms even when their syntax contains
    // delimiters that would otherwise start reader structure.
    let temp = fixture(
        "(defun character-literals ()\n  (memq character '(?\\\" ?\\' ?> ?λ ?\\N{LATIN SMALL LETTER A})))\n",
        &ordinary_status(),
        &lcov(""),
    );
    let path = temp.path().join("elisp/client.el");
    let forms = super::source::read_forms(&path).unwrap().1;
    assert_eq!(forms.len(), 1);
    assert_eq!(forms[0].start_line, 1);
    assert_eq!(forms[0].kind, "defun");
}

#[test]
fn source_reader_accepts_every_production_module() {
    // Intent: the host census validator must accept the exact source population
    // that the Emacs reader censused before instrumentation.
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    for entry in fs::read_dir(repo.join("elisp")).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() && path.extension().is_some_and(|extension| extension == "el") {
            super::source::read_forms(&path)
                .unwrap_or_else(|error| panic!("{}: {error:?}", path.display()));
        }
    }
}
