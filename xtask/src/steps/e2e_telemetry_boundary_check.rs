//! The `e2e-telemetry-boundary` static check (#839): browser diagnostics must be
//! captured once by the E2E harness and emitted only by its serialization boundary.
//!
//! A second Playwright `console`/`pageerror` listener silently duplicates diagnostics,
//! while a second raw `e2e.console_*` attribute exporter lets a test bypass the shared
//! first-20/drop-count policy. Both are source-shape ownership rules, so this host gate
//! scans tracked TypeScript rather than relying on convention. `git ls-files` keeps build
//! output and nested worktrees outside the census; listing or reading failures fail closed.

use std::path::Path;

use crate::result::{CommandResult, StepResult};

const STEP: &str = "e2e-telemetry-boundary";
const LISTENER_OWNER: &str = "end2end/tests/capture-trace.ts";
const ATTRIBUTE_OWNER: &str = "end2end/tests/performance.ts";
/// This spec constructs the expected OTLP payload; it is not an exporter. Keep this
/// exception path-specific so production telemetry still has one owner.
const SYNTHETIC_PAYLOAD_TEST: &str = "end2end/tests/boot-marks.spec.ts";

#[derive(Clone, Copy)]
enum Violation {
    Listener,
    Attribute,
}

fn authorized(path: &str, violation: Violation) -> bool {
    match violation {
        Violation::Listener => path == LISTENER_OWNER,
        Violation::Attribute => path == ATTRIBUTE_OWNER || path == SYNTHETIC_PAYLOAD_TEST,
    }
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
    for (path, source) in scanned {
        if !authorized(path, Violation::Listener) {
            for (line, event) in diagnostic_listeners(source) {
                found.push(format!(
                    "{path}:{line}: Playwright `{event}` listener must be installed only by {LISTENER_OWNER} (#839)"
                ));
            }
        }
        let mut in_block = false;
        for (index, raw) in source.lines().enumerate() {
            let (line, next_block) = code_on_line(raw, in_block);
            in_block = next_block;
            if (line.contains("e2e.console_json") || line.contains("e2e.console_dropped"))
                && !authorized(path, Violation::Attribute)
            {
                found.push(format!(
                    "{path}:{}: raw `e2e.console_*` diagnostic attributes must be emitted only by {ATTRIBUTE_OWNER} (#839)",
                    index + 1
                ));
            }
        }
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

/// Scan all tracked TypeScript source. A failed repository-root lookup or tracked-file
/// census is a failed gate: treating either as an empty population would disable it.
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
    let tracked = match crate::git::tracked_files(top, "*.ts") {
        Ok(tracked) => tracked,
        Err(error) => {
            result.push(
                StepResult::fail(STEP).detail(format!("cannot enumerate tracked sources: {error}")),
            );
            return;
        }
    };
    run_with(result, top, &tracked, |path| std::fs::read_to_string(path));
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;

    use super::{STEP, problems, run_with};
    use crate::CommandResult;

    #[test]
    fn allows_the_exact_harness_owners_and_synthetic_schema_payload() {
        let scanned = vec![
            (
                "end2end/tests/capture-trace.ts".to_string(),
                "page.on(\"console\", listener);\npage.once('pageerror', listener);".to_string(),
            ),
            (
                "end2end/tests/performance.ts".to_string(),
                "otlpAttribute(\"e2e.console_json\", value);\notlpAttribute(\"e2e.console_dropped\", value);".to_string(),
            ),
            (
                "end2end/tests/boot-marks.spec.ts".to_string(),
                "expect({ key: \"e2e.console_json\" });".to_string(),
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
            "otlpAttribute(\"e2e.console_dropped\", value);".to_string(),
        )])
        .expect("exporter leak");
        assert!(detail.contains("other.ts:1"), "{detail}");
        assert!(detail.contains("performance.ts"), "{detail}");
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
