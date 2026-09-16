//! Guest-side validation of the E2E seed trace before browser execution.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind};
use std::path::Path;

use anyhow::Context;

const REQUIRED_SEED_PROCESSES: [&str; 2] = ["e2e.seed.jaunder", "e2e.seed.test-support"];

/// Verify that a projected canonical trace-file path contains storage evidence
/// for every process that seeds the E2E database.
///
/// # Errors
///
/// Returns a stable `seed-trace-*` reason for missing, malformed, or incomplete
/// seed evidence. Unexpected filesystem failures retain their source context.
pub fn verify_seed_trace(trace_path: &Path) -> anyhow::Result<()> {
    let file = match File::open(trace_path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            anyhow::bail!("seed-trace-missing: {} is absent", trace_path.display());
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("opening seed trace {}", trace_path.display()));
        }
    };

    let mut seen = BTreeSet::new();
    let mut lines = BufReader::new(file).lines();
    let mut nonempty = false;
    for (line_number, line) in (&mut lines).enumerate() {
        let line = match line {
            Ok(line) => line,
            Err(error) if error.kind() == ErrorKind::InvalidData => {
                anyhow::bail!(
                    "seed-trace-malformed: {} line {} is not valid UTF-8",
                    trace_path.display(),
                    line_number + 1
                );
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading seed trace {}", trace_path.display()));
            }
        };
        if line.is_empty() {
            continue;
        }
        nonempty = true;
        let record: serde_json::Value = serde_json::from_str(&line).map_err(|error| {
            anyhow::anyhow!(
                "seed-trace-malformed: {} line {}: {error}",
                trace_path.display(),
                line_number + 1
            )
        })?;
        collect_storage_seed_processes(&record, &mut seen);
    }

    if !nonempty {
        anyhow::bail!("seed-trace-missing: {} is empty", trace_path.display());
    }

    let missing: Vec<_> = REQUIRED_SEED_PROCESSES
        .iter()
        .filter(|process| !seen.contains(**process))
        .copied()
        .collect();
    if !missing.is_empty() {
        anyhow::bail!(
            "seed-trace-incomplete: missing storage spans for {}",
            missing.join(", ")
        );
    }
    Ok(())
}

fn collect_storage_seed_processes(record: &serde_json::Value, seen: &mut BTreeSet<String>) {
    let Some(resource_spans) = record
        .get("resourceSpans")
        .and_then(serde_json::Value::as_array)
    else {
        return;
    };
    for resource_span in resource_spans {
        let process = resource_span
            .get("resource")
            .and_then(|resource| resource.get("attributes"))
            .and_then(serde_json::Value::as_array)
            .and_then(|attributes| {
                attributes.iter().find_map(|attribute| {
                    (attribute.get("key").and_then(serde_json::Value::as_str)
                        == Some("jaunder.e2e.seed_process"))
                    .then(|| {
                        attribute
                            .get("value")
                            .and_then(|value| value.get("stringValue"))
                            .and_then(serde_json::Value::as_str)
                    })
                    .flatten()
                })
            });
        let Some(process) = process.filter(|process| REQUIRED_SEED_PROCESSES.contains(process))
        else {
            continue;
        };
        let has_storage_span = resource_span
            .get("scopeSpans")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|scope_spans| {
                scope_spans.iter().any(|scope_span| {
                    scope_span
                        .get("spans")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|spans| {
                            spans.iter().any(|span| {
                                span.get("name")
                                    .and_then(serde_json::Value::as_str)
                                    .is_some_and(|name| name.starts_with("storage."))
                            })
                        })
                })
            });
        if has_storage_span {
            seen.insert(process.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verify(trace: &str) -> anyhow::Result<()> {
        let capture = tempfile::tempdir().expect("capture directory");
        let trace_path = capture.path().join("seed-trace.jsonl");
        std::fs::write(&trace_path, trace).expect("trace fixture");
        verify_seed_trace(&trace_path)
    }

    fn trace(process: &str, span_name: &str) -> String {
        format!(
            r#"{{"resourceSpans":[{{"resource":{{"attributes":[{{"key":"jaunder.e2e.seed_process","value":{{"stringValue":"{process}"}}}}]}},"scopeSpans":[{{"spans":[{{"name":"{span_name}"}}]}}]}}]}}
"#
        )
    }

    #[test]
    fn complete_seed_evidence_passes_despite_irrelevant_and_blank_records() {
        verify(
            &(trace("other-process", "storage.ignored")
                + "\n"
                + &trace("e2e.seed.jaunder", "storage.users.create")
                + &trace("e2e.seed.test-support", "storage.posts.create")),
        )
        .expect("both seed processes carry a storage span");
    }

    #[test]
    fn missing_or_empty_trace_has_a_stable_reason() {
        let capture = tempfile::tempdir().expect("capture directory");
        let missing = verify_seed_trace(&capture.path().join("missing.jsonl"))
            .expect_err("missing trace must fail");
        assert!(
            missing.to_string().contains("seed-trace-missing"),
            "{missing}"
        );

        let empty = verify("").expect_err("empty trace must fail");
        assert!(empty.to_string().contains("seed-trace-missing"), "{empty}");
    }

    #[test]
    fn malformed_trace_has_a_stable_reason() {
        let error = verify("not JSON\n").expect_err("invalid JSON must fail");
        assert!(
            error.to_string().contains("seed-trace-malformed"),
            "{error}"
        );
    }

    #[test]
    fn incomplete_trace_names_every_missing_seed_process() {
        let error = verify("{}\n").expect_err("trace without seed spans is insufficient");
        let message = error.to_string();
        assert!(message.contains("seed-trace-incomplete"), "{message}");
        for process in REQUIRED_SEED_PROCESSES {
            assert!(message.contains(process), "{message}");
        }
    }

    #[test]
    fn invalid_utf8_trace_has_a_stable_reason() {
        let capture = tempfile::tempdir().expect("capture directory");
        let trace_path = capture.path().join("seed-trace.jsonl");
        std::fs::write(&trace_path, b"\xff\n").expect("trace fixture");

        let error = verify_seed_trace(&trace_path).expect_err("invalid UTF-8 must fail");
        assert!(
            error.to_string().contains("seed-trace-malformed"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unexpected_trace_open_failure_retains_path_context() {
        let capture = tempfile::tempdir().expect("capture directory");
        let trace_path = capture.path().join("seed-trace.jsonl");
        std::os::unix::fs::symlink(&trace_path, &trace_path).expect("self-referential symlink");

        let error = verify_seed_trace(&trace_path).expect_err("a symlink loop cannot be opened");
        assert!(
            error
                .to_string()
                .contains(&format!("opening seed trace {}", trace_path.display())),
            "{error}"
        );
    }

    #[test]
    fn unexpected_trace_read_failure_retains_path_context() {
        let capture = tempfile::tempdir().expect("capture directory");
        let error =
            verify_seed_trace(capture.path()).expect_err("a directory cannot be read as JSONL");
        assert!(
            error
                .to_string()
                .contains(&format!("reading seed trace {}", capture.path().display())),
            "{error}"
        );
    }
}
