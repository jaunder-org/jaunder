//! The `target-arch-placement` static check (#520): the host/wasm boundary must be drawn
//! at module wiring, never inside a leaf file. A leaf file is wholly one target, decided
//! by its `mod` declaration — that is ADR-0070's file-level split, now machine-enforced.
//!
//! Exactly three placements of a `target_arch` cfg are permitted:
//!
//! 1. an **inner** attribute on the file (`#![cfg(… target_arch …)]`), and only in
//!    `lib.rs` — the whole-crate gate `client` and `csr` use;
//! 2. an **outer** attribute on a `mod` or `use` item, and only in a `mod.rs`/`lib.rs`
//!    — the per-vertical `mod component;` gates and their paired re-exports; and
//! 3. nothing else — not on a `fn`/`struct`/`impl`/`macro_rules!`, and not on a
//!    statement or expression inside a body.
//!
//! Both halves of rule 2 are load-bearing. File-scope alone would permit a gated `fn` in
//! a `mod.rs`; item-scope alone would permit a gated `pub(crate) use` in a leaf file —
//! which is exactly how `web/src/reactive.rs` had drifted before this check existed.
//!
//! Implemented with `syn` rather than a line scan: the invariant is structural, and a
//! line scan cannot tell an attribute on an item from one inside a function body.
//! Recognition is anchored on the attribute **path** (`cfg`/`cfg_attr`), never on token
//! text — syn models `//!` and `///` as `#[doc = "…"]` attributes, and eight files in
//! `web/src` quote the gate in prose, so a token-text scan would flag every one of them.
//!
//! Unlike [`crate::coverage::exempt`], an unparseable file is a **hard failure** here,
//! not a fail-closed no-op: silence would disable the guard rather than over-report.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use syn::spanned::Spanned;

use crate::result::{CommandResult, StepResult};

/// Source roots scanned recursively for `.rs` files. `web` is the only crate whose
/// host/wasm boundary runs *through* it; `client` and `csr` pass trivially under rule 1
/// and are policed so they keep doing so.
const POLICED_ROOTS: &[&str] = &["web/src", "client/src", "csr/src"];

/// The rule, quoted in every violation so the fix is obvious from the failure alone.
const RULE: &str = "`target_arch` is permitted only on a `mod`/`use` item in a \
                    `mod.rs`/`lib.rs`, or as a crate-level `#![cfg(…)]` in `lib.rs` (#520)";

/// True for `#[cfg(…)]` / `#[cfg_attr(…)]` whose arguments mention `target_arch`.
///
/// The path anchor is load-bearing: a doc comment is a `#[doc = "…"]` attribute, so
/// matching on token text alone would flag prose that merely quotes the gate. Reading
/// `Meta::List`'s tokens as a string handles `any(…)` / `all(…)` / `not(…)` nesting
/// without hand-parsing the predicate grammar.
fn is_target_arch_cfg(attr: &syn::Attribute) -> bool {
    if !(attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr")) {
        return false;
    }
    match &attr.meta {
        syn::Meta::List(list) => list.tokens.to_string().contains("target_arch"),
        _ => false,
    }
}

/// The 1-based line an attribute starts on. Both the "all" and "permitted" sets use this
/// same anchor, so a multi-line attribute cancels consistently between them.
fn attr_line(attr: &syn::Attribute) -> u32 {
    attr.span().start().line as u32
}

/// Collects every `target_arch` cfg attribute in the file, wherever it appears —
/// including on statements and expressions inside function bodies.
struct AllVisitor<'a> {
    out: &'a mut BTreeSet<u32>,
}

impl<'ast> syn::visit::Visit<'ast> for AllVisitor<'_> {
    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        if is_target_arch_cfg(attr) {
            self.out.insert(attr_line(attr));
        }
        syn::visit::visit_attribute(self, attr);
    }
}

/// Add the `target_arch` cfg lines carried by `attrs`.
fn add_attr_lines(attrs: &[syn::Attribute], out: &mut BTreeSet<u32>) {
    for attr in attrs {
        if is_target_arch_cfg(attr) {
            out.insert(attr_line(attr));
        }
    }
}

/// Rule 2: the attribute lines of every `mod` / `use` item, recursing into inline
/// `mod x { … }` bodies so a nested wiring block is treated the same as the top level.
fn collect_wiring_permitted(items: &[syn::Item], out: &mut BTreeSet<u32>) {
    for item in items {
        match item {
            syn::Item::Mod(m) => {
                add_attr_lines(&m.attrs, out);
                if let Some((_, inner)) = &m.content {
                    collect_wiring_permitted(inner, out);
                }
            }
            syn::Item::Use(u) => add_attr_lines(&u.attrs, out),
            _ => {}
        }
    }
}

/// The sorted 1-based lines in `src` where a `target_arch` cfg sits somewhere the rule
/// does not allow. `file_name` is the file's terminal component (`"mod.rs"`, `"lib.rs"`,
/// `"reactive.rs"`), which is what selects rules 1 and 2.
///
/// Returns `Err` if the file cannot be parsed — the caller reports that as its own
/// failure rather than skipping the file.
///
/// Two accepted imprecisions, both requiring source rustfmt would never produce:
/// - Lines are the unit, so a permitted and a violating cfg **on the same line**
///   (`#[cfg(target_arch = "wasm32")] mod a; #[cfg(target_arch = "wasm32")] fn b() {}`)
///   cancel to a false negative. rustfmt puts each item on its own line.
/// - Recognition substring-matches `target_arch` inside a `cfg`'s tokens, so a
///   hypothetical `#[cfg(feature = "no_target_arch")]` would false-positive. No such
///   feature exists; the alternative is hand-parsing the predicate grammar.
pub fn violations(file_name: &str, src: &str) -> syn::Result<Vec<u32>> {
    let file = syn::parse_file(src)?;

    let mut all = BTreeSet::new();
    syn::visit::visit_file(&mut AllVisitor { out: &mut all }, &file);

    let mut permitted = BTreeSet::new();
    // Rule 1 — the crate-level inner gate, `lib.rs` only.
    if file_name == "lib.rs" {
        add_attr_lines(&file.attrs, &mut permitted);
    }
    // Rule 2 — `mod`/`use` items, wiring files only.
    if matches!(file_name, "lib.rs" | "mod.rs") {
        collect_wiring_permitted(&file.items, &mut permitted);
    }

    Ok(all.difference(&permitted).copied().collect())
}

/// The failure detail for every offending line across the scanned files, or `None` when
/// clean. Pure given the `(path, source)` pairs, so it is unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    for (path, source) in scanned {
        let file_name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        match violations(file_name, source) {
            Ok(found) => {
                for ln in found {
                    lines.push(format!("{path}:{ln}: {RULE}"));
                }
            }
            Err(e) => lines.push(format!(
                "{path}: cannot parse — the target_arch placement guard cannot verify \
                 this file: {e}"
            )),
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Collect every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Scan every Rust file under each of [`POLICED_ROOTS`] and push the result step. A
/// missing root is a hard failure, so a moved/renamed tree can never quietly disable the
/// guard.
pub fn run(result: &mut CommandResult) {
    let mut files = Vec::new();
    for root in POLICED_ROOTS {
        if let Err(e) = rust_files(Path::new(root), &mut files) {
            result.push(
                StepResult::fail("target-arch-placement")
                    .detail(format!("cannot scan {root}: {e}")),
            );
            return;
        }
    }
    // A file we cannot read is reported, not skipped: silently dropping it would
    // disable the guard for exactly the file someone made unreadable. The sibling
    // checks `filter_map(… .ok())` here; this one does not, because its whole premise
    // is that no policed file escapes inspection.
    let mut scanned: Vec<(String, String)> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();
    for p in &files {
        match std::fs::read_to_string(p) {
            Ok(s) => scanned.push((p.display().to_string(), s)),
            Err(e) => unreadable.push(format!("{}: cannot read — {e}", p.display())),
        }
    }
    let step = match (problems(&scanned), unreadable.is_empty()) {
        (None, true) => StepResult::ok("target-arch-placement"),
        (found, _) => {
            let mut detail = unreadable.join("\n");
            if let Some(problems) = found {
                if !detail.is_empty() {
                    detail.push('\n');
                }
                detail.push_str(&problems);
            }
            StepResult::fail("target-arch-placement").detail(detail)
        }
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::{problems, violations};

    #[test]
    fn lib_rs_crate_level_inner_attr_is_permitted() {
        // Form 1 — the whole-crate gate `client` and `csr` use.
        let src = "#![cfg(target_arch = \"wasm32\")]\npub mod storage;\n";
        assert!(violations("lib.rs", src).unwrap().is_empty());
    }

    #[test]
    fn inner_attr_outside_lib_rs_is_flagged() {
        let src = "#![cfg(target_arch = \"wasm32\")]\nfn a() {}\n";
        assert_eq!(violations("mod.rs", src).unwrap(), vec![1]);
    }

    #[test]
    fn gated_mod_and_use_in_mod_rs_are_permitted() {
        // Form 2 — the real `auth/mod.rs` shape (attribute on its own line).
        let src = "#[cfg(target_arch = \"wasm32\")]\nmod component;\n\
                   #[cfg(target_arch = \"wasm32\")]\npub use component::{LoginPage};\n";
        assert!(violations("mod.rs", src).unwrap().is_empty());
    }

    #[test]
    fn cfg_any_wasm_or_test_on_a_mod_is_permitted() {
        // The real `feed_discovery/mod.rs:8` shape — `any(...)` still counts as form 2.
        let src = "#[cfg(any(target_arch = \"wasm32\", test))]\nmod labels;\n";
        assert!(violations("mod.rs", src).unwrap().is_empty());
    }

    #[test]
    fn gated_use_in_a_leaf_file_is_flagged() {
        // Item-scope alone would pass this; the file-scope half is what catches it.
        let src = "#[cfg(target_arch = \"wasm32\")]\npub(crate) use foo;\n";
        assert_eq!(violations("reactive.rs", src).unwrap(), vec![1]);
    }

    #[test]
    fn gated_fn_in_a_wiring_file_is_flagged() {
        // File-scope alone would pass this; the item-scope half is what catches it.
        let src = "#[cfg(target_arch = \"wasm32\")]\nfn helper() {}\n";
        assert_eq!(violations("mod.rs", src).unwrap(), vec![1]);
    }

    #[test]
    fn gated_macro_rules_is_flagged() {
        let src = "#[cfg(any(target_arch = \"wasm32\", test))]\nmacro_rules! m { () => {} }\n";
        assert_eq!(violations("reactive.rs", src).unwrap(), vec![1]);
    }

    #[test]
    fn cfg_on_a_statement_inside_a_body_is_flagged() {
        let src = "fn a() {\n    #[cfg(target_arch = \"wasm32\")]\n    let x = 1;\n}\n";
        assert_eq!(violations("mod.rs", src).unwrap(), vec![2]);
    }

    #[test]
    fn doc_comments_quoting_the_gate_are_not_flagged() {
        // syn models `//!` and `///` as `#[doc = "…"]` attributes, so a token-text
        // scan would flag both of these. Recognition is anchored on the `cfg` path.
        // These are the real `auth/mod.rs:9` and `media/component.rs:2` shapes, and
        // they are checked in NON-`lib.rs` files, where form 1 cannot mask the bug.
        let inner = "//! The UI is wasm-only (`#[cfg(target_arch = \"wasm32\")]`).\nmod api;\n";
        assert!(violations("mod.rs", inner).unwrap().is_empty());
        let outer = "/// Declared `#[cfg(target_arch = \"wasm32\")] mod component;`.\n\
                     pub fn f() {}\n";
        assert!(violations("component.rs", outer).unwrap().is_empty());
    }

    #[test]
    fn cfg_attr_carrying_target_arch_is_recognized() {
        let src = "#[cfg_attr(target_arch = \"wasm32\", allow(dead_code))]\nfn a() {}\n";
        assert_eq!(violations("mod.rs", src).unwrap(), vec![1]);
    }

    #[test]
    fn non_target_arch_cfgs_are_ignored() {
        // The check polices the host/wasm boundary only — `feature`/`test` gates on
        // any item are none of its business.
        let src = "#[cfg(feature = \"csr\")]\npub fn f() {}\n#[cfg(test)]\nmod t {}\n";
        assert!(violations("dom.rs", src).unwrap().is_empty());
    }

    #[test]
    fn pre_fix_reactive_shape_is_reported() {
        // The exact shape the `reactive` leaf split removed must be caught.
        let src = "#[cfg(any(target_arch = \"wasm32\", test))]\n\
                   macro_rules! invalidator_scope { () => {} }\n\
                   #[cfg(target_arch = \"wasm32\")]\n\
                   pub(crate) use invalidator_scope;\n";
        assert_eq!(violations("reactive.rs", src).unwrap(), vec![1, 3]);
    }

    #[test]
    fn unparseable_file_is_an_error_not_a_silent_pass() {
        assert!(violations("mod.rs", "fn (").is_err());
    }

    #[test]
    fn problems_reports_a_parse_failure_rather_than_passing_silently() {
        let detail =
            problems(&[("web/src/broken.rs".to_string(), "fn (".to_string())]).expect("a problem");
        assert!(detail.contains("web/src/broken.rs"));
        assert!(detail.contains("parse"));
    }

    #[test]
    fn problems_reports_path_line_and_the_rule() {
        let detail = problems(&[(
            "web/src/reactive.rs".to_string(),
            "#[cfg(target_arch = \"wasm32\")]\npub(crate) use foo;\n".to_string(),
        )])
        .expect("a problem");
        assert!(detail.contains("web/src/reactive.rs:1"));
        assert!(detail.contains("mod.rs"));
    }

    #[test]
    fn clean_tree_reports_none() {
        assert_eq!(
            problems(&[(
                "web/src/auth/mod.rs".to_string(),
                "#[cfg(target_arch = \"wasm32\")]\nmod component;\n".to_string()
            )]),
            None
        );
    }
}
