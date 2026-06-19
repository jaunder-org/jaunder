// Minimal stub — Task 3 will replace this with the full implementation,
// tests, JSON serialization, and sidecar writer.

#[derive(Clone, Copy)]
pub enum Mode {
    Fix,
    Check,
}

pub struct StepResult {
    pub name: String,
    pub ok: bool,
    pub skipped: bool,
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
    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
}

pub struct CommandResult {
    pub command: String,
    pub ok: bool,
    pub memoized: bool,
    pub steps: Vec<StepResult>,
}

impl CommandResult {
    pub fn new(command: &str) -> Self {
        Self {
            command: command.into(),
            ok: true,
            memoized: false,
            steps: Vec::new(),
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

    pub fn report(&self, _json: bool) {
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
        let verdict = if self.ok { "PASSED" } else { "FAILED" };
        println!("xtask {} {verdict}", self.command);
    }
}
