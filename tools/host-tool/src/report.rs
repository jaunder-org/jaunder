//! The host tool result contract: one JSON document on stdout, the process
//! status as the verdict. Human views are derived from the JSON by callers
//! (`scripts/step-report.jq`).

use std::io::Write;
use std::sync::{Mutex, OnceLock};
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
    /// A command's versioned machine-readable payload (e.g. the WASM coverage
    /// aggregate), carried verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<serde_json::Value>,
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
            evidence: None,
        }
    }
    /// A command that could not run: one failed step and status 2, so even a
    /// refusal or top-level error is still exactly one report.
    pub fn refused(command: &str, error: impl std::fmt::Display) -> Self {
        let mut result = Self::new(command);
        result.push(StepResult::fail(command).detail(error.to_string()));
        result.exit_override = Some(2);
        result
    }
    pub fn push(&mut self, step: StepResult) {
        self.steps.push(step);
        self.ok = self.steps.iter().all(|s| s.ok || s.skipped);
    }
    pub fn exit_code(&self) -> i32 {
        self.exit_override.unwrap_or(if self.ok { 0 } else { 1 })
    }
    /// Write the report as the single stdout document and return its status.
    pub fn emit(&self) -> i32 {
        let json = match serde_json::to_string(self) {
            Ok(json) => json,
            Err(error) => {
                eprintln!("{}: cannot serialize report: {error}", self.command);
                return 2;
            }
        };
        let written = match RESERVED_STDOUT.get() {
            Some(reserved) => match reserved.lock() {
                Ok(mut stdout) => writeln!(stdout, "{json}").and_then(|()| stdout.flush()),
                Err(_) => Err(std::io::Error::other("reserved stdout poisoned")),
            },
            None => writeln!(std::io::stdout(), "{json}"),
        };
        if let Err(error) = written {
            eprintln!("{}: cannot write report: {error}", self.command);
            return 2;
        }
        self.exit_code()
    }
}

/// The real stdout, reserved for the report once [`parse_args`] runs.
static RESERVED_STDOUT: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

/// Parse arguments, keeping stdout for the JSON report: help and version text
/// go to stderr (exit 0) and usage errors keep clap's stderr/exit 2 contract.
/// Afterwards the process's stdout descriptor points at stderr, so every child
/// process and stray print lands there and only [`CommandResult::emit`] writes
/// to the reserved real stdout.
pub fn parse_args<C: clap::Parser>() -> C {
    let cli = match C::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            eprint!("{}", error.render().ansi());
            std::process::exit(if error.use_stderr() { 2 } else { 0 });
        }
    };
    if let Err(error) = reserve_stdout() {
        eprintln!("warning: cannot reserve stdout for the report: {error}");
    }
    cli
}

#[cfg(unix)]
fn reserve_stdout() -> std::io::Result<()> {
    use std::os::fd::AsFd;
    std::io::stdout().flush()?;
    let saved = rustix::io::dup(std::io::stdout().as_fd())?;
    rustix::stdio::dup2_stdout(std::io::stderr().as_fd())?;
    let _ = RESERVED_STDOUT.set(Mutex::new(std::fs::File::from(saved)));
    Ok(())
}

#[cfg(not(unix))]
fn reserve_stdout() -> std::io::Result<()> {
    Ok(())
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
