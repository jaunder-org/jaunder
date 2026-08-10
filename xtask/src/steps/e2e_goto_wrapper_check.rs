//! The `e2e-goto-wrapper` static check (#867): forbids a raw `page.goto(` anywhere
//! under `end2end/tests` except in `helpers.ts`, so every document load goes through
//! the navigation wrapper and keeps its synchronisation barrier (and its boot count).
//! A raw `page.goto` returns as soon as Playwright's own wait condition is met, which
//! is before the wasm has mounted — the flake class the wrapper exists to remove.
//!
//! It is a host source-scan rather than a lint because the e2e suite has no linter in
//! the gate, and because the rule is about *which module* may make the call, which no
//! per-file lint can express.
//!
//! Exemptions are in-source markers (ADR-0094): `// e2e-goto-wrapper:allow <reason>`
//! on the line **immediately above** the site. Line form only, a reason is required,
//! the marked line must hold exactly one site, and an orphan marker fails. The token
//! is derived from [`STEP`], so the gate cannot be renamed out of sync with the
//! markers that exempt its sites. The census of live markers is **derived** from the
//! scan and printed, because a written exemption can never be re-verified.
//!
//! **Unreadable classes, stated rather than solved:**
//!
//! 1. Matching is per line (as in [`super::no_full_reload_check`]), so a call split
//!    across lines by the formatter would evade it. Prettier keeps `page.goto(` on
//!    one line, and this is a guardrail against accidental reintroduction, not
//!    against a determined adversary.
//! 2. Only the receiver spelled `page` is policed. A second `Page` handle bound to
//!    another name (`popup.goto(…)`) is invisible — there is no type information here.
//! 3. String literals are **not** tracked, so a `page.goto(` inside a string would be
//!    flagged. That direction is deliberate: the failure mode is a false alarm the
//!    author can see and rewrite, never a missed site.
//! 4. A marker is trusted, not verified. The gate checks that a reason exists and
//!    that the marker still points at a site; it can never check that the reason is
//!    true.

use std::collections::HashMap;
use std::path::Path;

use crate::files;
use crate::markers::marker_in_comment;
use crate::result::{CommandResult, StepResult};

/// This gate's step name. The marker token is derived from it — see [`marker_token`].
const STEP: &str = "e2e-goto-wrapper";

/// The call this gate polices. The `(` is part of the needle so that prose naming
/// `page.goto` in a doc comment or a test title is not a site.
const SITE: &str = "page.goto(";

/// Source roots scanned recursively for `.ts` files — the whole e2e suite.
const POLICED_ROOTS: &[&str] = &["end2end/tests"];

/// What a reader is told to do instead, printed with every failure.
const RECOVERY: &str = "  recovery: use `goto(page, path)` from `end2end/tests/helpers.ts` — it waits \
                        for the wasm to mount and counts the boot against the page's budget. If \
                        this load genuinely cannot use the wrapper, mark it with a \
                        `// e2e-goto-wrapper:allow <reason>` comment on the line directly above.";

/// The marker token this gate honors — its step name plus `:allow`. Derived rather
/// than declared so a rename cannot leave the markers pointing at a gate that no
/// longer exists (ADR-0094).
fn marker_token() -> String {
    format!("{STEP}:allow")
}

/// The wrapper's own home is the one file allowed to call `page.goto`: it *is* the
/// barrier. Nothing else is exempt by path — everything else exempts per site.
pub fn is_exempt_path(path: &str) -> bool {
    path.ends_with("end2end/tests/helpers.ts")
}

/// One scanned source line: how many sites it holds, and its real `//` comment.
struct Line<'a> {
    sites: usize,
    comment: Option<&'a str>,
}

/// Split one line into its code text, its trailing `//` comment, and the block-comment
/// state the next line starts in. `in_block` carries `/* … */` across lines, which is
/// what keeps a JSDoc block that *mentions* `page.goto(` from reading as a site.
fn split_line(line: &str, mut in_block: bool) -> (String, Option<&str>, bool) {
    let bytes = line.as_bytes();
    let mut code = String::new();
    let mut comment = None;
    // Start of the current run of code bytes, or `None` while inside a block comment.
    let mut start = (!in_block).then_some(0);
    let mut i = 0;
    while i < bytes.len() {
        if in_block {
            if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                in_block = false;
                i += 2;
                start = Some(i);
            } else {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            if let Some(s) = start {
                code.push_str(&line[s..i]);
            }
            start = None;
            in_block = true;
            i += 2;
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            if let Some(s) = start {
                code.push_str(&line[s..i]);
            }
            start = None;
            comment = Some(&line[i + 2..]);
            break;
        }
        i += 1;
    }
    if let Some(s) = start {
        code.push_str(&line[s..]);
    }
    (code, comment, in_block)
}

/// Every line of `source`, with its site count and its comment, walking the file once
/// so block-comment state carries across lines.
fn scan_lines(source: &str) -> Vec<Line<'_>> {
    let mut in_block = false;
    source
        .lines()
        .map(|line| {
            let (code, comment, next) = split_line(line, in_block);
            in_block = next;
            Line {
                sites: code.matches(SITE).count(),
                comment,
            }
        })
        .collect()
}

/// 1-based line numbers of every raw `page.goto(` in `source`, one entry per
/// occurrence, ignoring markers. Comment text — line or block — is not code, so prose
/// naming the call is never a site. Pure — unit-tested.
fn violations(source: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, line) in scan_lines(source).iter().enumerate() {
        for _ in 0..line.sites {
            out.push(i + 1);
        }
    }
    out
}

/// One row of the derived census: a live marker and the site it exempts.
struct Marked {
    /// The **site's** line, not the marker's — that is what a reader needs.
    line: usize,
    reason: String,
}

/// Sort every site and marker in the scanned files into failures and census rows.
fn audit(scanned: &[(String, String)]) -> (Vec<String>, Vec<(String, Marked)>) {
    let token = marker_token();
    let mut problems = Vec::new();
    let mut census = Vec::new();
    for (path, source) in scanned {
        if is_exempt_path(path) {
            continue;
        }
        let lines = scan_lines(source);
        let marker_at = |line: usize| -> Option<&str> {
            marker_in_comment(lines.get(line.checked_sub(1)?)?.comment?, &token)
        };
        // Site counts come from `violations`, so "what is a site" has one definition
        // here and the unit tests exercise the same answer the gate acts on.
        let mut sites_on_line: HashMap<usize, usize> = HashMap::new();
        for line in violations(source) {
            *sites_on_line.entry(line).or_insert(0) += 1;
        }
        let sites_on = |line: usize| sites_on_line.get(&line).copied().unwrap_or(0);

        for line in 1..=lines.len() {
            if sites_on(line) == 0 {
                continue;
            }
            match line.checked_sub(1).and_then(marker_at) {
                None => problems.push(format!(
                    "{path}:{line}: raw `page.goto` is forbidden outside \
                     `end2end/tests/helpers.ts` — it returns before the wasm has mounted and \
                     hides the load from the boot budget (#867)"
                )),
                Some("") => problems.push(format!(
                    "{path}:{line}: this site carries a bare `{token}` marker — an exemption \
                     with no reason is not an exemption; say why this load cannot use the wrapper"
                )),
                Some(reason) => {
                    let n = sites_on(line);
                    if n > 1 {
                        problems.push(format!(
                            "{path}:{line}: {n} `page.goto` sites share this line, so one marker \
                             cannot justify them — split the line so each carries its own"
                        ));
                    } else {
                        census.push((
                            path.clone(),
                            Marked {
                                line,
                                reason: reason.to_string(),
                            },
                        ));
                    }
                }
            }
        }

        // An orphan is a marker whose very next line holds no site: a live,
        // pre-approved exemption waiting for a future edit to land on it.
        for line in 1..=lines.len() {
            if marker_at(line).is_some() && sites_on(line + 1) == 0 {
                problems.push(format!(
                    "{path}:{line}: `{token}` marker on a line above no `page.goto` site — an \
                     orphan exemption; delete it"
                ));
            }
        }
    }
    problems.sort();
    census.sort_by(|a, b| (&a.0, a.1.line).cmp(&(&b.0, b.1.line)));
    (problems, census)
}

/// The derived census of live markers, one `file:line — reason` row per exempt site.
/// Derived from the scan rather than declared, so it cannot go stale (ADR-0094).
pub fn census(scanned: &[(String, String)]) -> Vec<String> {
    audit(scanned)
        .1
        .into_iter()
        .map(|(path, m)| format!("    - {path}:{} — {}", m.line, m.reason))
        .collect()
}

/// The failure detail for every offending line across the scanned files, or `None`
/// when every site is either wrapped or legitimately marked. The detail ends with the
/// `recovery:` line and then the derived census. Pure given the `(path, source)`
/// pairs, so it is unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let (mut lines, _) = audit(scanned);
    if lines.is_empty() {
        return None;
    }
    lines.push(RECOVERY.to_string());
    lines.extend(census(scanned));
    Some(lines.join("\n"))
}

/// Scan every TypeScript file under each of [`POLICED_ROOTS`] and push the result
/// step. A missing root is a hard failure, so a moved/renamed tree can never quietly
/// disable the guard. On success the step's detail is the derived marker census.
pub fn run(result: &mut CommandResult) {
    let mut files = Vec::new();
    for root in POLICED_ROOTS {
        match files::with_extension(Path::new(root), "ts") {
            Ok(found) => files.extend(found),
            Err(e) => {
                result.push(StepResult::fail(STEP).detail(format!("cannot scan {root}: {e}")));
                return;
            }
        }
    }
    let mut scanned = Vec::new();
    let mut read_errors = Vec::new();
    for p in &files {
        match std::fs::read_to_string(p) {
            Ok(s) => scanned.push((p.display().to_string(), s)),
            Err(e) => read_errors.push(format!("{}: cannot read: {e}", p.display())),
        }
    }
    let step = match (read_errors.is_empty(), problems(&scanned)) {
        (true, None) => {
            let rows = census(&scanned);
            StepResult::ok(STEP).detail(format!(
                "{} exempt site(s)\n{}",
                rows.len(),
                rows.join("\n")
            ))
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
    use super::{census, problems, violations};

    #[test]
    fn flags_a_raw_page_goto() {
        assert_eq!(violations("    await page.goto(url);\n"), vec![1]);
    }

    #[test]
    fn ignores_the_wrapper_call() {
        assert!(violations("    goto(page, \"/login\");\n").is_empty());
    }

    #[test]
    fn ignores_comment_lines() {
        assert!(violations("    // page.goto(url) is forbidden\n").is_empty());
    }

    /// A JSDoc block explaining the rule names the call in prose; block-comment state
    /// must carry across its lines or every such paragraph reads as a site.
    #[test]
    fn ignores_block_comment_prose() {
        let src = "/**\n * a raw page.goto(x) is a blind spot\n */\nawait goto(page, \"/\");\n";
        assert!(violations(src).is_empty());
    }

    #[test]
    fn a_marker_exempts_the_next_line() {
        assert!(
            problems(&[(
                "end2end/tests/x.ts".to_string(),
                "// e2e-goto-wrapper:allow the probe holds wasm so mount never completes\n\
                 await page.goto(url);\n"
                    .to_string()
            )])
            .is_none()
        );
    }

    #[test]
    fn a_bare_marker_fails() {
        let detail = problems(&[(
            "end2end/tests/x.ts".to_string(),
            "// e2e-goto-wrapper:allow\nawait page.goto(url);\n".to_string(),
        )])
        .expect("a problem");
        assert!(detail.contains("reason"));
    }

    #[test]
    fn an_orphan_marker_fails() {
        let detail = problems(&[(
            "end2end/tests/x.ts".to_string(),
            "// e2e-goto-wrapper:allow stale\nawait goto(page, \"/\");\n".to_string(),
        )])
        .expect("a problem");
        assert!(detail.contains("orphan"));
    }

    #[test]
    fn two_sites_on_one_marked_line_fail() {
        let detail = problems(&[(
            "end2end/tests/x.ts".to_string(),
            "// e2e-goto-wrapper:allow one reason\n\
             await page.goto(a); await page.goto(b);\n"
                .to_string(),
        )])
        .expect("a problem");
        assert!(detail.contains("share this line"));
    }

    #[test]
    fn the_helpers_module_may_call_page_goto() {
        // The wrapper itself is not a bypass.
        assert!(super::is_exempt_path("end2end/tests/helpers.ts"));
        assert!(
            problems(&[(
                "end2end/tests/helpers.ts".to_string(),
                "  await page.goto(url);\n".to_string()
            )])
            .is_none()
        );
    }

    #[test]
    fn clean_tree_reports_none() {
        assert_eq!(
            problems(&[(
                "end2end/tests/x.ts".to_string(),
                "    await goto(page, \"/\");\n".to_string()
            )]),
            None
        );
    }

    /// The census is derived from the scan, so it names the sites the tree actually
    /// has — the only control available over a premise nothing re-verifies.
    #[test]
    fn the_census_names_the_site_line_and_its_reason() {
        let rows = census(&[(
            "end2end/tests/x.ts".to_string(),
            "// e2e-goto-wrapper:allow the CLS probe holds the wasm\nawait page.goto(url);\n"
                .to_string(),
        )]);
        assert_eq!(
            rows,
            vec!["    - end2end/tests/x.ts:2 — the CLS probe holds the wasm"]
        );
    }

    #[test]
    fn a_failure_carries_the_recovery_line() {
        let detail = problems(&[(
            "end2end/tests/x.ts".to_string(),
            "await page.goto(url);\n".to_string(),
        )])
        .expect("a problem");
        assert!(detail.contains("recovery:"));
        assert!(detail.contains("end2end/tests/helpers.ts"));
    }
}
