//! Shared inherited local-log inputs for the two executable roots.
//!
//! Each root invokes its own resolver and asserts its observed policy; this
//! fixture only owns inputs and isolated child process construction.

use std::path::Path;
use std::process::Command;

/// Selects each executable's local-policy child branch.
pub const LOCAL_LOG_CHILD: &str = "JAUNDER_TEST_LOCAL_LOG_POLICY_CHILD";

/// One local-filter precedence case shared by both executable tests.
pub struct LocalLogScenario {
    pub name: &'static str,
    preferred: Option<&'static str>,
    fallback: &'static str,
}

impl LocalLogScenario {
    /// Builds an isolated child without changing the parent's environment.
    #[must_use]
    pub fn command(&self, executable: &Path) -> Command {
        let mut command = Command::new(executable);
        command.args([
            "--exact",
            "tests::subprocess_preserves_inherited_local_log_policy",
            "--nocapture",
        ]);
        for name in [
            "JAUNDER_LOG_FILTER",
            "JAUNDER_LOG_FORMAT",
            "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "JAUNDER_SLOW_OP_MS",
            "JAUNDER_E2E_SEED_PROCESS",
        ] {
            command.env_remove(name);
        }
        command
            .env(LOCAL_LOG_CHILD, self.name)
            .env("RUST_LOG", self.fallback);
        if let Some(value) = self.preferred {
            command.env("JAUNDER_LOG_FILTER", value);
        }
        command
    }
}

/// Preferred and fallback inherited inputs must exercise both roots.
#[must_use]
pub fn local_log_scenarios() -> [LocalLogScenario; 2] {
    [
        LocalLogScenario {
            name: "override",
            preferred: Some("warn"),
            fallback: "debug",
        },
        LocalLogScenario {
            name: "fallback",
            preferred: None,
            fallback: "off",
        },
    ]
}
