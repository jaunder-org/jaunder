//! Child-environment fixtures for the two executable telemetry roots.
//!
//! This fixture owns scenario inputs and process setup, not policy assertions.
//! Each root still invokes its own resolver and checks its observed snapshot.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt as _;
use std::path::Path;
use std::process::Command;

/// Selects the child branch in each executable's policy-wiring test.
pub const TRACE_FILTER_CHILD: &str = "JAUNDER_TEST_TRACE_POLICY_CHILD";

/// One maintained inherited-input case and its expected local diagnostic.
pub struct TraceFilterScenario {
    pub name: &'static str,
    value: Option<OsString>,
    pub expected_diagnostic: &'static str,
}

impl TraceFilterScenario {
    /// Builds an isolated child without changing its parent's environment.
    #[must_use]
    pub fn command(&self, executable: &Path) -> Command {
        let mut command = Command::new(executable);
        command.args([
            "--exact",
            "tests::subprocess_keeps_inherited_trace_and_log_filters_independent",
            "--nocapture",
        ]);
        for name in [
            "JAUNDER_TRACE_FILTER",
            "JAUNDER_LOG_FORMAT",
            "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "JAUNDER_SLOW_OP_MS",
            "JAUNDER_E2E_SEED_PROCESS",
        ] {
            command.env_remove(name);
        }
        command
            .env(TRACE_FILTER_CHILD, self.name)
            .env("JAUNDER_LOG_FILTER", "warn")
            .env("RUST_LOG", "off");
        if let Some(value) = &self.value {
            command.env("JAUNDER_TRACE_FILTER", value);
        }
        command
    }
}

/// The same inherited inputs must exercise both executable roots.
#[must_use]
pub fn trace_filter_scenarios() -> [TraceFilterScenario; 5] {
    [
        ("override", Some(OsString::from("debug")), ""),
        ("absent", None, ""),
        ("blank", Some(OsString::from("   ")), warning()),
        (
            "malformed",
            Some(OsString::from("raw-token==secret")),
            warning(),
        ),
        (
            "nonunicode",
            Some(OsString::from_vec(vec![0xff])),
            warning(),
        ),
    ]
    .map(|(name, value, expected_diagnostic)| TraceFilterScenario {
        name,
        value,
        expected_diagnostic,
    })
}

fn warning() -> &'static str {
    "host.telemetry.trace_filter: invalid configured value; using INFO trace filter\n"
}
