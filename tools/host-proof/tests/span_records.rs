//! test-local joins a `dev` run's trace without changing its report or status.
use std::{fs, path::Path, process::Command};

const TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
const CALLER_SPAN: &str = "00f067aa0ba902b7";

/// Keeps git, run directly or by test-local, to the fixture: no inherited
/// `GIT_*` override (as set inside a Git hook) and none of the developer's
/// own configuration or templates.
fn isolate<'a>(command: &'a mut Command, home: &Path) -> &'a mut Command {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        // Temp roots may live inside the real checkout in CI.
        .env("GIT_CEILING_DIRECTORIES", home.parent().unwrap())
}

/// A fixture directory: `checkout/` (a Git checkout), `home/` and `bundle/`.
struct Fixture {
    temp: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            temp: tempfile::tempdir().unwrap(),
        };
        let root = fixture.root();
        fs::create_dir(&root).unwrap();
        fs::create_dir(fixture.home()).unwrap();
        assert!(
            isolate(
                host_proof::git::at(&root).args(["init", "--quiet"]),
                &fixture.home()
            )
            .status()
            .unwrap()
            .success()
        );
        fs::write(root.join("flake.nix"), "{}").unwrap();
        fs::create_dir(root.join("tools")).unwrap();
        fs::write(root.join("tools/Cargo.toml"), "").unwrap();
        fixture
    }

    fn root(&self) -> std::path::PathBuf {
        self.temp.path().join("checkout")
    }

    fn home(&self) -> std::path::PathBuf {
        self.temp.path().join("home")
    }

    /// A CSR bundle whose Vite manifest is valid but which has no system
    /// artifacts: validation passes its manifest check and fails the next.
    fn bundle_without_inventory(&self) -> std::path::PathBuf {
        let bundle = self.temp.path().join("bundle");
        fs::create_dir_all(bundle.join(".vite")).unwrap();
        fs::create_dir_all(bundle.join("assets")).unwrap();
        fs::write(bundle.join("assets/index.js"), "entry").unwrap();
        fs::write(bundle.join("assets/csr_bg.wasm"), "wasm").unwrap();
        fs::write(
            bundle.join(".vite/manifest.json"),
            r#"{"index.html":{"file":"assets/index.js"},"csr_bg.wasm":{"file":"assets/csr_bg.wasm"}}"#,
        )
        .unwrap();
        bundle
    }

    fn invoke(&self, bundle: &Path, trace: &[(&str, &str)]) -> std::process::Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_test-local"));
        isolate(&mut command, &self.home())
            .current_dir(self.root())
            .env_remove("JAUNDER_TRACE_DIR")
            .env_remove("TRACEPARENT")
            .env_remove("TRACESTATE")
            .env("JAUNDER_CSR_BUNDLE_DIR", bundle)
            .envs(trace.iter().copied())
            .args(["--", "-p", "storage", "private-filter"]);
        command.output().unwrap()
    }

    /// Runs test-local inside the caller's step; returns its span records.
    fn traced(&self, bundle: &Path) -> Vec<serde_json::Value> {
        let run = self.temp.path().join("run.trace.d");
        fs::create_dir(&run).unwrap();
        let traceparent = format!("00-{TRACE_ID}-{CALLER_SPAN}-01");
        let output = self.invoke(
            bundle,
            &[
                ("JAUNDER_TRACE_DIR", run.to_str().unwrap()),
                ("TRACEPARENT", &traceparent),
            ],
        );
        assert_eq!(output.status.code(), Some(1));
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["command"], "test-local");
        let text = record_file(&run);
        // Spans carry names and outcomes, never arguments or environment.
        assert!(!text.contains("private"));
        text.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

/// The text of the run directory's one record file, named `<pid>-<random>.jsonl`.
fn record_file(directory: &Path) -> String {
    let files: Vec<_> = fs::read_dir(directory).unwrap().collect();
    assert_eq!(files.len(), 1, "one record file per process");
    let path = files.into_iter().next().unwrap().unwrap().path();
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let (pid, rest) = name.split_once('-').unwrap();
    assert!(
        pid.parse::<u32>().is_ok() && rest.ends_with(".jsonl"),
        "{name}"
    );
    fs::read_to_string(path).unwrap()
}

/// The `kind` record of the span `name`.
fn span<'a>(records: &'a [serde_json::Value], name: &str, kind: &str) -> &'a serde_json::Value {
    records
        .iter()
        .find(|record| record["name"] == name && record["kind"] == kind)
        .unwrap_or_else(|| panic!("no {kind} record for {name}: {records:?}"))
}

#[test]
fn phases_nest_under_the_callers_step() {
    let fixture = Fixture::new();
    let records = fixture.traced(&fixture.bundle_without_inventory());

    assert!(records.iter().all(|record| record["trace_id"] == TRACE_ID));
    let command = span(&records, "xtask.command", "start");
    assert_eq!(command["parent_id"], CALLER_SPAN);
    assert!(command.get("attributes").is_none());
    for (child, parent) in [
        ("xtask.checkout", "xtask.command"),
        ("xtask.hooks", "xtask.command"),
        ("xtask.dispatch", "xtask.command"),
        ("xtask.artifact-lock-wait", "xtask.dispatch"),
        ("test-local-csr-prerequisite", "xtask.dispatch"),
        ("xtask.csr-manifest-check", "test-local-csr-prerequisite"),
        ("xtask.csr-inventory-check", "test-local-csr-prerequisite"),
    ] {
        assert_eq!(
            span(&records, child, "start")["parent_id"],
            span(&records, parent, "start")["span_id"],
            "{child} under {parent}"
        );
        assert_eq!(
            span(&records, child, "end")["span_id"],
            span(&records, child, "start")["span_id"]
        );
    }
    assert_eq!(
        span(&records, "xtask.command", "end")["attributes"],
        serde_json::json!({"process.exit.code": 1, "jaunder.status": "failed"})
    );
    // The validation phases run one after the other, the second failing.
    assert!(
        span(&records, "xtask.csr-manifest-check", "end")["time_ns"].as_u64()
            <= span(&records, "xtask.csr-inventory-check", "start")["time_ns"].as_u64()
    );
    for (name, status) in [
        ("xtask.csr-manifest-check", "ok"),
        ("xtask.csr-inventory-check", "failed"),
        ("test-local-csr-prerequisite", "failed"),
    ] {
        assert_eq!(
            span(&records, name, "end")["attributes"],
            serde_json::json!({"jaunder.status": status}),
            "{name}"
        );
    }
}

#[test]
fn csr_validation_stops_at_its_first_failed_phase() {
    let fixture = Fixture::new();
    let bundle = fixture.bundle_without_inventory();
    fs::write(bundle.join(".vite/manifest.json"), "corrupt").unwrap();
    let records = fixture.traced(&bundle);

    assert_eq!(
        span(&records, "xtask.csr-manifest-check", "end")["attributes"],
        serde_json::json!({"jaunder.status": "failed"})
    );
    assert!(
        !records
            .iter()
            .any(|record| record["name"] == "xtask.csr-inventory-check"),
        "{records:?}"
    );
}

/// The report with its timings zeroed: everything tracing must leave alone.
fn report_without_timings(stdout: &[u8]) -> serde_json::Value {
    let mut report: serde_json::Value = serde_json::from_slice(stdout).unwrap();
    report["duration_ms"] = 0.into();
    for step in report["steps"].as_array_mut().unwrap() {
        step["duration_ms"] = 0.into();
    }
    report
}

#[test]
fn tracing_never_changes_the_report_or_status() {
    let fixture = Fixture::new();
    let bundle = fixture.bundle_without_inventory();
    // The first run installs the checkout's hooks and says so on stderr; the
    // runs compared below all start with them installed.
    fixture.invoke(&bundle, &[]);
    let untraced = fixture.invoke(&bundle, &[]);
    assert_eq!(untraced.status.code(), Some(1));
    let untraced_report = report_without_timings(&untraced.stdout);
    // The run reached CSR validation's phases, not an earlier refusal.
    assert_eq!(
        untraced_report["steps"].as_array().unwrap().last().unwrap()["name"],
        "test-local-csr-prerequisite"
    );

    let run = fixture.temp.path().join("run.trace.d");
    fs::create_dir(&run).unwrap();
    let blocked = fixture.temp.path().join("not-a-directory");
    fs::write(&blocked, "blocked").unwrap();
    let traceparent = format!("00-{TRACE_ID}-{CALLER_SPAN}-01");
    for trace in [
        vec![
            ("JAUNDER_TRACE_DIR", run.to_str().unwrap()),
            ("TRACEPARENT", traceparent.as_str()),
        ],
        vec![("JAUNDER_TRACE_DIR", "")],
    ] {
        let output = fixture.invoke(&bundle, &trace);
        assert_eq!(output.status, untraced.status, "{trace:?}");
        assert_eq!(
            report_without_timings(&output.stdout),
            untraced_report,
            "{trace:?}"
        );
        assert_eq!(output.stderr, untraced.stderr, "{trace:?}");
    }
    assert!(!record_file(&run).is_empty());

    let unwritable = fixture.invoke(&bundle, &[("JAUNDER_TRACE_DIR", blocked.to_str().unwrap())]);
    assert_eq!(unwritable.status, untraced.status);
    assert_eq!(report_without_timings(&unwritable.stdout), untraced_report);
    let stderr = String::from_utf8_lossy(&unwritable.stderr);
    let (warnings, rest): (Vec<_>, Vec<_>) = stderr
        .lines()
        .partition(|line| line.starts_with("host-proof: tracing disabled: "));
    assert_eq!(warnings.len(), 1, "{stderr}");
    assert_eq!(
        rest,
        String::from_utf8_lossy(&untraced.stderr)
            .lines()
            .collect::<Vec<_>>()
    );
}
