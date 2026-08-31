//! Strict shared model for Playwright's JSON reporter output.
//!
//! The reporter tree is nested by suite, while each spec carries its project
//! tests. Consumers use [`PlaywrightReport::visit_specs`] so traversal and the
//! structural arrays that define the report population have one owner.

use serde::Deserialize;

/// The root Playwright JSON report.
#[derive(Debug, Deserialize)]
pub(crate) struct PlaywrightReport {
    #[serde(default)]
    pub suites: Vec<PlaywrightSuite>,
}

/// A recursively nested Playwright suite.
#[derive(Debug, Deserialize)]
pub(crate) struct PlaywrightSuite {
    #[serde(default)]
    pub suites: Vec<PlaywrightSuite>,
    pub specs: Vec<PlaywrightSpec>,
}

/// One selected Playwright spec.
#[derive(Debug, Deserialize)]
pub(crate) struct PlaywrightSpec {
    pub id: Option<String>,
    pub title: Option<String>,
    pub file: Option<String>,
    pub line: Option<u64>,
    pub tests: Vec<PlaywrightTest>,
}

/// One project's execution of a spec.
#[derive(Debug, Deserialize)]
pub(crate) struct PlaywrightTest {
    #[serde(rename = "projectId")]
    pub project_id: Option<String>,
    #[serde(rename = "projectName")]
    pub project_name: Option<String>,
    #[serde(default)]
    pub results: Vec<PlaywrightAttempt>,
}

/// One completed Playwright attempt.
#[derive(Debug, Deserialize)]
pub(crate) struct PlaywrightAttempt {
    pub retry: Option<u32>,
    pub duration: Option<f64>,
}

impl PlaywrightReport {
    /// Visit every spec in reporter order, including nested suites.
    pub(crate) fn visit_specs<'a>(&'a self, visit: &mut impl FnMut(&'a PlaywrightSpec)) {
        for suite in &self.suites {
            suite.visit_specs(visit);
        }
    }
}

impl PlaywrightSuite {
    fn visit_specs<'a>(&'a self, visit: &mut impl FnMut(&'a PlaywrightSpec)) {
        for spec in &self.specs {
            visit(spec);
        }
        for suite in &self.suites {
            suite.visit_specs(visit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visits_nested_specs_in_reporter_order() {
        let report: PlaywrightReport = serde_json::from_str(
            r#"{"suites":[{"suites":[{"suites":[],"specs":[{"tests":[]}]}],"specs":[{"tests":[]}]}]}"#,
        )
        .unwrap();
        let mut specs = Vec::new();
        report.visit_specs(&mut |spec| specs.push(spec.tests.len()));
        assert_eq!(specs, [0, 0]);
    }

    #[test]
    fn rejects_a_spec_without_its_required_tests_array() {
        let error =
            serde_json::from_str::<PlaywrightReport>(r#"{"suites":[{"suites":[],"specs":[{}]}]}"#)
                .unwrap_err();
        assert!(error.to_string().contains("tests"));
    }
}
