use std::time::Instant;

use xshell::Shell;

use crate::result::{PhaseName, PhaseOutcome, PhaseRecord, StepResult};

/// Run a command as a named step. Non-zero exit becomes a failed StepResult
/// rather than a panic, so one failing step does not abort the others.
/// On failure, stderr (and stdout if non-empty) are captured into `detail`.
pub fn step(sh: &Shell, name: &str, program: &str, args: &[&str]) -> StepResult {
    step_with_env(sh, name, program, args, &[])
}

pub fn step_with_env(
    sh: &Shell,
    name: &str,
    program: &str,
    args: &[&str],
    env: &[(String, String)],
) -> StepResult {
    let traced = crate::invocation_trace::active()
        && matches!(name, "test-local-image-worker" | "test-local");
    let phase = traced.then(|| {
        crate::invocation_trace::phase(if name == "test-local" {
            "test-local"
        } else {
            "test-local-image-worker"
        })
    });
    let start = (!traced).then(Instant::now);
    let mut cmd = sh.cmd(program).args(args).quiet().ignore_status();
    for (key, value) in env {
        cmd = cmd.env(key, value);
    }
    let result = match cmd.output() {
        Ok(output) => {
            let phases = (name == "ert").then(|| {
                // Check uniqueness across both completion outcomes first. Only
                // the sole event may then be matched to the devtool status.
                let phase = ert_child_phase(&output.stderr);
                let expected = if output.status.success() {
                    PhaseOutcome::Success
                } else {
                    PhaseOutcome::Failed
                };
                if phase.outcome == expected {
                    phase
                } else {
                    PhaseRecord::unavailable(
                        PhaseName::GateExecution,
                        "ert: missing or conflicting current child evidence",
                    )
                }
            });
            let step = if output.status.success() {
                StepResult::ok(name)
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let detail = match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
                    (false, false) => format!("{}\n{}", stdout.trim(), stderr.trim()),
                    (false, true) => stdout.trim().to_string(),
                    (true, false) => stderr.trim().to_string(),
                    (true, true) => {
                        format!("exited with status {}", output.status.code().unwrap_or(-1))
                    }
                };
                StepResult::fail(name).detail(detail)
            };
            step.phases(phases)
        }
        Err(err) => {
            let phases = (name == "ert").then(|| ert_child_phase(&[]));
            StepResult::fail(name)
                .detail(err.to_string())
                .phases(phases)
        }
    };
    if let Some(phase) = phase {
        phase.finish(result.ok);
    }
    match start {
        Some(start) => result.with_duration(start.elapsed()),
        None => result,
    }
}

/// Only the current devtool subprocess's captured stderr is eligible. Do not
/// use retained diagnostic logs or Nix realization as child timing evidence.
pub(crate) fn ert_child_phase(stderr: &[u8]) -> PhaseRecord {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Event {
        event: String,
        schema_version: u8,
        check: String,
        duration_ms: u64,
        outcome: String,
    }
    const MARKER: &[u8] = b"devtool-check-timing";
    let unavailable = || {
        PhaseRecord::unavailable(
            PhaseName::GateExecution,
            "ert: missing, invalid or ambiguous current child event",
        )
    };
    let mut observed = None;
    for line in stderr.split(|b| *b == b'\n') {
        // Ordinary diagnostics are not events. Once the completion-event name
        // appears, however, a bad candidate must not hide beside a valid one.
        if !line.windows(MARKER.len()).any(|w| w == MARKER) {
            continue;
        }
        // Nix merges child stdout/stderr: Emacs can leave stdout unterminated
        // before the event. Accept only one bounded JSON suffix, not nested
        // JSON, concatenated markers, or an oversized/truncated candidate.
        let Some(start) = line.iter().rposition(|b| *b == b'{') else {
            return unavailable();
        };
        if line.len() - start > 256 || line[..start].windows(MARKER.len()).any(|w| w == MARKER) {
            return unavailable();
        }
        let Ok(event) = serde_json::from_slice::<Event>(&line[start..]) else {
            return unavailable();
        };
        if event.event != "devtool-check-timing"
            || event.schema_version != 1
            || event.check != "ert"
            || !matches!(event.outcome.as_str(), "success" | "failure")
            || observed.is_some()
        {
            return unavailable();
        }
        observed = Some(event);
    }
    let Some(event) = observed else {
        return unavailable();
    };
    PhaseRecord {
        name: PhaseName::GateExecution,
        duration_ms: Some(u128::from(event.duration_ms)),
        outcome: if event.outcome == "success" {
            PhaseOutcome::Success
        } else {
            PhaseOutcome::Failed
        },
        detail: "ert: current devtool child (monotonic interval)".into(),
        nix: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::{PhaseName, PhaseOutcome};

    const EVENT: &str = r#"{"event":"devtool-check-timing","schema_version":1,"check":"ert","duration_ms":31,"outcome":"success"}"#;

    #[test]
    fn ert_current_child_event_is_bounded_unique_and_versioned() {
        let phase = ert_child_phase(EVENT.as_bytes());
        assert_eq!(phase.name, PhaseName::GateExecution);
        assert_eq!(phase.duration_ms, Some(31));
        assert_eq!(phase.outcome, PhaseOutcome::Success);
        assert_eq!(
            ert_child_phase(format!("Emacs output{EVENT}").as_bytes()).duration_ms,
            Some(31)
        );
        let failed = EVENT.replace("success", "failure");
        assert_eq!(
            ert_child_phase(failed.as_bytes()).outcome,
            PhaseOutcome::Failed
        );
        for missing in [
            String::new(),
            "not json".into(),
            EVENT.replace("schema_version\":1", "schema_version\":2"),
            EVENT.replace("ert", "tsc"),
            EVENT.replace("31", "-1"),
            EVENT.replace("success", "unknown"),
            format!("{EVENT}\n{EVENT}"),
            format!("{EVENT}\n{}", EVENT.replace("success", "failure")),
            format!(r#"{{"event":"{}"}}"#, "x".repeat(256)),
            format!("{EVENT}{EVENT}"),
        ] {
            let phase = ert_child_phase(missing.as_bytes());
            assert_eq!(phase.duration_ms, None, "{missing}");
            assert_eq!(phase.outcome, PhaseOutcome::Unavailable);
        }
    }

    #[cfg(unix)]
    #[test]
    fn ert_candidate_errors_cannot_hide_beside_a_valid_completion() {
        let sh = Shell::new().unwrap();
        for invalid in [
            EVENT.replace("schema_version\":1", "schema_version\":2"),
            EVENT.replace("duration_ms\":31", "duration_ms\":\"31\""),
            EVENT.trim_end_matches('}').to_string(),
            format!("{EVENT}{}", " ".repeat(256)),
        ] {
            for stderr in [format!("{EVENT}\n{invalid}"), format!("{invalid}\n{EVENT}")] {
                assert_eq!(
                    ert_child_phase(stderr.as_bytes()).outcome,
                    PhaseOutcome::Unavailable,
                    "{stderr}"
                );
                for exit in ["0", "23"] {
                    let result = step(
                        &sh,
                        "ert",
                        "sh",
                        &[
                            "-c",
                            "printf '%s\\n' \"$1\" >&2; exit \"$2\"",
                            "child",
                            &stderr,
                            exit,
                        ],
                    );
                    assert_eq!(result.ok, exit == "0");
                    assert_eq!(result.phases[0].outcome, PhaseOutcome::Unavailable);
                    if exit != "0" {
                        assert!(
                            result
                                .detail
                                .as_deref()
                                .unwrap()
                                .contains(invalid.trim_end())
                        );
                    }
                }
            }
        }
        assert_eq!(
            ert_child_phase(
                format!("ordinary diagnostic\n{EVENT}\nnot JSON diagnostics").as_bytes()
            )
            .duration_ms,
            Some(31)
        );
    }

    #[cfg(unix)]
    #[test]
    fn ert_host_projects_only_current_stderr_and_keeps_step_duration() {
        let sh = Shell::new().unwrap();
        let script = format!("sleep 0.04; printf '%s\\n' '{EVENT}' >&2");
        let step = step(&sh, "ert", "sh", &["-c", &script]);
        assert!(step.ok);
        assert!(step.duration_ms >= 30);
        assert_eq!(step.phases[0].duration_ms, Some(31));
        let stdout_only = format!("printf '%s\\n' '{EVENT}'");
        let step = super::step(&sh, "ert", "sh", &["-c", &stdout_only]);
        assert_eq!(step.phases[0].outcome, PhaseOutcome::Unavailable);
        let failed = format!("{script}; exit 23");
        let step = super::step(&sh, "ert", "sh", &["-c", &failed]);
        assert!(!step.ok);
        assert_eq!(step.phases[0].outcome, PhaseOutcome::Unavailable);
        assert!(step.detail.unwrap().contains("devtool-check-timing"));
        let actual_failure = failed.replace("success", "failure");
        let step = super::step(&sh, "ert", "sh", &["-c", &actual_failure]);
        assert!(!step.ok);
        assert_eq!(step.phases[0].outcome, PhaseOutcome::Failed);
        assert_eq!(step.phases[0].duration_ms, Some(31));
        let mixed = format!(
            "{script}; printf '%s\\n' '{}' >&2",
            EVENT.replace("success", "failure")
        );
        let step = super::step(&sh, "ert", "sh", &["-c", &mixed]);
        assert!(step.ok);
        assert_eq!(step.phases[0].outcome, PhaseOutcome::Unavailable);
    }
}
