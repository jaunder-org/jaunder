//! The `traced-context` static check (#681): keeps every e2e spec attributable, by
//! forbidding the two ways a spec can silently detach its work from a test span —
//! a raw `newContext(`, and importing `test` from `@playwright/test`.
//!
//! **Raw `newContext(`.** A context made straight off the `browser` fixture does
//! **not** inherit the config-level `extraHTTPHeaders`, so its requests carry the
//! run-wide traceparent from `playwright.config.ts` instead of the per-test one.
//! Under `fullyParallel` that id is shared by every test at once, so the
//! flow-coverage gate cannot attribute anything it drives.
//!
//! **Upstream `test`.** A spec importing `test` from `@playwright/test` gets the
//! plain Playwright test, which emits no `e2e.test` span at all — so there is no
//! test span for the walk to reach, and everything the spec drives is unattributable
//! for want of a destination rather than for want of a traceparent.
//!
//! Both fail the same way: the hits land in the orphan bucket and the fn reads as
//! uncovered even though a test exercises it. That is a *silent* under-report — the
//! suite still passes, the snapshot just quietly shrinks. Which is why this is a
//! gate and not a convention: nothing else in the build notices.
//!
//! The sanctioned doors are the `tracedContext` fixture, which closes over the ids,
//! and the `test` re-exported from `./fixtures`, which opens the `e2e.test` span.
//! `fixtures.ts` is exempt because it *is* both of them.
//!
//! Accepted limitation (as in [`super::no_full_reload_check`]): matching is
//! per-line, so a call — or an import clause — split across lines by the formatter
//! could evade it. A guardrail against accidental reintroduction, not a determined
//! adversary.

use std::path::Path;

use crate::files;
use crate::result::{CommandResult, StepResult};

/// The e2e spec tree scanned for both rules.
const SPEC_ROOT: &str = "end2end/tests";

/// The one file allowed to do either — it implements `tracedContext` and wraps
/// `test`.
const EXEMPT: &str = "fixtures.ts";

/// This gate's step name, spelled once so the reported name cannot drift between the
/// ok, fail, and cannot-scan arms.
const STEP: &str = "traced-context";

/// The upstream module whose `test` value a spec must not import.
const PLAYWRIGHT: &str = "@playwright/test";

/// What a rejected line did. Both routes end in the same unattributable flow, but
/// they have different remedies, so the report distinguishes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Violation {
    /// A context built off `browser`, carrying the run-wide traceparent.
    RawContext,
    /// Playwright's own `test`, which opens no `e2e.test` span.
    UpstreamTest,
}

/// Every rejected line of one file, as `(1-based line, reason)` in line order.
/// Comment lines (`//`, and `*` continuation lines inside a doc block) are skipped so
/// the fixture's own prose and any explanatory comment do not trip the guard —
/// several specs carry exactly such a note. Pure — unit-tested.
fn violations(source: &str) -> Vec<(usize, Violation)> {
    let mut out = Vec::new();
    for (i, raw) in source.lines().enumerate() {
        let line = raw.trim_start();
        if line.starts_with("//") || line.starts_with('*') {
            continue;
        }
        if line.contains(".newContext(") {
            out.push((i + 1, Violation::RawContext));
        }
        if imports_upstream_test(line) {
            out.push((i + 1, Violation::UpstreamTest));
        }
    }
    out
}

/// Whether a line imports the `test` **value** from `@playwright/test`.
///
/// Deliberately narrow: `import type { Page } from "@playwright/test"`, a per-binding
/// `type Page`, and value imports of anything else (`expect`, `devices`, …) are all
/// legitimate — the spec tree does each of them — and flagging them would make the
/// rule unfollowable. `test as base` does bind the upstream test, so it counts.
fn imports_upstream_test(line: &str) -> bool {
    if !line.contains(PLAYWRIGHT) || line.contains("import type") {
        return false;
    }
    let (Some(open), Some(close)) = (line.find('{'), line.rfind('}')) else {
        return false;
    };
    line[open + 1..close]
        .split(',')
        // The first word of a binding is the imported name; `as`-renames and the
        // `type` marker both follow it.
        .any(|binding| binding.split_whitespace().next() == Some("test"))
}

/// The failure detail for every offending line, or `None` when clean. Pure given
/// the `(path, source)` pairs, so it is unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    for (path, source) in scanned {
        for (ln, violation) in violations(source) {
            lines.push(format!("{path}:{ln}: {}", remedy(violation)));
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// One violation's message: what was written, why it under-reports, and the door to
/// use instead.
fn remedy(violation: Violation) -> &'static str {
    match violation {
        Violation::RawContext => {
            "raw `newContext(` in an e2e spec — a context built off `browser` does not inherit \
             the per-test traceparent, so everything it drives is unattributable and the \
             flow-coverage gate silently under-reports. Use the `tracedContext` fixture instead \
             (#681)"
        }
        Violation::UpstreamTest => {
            "`test` imported from `@playwright/test` — that `test` opens no `e2e.test` span, so \
             every server fn the spec drives is unattributable and the flow-coverage gate \
             silently under-reports. Import `test` from `./fixtures` instead (type-only imports \
             from `@playwright/test` are fine) (#681)"
        }
    }
}

/// Scan the spec tree and push the result step. A missing root is a hard failure,
/// so a moved/renamed tree can never quietly disable the guard.
pub fn run(result: &mut CommandResult) {
    let files = match files::with_extension(Path::new(SPEC_ROOT), "ts") {
        Ok(files) => files,
        Err(e) => {
            result.push(StepResult::fail(STEP).detail(format!("cannot scan {SPEC_ROOT}: {e}")));
            return;
        }
    };
    // A file we listed but cannot READ is surfaced as a failure, not dropped: an
    // unexamined spec could hold either violation, and a spec that passes unread is
    // precisely the silent under-report this gate exists to prevent.
    let mut scanned = Vec::new();
    let mut read_errors = Vec::new();
    for path in files
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n != EXEMPT))
    {
        match std::fs::read_to_string(path) {
            Ok(s) => scanned.push((path.display().to_string(), s)),
            Err(e) => read_errors.push(format!("{}: cannot read: {e}", path.display())),
        }
    }
    let step = match (read_errors.is_empty(), problems(&scanned)) {
        (true, None) => {
            StepResult::ok(STEP).detail(format!("{} spec file(s) clean", scanned.len()))
        }
        (_, prob) => {
            read_errors.extend(prob);
            StepResult::fail(STEP).detail(read_errors.join("\n"))
        }
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::{Violation, imports_upstream_test, problems, violations};

    /// The line numbers flagged for `kind`, so a test can assert one rule without
    /// restating the other's absence.
    fn lines_for(source: &str, kind: Violation) -> Vec<usize> {
        violations(source)
            .into_iter()
            .filter(|(_, v)| *v == kind)
            .map(|(ln, _)| ln)
            .collect()
    }

    #[test]
    fn flags_both_browser_and_page_derived_context_creation() {
        assert_eq!(
            lines_for(
                "  const ctx = await browser.newContext();\n",
                Violation::RawContext
            ),
            vec![1]
        );
        // The `context.browser()!.newContext()` form evades a `browser.` prefix
        // match, which is why the check keys on the method, not the receiver.
        assert_eq!(
            lines_for(
                "  const g = await context.browser()!.newContext();\n",
                Violation::RawContext
            ),
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
    fn flags_test_imported_from_playwright() {
        assert_eq!(
            lines_for(
                "import { test, expect } from \"@playwright/test\";\n",
                Violation::UpstreamTest
            ),
            vec![1]
        );
        // A rename still binds the upstream test.
        assert_eq!(
            lines_for(
                "import { test as base } from \"@playwright/test\";\n",
                Violation::UpstreamTest
            ),
            vec![1]
        );
    }

    #[test]
    fn type_only_and_non_test_playwright_imports_are_legitimate() {
        // The spec tree does all three of these; flagging them would leave no way to
        // name Playwright's types or use its matchers.
        assert!(!imports_upstream_test(
            "import type { Page } from \"@playwright/test\";"
        ));
        assert!(!imports_upstream_test(
            "import { expect, type Page, type Locator } from \"@playwright/test\";"
        ));
        assert!(!imports_upstream_test(
            "import type { TestInfo } from \"@playwright/test\";"
        ));
    }

    #[test]
    fn test_from_the_fixtures_module_is_the_sanctioned_door() {
        assert!(violations("import { test, expect } from \"./fixtures\";\n").is_empty());
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
    fn problems_names_the_fixtures_import_as_the_remedy() {
        let scanned = vec![(
            "end2end/tests/posts.spec.ts".to_string(),
            "import { test } from \"@playwright/test\";\n".to_string(),
        )];
        let detail = problems(&scanned).expect("a problem");
        assert!(detail.contains("end2end/tests/posts.spec.ts:1"), "{detail}");
        assert!(detail.contains("./fixtures"), "{detail}");
    }

    #[test]
    fn problems_is_none_when_every_spec_uses_the_fixture() {
        let scanned = vec![(
            "end2end/tests/posts.spec.ts".to_string(),
            "import { test } from \"./fixtures\";\nconst ctx = await tracedContext();\n"
                .to_string(),
        )];
        assert_eq!(problems(&scanned), None);
    }
}
