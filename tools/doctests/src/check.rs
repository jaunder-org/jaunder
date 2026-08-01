//! The rules: which fence forms are allowed, and what makes a `compile_fail`
//! non-vacuous.
//!
//! # The vocabulary is a closed set, not a model of rustdoc
//!
//! A fence's info string must be exactly one of [`PLAIN`], [`COMPILE_FAIL`], or
//! [`TEXT`] (compared with whitespace removed). Everything else fails, including
//! `ignore`, `no_run`, `should_panic`, language tags, and any unrecognized word.
//! The set grows only by a deliberate edit here — never by a fence tagging itself.
//!
//! Denying by default rather than emulating rustdoc's collection rules is the
//! point. Two of the forms are actively dangerous:
//!
//! - `ignore` **is** collected and reported (as ignored), so a check that asked
//!   only "did this fence appear in the run" would accept it — a one-word
//!   self-exemption that silences a proof while leaving the appearance of one.
//! - A wholly unrecognized word makes rustdoc treat the block as non-Rust and skip
//!   it **with no warning at all** (probed 2026-08-01). A typo deletes a proof and
//!   reports green forever.
//!
//! `text` is the one way to say "this is an illustration, not a proof", and it says
//! so by ceasing to claim otherwise: it renders as non-Rust and reads as prose. A
//! `-compile_fail` / `+text` edit is a reviewable diff hunk, exactly like deleting
//! the fence. No gate can stop a human deleting a proof; the job is to stop it
//! happening *silently*.
//!
//! # The companion rule
//!
//! A `compile_fail` passes if its snippet fails to compile *for any reason* — a
//! renamed path, an import that stopped resolving — so it can rot into vacuous
//! truth while still reporting green. The defence, which `macros/src/lib.rs`
//! already used in one place, is a positive companion carrying the identical
//! fixture: if the companion compiles, the negative failed for the stated reason.
//!
//! Made mechanical: every `compile_fail` must carry at least one `#`-hidden line,
//! and every hidden line must appear verbatim in some plain fence in the **same
//! doc comment**. The hidden prelude *is* the fixture, so matching it is precisely
//! what proves the negative discriminates. There is no exemption.
//!
//! Scoping to one doc comment is load-bearing. A file-wide or "any plain fence
//! nearby" rule would let one companion silently cover negatives whose fixtures it
//! shares nothing with — the region-scoped exemption ADR-0085 principle 4 forbids.

use serde::{Deserialize, Serialize};

use crate::fence::Scan;

/// A passing example. Also the companion form the companion rule looks for.
pub const PLAIN: &str = "";
/// A negative proof: the snippet must NOT compile.
pub const COMPILE_FAIL: &str = "compile_fail";
/// An illustration. Not collected by rustdoc, not a proof, and says so.
pub const TEXT: &str = "text";

/// One thing wrong with the fence population.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    /// Repo-relative path.
    pub file: String,
    /// 1-based line of the fence, or of the offending attribute.
    pub line: usize,
    pub kind: Kind,
    /// Human detail, ending in the recovery instruction.
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// An info string outside the accepted vocabulary.
    BannedAttribute,
    /// A fence inside a multi-line `#[doc = "…"]` value.
    MultilineDocAttr,
    /// A `compile_fail` with no hidden prelude, or a hidden line no companion in
    /// the same doc comment carries.
    MissingCompanion,
    /// Scanned in the tree, absent from the run.
    NotRun,
    /// The run reported this doctest as FAILED.
    Failed,
    /// Reported by the run, matched by no scanned fence.
    Orphan,
    /// A file under a scan root that could not be read or parsed.
    Unreadable,
}

/// The info string with all whitespace removed, so `compile_fail, intent_only` and
/// `compile_fail,intent_only` are the same string and neither can sneak past by
/// spacing.
fn normalized(info: &str) -> String {
    info.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Vocabulary and companion violations in one file's scan.
pub fn fence_violations(file: &str, scan: &Scan) -> Vec<Violation> {
    let mut out = Vec::new();

    for line in &scan.multiline_doc_attrs {
        out.push(Violation {
            file: file.to_string(),
            line: *line,
            kind: Kind::MultilineDocAttr,
            detail: "a multi-line `#[doc = \"…\"]` value cannot be keyed to a fence's source \
                     line: the runner reports the attribute's line plus a markdown offset. \
                     recovery: rewrite it as `///` lines, or as one `#[doc]` per line."
                .to_string(),
        });
    }

    for fence in &scan.fences {
        let info = normalized(&fence.info);
        if info != PLAIN && info != COMPILE_FAIL && info != TEXT {
            out.push(Violation {
                file: file.to_string(),
                line: fence.line,
                kind: Kind::BannedAttribute,
                detail: format!(
                    "fence attribute `{}` is outside the accepted vocabulary \
                     (``` , ```{COMPILE_FAIL}, ```{TEXT}). recovery: make it a real test, or \
                     mark it ```{TEXT} to say it is an illustration.",
                    fence.info
                ),
            });
            continue;
        }
        if info != COMPILE_FAIL {
            continue;
        }
        if fence.hidden.is_empty() {
            out.push(Violation {
                file: file.to_string(),
                line: fence.line,
                kind: Kind::MissingCompanion,
                detail: "a `compile_fail` carries no `#`-hidden prelude, so nothing ties it to \
                         a positive companion and it would still pass if its paths stopped \
                         resolving. recovery: hide the fixture lines with `# ` and repeat them \
                         in a plain fence in this doc comment."
                    .to_string(),
            });
            continue;
        }
        // Only fences in the SAME doc comment count — see the module doc.
        let companion_lines: Vec<&String> = scan
            .fences
            .iter()
            .filter(|f| f.doc_block == fence.doc_block && normalized(&f.info) == PLAIN)
            .flat_map(|f| f.hidden.iter().chain(f.visible.iter()))
            .collect();
        for needed in &fence.hidden {
            if needed.is_empty() || companion_lines.contains(&needed) {
                continue;
            }
            out.push(Violation {
                file: file.to_string(),
                line: fence.line,
                kind: Kind::MissingCompanion,
                detail: format!(
                    "hidden prelude line `{needed}` appears in no plain fence in this doc \
                     comment, so this `compile_fail` would still pass if that line stopped \
                     compiling. recovery: add or extend a companion fence carrying it."
                ),
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fence::fences;

    fn violations(src: &str) -> Vec<Violation> {
        let scan = fences(src).expect("parses");
        fence_violations("f.rs", &scan)
    }

    fn kinds(src: &str) -> Vec<Kind> {
        violations(src).into_iter().map(|v| v.kind).collect()
    }

    #[test]
    fn the_three_accepted_forms_pass() {
        let src = "\n/// ```\n/// # let s = 1;\n/// let _ = s;\n/// ```\n///\n/// ```compile_fail\n/// # let s = 1;\n/// let _: &str = s;\n/// ```\n///\n/// ```text\n/// not rust\n/// ```\npub struct A;\n";
        assert!(kinds(src).is_empty(), "{:?}", kinds(src));
    }

    #[test]
    fn ignore_is_banned() {
        let src = "\n/// ```ignore\n/// let x = 1;\n/// ```\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::BannedAttribute]);
    }

    #[test]
    fn no_run_and_should_panic_are_banned() {
        let a = "\n/// ```no_run\n/// let x = 1;\n/// ```\npub struct A;\n";
        let b = "\n/// ```should_panic\n/// panic!();\n/// ```\npub struct B;\n";
        assert_eq!(kinds(a), vec![Kind::BannedAttribute]);
        assert_eq!(kinds(b), vec![Kind::BannedAttribute]);
    }

    #[test]
    fn an_unknown_tag_is_banned_because_rustdoc_drops_it_silently() {
        // Probed: a wholly unrecognized word makes rustdoc skip the block with no
        // warning at all, deleting the proof.
        let src = "\n/// ```rust,nocheck\n/// let x = 1;\n/// ```\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::BannedAttribute]);
    }

    #[test]
    fn a_language_tag_is_banned_text_is_the_only_non_rust_form() {
        let src = "\n/// ```sql\n/// SELECT 1;\n/// ```\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::BannedAttribute]);
    }

    #[test]
    fn a_compile_fail_with_a_trailing_marker_word_is_banned() {
        // No exemption marker ships. A `compile_fail,<anything>` is an
        // unrecognized form, so adding one back is a deliberate edit here.
        let src =
            "\n/// ```compile_fail,intent_only\n/// let _ = nope();\n/// ```\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::BannedAttribute]);
    }

    #[test]
    fn whitespace_cannot_smuggle_a_variant_past_the_vocabulary() {
        let src =
            "\n/// ```compile_fail, intent_only\n/// let _ = nope();\n/// ```\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::BannedAttribute]);
    }

    #[test]
    fn a_multiline_doc_attribute_is_reported() {
        let src = "\n#[doc = \"```\\nlet x = 1;\\n```\\n\"]\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::MultilineDocAttr]);
    }

    #[test]
    fn the_companion_rule_has_no_exemption() {
        // Every `compile_fail` needs a matched hidden prelude, full stop — the
        // three proofs that would have needed an exemption were made
        // discriminating instead.
        let src = "\n/// ```compile_fail\n/// let _ = nope();\n/// ```\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::MissingCompanion]);
    }

    #[test]
    fn a_compile_fail_with_no_hidden_prelude_fails() {
        // Forces the fixture to be explicit and therefore matchable. This is the
        // rule that catches macros/src/lib.rs:43, :220 and :274.
        let src = "\n/// ```\n/// let a = 1;\n/// ```\n///\n/// ```compile_fail\n/// let _: i32 = \"x\";\n/// ```\npub struct A;\n";
        assert_eq!(kinds(src), vec![Kind::MissingCompanion]);
    }

    #[test]
    fn a_hidden_line_matched_by_no_companion_fails() {
        let src = "\n/// ```\n/// # use foo::Bar;\n/// let _ = Bar;\n/// ```\n///\n/// ```compile_fail\n/// # use foo::Baz;\n/// let _ = Baz.nope();\n/// ```\npub struct A;\n";
        let v = violations(src);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, Kind::MissingCompanion);
        assert!(v[0].detail.contains("use foo::Baz;"), "{}", v[0].detail);
    }

    #[test]
    fn a_hidden_line_matching_a_companions_visible_line_passes() {
        // macros/src/lib.rs:51 shows `use macros::StrNewtype;` visibly while the
        // negative at :43 hides it; both spellings are the same fixture line.
        let src = "\n/// ```\n/// use foo::Bar;\n/// let s = Bar;\n/// ```\n///\n/// ```compile_fail\n/// # use foo::Bar;\n/// # let s = Bar;\n/// let _: i32 = s;\n/// ```\npub struct A;\n";
        assert!(kinds(src).is_empty(), "{:?}", kinds(src));
    }

    #[test]
    fn a_companion_in_a_different_doc_comment_does_not_count() {
        // Otherwise one companion silently covers unrelated negatives elsewhere in
        // the file — the region-scoped exemption ADR-0085 principle 4 forbids.
        let src = "\n/// ```\n/// # use foo::Bar;\n/// let _ = Bar;\n/// ```\npub struct A;\n\n/// ```compile_fail\n/// # use foo::Bar;\n/// let _: i32 = Bar;\n/// ```\npub struct B;\n";
        assert_eq!(kinds(src), vec![Kind::MissingCompanion]);
    }

    #[test]
    fn a_text_fence_needs_no_companion() {
        let src = "\n/// ```text\n/// illustration\n/// ```\npub struct A;\n";
        assert!(kinds(src).is_empty(), "{:?}", kinds(src));
    }
}
