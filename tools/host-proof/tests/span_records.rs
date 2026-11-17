//! csr-prerequisite joins a `dev` run's trace without changing its output or status.
use std::{fs, path::Path, process::Command};

const TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
const CALLER_SPAN: &str = "00f067aa0ba902b7";

/// A temporary directory holding the bundle and, when traced, the run directory.
struct Fixture {
    temp: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            temp: tempfile::tempdir().unwrap(),
        }
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
        Command::new(env!("CARGO_BIN_EXE_csr-prerequisite"))
            .arg(bundle)
            .env_remove("JAUNDER_TRACE_DIR")
            .env_remove("TRACEPARENT")
            .env_remove("TRACESTATE")
            .envs(trace.iter().copied())
            .output()
            .unwrap()
    }

    /// Runs csr-prerequisite inside the caller's step; returns its span records.
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
        assert!(output.stdout.is_empty());
        let text = record_file(&run);
        // Spans carry names and outcomes, never arguments or environment.
        assert!(!text.contains("bundle"));
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
    for name in ["xtask.csr-manifest-check", "xtask.csr-inventory-check"] {
        assert_eq!(
            span(&records, name, "start")["parent_id"],
            CALLER_SPAN,
            "{name} under the caller's step"
        );
        assert_eq!(
            span(&records, name, "end")["span_id"],
            span(&records, name, "start")["span_id"]
        );
    }
    // The validation phases run one after the other, the second failing.
    assert!(
        span(&records, "xtask.csr-manifest-check", "end")["time_ns"].as_u64()
            <= span(&records, "xtask.csr-inventory-check", "start")["time_ns"].as_u64()
    );
    for (name, status) in [
        ("xtask.csr-manifest-check", "ok"),
        ("xtask.csr-inventory-check", "failed"),
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

#[test]
fn a_missing_root_is_refused_on_stderr() {
    let fixture = Fixture::new();
    let output = fixture.invoke(&fixture.temp.path().join("missing"), &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("csr-prerequisite: resolving CSR bundle root"),
        "{stderr}"
    );
}

#[test]
fn tracing_never_changes_the_output_or_status() {
    let fixture = Fixture::new();
    let bundle = fixture.bundle_without_inventory();
    let untraced = fixture.invoke(&bundle, &[]);
    assert_eq!(untraced.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&untraced.stderr).contains("invalid system artifacts"),
        "the run reached CSR validation's second phase"
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
        assert_eq!(output.stdout, untraced.stdout, "{trace:?}");
        assert_eq!(output.stderr, untraced.stderr, "{trace:?}");
    }
    assert!(!record_file(&run).is_empty());

    let unwritable = fixture.invoke(&bundle, &[("JAUNDER_TRACE_DIR", blocked.to_str().unwrap())]);
    assert_eq!(unwritable.status, untraced.status);
    assert_eq!(unwritable.stdout, untraced.stdout);
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
