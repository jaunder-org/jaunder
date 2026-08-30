//! The `e2e-goto-wrapper` static check (#867): forbids a raw `page.goto(` anywhere
//! under `end2end/tests`, so every document load goes through the navigation wrapper
//! and keeps its synchronisation barrier (and its boot count). A raw `page.goto`
//! returns as soon as Playwright's own wait condition is met, which is before the wasm
//! has mounted — the flake class the wrapper exists to remove.
//!
//! It is a host source-scan rather than a lint because the e2e suite has no linter in
//! the gate, and because the rule is about *which call* may be written, which no
//! per-file lint can express.
//!
//! **No file is exempt, including the wrapper's own.** `helpers.ts` holds exactly one
//! `page.goto` — the barrier itself — and it carries an ordinary marker like every
//! other exempt site. A whole-file exemption would be an ADR-0085 principle-4 region
//! exemption: it would let a second, unreviewed `page.goto` enter that file silently,
//! and it would hide the wrapper from the census.
//!
//! Exemptions are in-source markers (ADR-0094): `// e2e-goto-wrapper:allow <reason>`
//! on the line **immediately above** the site. Line form only, a reason is required,
//! the marked line must hold exactly one site, and an orphan marker fails. The token
//! is derived from [`STEP`], so the gate cannot be renamed out of sync with the
//! markers that exempt its sites. The census of live markers is **derived** from the
//! scan and included with failure diagnostics, because a written exemption can never
//! be re-verified; a clean check stays terse.
//!
//! **Unreadable classes, stated rather than solved:**
//!
//! 1. Matching is per line, so a call split across lines by the formatter could evade it.
//!    Prettier keeps `page.goto(` on one line, and this is a guardrail against accidental
//!    reintroduction, not against a determined adversary.
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

/// 1-based line numbers of every site in already-scanned `lines`, one entry per
/// occurrence. The single definition of "where the sites are": [`audit`] and
/// [`violations`] both read it, so the gate acts on exactly what the tests exercise.
fn site_lines(lines: &[Line<'_>]) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        for _ in 0..line.sites {
            out.push(i + 1);
        }
    }
    out
}

/// 1-based line numbers of every raw `page.goto(` in `source`, one entry per
/// occurrence, ignoring markers. Comment text — line or block — is not code, so prose
/// naming the call is never a site.
///
/// Test-facing: the gate walks each file once through [`audit`], which reads
/// [`site_lines`] off that same walk, so this is the same answer reached from a
/// source string instead of a scanned file.
#[cfg(test)]
fn violations(source: &str) -> Vec<usize> {
    site_lines(&scan_lines(source))
}

/// One row of the derived census: a live marker and the site it exempts.
struct Marked {
    path: String,
    /// The **site's** line, not the marker's — that is what a reader needs.
    line: usize,
    reason: String,
}

/// What one pass over the scanned files found: the failures, and the census of live
/// markers. Both come out of the same walk — the census is not recomputed, because a
/// second walk is a second chance for the two answers to disagree.
struct Audit {
    problems: Vec<String>,
    census: Vec<Marked>,
}

impl Audit {
    /// The census as printable rows, `file:line — reason`.
    fn census_rows(&self) -> Vec<String> {
        self.census
            .iter()
            .map(|m| format!("    - {}:{} — {}", m.path, m.line, m.reason))
            .collect()
    }
}

/// Sort every site and marker in the scanned files into failures and census rows.
fn audit(scanned: &[(String, String)]) -> Audit {
    let token = marker_token();
    let mut problems = Vec::new();
    let mut census = Vec::new();
    for (path, source) in scanned {
        // One walk per file: the site counts and the comments both come from it.
        let lines = scan_lines(source);
        // The marker ON `line`, if any. The site loop asks about the line above its
        // site; the orphan loop asks about the marker's own line.
        let marker_on = |line: usize| -> Option<&str> {
            marker_in_comment(lines.get(line.checked_sub(1)?)?.comment?, &token)
        };
        let mut sites_on_line: HashMap<usize, usize> = HashMap::new();
        for line in site_lines(&lines) {
            *sites_on_line.entry(line).or_insert(0) += 1;
        }
        let sites_on = |line: usize| sites_on_line.get(&line).copied().unwrap_or(0);

        for line in 1..=lines.len() {
            if sites_on(line) == 0 {
                continue;
            }
            match line.checked_sub(1).and_then(marker_on) {
                None => problems.push(format!(
                    "{path}:{line}: raw `page.goto` is forbidden — it returns before the wasm \
                     has mounted and hides the load from the boot budget (#867)"
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
                        census.push(Marked {
                            path: path.clone(),
                            line,
                            reason: reason.to_string(),
                        });
                    }
                }
            }
        }

        // An orphan is a marker whose very next line holds no site: a live,
        // pre-approved exemption waiting for a future edit to land on it.
        for line in 1..=lines.len() {
            if marker_on(line).is_some() && sites_on(line + 1) == 0 {
                problems.push(format!(
                    "{path}:{line}: `{token}` marker on a line above no `page.goto` site — an \
                     orphan exemption; delete it"
                ));
            }
        }
    }
    problems.sort();
    census.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
    Audit { problems, census }
}

/// The derived census of live markers, one `file:line — reason` row per exempt site.
/// Derived from the scan rather than declared, so it cannot go stale (ADR-0094).
///
/// Test-facing, like [`problems`]: [`run`] reads both off one [`Audit`].
#[cfg(test)]
fn census(scanned: &[(String, String)]) -> Vec<String> {
    audit(scanned).census_rows()
}

/// The failure detail for every offending line across the scanned files, or `None`
/// when every site is either wrapped or legitimately marked. The detail ends with the
/// `recovery:` line and then the derived census. Pure given the `(path, source)`
/// pairs, so it is unit-tested directly.
///
/// Test-facing: [`run`] calls [`audit`] once and hands the result to [`detail`], so
/// the census it prints on failure and the failures it reports come from a single walk.
#[cfg(test)]
fn problems(scanned: &[(String, String)]) -> Option<String> {
    detail(&audit(scanned))
}

/// The failure detail for an already-computed [`Audit`], or `None` when it is clean.
fn detail(found: &Audit) -> Option<String> {
    if found.problems.is_empty() {
        return None;
    }
    let mut lines = found.problems.clone();
    lines.push(RECOVERY.to_string());
    lines.extend(found.census_rows());
    Some(lines.join("\n"))
}

/// Convert an audit and any file-read failures into the step result. Successful
/// checks carry no detail; the derived census is useful only when there is a problem
/// to investigate.
fn audit_step_result(mut read_errors: Vec<String>, found: &Audit) -> StepResult {
    match (read_errors.is_empty(), detail(found)) {
        (true, None) => StepResult::ok(STEP),
        (_, problems) => {
            read_errors.extend(problems);
            StepResult::fail(STEP).detail(read_errors.join("\n"))
        }
    }
}

/// Scan every TypeScript file under each of [`POLICED_ROOTS`] and push the result
/// step. A missing root is a hard failure, so a moved/renamed tree can never quietly
/// disable the guard. Successful checks carry no detail.
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
    // One walk of the tree supplies both failures and the census attached to failure
    // diagnostics.
    let found = audit(&scanned);
    result.push(audit_step_result(read_errors, &found));
}

#[cfg(test)]
mod tests {
    use super::{audit, audit_step_result, census, problems, violations};

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

    /// The wrapper's own module is not exempt by being the wrapper: an unmarked
    /// `page.goto` in `helpers.ts` fails like anywhere else, and the marked one enters
    /// the census. A whole-file exemption would hide a second call added there later.
    #[test]
    fn the_helpers_module_is_scanned_like_any_other_file() {
        assert!(
            problems(&[(
                "end2end/tests/helpers.ts".to_string(),
                "  await page.goto(url);\n".to_string()
            )])
            .is_some(),
            "an unmarked call in the wrapper's own module still fails"
        );
        assert_eq!(
            census(&[(
                "end2end/tests/helpers.ts".to_string(),
                "  // e2e-goto-wrapper:allow this call IS the wrapper\n  await page.goto(url);\n"
                    .to_string()
            )]),
            vec!["    - end2end/tests/helpers.ts:2 — this call IS the wrapper"]
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

    #[test]
    fn clean_success_carries_no_detail() {
        let found = audit(&[(
            "end2end/tests/x.ts".to_string(),
            "// e2e-goto-wrapper:allow the probe owns this raw load\nawait page.goto(url);\n"
                .to_string(),
        )]);
        let result = audit_step_result(Vec::new(), &found);
        assert!(result.ok);
        assert_eq!(result.detail, None);
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
