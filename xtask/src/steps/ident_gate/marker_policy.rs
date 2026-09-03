use std::collections::HashMap;

use super::traversal::{MentionContext, Scan};

/// Why a mention is not exempt — each variant is a different message, because
/// "you forgot a marker" and "your marker has no reason" need different fixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Why {
    /// No marker on the line above.
    Unmarked,
    /// A marker with no reason text after the token.
    NoReason,
    /// The marked line carries this many sites of the same gate — more than one,
    /// so a single marker cannot say which it justifies.
    Shared(usize),
}

/// A mention the gate's marker does not cover.
#[derive(Debug, Clone)]
pub(super) struct Unexempt {
    pub(super) line: usize,
    pub(super) context: MentionContext,
    pub(super) why: Why,
}

/// A legitimately marked site — one row of the derived census.
#[derive(Debug, Clone)]
pub(super) struct Marked {
    /// The **site's** line, not the marker's: that is what a reader needs and what
    /// the failure messages already print.
    pub(super) line: usize,
    pub(super) reason: String,
}

/// Every mention of one source, sorted into the three outcomes.
#[derive(Debug, Default)]
pub(super) struct Classified {
    pub(super) unexempt: Vec<Unexempt>,
    pub(super) marked: Vec<Marked>,
    /// Lines carrying this gate's marker whose next line holds no site.
    pub(super) orphans: Vec<usize>,
}

/// Sort every mention into marked / unexempt, and find the markers that cover
/// nothing.
///
/// The marker sits on the line **immediately above** its site — the one position
/// `rustfmt` and `leptosfmt` both preserve (#778). A trailing marker is therefore
/// not an exemption at all: the site sees nothing above it, and the marker itself
/// points at a line with no site, so it fails twice over.
pub(super) fn classify(source: &str, found: &Scan, token: &str) -> Classified {
    // File-aware, deliberately: a per-line read would treat the interior of a
    // multi-line string or of a `/* … */` block as ordinary code and hand its `//`
    // the force of a marker — an exemption nobody wrote, on a security gate.
    let comments = crate::markers::line_comments(source);
    // 1-based line → the marker's reason, for every line carrying this gate's token.
    let marker_at = |line: usize| -> Option<&str> {
        crate::markers::marker_in_comment((*comments.get(line.checked_sub(1)?)?)?, token)
    };

    let mut sites_on_line: HashMap<usize, usize> = HashMap::new();
    for m in &found.mentions {
        *sites_on_line.entry(m.line).or_insert(0) += 1;
    }

    let mut out = Classified::default();
    for m in &found.mentions {
        let unexempt = |why| Unexempt {
            line: m.line,
            context: m.context.clone(),
            why,
        };
        match m.line.checked_sub(1).and_then(marker_at) {
            None => out.unexempt.push(unexempt(Why::Unmarked)),
            Some("") => out.unexempt.push(unexempt(Why::NoReason)),
            Some(reason) => {
                let sites = sites_on_line.get(&m.line).copied().unwrap_or(1);
                if sites > 1 {
                    out.unexempt.push(unexempt(Why::Shared(sites)));
                } else {
                    out.marked.push(Marked {
                        line: m.line,
                        reason: reason.to_string(),
                    });
                }
            }
        }
    }

    // An orphan is a marker whose very next line holds no site. Test regions are
    // exempt wholesale, so a marker inside one is never an orphan.
    for line in 1..=comments.len() {
        if marker_at(line).is_some()
            && !sites_on_line.contains_key(&(line + 1))
            && !found.in_test_code(line)
        {
            out.orphans.push(line);
        }
    }
    out
}

/// The marker rule (#778), tested here rather than three times over: a marker on
/// the line ABOVE a site exempts it, and nothing else does.
#[cfg(test)]
mod marker_tests {
    use super::super::traversal::scan;
    use super::{Classified, Why, classify};

    const TOKEN: &str = "guard:allow";

    fn classified(src: &str) -> Classified {
        let s = scan(src, &["GUARDED"], None).unwrap();
        classify(src, &s, TOKEN)
    }

    #[test]
    fn a_marked_site_is_exempt_and_enters_the_census() {
        let c = classified("// guard:allow because reasons\nfn a() { GUARDED; }\n");
        assert!(c.unexempt.is_empty());
        assert_eq!(c.marked.len(), 1);
        assert_eq!(c.marked[0].line, 2, "the census names the SITE line");
        assert_eq!(c.marked[0].reason, "because reasons");
        assert!(c.orphans.is_empty());
    }

    #[test]
    fn an_unmarked_site_is_unexempt() {
        let c = classified("fn a() { GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 1);
        assert_eq!(c.unexempt[0].why, Why::Unmarked);
        assert_eq!(c.unexempt[0].context.legacy_label(), "a");
        assert!(c.marked.is_empty());
    }

    #[test]
    fn a_bare_marker_is_unexempt() {
        let c = classified("// guard:allow\nfn a() { GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 1);
        assert_eq!(c.unexempt[0].why, Why::NoReason);
        assert!(c.marked.is_empty());
    }

    /// Trailing is the position the formatters relocate, so honoring it would let
    /// someone write a marker that stops working on the next `cargo xtask check`.
    /// It fails twice over: the site sees nothing above it, and the marker points
    /// at a line with no site.
    #[test]
    fn a_trailing_marker_does_not_exempt() {
        let c = classified("fn a() { GUARDED; } // guard:allow trailing\n");
        assert_eq!(c.unexempt.len(), 1);
        assert_eq!(c.unexempt[0].why, Why::Unmarked);
        assert_eq!(c.orphans, vec![1]);
    }

    #[test]
    fn a_marker_two_lines_above_does_not_exempt() {
        let c = classified("// guard:allow far\n\nfn a() { GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 1);
        assert_eq!(c.orphans, vec![1]);
    }

    #[test]
    fn a_marker_below_the_site_does_not_exempt() {
        let c = classified("fn a() { GUARDED; }\n// guard:allow below\n");
        assert_eq!(c.unexempt.len(), 1);
        assert_eq!(c.orphans, vec![2]);
    }

    #[test]
    fn two_sites_on_the_marked_line_are_both_unexempt() {
        let c = classified("// guard:allow reason\nfn a() { GUARDED; GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 2);
        assert!(c.unexempt.iter().all(|u| u.why == Why::Shared(2)));
        assert!(c.marked.is_empty());
    }

    #[test]
    fn two_sites_on_an_unmarked_line_are_unmarked_not_shared() {
        let c = classified("fn a() { GUARDED; GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 2);
        assert!(c.unexempt.iter().all(|u| u.why == Why::Unmarked));
    }

    #[test]
    fn a_marker_with_no_site_below_is_an_orphan() {
        let c = classified("// guard:allow reason\nfn a() { harmless(); }\n");
        assert_eq!(c.orphans, vec![1]);
        assert!(c.unexempt.is_empty());
    }

    #[test]
    fn a_marker_on_a_test_code_site_is_not_an_orphan() {
        let src = "#[cfg(test)]\nmod t {\n  // guard:allow fixture\n  fn f() { GUARDED; }\n}\n";
        let c = classified(src);
        assert!(c.orphans.is_empty());
        assert!(c.unexempt.is_empty());
        assert!(c.marked.is_empty(), "test code is not part of the census");
    }

    /// The harder half: a marker in test code whose site is GONE. Test regions are
    /// exempt wholesale, so it is not an orphan either.
    #[test]
    fn a_stale_marker_inside_test_code_is_not_an_orphan() {
        let src = "#[cfg(test)]\nmod t {\n  // guard:allow stale\n  fn f() { harmless(); }\n}\n";
        assert!(classified(src).orphans.is_empty());
    }

    #[test]
    fn a_marker_inside_a_string_literal_exempts_nothing() {
        let c = classified("fn b() { let s = \"// guard:allow x\"; }\nfn a() { GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 1);
        assert!(c.orphans.is_empty());
    }

    /// The false-PASS a per-line scan allows: the marker text is the interior of a
    /// multi-line string, so it is not a comment and exempts nothing.
    #[test]
    fn a_marker_inside_a_multi_line_string_exempts_nothing() {
        let src = "fn b() { let s = \"a\n// guard:allow x\"; }\nfn a() { GUARDED; }\n";
        let c = classified(src);
        assert_eq!(c.unexempt.len(), 1, "the site must stay unexempt");
        assert!(c.orphans.is_empty());
    }

    #[test]
    fn a_marker_inside_a_multi_line_raw_string_exempts_nothing() {
        let src = "fn b() { let s = r#\"a\n// guard:allow x\n\"#; }\nfn a() { GUARDED; }\n";
        assert_eq!(classified(src).unexempt.len(), 1);
    }

    #[test]
    fn a_marker_inside_a_block_comment_exempts_nothing() {
        let src = "/* // guard:allow x */\nfn a() { GUARDED; }\n";
        let c = classified(src);
        assert_eq!(c.unexempt.len(), 1);
        assert!(c.orphans.is_empty());
    }

    #[test]
    fn a_doc_comment_marker_exempts_nothing() {
        let c = classified("/// guard:allow x\nfn a() { GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 1);
        assert!(c.orphans.is_empty(), "a doc comment carries no marker");
    }

    #[test]
    fn another_gates_marker_does_not_exempt() {
        let c = classified("// other:allow reason\nfn a() { GUARDED; }\n");
        assert_eq!(c.unexempt.len(), 1);
        assert!(
            c.orphans.is_empty(),
            "a foreign token is not this gate's orphan"
        );
    }

    #[test]
    fn a_site_inside_a_macro_body_is_exempted_from_the_line_above() {
        let src = "fn a() -> V {\n    // guard:allow reason\n    m! { GUARDED }\n}\n";
        let c = classified(src);
        assert!(c.unexempt.is_empty());
        assert_eq!(c.marked.len(), 1);
        assert_eq!(c.marked[0].line, 3);
    }

    #[test]
    fn a_multi_line_statement_is_marked_above_the_ident_line() {
        let src =
            "fn a() {\n    take(\n        // guard:allow reason\n        GUARDED,\n    );\n}\n";
        let c = classified(src);
        assert!(c.unexempt.is_empty());
        assert_eq!(c.marked[0].line, 4);
    }

    /// Above the IDENT's line, not above the statement that contains it.
    #[test]
    fn a_marker_above_the_statements_first_line_does_not_exempt() {
        let src = "fn a() {\n    // guard:allow reason\n    take(\n        GUARDED,\n    );\n}\n";
        let c = classified(src);
        assert_eq!(c.unexempt.len(), 1);
        assert_eq!(c.orphans, vec![2]);
    }

    #[test]
    fn the_census_comes_back_in_line_order() {
        let src = "// guard:allow first\nfn a() { GUARDED; }\n// guard:allow second\nfn b() { GUARDED; }\n";
        let c = classified(src);
        assert_eq!(
            c.marked.iter().map(|m| m.line).collect::<Vec<_>>(),
            vec![2, 4]
        );
    }
}
