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
//!
//! # Why doctests do not feed the coverage gate
//!
//! They deliberately do not, and the reason is worth stating where the code lives
//! rather than leaving the next reader to wonder whether it was an oversight.
//! `cargo llvm-cov --doctests` is unstable, and ADR-0050's stateless coverage gate
//! measures the nextest suite only. So the `--doc` run happens **outside** any
//! llvm-cov instrumentation and contributes no profraw: adding these ~50 doctests
//! moves the coverage numbers by exactly zero lines, which is asserted at ship by
//! diffing the gate's verdict against the branch point.
//!
//! That also means coverage is not evidence about doctests, in either direction. A
//! doctest cannot raise a coverage figure, and a coverage figure says nothing about
//! whether the fence population was evaluated. This module is the only thing that
//! answers the latter.

use serde::{Deserialize, Serialize};

use crate::fence::{fences, Scan};
use crate::libtest::run_entries;

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

/// One `.rs` file under a scan root, carrying both spellings of its path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    /// Repo-relative, for the report.
    pub path: String,
    /// As the runner prints it — repo-relative for a workspace run,
    /// manifest-relative for a `--manifest-path` run.
    pub run_path: String,
    pub source: String,
}

/// Every problem with the population, in `(file, line, kind)` order.
///
/// Reconciliation runs in **both** directions, and each catches something the
/// other cannot:
///
/// - *tree → run*: a fence that exists but was never evaluated. Every one of the
///   five shrink vectors lands here — a `#[cfg(feature)]` gate, a `#[cfg(test)]`
///   module, an unrecognized info string, a crate outside the run's reach, a crate
///   with no lib target — whatever the cause, the proof did not run.
/// - *run → tree*: a reported doctest matching no scanned fence. Without this a
///   scanner bug or an unhandled doc form shrinks the gate's **own** population
///   silently, which is ADR-0085 principle 6 turned on the gate itself.
///
/// A fence counts as run only when an entry with the same `(run_path, line)` is
/// present and is neither ignored nor failed — `ignore` blocks are reported by
/// libtest, so presence alone is not evidence a proof was evaluated.
///
/// Pure given `(scanned, output)`, so it is unit-tested directly.
pub fn problems(scanned: &[ScannedFile], output: &str) -> Vec<Violation> {
    let entries = run_entries(output);
    let mut out = Vec::new();
    // `run_path` is the join key; `path` is what the report says.
    let mut matched: Vec<(String, usize)> = Vec::new();

    for file in scanned {
        let scan = match fences(&file.source) {
            Ok(scan) => scan,
            Err(e) => {
                out.push(Violation {
                    file: file.path.clone(),
                    line: 0,
                    kind: Kind::Unreadable,
                    detail: format!(
                        "{e} — an unparsed file is invisible to this gate, so it fails rather \
                         than shrinking the population. recovery: fix the syntax."
                    ),
                });
                continue;
            }
        };
        out.extend(fence_violations(&file.path, &scan));

        for fence in &scan.fences {
            let entry = entries
                .iter()
                .find(|e| e.file == file.run_path && e.line == fence.line);
            if entry.is_some() {
                matched.push((file.run_path.clone(), fence.line));
            }
            let expected_to_run = normalized(&fence.info) != TEXT;
            match (expected_to_run, entry) {
                (true, Some(e)) if e.failed => out.push(Violation {
                    file: file.path.clone(),
                    line: fence.line,
                    kind: Kind::Failed,
                    detail: "the runner reported this doctest as FAILED. recovery: read the \
                             run log for the compiler output."
                        .to_string(),
                }),
                (true, Some(e)) if e.ignored => out.push(Violation {
                    file: file.path.clone(),
                    line: fence.line,
                    kind: Kind::NotRun,
                    detail: "the runner collected this fence but skipped it, so the proof was \
                             not evaluated. recovery: make it a real test, or mark it ```text."
                        .to_string(),
                }),
                (true, None) => out.push(Violation {
                    file: file.path.clone(),
                    line: fence.line,
                    kind: Kind::NotRun,
                    detail: "this fence is in the tree but absent from the run, so whatever it \
                             asserts was never checked — a cfg gate, an unrecognized info \
                             string, or a crate the run cannot reach. recovery: bring it into \
                             the run, or mark it ```text if no run can reach it."
                        .to_string(),
                }),
                (false, Some(_)) => out.push(Violation {
                    file: file.path.clone(),
                    line: fence.line,
                    kind: Kind::Orphan,
                    detail: "a ```text fence must not be collected, but the runner reported it. \
                             recovery: the marker and the content disagree — fix one."
                        .to_string(),
                }),
                _ => {}
            }
        }
    }

    // run → tree. Only files we actually scanned are in scope: a run may legitimately
    // report entries from paths outside this half's roots (the other half gates those).
    let scanned_run_paths: Vec<&String> = scanned.iter().map(|f| &f.run_path).collect();
    for entry in &entries {
        if !scanned_run_paths.contains(&&entry.file) {
            continue;
        }
        if matched
            .iter()
            .any(|(p, l)| *p == entry.file && *l == entry.line)
        {
            continue;
        }
        let path = scanned
            .iter()
            .find(|f| f.run_path == entry.file)
            .map_or(entry.file.clone(), |f| f.path.clone());
        out.push(Violation {
            file: path,
            line: entry.line,
            kind: Kind::Orphan,
            detail: "the runner reported a doctest here that the scanner did not find, so the \
                     gate cannot see part of its own population. recovery: this is a scanner \
                     gap — fix the scanner, do not silence it."
                .to_string(),
        });
    }

    out.sort_by(|a, b| (&a.file, a.line, a.kind).cmp(&(&b.file, b.line, b.kind)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // ---- reconciliation, both directions ----------------------------------

    fn file(path: &str, run_path: &str, source: &str) -> ScannedFile {
        ScannedFile {
            path: path.to_string(),
            run_path: run_path.to_string(),
            source: source.to_string(),
        }
    }

    /// A companion at line 2 and a matched `compile_fail` at line 7.
    const OK_SRC: &str = "\n/// ```\n/// # use foo::Bar;\n/// let _ = Bar;\n/// ```\n///\n/// ```compile_fail\n/// # use foo::Bar;\n/// let _: i32 = Bar;\n/// ```\npub struct A;\n";

    #[test]
    fn a_tree_matching_its_run_has_no_problems() {
        let out =
            "test a.rs - a::A (line 2) ... ok\ntest a.rs - a::A (line 7) - compile fail ... ok\n";
        let v = problems(&[file("a.rs", "a.rs", OK_SRC)], out);
        assert!(v.is_empty(), "{v:?}");
    }

    #[test]
    fn a_fence_in_the_tree_but_not_in_the_run_fails() {
        // Every shrink vector lands here: cfg gate, unknown info string, a crate
        // out of reach, a crate with no lib target. Whatever the cause, the proof
        // was never evaluated.
        let out = "test a.rs - a::A (line 2) ... ok\n";
        let v = problems(&[file("a.rs", "a.rs", OK_SRC)], out);
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].kind, Kind::NotRun);
        assert_eq!(v[0].line, 7);
    }

    #[test]
    fn a_run_entry_matching_no_scanned_fence_fails() {
        // The gate shrinking its OWN population — principle 6 turned inward.
        let out = "test a.rs - a::A (line 2) ... ok\ntest a.rs - a::A (line 7) ... ok\ntest a.rs - a::A (line 99) ... ok\n";
        let v = problems(&[file("a.rs", "a.rs", OK_SRC)], out);
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].kind, Kind::Orphan);
        assert_eq!(v[0].line, 99);
    }

    #[test]
    fn a_text_fence_must_not_appear_in_the_run() {
        let src = "\n/// ```text\n/// prose\n/// ```\npub struct A;\n";
        let v = problems(
            &[file("a.rs", "a.rs", src)],
            "test a.rs - a::A (line 2) ... ok\n",
        );
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].kind, Kind::Orphan);
    }

    #[test]
    fn an_ignored_run_entry_does_not_count_as_run() {
        // `ignore` blocks ARE reported by libtest, so presence alone is not proof
        // the assertion was evaluated — which is why `ignore` is banned outright.
        let src = "\n/// ```ignore\n/// let x = 1;\n/// ```\npub struct A;\n";
        let v = problems(
            &[file("a.rs", "a.rs", src)],
            "test a.rs - a::A (line 2) ... ignored\n",
        );
        let ks: Vec<_> = v.iter().map(|x| x.kind).collect();
        assert!(ks.contains(&Kind::BannedAttribute), "{ks:?}");
        assert!(ks.contains(&Kind::NotRun), "{ks:?}");
    }

    #[test]
    fn a_failed_doctest_is_named_as_failed_not_as_unrun() {
        // Folding a failure into NotRun ("never evaluated") would be a misleading
        // message for the commonest case.
        let out = "test a.rs - a::A (line 2) ... ok\ntest a.rs - a::A (line 7) ... FAILED\n";
        let v = problems(&[file("a.rs", "a.rs", OK_SRC)], out);
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].kind, Kind::Failed);
        assert_eq!(v[0].line, 7);
    }

    #[test]
    fn the_run_path_is_used_for_matching_and_the_repo_path_for_reporting() {
        // A `--manifest-path xtask/Cargo.toml` run prints `src/…`.
        let out = "test src/a.rs - a::A (line 2) ... ok\ntest src/a.rs - a::A (line 7) - compile fail ... ok\n";
        let v = problems(&[file("xtask/src/a.rs", "src/a.rs", OK_SRC)], out);
        assert!(v.is_empty(), "{v:?}");
    }

    #[test]
    fn a_failure_is_reported_against_the_repo_relative_path() {
        let out = "test src/a.rs - a::A (line 2) ... ok\n";
        let v = problems(&[file("xtask/src/a.rs", "src/a.rs", OK_SRC)], out);
        assert_eq!(v[0].file, "xtask/src/a.rs");
    }

    #[test]
    fn an_unparseable_file_is_a_violation_not_a_skip() {
        let v = problems(&[file("a.rs", "a.rs", "fn f( {")], "");
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].kind, Kind::Unreadable);
    }

    #[test]
    fn entries_from_files_this_half_did_not_scan_are_left_alone() {
        // The workspace producer and the host step each reconcile their own roots;
        // neither may claim the other's entries are orphans.
        let out = "test a.rs - a::A (line 2) ... ok\ntest a.rs - a::A (line 7) - compile fail ... ok\ntest elsewhere/b.rs - b::B (line 3) ... ok\n";
        let v = problems(&[file("a.rs", "a.rs", OK_SRC)], out);
        assert!(v.is_empty(), "{v:?}");
    }
}
