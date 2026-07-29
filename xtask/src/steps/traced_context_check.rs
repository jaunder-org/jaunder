//! The `traced-context` static check (#681): forbids raw `newContext(` in the e2e
//! specs, so every browser context a test opens carries that test's traceparent.
//!
//! A context made straight off the `browser` fixture does **not** inherit the
//! config-level `extraHTTPHeaders`, so its requests carry the run-wide traceparent
//! from `playwright.config.ts` instead of the per-test one. Under `fullyParallel`
//! that id is shared by every test at once, so the flow-coverage gate cannot
//! attribute anything it drives — the hits land in the orphan bucket and the fn
//! reads as uncovered even though a test exercises it. That is a *silent*
//! under-report: the suite still passes, the snapshot just quietly shrinks.
//!
//! The sanctioned door is the `tracedContext` fixture, which closes over the ids.
//! `fixtures.ts` is exempt because it *is* that door.
//!
//! Accepted limitation (as in [`super::no_full_reload_check`]): matching is
//! per-line, so a call split across lines by the formatter could evade it — a
//! guardrail against accidental reintroduction, not a determined adversary.

use std::path::{Path, PathBuf};

use crate::result::{CommandResult, StepResult};

/// The e2e spec tree scanned for raw context creation.
const SPEC_ROOT: &str = "end2end/tests";

/// The one file allowed to call `newContext` — it implements `tracedContext`.
const EXEMPT: &str = "fixtures.ts";

/// 1-based line numbers calling `newContext(`. Comment lines (`//`, and `*`
/// continuation lines inside a doc block) are skipped so the fixture's own prose
/// and any explanatory comment do not trip the guard. Pure — unit-tested.
fn violations(source: &str) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .filter(|(_, raw)| {
            let t = raw.trim_start();
            !t.starts_with("//") && !t.starts_with('*') && t.contains(".newContext(")
        })
        .map(|(i, _)| i + 1)
        .collect()
}

/// The failure detail for every offending line, or `None` when clean. Pure given
/// the `(path, source)` pairs, so it is unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    for (path, source) in scanned {
        for ln in violations(source) {
            lines.push(format!(
                "{path}:{ln}: raw `newContext(` in an e2e spec — a context built off `browser` \
                 does not inherit the per-test traceparent, so everything it drives is \
                 unattributable and the flow-coverage gate silently under-reports. Use the \
                 `tracedContext` fixture instead (#681)"
            ));
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Collect every `.ts` file under `dir` except the exempt fixture module.
fn spec_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            spec_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "ts")
            && path.file_name().is_some_and(|n| n != EXEMPT)
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Scan the spec tree and push the result step. A missing root is a hard failure,
/// so a moved/renamed tree can never quietly disable the guard.
pub fn run(result: &mut CommandResult) {
    let mut files = Vec::new();
    if let Err(e) = spec_files(Path::new(SPEC_ROOT), &mut files) {
        result.push(
            StepResult::fail("traced-context").detail(format!("cannot scan {SPEC_ROOT}: {e}")),
        );
        return;
    }
    let scanned: Vec<(String, String)> = files
        .iter()
        .filter_map(|p| {
            std::fs::read_to_string(p)
                .ok()
                .map(|s| (p.display().to_string(), s))
        })
        .collect();
    let step = match problems(&scanned) {
        None => {
            StepResult::ok("traced-context").detail(format!("{} spec file(s) clean", scanned.len()))
        }
        Some(detail) => StepResult::fail("traced-context").detail(detail),
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::{problems, violations};

    #[test]
    fn flags_both_browser_and_page_derived_context_creation() {
        assert_eq!(
            violations("  const ctx = await browser.newContext();\n"),
            vec![1]
        );
        // The `context.browser()!.newContext()` form evades a `browser.` prefix
        // match, which is why the check keys on the method, not the receiver.
        assert_eq!(
            violations("  const g = await context.browser()!.newContext();\n"),
            vec![1]
        );
    }

    #[test]
    fn ignores_the_sanctioned_fixture_call_and_prose() {
        assert!(violations("  const ctx = await tracedContext();\n").is_empty());
        assert!(violations("// use browser.newContext() only in fixtures\n").is_empty());
        assert!(violations(" * `browser.newContext()` does not inherit headers\n").is_empty());
    }

    #[test]
    fn problems_names_the_file_line_and_the_remedy() {
        let scanned = vec![(
            "end2end/tests/visibility.spec.ts".to_string(),
            "const ctx = await browser.newContext();\n".to_string(),
        )];
        let detail = problems(&scanned).expect("a problem");
        assert!(
            detail.contains("end2end/tests/visibility.spec.ts:1"),
            "{detail}"
        );
        assert!(detail.contains("tracedContext"), "{detail}");
    }

    #[test]
    fn problems_is_none_when_every_spec_uses_the_fixture() {
        let scanned = vec![(
            "end2end/tests/posts.spec.ts".to_string(),
            "const ctx = await tracedContext();\n".to_string(),
        )];
        assert_eq!(problems(&scanned), None);
    }
}
