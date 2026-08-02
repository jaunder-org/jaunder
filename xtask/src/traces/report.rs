//! Reader for Playwright's `json` reporter output (#794).
//!
//! Supplies the **denominator** for the span-coverage section: what Playwright
//! itself says a test took, wall-clock, including everything the trace cannot see.
//!
//! This is deliberately not `e2e.total_ms`. That attribute is the `e2e.test`
//! span's own duration — a different quantity, and precisely the one that
//! *excludes* the fixture overhead the coverage section exists to measure.
//! Comparing the span tree against the span's own duration would report ~100 %
//! coverage no matter how much time was invisible.
//!
//! The report lands per combo at
//! `.xtask/diagnostics/e2e-<backend>-<browser>/playwright-report-<backend>.json`
//! and is documented in `docs/observability.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

/// Identity of one test *attempt*, matching the `e2e.test.lifecycle` span's
/// `e2e.test` / `e2e.project` / `e2e.retry` attributes.
///
/// Retry is part of the key because spans are exported for every attempt, and a
/// retried test's attempts have genuinely different durations. Matching on title
/// alone would compare one attempt's spans against another attempt's wall-clock.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttemptKey {
    pub test: String,
    pub project: String,
    pub retry: u64,
}

/// Per-attempt wall-clock durations, keyed for joining against the span tree.
#[derive(Debug, Default, Clone)]
pub struct ReportedDurations(HashMap<AttemptKey, f64>);

impl ReportedDurations {
    pub fn get(&self, key: &AttemptKey) -> Option<f64> {
        self.0.get(key).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Parse one Playwright `json` report.
    ///
    /// Shape: `suites[]` nest arbitrarily; each carries `specs[]`, each spec has
    /// `tests[]` (one per project), each test has `results[]` indexed by retry.
    /// Unknown or missing fields are skipped rather than erroring — a report from
    /// a future Playwright with extra keys should still yield what it does have.
    pub fn from_value(root: &Value) -> Self {
        let mut durations = HashMap::new();
        if let Some(suites) = root.get("suites").and_then(Value::as_array) {
            for suite in suites {
                walk_suite(suite, &mut durations);
            }
        }
        Self(durations)
    }

    pub fn from_path(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read playwright report {}", path.display()))?;
        let value: Value = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse playwright report {}", path.display()))?;
        Ok(Self::from_value(&value))
    }

    /// Merge several combos' reports into one lookup.
    pub fn from_paths(paths: &[PathBuf]) -> Result<Self> {
        let mut merged = HashMap::new();
        for path in paths {
            merged.extend(Self::from_path(path)?.0);
        }
        Ok(Self(merged))
    }
}

fn walk_suite(suite: &Value, out: &mut HashMap<AttemptKey, f64>) {
    if let Some(specs) = suite.get("specs").and_then(Value::as_array) {
        for spec in specs {
            let title = spec
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let Some(tests) = spec.get("tests").and_then(Value::as_array) else {
                continue;
            };
            for test in tests {
                let project = test
                    .get("projectName")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let Some(results) = test.get("results").and_then(Value::as_array) else {
                    continue;
                };
                for result in results {
                    // `retry` is present on each result; fall back to positional
                    // index only if absent.
                    let retry = result.get("retry").and_then(Value::as_u64);
                    let Some(duration) = result.get("duration").and_then(Value::as_f64) else {
                        continue;
                    };
                    let key = AttemptKey {
                        test: title.clone(),
                        project: project.clone(),
                        retry: retry.unwrap_or(0),
                    };
                    out.insert(key, duration);
                }
            }
        }
    }
    // Suites nest; `describe` blocks and per-file suites both appear here.
    if let Some(children) = suite.get("suites").and_then(Value::as_array) {
        for child in children {
            walk_suite(child, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report() -> Value {
        json!({
            "suites": [{
                "title": "auth.spec.ts",
                "specs": [{
                    "title": "logs in",
                    "tests": [{
                        "projectName": "chromium",
                        "results": [
                            { "retry": 0, "duration": 1200.0 },
                            { "retry": 1, "duration": 900.0 }
                        ]
                    }]
                }],
                "suites": [{
                    "title": "nested describe",
                    "specs": [{
                        "title": "deep test",
                        "tests": [{
                            "projectName": "firefox",
                            "results": [{ "retry": 0, "duration": 42.0 }]
                        }]
                    }]
                }]
            }]
        })
    }

    #[test]
    fn reads_each_attempt_separately() {
        let durations = ReportedDurations::from_value(&report());
        let key = |retry| AttemptKey {
            test: "logs in".to_owned(),
            project: "chromium".to_owned(),
            retry,
        };
        // Retries are distinct attempts with distinct wall-clocks; collapsing them
        // would compare one attempt's spans against another's duration.
        assert_eq!(durations.get(&key(0)), Some(1200.0));
        assert_eq!(durations.get(&key(1)), Some(900.0));
    }

    #[test]
    fn descends_into_nested_suites() {
        let durations = ReportedDurations::from_value(&report());
        assert_eq!(
            durations.get(&AttemptKey {
                test: "deep test".to_owned(),
                project: "firefox".to_owned(),
                retry: 0,
            }),
            Some(42.0),
        );
    }

    #[test]
    fn unknown_test_is_absent_not_zero() {
        let durations = ReportedDurations::from_value(&report());
        // Absent must stay absent: a 0.0 denominator would render as "100%
        // uncovered" and look like a catastrophic instrumentation failure.
        assert_eq!(
            durations.get(&AttemptKey {
                test: "never ran".to_owned(),
                project: "chromium".to_owned(),
                retry: 0,
            }),
            None,
        );
    }

    #[test]
    fn empty_report_yields_no_rows() {
        let durations = ReportedDurations::from_value(&json!({}));
        assert!(durations.is_empty());
    }

    #[test]
    fn result_without_duration_is_skipped() {
        let durations = ReportedDurations::from_value(&json!({
            "suites": [{
                "specs": [{
                    "title": "t",
                    "tests": [{ "projectName": "chromium", "results": [{ "retry": 0 }] }]
                }]
            }]
        }));
        assert!(durations.is_empty());
    }
}
