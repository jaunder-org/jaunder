//! The `xlang-literal` static check (#767): a constant spelled once in Rust and
//! once in TypeScript must agree on both sides.
//!
//! A few literals in this repo cannot be declared once, because no import spans
//! the boundary they cross — the CSR mount marker crosses into an
//! `inline_js` string as an opaque JS literal, and the boot-mark prefix is read
//! by the Playwright harness in Node. #251 (D6) decided those duplications are
//! necessary; nothing checked them.
//!
//! Drift in that class is silent at build time and maximally expensive at test
//! time: nothing fails to compile, `cargo xtask check` never runs e2e, and the
//! whole `{sqlite,postgres}×{chromium,firefox}` matrix goes red with dozens of
//! Playwright timeouts whose output never names the one-word typo that caused
//! them. This gate turns that into one line here.
//!
//! **How a site is located.** Each [`Site`] carries a literal `anchor` — the
//! *declaration* form, `MOUNTED_ATTR = ` rather than `data-mounted` — and the
//! quote that opens the literal directly after it. The anchor **locates a site;
//! it never decides a violation.** The violation is exact string inequality
//! between two extracted literals, so an out-of-date anchor cannot pass
//! silently: zero occurrences or more than one are both hard failures, as are an
//! unreadable file and a malformed literal.
//!
//! Anchoring on the declaration rather than the value is load-bearing, not
//! stylistic. Every policed site carries a prose comment naming its counterpart,
//! and those comments mention the value — an anchor that matched the value would
//! let a documentation edit change this gate's verdict.
//!
//! **What this gate does not claim.** Its population is exactly [`PAIRS`]. A
//! third literal duplicated across languages that nobody added to the table is
//! unpoliced, and recorded nowhere — no green run here implies it was examined.
//! That limit is inherent to a declared-pair design: there is no structural
//! property distinguishing "duplicated across languages on purpose" from "two
//! files that happen to share a string." Stated here per ADR-0085's honesty
//! obligation. See `docs/adr/drafts/cross-language-literal-agreement.md`.

use std::path::Path;

use crate::result::{CommandResult, StepResult};

/// One side of a cross-language literal pair: where the literal is declared, and
/// how to find it.
///
/// `anchor` must be the declaration form and must occur exactly once in `file`;
/// `quote` must be the character immediately following it. See the module doc for
/// why both of those are stricter than they need to be to work today.
pub struct Site {
    pub file: &'static str,
    pub anchor: &'static str,
    pub quote: char,
}

/// The literal `site` declares, read out of `source`.
///
/// `Err` carries a message naming `site.file` and what went wrong — every failure
/// path here means the gate could not locate what it was asked to compare, which
/// must be loud rather than treated as agreement.
pub fn literal_in(site: &Site, source: &str) -> Result<String, String> {
    // Occurrences, not matching lines: two anchors on one line must not resolve
    // to a silent first-wins.
    let occurrences = source.matches(site.anchor).count();
    if occurrences != 1 {
        let what = if occurrences == 0 {
            "not found".to_string()
        } else {
            format!("found {occurrences} times")
        };
        return Err(format!(
            "{}: anchor `{}` {what} — the gate cannot locate the literal it is \
             asked to compare. If the declaration moved, was renamed, or gained a \
             second occurrence, update the `xlang-literal` table rather than \
             leaving the anchor matching nothing (#767)",
            site.file, site.anchor,
        ));
    }

    // The count above proves there is exactly one, so this cannot be `None`.
    let start = source.find(site.anchor).expect("exactly one occurrence") + site.anchor.len();
    let rest = &source[start..];
    if !rest.starts_with(site.quote) {
        return Err(format!(
            "{}: anchor `{}` is not immediately followed by a {} literal — the \
             declaration's shape changed, so the table is stale (#767)",
            site.file, site.anchor, site.quote,
        ));
    }

    let mut literal = String::new();
    let mut escaped = false;
    for c in rest[site.quote.len_utf8()..].chars() {
        if escaped {
            // The backslash is kept: the gate compares literals to each other, not
            // to a decoded value, and both sides are read the same way.
            literal.push('\\');
            literal.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == site.quote {
            return Ok(literal);
        } else {
            literal.push(c);
        }
    }
    Err(format!(
        "{}: unterminated {} literal after anchor `{}` (#767)",
        site.file, site.quote, site.anchor,
    ))
}

/// Two declarations of the same constant, in two languages, that must agree.
pub struct Pair {
    pub key: &'static str,
    pub a: Site,
    pub b: Site,
}

/// The cross-language literal pairs this gate polices. Adding a pair is a row
/// here; see the module doc for what this table does **not** claim.
pub const PAIRS: &[Pair] = &[
    Pair {
        key: "mount-marker",
        a: Site {
            file: "csr/src/lib.rs",
            anchor: "setAttribute(",
            quote: '\'',
        },
        b: Site {
            file: "end2end/tests/mount.ts",
            anchor: "MOUNTED_ATTR = ",
            quote: '"',
        },
    },
    Pair {
        key: "mark-prefix",
        a: Site {
            file: "client/src/perf/mod.rs",
            anchor: "MARK_PREFIX: &str = ",
            quote: '"',
        },
        b: Site {
            file: "end2end/tests/capture-trace.ts",
            anchor: "MARK_PREFIX = ",
            quote: '"',
        },
    },
];

/// Every disagreement, unreadable file, and unlocatable site across [`PAIRS`],
/// or `None` when every pair agrees. `root` is the repository root the site paths
/// are relative to — a parameter rather than a constant so the failure paths can
/// be tested against a fixture tree.
pub fn problems(root: &Path) -> Option<String> {
    let mut lines = Vec::new();

    for pair in PAIRS {
        let mut literals = Vec::new();
        for site in [&pair.a, &pair.b] {
            let path = root.join(site.file);
            match std::fs::read_to_string(&path) {
                Err(e) => lines.push(format!(
                    "{}: cannot read {}: {e} — a moved or deleted site must fail \
                     rather than quietly shrink what this gate checks (#767)",
                    pair.key,
                    path.display(),
                )),
                Ok(source) => match literal_in(site, &source) {
                    Err(message) => lines.push(format!("{}: {message}", pair.key)),
                    Ok(literal) => literals.push(literal),
                },
            }
        }

        // Only compare when BOTH sides yielded a literal. A site we could not read
        // has no value to disagree with, and reporting that as drift would send the
        // reader hunting for a mismatch that does not exist.
        if let [a, b] = literals.as_slice()
            && a != b
        {
            lines.push(format!(
                "{}: cross-language literals disagree — {} says {a:?}, {} says {b:?}. \
                 They must be identical; if they drift, every e2e test times out and \
                 the matrix goes red without naming the cause (#767)",
                pair.key, pair.a.file, pair.b.file,
            ));
        }
    }

    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Check every declared pair and push the result step. Paths resolve against the
/// cwd, which for xtask is always the repository root.
pub fn run(result: &mut CommandResult) {
    let step = match problems(Path::new(".")) {
        None => StepResult::ok("xlang-literal"),
        Some(detail) => StepResult::fail("xlang-literal").detail(detail),
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{PAIRS, Site, literal_in, problems};

    fn ts_site() -> Site {
        Site {
            file: "end2end/tests/mount.ts",
            anchor: "MOUNTED_ATTR = ",
            quote: '"',
        }
    }

    fn rust_site() -> Site {
        Site {
            file: "csr/src/lib.rs",
            anchor: "setAttribute(",
            quote: '\'',
        }
    }

    #[test]
    fn extracts_a_double_quoted_literal() {
        let src = "export const MOUNTED_ATTR = \"data-mounted\";\n";
        assert_eq!(literal_in(&ts_site(), src).unwrap(), "data-mounted");
    }

    #[test]
    fn extracts_a_single_quoted_literal() {
        let src = "        document.body.setAttribute('data-mounted', 'true');\n";
        assert_eq!(literal_in(&rust_site(), src).unwrap(), "data-mounted");
    }

    /// The whole point of the gate: a locator that has stopped locating anything
    /// must be loud, never a pass.
    #[test]
    fn a_missing_anchor_is_an_error_naming_the_file_and_anchor() {
        let e = literal_in(&ts_site(), "export const OTHER = \"z\";\n").unwrap_err();
        assert!(e.contains("end2end/tests/mount.ts"), "{e}");
        assert!(e.contains("MOUNTED_ATTR = "), "{e}");
        assert!(e.contains("not found"), "{e}");
        assert!(e.contains("#767"), "{e}");
    }

    #[test]
    fn a_repeated_anchor_is_an_error_naming_the_count() {
        let src = "export const MOUNTED_ATTR = \"a\";\nexport const MOUNTED_ATTR = \"b\";\n";
        let e = literal_in(&ts_site(), src).unwrap_err();
        assert!(e.contains("2 times"), "{e}");
        assert!(e.contains("end2end/tests/mount.ts"), "{e}");
    }

    /// Counting occurrences rather than matching lines — two anchors on one line
    /// must not resolve to a silent first-wins.
    #[test]
    fn two_anchors_on_one_line_also_fail() {
        let src = "MOUNTED_ATTR = \"a\"; MOUNTED_ATTR = \"b\";\n";
        let e = literal_in(&ts_site(), src).unwrap_err();
        assert!(e.contains("2 times"), "{e}");
    }

    /// The anchor ends immediately before the opening quote, so anything else
    /// there means the declaration's shape changed and the table is stale.
    #[test]
    fn an_anchor_not_immediately_followed_by_the_quote_is_an_error() {
        let e = literal_in(&ts_site(), "export const MOUNTED_ATTR = someIdent;\n").unwrap_err();
        assert!(e.contains("not immediately followed"), "{e}");
        assert!(e.contains("end2end/tests/mount.ts"), "{e}");
    }

    #[test]
    fn an_unterminated_literal_is_an_error() {
        let e =
            literal_in(&ts_site(), "export const MOUNTED_ATTR = \"data-mounted;\n").unwrap_err();
        assert!(e.contains("unterminated"), "{e}");
    }

    #[test]
    fn an_escaped_quote_does_not_end_the_literal() {
        let src = "export const MOUNTED_ATTR = \"a\\\"b\";\n";
        assert_eq!(literal_in(&ts_site(), src).unwrap(), "a\\\"b");
    }

    #[test]
    fn an_empty_literal_extracts_as_empty_rather_than_erroring() {
        let src = "export const MOUNTED_ATTR = \"\";\n";
        assert_eq!(literal_in(&ts_site(), src).unwrap(), "");
    }

    /// Build a fixture root holding every file `PAIRS` names, with the literal of
    /// each site set from `values` — indexed the way `PAIRS` is walked: pair 0
    /// side a, pair 0 side b, pair 1 side a, pair 1 side b.
    fn fixture_root(values: [&str; 4]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut values = values.into_iter();
        for pair in PAIRS {
            for site in [&pair.a, &pair.b] {
                let value = values.next().expect("one value per site");
                let path = dir.path().join(site.file);
                std::fs::create_dir_all(path.parent().expect("site files are nested"))
                    .expect("mkdir");
                let q = site.quote;
                std::fs::write(
                    &path,
                    format!("prefix {}{q}{value}{q} suffix\n", site.anchor),
                )
                .expect("write");
            }
        }
        dir
    }

    #[test]
    fn agreeing_pairs_report_no_problem() {
        let dir = fixture_root(["data-mounted", "data-mounted", "jaunder.", "jaunder."]);
        assert_eq!(problems(dir.path()), None);
    }

    /// The gate asserts agreement, not a value: a consistent rename must pass.
    #[test]
    fn a_consistent_rename_on_both_sides_passes() {
        let dir = fixture_root(["data-ready", "data-ready", "jaunder.", "jaunder."]);
        assert_eq!(problems(dir.path()), None);
    }

    #[test]
    fn mount_marker_drift_names_the_key_both_files_and_both_values() {
        let dir = fixture_root(["data-mounted", "data-mountd", "jaunder.", "jaunder."]);
        let detail = problems(dir.path()).expect("a problem");
        assert!(detail.contains("mount-marker"), "{detail}");
        assert!(detail.contains("csr/src/lib.rs"), "{detail}");
        assert!(detail.contains("end2end/tests/mount.ts"), "{detail}");
        assert!(detail.contains("data-mounted"), "{detail}");
        assert!(detail.contains("data-mountd"), "{detail}");
    }

    /// The same for the second pair — proving the table is a loop, not a special
    /// case wrapped around one comparison.
    #[test]
    fn mark_prefix_drift_names_the_key_both_files_and_both_values() {
        let dir = fixture_root(["data-mounted", "data-mounted", "jaunder.", "jaunder-"]);
        let detail = problems(dir.path()).expect("a problem");
        assert!(detail.contains("mark-prefix"), "{detail}");
        assert!(detail.contains("client/src/perf/mod.rs"), "{detail}");
        assert!(
            detail.contains("end2end/tests/capture-trace.ts"),
            "{detail}"
        );
        assert!(detail.contains("jaunder."), "{detail}");
        assert!(detail.contains("jaunder-"), "{detail}");
    }

    /// A missing site file is a hard failure, never a skip or a pass. Making
    /// `root` a parameter is what lets this arm be tested at all.
    #[test]
    fn a_missing_site_file_fails_and_names_the_path() {
        let dir = fixture_root(["data-mounted", "data-mounted", "jaunder.", "jaunder."]);
        std::fs::remove_file(dir.path().join("csr/src/lib.rs")).expect("remove");
        let detail = problems(dir.path()).expect("a problem");
        assert!(detail.contains("csr/src/lib.rs"), "{detail}");
        assert!(detail.contains("cannot read"), "{detail}");
    }

    /// An extraction failure on one side must not be reported as a disagreement:
    /// there is no second value to disagree with, and saying so would send the
    /// reader looking for a drift that is not there.
    #[test]
    fn an_unlocatable_site_reports_the_anchor_not_a_mismatch() {
        let dir = fixture_root(["data-mounted", "data-mounted", "jaunder.", "jaunder."]);
        std::fs::write(dir.path().join("end2end/tests/mount.ts"), "nothing here\n").expect("write");
        let detail = problems(dir.path()).expect("a problem");
        assert!(detail.contains("not found"), "{detail}");
        assert!(!detail.contains("disagree"), "{detail}");
    }

    #[test]
    fn every_pair_key_is_unique() {
        let mut keys: Vec<&str> = PAIRS.iter().map(|p| p.key).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "duplicate key in PAIRS");
    }

    /// Every table entry must resolve against the **real** tree. Every other test
    /// in this file feeds a fixture, which by construction cannot notice that an
    /// anchor no longer matches the source it was written for — so without this,
    /// a refactor of `csr/src/lib.rs` or `mount.ts` would silently disarm the gate
    /// and every run would stay green.
    ///
    /// The test binary's cwd is the `xtask` package, not the repository root,
    /// hence `CARGO_MANIFEST_DIR`.
    #[test]
    fn the_real_table_resolves_and_agrees() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for pair in PAIRS {
            for site in [&pair.a, &pair.b] {
                let path = root.join(site.file);
                let source = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                literal_in(site, &source).unwrap_or_else(|e| panic!("{e}"));
            }
        }
        assert_eq!(problems(&root), None);
    }
}
