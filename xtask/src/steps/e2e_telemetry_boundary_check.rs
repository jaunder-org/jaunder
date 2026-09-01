//! The `e2e-telemetry-boundary` static check (#839): browser diagnostics must be
//! captured once by the E2E harness and emitted only by its serialization boundary.
//!
//! A second Playwright `console`/`pageerror` listener silently duplicates diagnostics,
//! while a second raw `e2e.console_*` attribute exporter lets a test bypass the shared
//! first-20/drop-count policy. Both are source-shape ownership rules, so this host gate
//! scans tracked TypeScript, TSX, and Rust rather than relying on convention. `git ls-files`
//! keeps build output and nested worktrees outside the census; listing or reading failures fail closed.

use std::path::Path;

use crate::result::{CommandResult, StepResult};

const STEP: &str = "e2e-telemetry-boundary";
const LISTENER_OWNER: &str = "end2end/tests/capture-trace.ts";
const ATTRIBUTE_OWNER: &str = "end2end/tests/performance.ts";
/// These specs inspect expected OTLP payloads; they are not exporters. Keep these
/// exceptions path-specific so production telemetry still has one owner.
const SYNTHETIC_PAYLOAD_TESTS: [&str; 2] = [
    "end2end/tests/boot-marks.spec.ts",
    "end2end/tests/client-telemetry.spec.ts",
];
const CHECK_SOURCE: &str = "xtask/src/steps/e2e_telemetry_boundary_check.rs";
// Keep the detector's own literal patterns out of its tracked Rust census.
const CONSOLE_JSON_ATTRIBUTE: &str = concat!("e2e.console_", "json");
const CONSOLE_DROPPED_ATTRIBUTE: &str = concat!("e2e.console_", "dropped");

fn authorized_attribute(path: &str) -> bool {
    path == ATTRIBUTE_OWNER || SYNTHETIC_PAYLOAD_TESTS.contains(&path)
}

/// Return every Playwright diagnostic listener site after removing comments and
/// insignificant whitespace. Flattening whitespace keeps a prettier-wrapped
/// `page.on(\n  "console", …)` from escaping the ownership gate.
fn diagnostic_listeners(source: &str) -> Vec<(usize, &'static str)> {
    let mut compact = Vec::new();
    let mut source_lines = Vec::new();
    let mut in_block = false;
    for (index, raw) in source.lines().enumerate() {
        let (line, next_block) = code_on_line(raw, in_block);
        in_block = next_block;
        for byte in line.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
            compact.push(byte);
            source_lines.push(index + 1);
        }
    }

    let mut found = Vec::new();
    for (pattern, event) in [
        (b".on(\"console\"".as_slice(), "console"),
        (b".on('console'".as_slice(), "console"),
        (b".on(\"pageerror\"".as_slice(), "pageerror"),
        (b".on('pageerror'".as_slice(), "pageerror"),
        (b".once(\"console\"".as_slice(), "console"),
        (b".once('console'".as_slice(), "console"),
        (b".once(\"pageerror\"".as_slice(), "pageerror"),
        (b".once('pageerror'".as_slice(), "pageerror"),
    ] {
        for (offset, window) in compact.windows(pattern.len()).enumerate() {
            if window == pattern {
                found.push((source_lines[offset], event));
            }
        }
    }
    found.sort_unstable();
    found
}

/// Code on this line, carrying block-comment state across lines. The check intentionally
/// sees code-shaped strings as code: a false alarm is reviewable, whereas a new listener
/// or exporter escaping the census would silently corrupt telemetry.
fn code_on_line(line: &str, mut in_block: bool) -> (String, bool) {
    let bytes = line.as_bytes();
    let mut code = String::new();
    let mut start = (!in_block).then_some(0);
    let mut index = 0;
    while index < bytes.len() {
        if in_block {
            if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                in_block = false;
                index += 2;
                start = Some(index);
            } else {
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            if let Some(start) = start {
                code.push_str(&line[start..index]);
            }
            start = None;
            in_block = true;
            index += 2;
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            if let Some(start) = start {
                code.push_str(&line[start..index]);
            }
            start = None;
            break;
        }
        index += 1;
    }
    if let Some(start) = start {
        code.push_str(&line[start..]);
    }
    (code, in_block)
}

/// All rejected sites as `path:line: reason`. Pure over the tracked file population.
fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut found = Vec::new();
    let mut listener_owner_seen = false;
    let mut attribute_owner_seen = false;
    for (path, source) in scanned {
        let listeners = diagnostic_listeners(source);
        if path == LISTENER_OWNER {
            listener_owner_seen = true;
            for event in ["console", "pageerror"] {
                let count = listeners
                    .iter()
                    .filter(|(_, found_event)| *found_event == event)
                    .count();
                if count != 1 {
                    found.push(format!(
                        "{path}: expected exactly one Playwright `{event}` listener (#839); found {count}"
                    ));
                }
            }
        } else {
            for (line, event) in listeners {
                found.push(format!(
                    "{path}:{line}: Playwright `{event}` listener must be installed only by {LISTENER_OWNER} (#839)"
                ));
            }
        }

        let mut attributes = [0; 2];
        let mut in_block = false;
        for (index, raw) in source.lines().enumerate() {
            let (line, next_block) = code_on_line(raw, in_block);
            in_block = next_block;
            for (attribute_index, attribute) in [CONSOLE_JSON_ATTRIBUTE, CONSOLE_DROPPED_ATTRIBUTE]
                .iter()
                .enumerate()
            {
                let count = line.matches(attribute).count();
                attributes[attribute_index] += count;
                if count != 0 && !authorized_attribute(path) {
                    found.push(format!(
                        "{path}:{}: raw `e2e.console_*` diagnostic attributes must be emitted only by {ATTRIBUTE_OWNER} (#839)",
                        index + 1
                    ));
                }
            }
        }
        if path == ATTRIBUTE_OWNER {
            attribute_owner_seen = true;
            for (attribute, count) in [CONSOLE_JSON_ATTRIBUTE, CONSOLE_DROPPED_ATTRIBUTE]
                .iter()
                .zip(attributes)
            {
                if count != 1 {
                    found.push(format!(
                        "{path}: expected exactly one `{attribute}` diagnostic attribute (#839); found {count}"
                    ));
                }
            }
        }
    }
    if !listener_owner_seen {
        found.push(format!(
            "{LISTENER_OWNER}: expected diagnostic listener owner is not tracked (#839)"
        ));
    }
    if !attribute_owner_seen {
        found.push(format!(
            "{ATTRIBUTE_OWNER}: expected diagnostic attribute owner is not tracked (#839)"
        ));
    }
    (!found.is_empty()).then(|| found.join("\n"))
}

fn run_with(
    result: &mut CommandResult,
    top: &Path,
    tracked: &[String],
    mut read: impl FnMut(&Path) -> std::io::Result<String>,
) {
    let mut scanned = Vec::with_capacity(tracked.len());
    for path in tracked {
        match read(&top.join(path)) {
            Ok(source) => scanned.push((path.clone(), source)),
            Err(error) => {
                result
                    .push(StepResult::fail(STEP).detail(format!("{path}: cannot read — {error}")));
                return;
            }
        }
    }
    match problems(&scanned) {
        Some(detail) => result.push(StepResult::fail(STEP).detail(detail)),
        None => result.push(StepResult::ok(STEP)),
    }
}

/// Scan all tracked TypeScript, TSX, and Rust source. A failed repository-root lookup or
/// tracked-file census is a failed gate: treating either as an empty population would disable it.
pub fn run(result: &mut CommandResult) {
    let top = match crate::git::toplevel(Path::new(".")) {
        Ok(top) => top,
        Err(error) => {
            result.push(
                StepResult::fail(STEP).detail(format!("cannot enumerate tracked sources: {error}")),
            );
            return;
        }
    };
    let top = Path::new(&top);
    let mut tracked = Vec::new();
    for glob in ["*.ts", "*.tsx", "*.rs"] {
        match crate::git::tracked_files(top, glob) {
            Ok(files) => tracked.extend(files),
            Err(error) => {
                result.push(
                    StepResult::fail(STEP)
                        .detail(format!("cannot enumerate tracked sources: {error}")),
                );
                return;
            }
        }
    }
    tracked.retain(|path| path != CHECK_SOURCE);
    tracked.sort();
    run_with(result, top, &tracked, |path| std::fs::read_to_string(path));
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;

    use super::{CONSOLE_DROPPED_ATTRIBUTE, CONSOLE_JSON_ATTRIBUTE, STEP, problems, run_with};
    use crate::CommandResult;

    #[test]
    fn allows_the_exact_harness_census_and_synthetic_schema_payload() {
        let scanned = vec![
            (
                "end2end/tests/capture-trace.ts".to_string(),
                "page.on(\"console\", listener);\npage.once('pageerror', listener);".to_string(),
            ),
            (
                "end2end/tests/performance.ts".to_string(),
                format!(
                    "otlpAttribute(\"{CONSOLE_JSON_ATTRIBUTE}\", value);\notlpAttribute(\"{CONSOLE_DROPPED_ATTRIBUTE}\", value);"
                ),
            ),
            (
                "end2end/tests/boot-marks.spec.ts".to_string(),
                format!("expect({{ key: \"{CONSOLE_JSON_ATTRIBUTE}\" }});"),
            ),
            (
                "app/src/telemetry.rs".to_string(),
                "let unrelated = \"trace_id\";".to_string(),
            ),
        ];
        assert_eq!(problems(&scanned), None);
    }

    #[test]
    fn rejects_a_listener_outside_the_capture_harness() {
        let detail = problems(&[(
            "end2end/tests/orders.spec.ts".to_string(),
            "page.on(\"console\", listener);".to_string(),
        )])
        .expect("listener leak");
        assert!(detail.contains("orders.spec.ts:1"), "{detail}");
        assert!(detail.contains("capture-trace.ts"), "{detail}");
    }

    #[test]
    fn rejects_a_multiline_listener_outside_the_capture_harness() {
        let detail = problems(&[(
            "client/src/telemetry.ts".to_string(),
            "page.on(\n  \"pageerror\",\n  listener,\n);".to_string(),
        )])
        .expect("multiline listener leak");
        assert!(detail.contains("telemetry.ts:1"), "{detail}");
        assert!(detail.contains("pageerror"), "{detail}");
    }

    #[test]
    fn rejects_a_raw_diagnostic_export_outside_the_serializer() {
        let detail = problems(&[(
            "end2end/tests/other.ts".to_string(),
            format!("otlpAttribute(\"{CONSOLE_DROPPED_ATTRIBUTE}\", value);"),
        )])
        .expect("exporter leak");
        assert!(detail.contains("other.ts:1"), "{detail}");
        assert!(detail.contains("performance.ts"), "{detail}");
    }

    #[test]
    fn rejects_a_rust_production_exporter_leak() {
        let detail = problems(&[(
            "app/src/telemetry.rs".to_string(),
            format!("span.set_attribute(\"{CONSOLE_JSON_ATTRIBUTE}\", value);"),
        )])
        .expect("Rust exporter leak");
        assert!(detail.contains("telemetry.rs:1"), "{detail}");
        assert!(detail.contains("performance.ts"), "{detail}");
    }

    #[test]
    fn rejects_duplicate_listener_in_the_capture_harness() {
        let detail = problems(&[(
            "end2end/tests/capture-trace.ts".to_string(),
            "page.on(\"console\", listener);\npage.on(\"console\", duplicate);\npage.on(\"pageerror\", listener);"
                .to_string(),
        )])
        .expect("duplicate listener");
        assert!(
            detail.contains("exactly one Playwright `console` listener"),
            "{detail}"
        );
        assert!(detail.contains("found 2"), "{detail}");
    }

    #[test]
    fn rejects_missing_required_owner_sites() {
        let detail = problems(&[
            (
                "end2end/tests/capture-trace.ts".to_string(),
                "page.on(\"console\", listener);".to_string(),
            ),
            (
                "end2end/tests/performance.ts".to_string(),
                format!("otlpAttribute(\"{CONSOLE_JSON_ATTRIBUTE}\", value);"),
            ),
        ])
        .expect("missing owner sites");
        assert!(
            detail.contains("Playwright `pageerror` listener"),
            "{detail}"
        );
        assert!(
            detail.contains(&format!(
                "`{CONSOLE_DROPPED_ATTRIBUTE}` diagnostic attribute"
            )),
            "{detail}"
        );
    }

    #[test]
    fn rejects_duplicate_attribute_in_the_serializer() {
        let detail = problems(&[(
            "end2end/tests/performance.ts".to_string(),
            format!(
                "otlpAttribute(\"{CONSOLE_JSON_ATTRIBUTE}\", value);\notlpAttribute(\"{CONSOLE_JSON_ATTRIBUTE}\", duplicate);\notlpAttribute(\"{CONSOLE_DROPPED_ATTRIBUTE}\", value);"
            ),
        )])
        .expect("duplicate attribute");
        assert!(
            detail.contains(&format!(
                "exactly one `{CONSOLE_JSON_ATTRIBUTE}` diagnostic attribute"
            )),
            "{detail}"
        );
        assert!(detail.contains("found 2"), "{detail}");
    }

    #[test]
    fn unreadable_tracked_source_fails_closed() {
        let mut result = CommandResult::new("test");
        run_with(
            &mut result,
            Path::new("/repo"),
            &["end2end/tests/capture-trace.ts".to_string()],
            |_| Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied")),
        );
        assert_eq!(result.steps.len(), 1);
        assert!(!result.steps[0].ok);
        assert_eq!(result.steps[0].name, STEP);
        assert!(
            result.steps[0]
                .detail
                .as_deref()
                .unwrap()
                .contains("denied")
        );
    }
}
