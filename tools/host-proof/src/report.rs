//! The focused-proof result contract: one JSON document on stdout, the process
//! status as the verdict. Human views are derived from the JSON by callers.

use std::time::Duration;

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct StepResult {
    pub name: String,
    pub ok: bool,
    pub skipped: bool,
    pub duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl StepResult {
    fn new(name: &str, ok: bool) -> Self {
        Self {
            name: name.into(),
            ok,
            skipped: false,
            duration_ms: 0,
            detail: None,
        }
    }
    pub fn ok(name: &str) -> Self {
        Self::new(name, true)
    }
    pub fn fail(name: &str) -> Self {
        Self::new(name, false)
    }
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration_ms = duration.as_millis();
        self
    }
}

#[derive(Debug, Serialize)]
pub struct CommandResult {
    pub command: String,
    pub ok: bool,
    pub duration_ms: u128,
    pub steps: Vec<StepResult>,
    /// A supervised child's conventional status that must survive the report.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_override: Option<i32>,
    /// A diagnostic's pre-rendered human report (trace tables), shown verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
}

impl CommandResult {
    pub fn new(command: &str) -> Self {
        Self {
            command: command.into(),
            ok: true,
            duration_ms: 0,
            steps: Vec::new(),
            exit_override: None,
            report: None,
        }
    }
    pub fn push(&mut self, step: StepResult) {
        self.steps.push(step);
        self.ok = self.steps.iter().all(|s| s.ok || s.skipped);
    }
    pub fn exit_code(&self) -> i32 {
        self.exit_override.unwrap_or(if self.ok { 0 } else { 1 })
    }
    /// Print the report as the single stdout document and return its status.
    pub fn emit(&self) -> i32 {
        match serde_json::to_string(self) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("{}: cannot serialize report: {error}", self.command);
                return 2;
            }
        }
        self.exit_code()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_ignores_skips_and_honors_override() {
        let mut result = CommandResult::new("x");
        result.push(StepResult::ok("a"));
        let mut skipped = StepResult::fail("b");
        skipped.skipped = true;
        result.push(skipped);
        assert!(result.ok);
        assert_eq!(result.exit_code(), 0);
        result.push(StepResult::fail("c").detail("boom"));
        assert_eq!(result.exit_code(), 1);
        result.exit_override = Some(130);
        assert_eq!(result.exit_code(), 130);
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["steps"][2]["detail"], "boom");
        assert!(json["steps"][0].get("detail").is_none());
    }
}
