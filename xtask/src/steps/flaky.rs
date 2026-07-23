//! Surface Playwright "flaky" tests — failed on the first attempt, passed on a
//! retry — from a combo's JSON report into the `e2e` command's result envelope.
//!
//! With `JAUNDER_E2E_RETRIES=1` (set by the warm gate in `flake.nix`) a
//! fail-then-pass exits 0 and the combo check goes green, so the flake would be
//! invisible without this. `nix::e2e_combo` has already lifted the report out of
//! the VM to
//! `.xtask/diagnostics/e2e-<backend>-<browser>/playwright-report-<backend>.json`
//! (best-effort, success path included); we read it there, attach the flaky
//! specs to the sidecar (`CommandResult.flaky`), print a `flaky:` sentinel (a
//! greppable line mirroring `xtask-done:`), and append a table to the GitHub
//! Actions run summary. Replaces the former inline `jq` step in `ci.yml` so the
//! logic is one tested code path rather than shell smeared across the matrix.

use std::io::Write;

use serde::Serialize;

use crate::result::{CommandResult, StepResult};

/// One flaky spec, identified for a human: source location + title. Serialized
/// into `.xtask/last-result.json` under `flaky`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct FlakySpec {
    pub file: String,
    pub line: u64,
    pub title: String,
}

/// Read the combo's lifted Playwright report and, if it has flaky tests, attach
/// them to `result`, emit the `flaky:` sentinel, and append to the GitHub step
/// summary. Best-effort: a missing report (e.g. the combo failed before
/// Playwright wrote one) is a silent no-op — `e2e_combo` already owns that
/// failure.
pub fn collect(result: &mut CommandResult, backend: &str, browser: &str) {
    let path =
        format!(".xtask/diagnostics/e2e-{backend}-{browser}/playwright-report-{backend}.json");
    let Ok(json) = std::fs::read_to_string(&path) else {
        return;
    };
    let specs = parse_flaky(&json);
    result.push(StepResult::ok("flaky-scan").detail(format!("{} flaky test(s)", specs.len())));
    if specs.is_empty() {
        return;
    }
    // One greppable line, only when there IS a flake — scoped to positive news,
    // unlike the always-on `xtask-done:`.
    let locations: Vec<String> = specs
        .iter()
        .map(|s| format!("{}:{}", s.file, s.line))
        .collect();
    eprintln!(
        "flaky: command={} count={} tests={}",
        result.command,
        specs.len(),
        locations.join(",")
    );
    append_step_summary(&result.command, &specs);
    result.flaky = specs;
}

/// Append the flaky table to `$GITHUB_STEP_SUMMARY` when running under GitHub
/// Actions. Best-effort: no env var (a local run) or an unwritable file is a
/// silent no-op.
fn append_step_summary(command: &str, specs: &[FlakySpec]) {
    let Ok(summary_path) = std::env::var("GITHUB_STEP_SUMMARY") else {
        return;
    };
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(summary_path)
    else {
        return;
    };
    let _ = file.write_all(render_summary(command, specs).as_bytes());
}

/// Render the Markdown block for the run summary. Pure, so it is unit-tested.
pub fn render_summary(command: &str, specs: &[FlakySpec]) -> String {
    let mut out = format!("### Flaky — {command}: {}\n", specs.len());
    for s in specs {
        out.push_str(&format!("- `{}:{}` \u{203a} {}\n", s.file, s.line, s.title));
    }
    out
}

/// Extract flaky specs from a Playwright JSON report. A spec is flaky when any
/// of its `tests` has `status == "flaky"` (Playwright's label for a
/// failed-then-passed-on-retry test). Walks the report recursively — Playwright
/// nests `suites` arbitrarily — matching any object shaped like a spec (has
/// `tests`, `file`, `line`, `title`). Untyped and tolerant: a malformed or
/// unexpected report yields an empty list rather than an error. Pure; the sole
/// tested core.
pub fn parse_flaky(report_json: &str) -> Vec<FlakySpec> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(report_json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk(&root, &mut out);
    out.sort();
    out.dedup();
    out
}

fn walk(value: &serde_json::Value, out: &mut Vec<FlakySpec>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(spec) = as_flaky_spec(map) {
                out.push(spec);
            }
            for child in map.values() {
                walk(child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                walk(item, out);
            }
        }
        _ => {}
    }
}

/// `Some` iff `map` is a spec object (`tests` + `file` + `line` + `title`) with
/// at least one flaky test.
fn as_flaky_spec(map: &serde_json::Map<String, serde_json::Value>) -> Option<FlakySpec> {
    let tests = map.get("tests")?.as_array()?;
    let file = map.get("file")?.as_str()?;
    let line = map.get("line")?.as_u64()?;
    let title = map.get("title")?.as_str()?;
    let flaky = tests
        .iter()
        .any(|t| t.get("status").and_then(serde_json::Value::as_str) == Some("flaky"));
    flaky.then(|| FlakySpec {
        file: file.to_owned(),
        line,
        title: title.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A report with two flaky specs (one nested in a child suite), one clean
    // spec, and matching `stats` — the shape Playwright's `json` reporter emits.
    const REPORT: &str = r#"{
      "stats": { "expected": 100, "unexpected": 0, "flaky": 2, "skipped": 0 },
      "suites": [
        {
          "title": "visibility.spec.ts",
          "specs": [
            {
              "title": "Subscriber sees the post",
              "file": "tests/visibility.spec.ts",
              "line": 150,
              "tests": [{ "status": "flaky" }]
            },
            {
              "title": "clean pass",
              "file": "tests/visibility.spec.ts",
              "line": 107,
              "tests": [{ "status": "expected" }]
            }
          ],
          "suites": [
            {
              "title": "nested",
              "specs": [
                {
                  "title": "nested flaky one",
                  "file": "tests/admin-site.spec.ts",
                  "line": 47,
                  "tests": [{ "status": "flaky" }]
                }
              ]
            }
          ]
        }
      ]
    }"#;

    #[test]
    fn parse_flaky_finds_nested_specs_sorted_and_deduped() {
        let specs = parse_flaky(REPORT);
        assert_eq!(
            specs,
            vec![
                FlakySpec {
                    file: "tests/admin-site.spec.ts".into(),
                    line: 47,
                    title: "nested flaky one".into(),
                },
                FlakySpec {
                    file: "tests/visibility.spec.ts".into(),
                    line: 150,
                    title: "Subscriber sees the post".into(),
                },
            ],
            "both flaky specs (incl. the nested one) found, sorted by file/line; the expected spec excluded"
        );
    }

    #[test]
    fn parse_flaky_returns_empty_for_no_flaky_or_malformed() {
        let no_flaky = r#"{ "suites": [ { "specs": [
            { "title": "ok", "file": "a.spec.ts", "line": 1, "tests": [{ "status": "expected" }] }
        ] } ] }"#;
        assert!(parse_flaky(no_flaky).is_empty());
        assert!(parse_flaky("not json at all").is_empty());
        // A spec missing `line` is not a valid spec shape — ignored, not a panic.
        let malformed = r#"{ "specs": [ { "title": "x", "file": "a.ts", "tests": [{ "status": "flaky" }] } ] }"#;
        assert!(parse_flaky(malformed).is_empty());
    }

    #[test]
    fn render_summary_lists_count_and_each_spec() {
        let specs = vec![FlakySpec {
            file: "tests/x.spec.ts".into(),
            line: 5,
            title: "t".into(),
        }];
        let md = render_summary("e2e-sqlite-firefox", &specs);
        assert!(md.starts_with("### Flaky \u{2014} e2e-sqlite-firefox: 1\n"));
        assert!(md.contains("- `tests/x.spec.ts:5` \u{203a} t\n"));
    }
}
