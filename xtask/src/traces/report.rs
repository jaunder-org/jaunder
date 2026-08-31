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

use anyhow::{Context, Result, ensure};

use crate::playwright_report::{PlaywrightReport, PlaywrightSpec};

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
///
/// **Scoped by trace-file source, because `AttemptKey` alone is ambiguous across
/// backends.** Playwright's `projectName` is the browser (`chromium`/`firefox`)
/// and carries no backend, so `sqlite × chromium` and `postgres × chromium`
/// produce identical keys with *different* durations. `traces run` builds every
/// combo at once, so a flat map would let one backend's durations overwrite the
/// other's and silently mis-state roughly half the coverage rows.
///
/// `per_source` keys on `Span.source` (the per-combo trace file) so each combo
/// joins against its own report. `global` is the single-combo path, where the
/// caller passes reports with no pairing information; a conflicting duplicate
/// there is an error rather than a silent overwrite.
#[derive(Debug, Default, Clone)]
pub struct ReportedDurations {
    per_source: HashMap<String, HashMap<AttemptKey, f64>>,
    global: HashMap<AttemptKey, f64>,
}

impl ReportedDurations {
    /// Look up an attempt, preferring the entry for its own trace file.
    pub fn get(&self, source: &str, key: &AttemptKey) -> Option<f64> {
        self.per_source
            .get(source)
            .and_then(|scoped| scoped.get(key))
            .or_else(|| self.global.get(key))
            .copied()
    }

    pub fn is_empty(&self) -> bool {
        self.per_source.values().all(HashMap::is_empty) && self.global.is_empty()
    }

    pub fn len(&self) -> usize {
        self.per_source.values().map(HashMap::len).sum::<usize>() + self.global.len()
    }

    /// Parse one Playwright `json` report.
    ///
    /// The shared report seam owns the nested suite traversal and rejects a
    /// structurally incomplete spec/test array before duration extraction.
    /// Build from an in-memory report. Test constructor — the production paths
    /// read from disk via [`Self::from_paths`] / [`Self::from_labeled`].
    #[cfg(test)]
    pub fn from_value(root: &serde_json::Value) -> Self {
        Self::try_from_value(root).expect("test Playwright report must be structurally valid")
    }

    #[cfg(test)]
    fn try_from_value(root: &serde_json::Value) -> Result<Self> {
        let report =
            serde_json::from_value(root.clone()).context("failed to parse Playwright report")?;
        Ok(Self {
            per_source: HashMap::new(),
            global: attempts_in(&report),
        })
    }

    fn read(path: &Path) -> Result<HashMap<AttemptKey, f64>> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read playwright report {}", path.display()))?;
        let report: PlaywrightReport = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse playwright report {}", path.display()))?;
        Ok(attempts_in(&report))
    }

    /// Unpaired reports, as the CLI's `--playwright-report` supplies them.
    ///
    /// Errors on a key present in two reports with **different** durations: that
    /// means reports from two combos were passed with no way to tell which trace
    /// belongs to which, and silently keeping one would mis-state the coverage
    /// numbers. Callers that know the pairing use [`Self::from_labeled`].
    pub fn from_paths(paths: &[PathBuf]) -> Result<Self> {
        let mut merged: HashMap<AttemptKey, f64> = HashMap::new();
        for path in paths {
            for (key, duration) in Self::read(path)? {
                if let Some(existing) = merged.get(&key) {
                    ensure!(
                        (*existing - duration).abs() < f64::EPSILON,
                        "playwright reports disagree on {} [{}] retry {}: {existing} ms vs {duration} ms. \
                         Reports from different backends share test+project+retry keys, so they cannot be \
                         merged — analyze one combo at a time, or use `traces run`, which pairs each report \
                         with its own trace file.",
                        key.test,
                        key.project,
                        key.retry,
                    );
                }
                merged.insert(key, duration);
            }
        }
        Ok(Self {
            per_source: HashMap::new(),
            global: merged,
        })
    }

    /// Reports paired with the `Span.source` label of the trace file they belong
    /// to — the `traces run` path, where every combo is analyzed together.
    pub fn from_labeled(pairs: &[(String, PathBuf)]) -> Result<Self> {
        let mut per_source = HashMap::new();
        for (source, path) in pairs {
            per_source.insert(source.clone(), Self::read(path)?);
        }
        Ok(Self {
            per_source,
            global: HashMap::new(),
        })
    }
}

fn attempts_in(report: &PlaywrightReport) -> HashMap<AttemptKey, f64> {
    let mut out = HashMap::new();
    report.visit_specs(&mut |spec| insert_attempts(spec, &mut out));
    out
}

fn insert_attempts(spec: &PlaywrightSpec, out: &mut HashMap<AttemptKey, f64>) {
    let title = spec.title.clone().unwrap_or_default();
    for test in &spec.tests {
        let project = test.project_name.clone().unwrap_or_default();
        for result in &test.results {
            let Some(duration) = result.duration else {
                continue;
            };
            let key = AttemptKey {
                test: title.clone(),
                project: project.clone(),
                retry: result.retry.unwrap_or(0).into(),
            };
            out.insert(key, duration);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
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
        assert_eq!(durations.get("any-source", &key(0)), Some(1200.0));
        assert_eq!(durations.get("any-source", &key(1)), Some(900.0));
    }

    #[test]
    fn descends_into_nested_suites() {
        let durations = ReportedDurations::from_value(&report());
        assert_eq!(
            durations.get(
                "any-source",
                &AttemptKey {
                    test: "deep test".to_owned(),
                    project: "firefox".to_owned(),
                    retry: 0,
                }
            ),
            Some(42.0),
        );
    }

    #[test]
    fn unknown_test_is_absent_not_zero() {
        let durations = ReportedDurations::from_value(&report());
        // Absent must stay absent: a 0.0 denominator would render as "100%
        // uncovered" and look like a catastrophic instrumentation failure.
        assert_eq!(
            durations.get(
                "any-source",
                &AttemptKey {
                    test: "never ran".to_owned(),
                    project: "chromium".to_owned(),
                    retry: 0,
                }
            ),
            None,
        );
    }

    // The defect this guards: `projectName` is the BROWSER and names no backend,
    // so sqlite×chromium and postgres×chromium yield identical keys with
    // different durations. A flat merge silently kept one and mis-stated roughly
    // half the coverage rows.
    #[test]
    fn per_source_scoping_keeps_two_backends_apart() {
        let mut durations = ReportedDurations::default();
        durations.per_source.insert(
            "sqlite-chromium.jsonl".to_owned(),
            attempts_in(&serde_json::from_value(report()).unwrap()),
        );
        durations.per_source.insert(
            "postgres-chromium.jsonl".to_owned(),
            attempts_in(
                &serde_json::from_value(json!({
                    "suites": [{
                        "specs": [{
                            "title": "logs in",
                            "tests": [{
                                "projectName": "chromium",
                                "results": [{ "retry": 0, "duration": 5555.0 }]
                            }]
                        }]
                    }]
                }))
                .unwrap(),
            ),
        );
        let key = AttemptKey {
            test: "logs in".to_owned(),
            project: "chromium".to_owned(),
            retry: 0,
        };
        assert_eq!(durations.get("sqlite-chromium.jsonl", &key), Some(1200.0));
        assert_eq!(durations.get("postgres-chromium.jsonl", &key), Some(5555.0));
    }

    #[test]
    fn unpaired_reports_that_disagree_are_an_error_not_an_overwrite() {
        let dir = std::env::temp_dir().join(format!("report-conflict-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let one = dir.join("a.json");
        let two = dir.join("b.json");
        std::fs::write(&one, report().to_string()).unwrap();
        std::fs::write(
            &two,
            json!({
                "suites": [{
                    "specs": [{
                        "title": "logs in",
                        "tests": [{
                            "projectName": "chromium",
                            "results": [{ "retry": 0, "duration": 9999.0 }]
                        }]
                    }]
                }]
            })
            .to_string(),
        )
        .unwrap();

        let err = ReportedDurations::from_paths(&[one, two]).unwrap_err();
        std::fs::remove_dir_all(&dir).ok();
        let message = err.to_string();
        assert!(
            message.contains("disagree") && message.contains("logs in"),
            "the error must name the colliding attempt, not silently pick one: {message}",
        );
    }

    #[test]
    fn identical_duplicates_across_reports_are_fine() {
        // Same combo's report passed twice is harmless — only DISAGREEMENT is
        // ambiguous.
        let dir = std::env::temp_dir().join(format!("report-dupe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let one = dir.join("a.json");
        let two = dir.join("b.json");
        std::fs::write(&one, report().to_string()).unwrap();
        std::fs::write(&two, report().to_string()).unwrap();
        let merged = ReportedDurations::from_paths(&[one, two]);
        std::fs::remove_dir_all(&dir).ok();
        assert!(merged.is_ok());
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
