use std::io::Write;
use std::path::Path;

use serde::Serialize;

#[derive(Clone, Copy)]
pub enum Mode {
    Fix,
    Check,
}

#[derive(Serialize)]
pub struct StepResult {
    pub name: String,
    pub ok: bool,
    pub skipped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl StepResult {
    pub fn ok(name: &str) -> Self {
        Self {
            name: name.into(),
            ok: true,
            skipped: false,
            detail: None,
        }
    }
    pub fn fail(name: &str) -> Self {
        Self {
            name: name.into(),
            ok: false,
            skipped: false,
            detail: None,
        }
    }
    pub fn skip(name: &str) -> Self {
        Self {
            name: name.into(),
            ok: true,
            skipped: true,
            detail: None,
        }
    }
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

#[derive(Serialize)]
pub struct CommandResult {
    pub command: String,
    pub ok: bool,
    pub duration_ms: u128,
    pub finished_at_unix: u64,
    pub steps: Vec<StepResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<crate::coverage::CoverageReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit: Option<crate::audit_wasm::AuditReport>,
    /// Pre-rendered `traces analyze` report text. Human-facing only — `traces
    /// analyze` rejects `--json`, so this is never serialized (skipped when None,
    /// and never Some on a `--json` run).
    #[serde(skip)]
    pub traces: Option<String>,
}

impl CommandResult {
    pub fn new(command: &str) -> Self {
        Self {
            command: command.into(),
            ok: true,
            duration_ms: 0,
            finished_at_unix: 0,
            steps: Vec::new(),
            coverage: None,
            audit: None,
            traces: None,
        }
    }

    pub fn push(&mut self, step: StepResult) {
        self.steps.push(step);
        self.ok = self.steps.iter().all(|s| s.ok || s.skipped);
    }

    pub fn exit_code(&self) -> i32 {
        if self.ok {
            0
        } else {
            1
        }
    }

    pub fn report(&self, json: bool) {
        if let Err(err) = self.write_sidecar() {
            eprintln!("xtask: warning: could not write sidecar: {err}");
        }
        if json {
            println!("{}", serde_json::to_string_pretty(self).unwrap());
        } else {
            self.print_human();
        }
    }

    fn write_sidecar(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(".xtask")?;
        let mut f = std::fs::File::create(Path::new(".xtask/last-result.json"))?;
        f.write_all(serde_json::to_string_pretty(self).unwrap().as_bytes())?;
        Ok(())
    }

    fn print_human(&self) {
        for s in &self.steps {
            let mark = if s.skipped {
                "skip"
            } else if s.ok {
                " ok "
            } else {
                "FAIL"
            };
            let detail = s
                .detail
                .as_deref()
                .map(|d| format!(" — {d}"))
                .unwrap_or_default();
            println!("[{mark}] {}{detail}", s.name);
        }
        // Informational payload: the audit subcommand's whole point is this table,
        // not the pass/fail line, so render it inline when present.
        if let Some(audit) = &self.audit {
            print!("{}", crate::audit_wasm::render_table(audit));
        }
        // Same informational-payload treatment for `traces analyze`: the report
        // tables are the point, not the pass/fail line.
        if let Some(traces) = &self.traces {
            print!("{traces}");
        }
        let verdict = if self.ok { "PASSED" } else { "FAILED" };
        println!(
            "xtask {} {verdict} in {} ms",
            self.command, self.duration_ms
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_ok_reflects_steps_and_serializes_flat() {
        let mut r = CommandResult::new("validate");
        r.push(StepResult::ok("clippy").detail("0 warnings"));
        r.push(StepResult::fail("nix-coverage"));
        assert!(!r.ok);
        assert_eq!(r.exit_code(), 1);

        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["command"], "validate");
        assert_eq!(v["ok"], false);
        assert_eq!(v["steps"][0]["name"], "clippy");
        assert_eq!(v["steps"][0]["detail"], "0 warnings");
        assert_eq!(v["steps"][1]["ok"], false);
    }

    #[test]
    fn audit_report_serializes_in_envelope() {
        let mut r = CommandResult::new("audit-wasm");
        r.push(StepResult::ok("audit-wasm").detail("2 artifact(s)"));
        r.audit = Some(crate::audit_wasm::AuditReport {
            site_path: "/nix/store/x-jaunder-site".into(),
            artifacts: vec![crate::audit_wasm::ArtifactMetrics {
                path: "/nix/store/x-jaunder-site/pkg/jaunder.wasm".into(),
                raw_bytes: 2 * 1024 * 1024,
                gzip_bytes: 700 * 1024,
                brotli_bytes: 600 * 1024,
            }],
        });
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["audit"]["site_path"], "/nix/store/x-jaunder-site");
        assert_eq!(v["audit"]["artifacts"][0]["raw_bytes"], 2 * 1024 * 1024);
    }

    #[test]
    fn skipped_step_does_not_fail_result() {
        let mut r = CommandResult::new("check");
        r.push(StepResult::skip("clippy"));
        assert!(r.ok);
        assert_eq!(r.exit_code(), 0);
    }
}
