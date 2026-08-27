//! Reads every rustdoc code fence out of a Rust source file, keyed the way the
//! doctest runner reports it.
//!
//! **Why `syn` and not a line scan.** ADR-0085 principle 5 forbids a line-based
//! scan for an invariant spanning more than one line, and the companion rule is
//! scoped to a *doc comment* — a multi-line syntactic unit whose boundaries a line
//! scan has to guess at. Parsing also makes `#[doc = "…"]` attributes visible,
//! which a scan for `///` cannot see.
//!
//! **The key.** A `///` line desugars to one `#[doc]` attribute per source line,
//! each carrying its own span, so the line a fence opens on is read directly and
//! matches libtest's `(line N)` exactly. Verified against this tree: libtest
//! reports `common/src/token.rs - token::RawToken (line 56)` for the fence opening
//! at `token.rs:56`.

use syn::visit::Visit;

/// One rustdoc code fence, keyed the way the doctest runner reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fence {
    /// 1-based source line the opening ``` sits on — the runner's key.
    pub line: usize,
    /// The info string after the opening backticks, trimmed.
    pub info: String,
    /// Body lines rustdoc hides (`# `-prefixed), stored with the `# ` stripped so
    /// a hidden line and a visible line carrying the same code compare equal.
    pub hidden: Vec<String>,
    /// Body lines rustdoc shows, likewise trimmed.
    pub visible: Vec<String>,
    /// Index of the doc comment this fence belongs to. Fences sharing a value sit
    /// in one doc comment — the scope of the companion rule.
    pub doc_block: usize,
}

/// Everything the scanner read out of one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    pub fences: Vec<Fence>,
    /// 1-based lines of `#[doc = "…"]` attributes whose value spans several
    /// markdown lines.
    ///
    /// Rejected rather than scanned: libtest keys a fence inside one of these by
    /// the attribute's line plus a markdown-relative offset, not by any line a
    /// fence opens on, so the reconciliation key cannot address it. A *single*-line
    /// `#[doc = "…"]` is indistinguishable from `///` and keys correctly, so it is
    /// allowed — the discriminator is a newline in the value.
    pub multiline_doc_attrs: Vec<usize>,
}

/// One `#[doc]` attribute's text and the source line it sits on.
struct DocLine {
    line: usize,
    text: String,
}

/// Collects the `#[doc]` attribute runs of every item in a file, in source order.
#[derive(Default)]
struct Collector {
    /// One entry per doc comment: its attribute lines.
    blocks: Vec<Vec<DocLine>>,
    multiline_doc_attrs: Vec<usize>,
}

impl Collector {
    /// Record one item's doc attributes as a single doc comment.
    fn take(&mut self, attrs: &[syn::Attribute]) {
        let mut block = Vec::new();
        for attr in attrs {
            if !attr.path().is_ident("doc") {
                continue;
            }
            let syn::Meta::NameValue(nv) = &attr.meta else {
                continue;
            };
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
            else {
                continue;
            };
            let line = s.span().start().line;
            let text = s.value();
            if text.contains('\n') {
                self.multiline_doc_attrs.push(line);
                continue;
            }
            block.push(DocLine { line, text });
        }
        if !block.is_empty() {
            self.blocks.push(block);
        }
    }
}

/// Visit every item form that can carry doc comments. Each arm records the item's
/// own attributes and then recurses, so a fence nested in a module or an impl is
/// found rather than silently outside the population.
macro_rules! visit_attrs {
    ($($method:ident : $ty:ty => $free:path),* $(,)?) => {
        $(
            fn $method(&mut self, i: &'ast $ty) {
                self.take(&i.attrs);
                $free(self, i);
            }
        )*
    };
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_file(&mut self, i: &'ast syn::File) {
        self.take(&i.attrs);
        syn::visit::visit_file(self, i);
    }

    visit_attrs! {
        visit_item_mod: syn::ItemMod => syn::visit::visit_item_mod,
        visit_item_fn: syn::ItemFn => syn::visit::visit_item_fn,
        visit_item_struct: syn::ItemStruct => syn::visit::visit_item_struct,
        visit_item_enum: syn::ItemEnum => syn::visit::visit_item_enum,
        visit_item_trait: syn::ItemTrait => syn::visit::visit_item_trait,
        visit_item_type: syn::ItemType => syn::visit::visit_item_type,
        visit_item_const: syn::ItemConst => syn::visit::visit_item_const,
        visit_item_static: syn::ItemStatic => syn::visit::visit_item_static,
        visit_item_union: syn::ItemUnion => syn::visit::visit_item_union,
        visit_item_macro: syn::ItemMacro => syn::visit::visit_item_macro,
        visit_item_impl: syn::ItemImpl => syn::visit::visit_item_impl,
        visit_item_use: syn::ItemUse => syn::visit::visit_item_use,
        visit_item_extern_crate: syn::ItemExternCrate => syn::visit::visit_item_extern_crate,
        visit_impl_item_fn: syn::ImplItemFn => syn::visit::visit_impl_item_fn,
        visit_impl_item_const: syn::ImplItemConst => syn::visit::visit_impl_item_const,
        visit_impl_item_type: syn::ImplItemType => syn::visit::visit_impl_item_type,
        visit_trait_item_fn: syn::TraitItemFn => syn::visit::visit_trait_item_fn,
        visit_trait_item_const: syn::TraitItemConst => syn::visit::visit_trait_item_const,
        visit_trait_item_type: syn::TraitItemType => syn::visit::visit_trait_item_type,
        visit_field: syn::Field => syn::visit::visit_field,
        visit_variant: syn::Variant => syn::visit::visit_variant,
        visit_foreign_item_fn: syn::ForeignItemFn => syn::visit::visit_foreign_item_fn,
        visit_item_trait_alias: syn::ItemTraitAlias => syn::visit::visit_item_trait_alias,
        visit_item_foreign_mod: syn::ItemForeignMod => syn::visit::visit_item_foreign_mod,
        visit_foreign_item_static: syn::ForeignItemStatic => syn::visit::visit_foreign_item_static,
        visit_foreign_item_type: syn::ForeignItemType => syn::visit::visit_foreign_item_type,
    }
}

/// Strip rustdoc's one leading space from a doc-comment line's text.
fn undent(text: &str) -> &str {
    text.strip_prefix(' ').unwrap_or(text)
}

/// The `# `-hidden body line's code, or `None` when the line is visible.
///
/// `#` alone is a hidden empty line; `##` is an escaped literal `#`, not a hidden
/// line, so it stays visible (with the escape removed, as rustdoc renders it).
///
/// **Takes the trimmed line, not the undented one.** rustdoc's `map_line` trims
/// before testing for `# `, so `///   # use a::B;` IS hidden — and a scanner that
/// tested the merely-undented text would call it visible, leaving a real,
/// compiled fixture line outside the set the companion rule has to match. That
/// would be a silent hole in the one property this gate exists to guarantee.
fn hidden_code(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix('#')?;
    if rest.is_empty() {
        return Some("");
    }
    if rest.starts_with('#') {
        return None;
    }
    rest.strip_prefix(' ').map(str::trim_end)
}

/// A visible body line as rustdoc renders it: a leading `##` is an escape for a
/// literal `#`, so one `#` is dropped.
///
/// Only for a leading `##`. An attribute line like `#[derive(Clone)]` starts with
/// `#` but is neither hidden nor escaped, and stripping it would silently rewrite
/// a companion's fixture line so it no longer matched the negatives' hidden copy
/// of the same line.
fn visible_code(trimmed: &str) -> &str {
    if trimmed.starts_with("##") {
        &trimmed[1..]
    } else {
        trimmed
    }
}

/// The delimiter that opened a fenced code block.
struct OpenFence {
    fence: Fence,
    marker: char,
    marker_len: usize,
}

/// A CommonMark fence marker after at most three leading spaces.
fn marker(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 {
        return None;
    }

    let line = &line[indent..];
    let marker = line.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }

    let marker_len = line
        .bytes()
        .take_while(|byte| *byte == marker as u8)
        .count();
    if marker_len < 3 {
        return None;
    }

    Some((marker, marker_len, &line[marker_len..]))
}

/// A CommonMark fence opener. Backtick info strings cannot contain backticks.
fn opener(line: &str) -> Option<(char, usize, &str)> {
    let (marker, marker_len, info) = marker(line)?;
    (marker != '`' || !info.contains('`')).then_some((marker, marker_len, info))
}

/// Whether `line` is a valid closer for a given opener.
fn is_closer(line: &str, opener: &OpenFence) -> bool {
    let Some((marker, marker_len, suffix)) = marker(line) else {
        return false;
    };

    marker == opener.marker
        && marker_len >= opener.marker_len
        && suffix.bytes().all(|byte| byte == b' ' || byte == b'\t')
}

/// Every fence in `source`, or a message describing why it is not readable Rust.
///
/// A file that will not parse is **not** silently skipped: an unparsed file is a
/// file the gate cannot see, and a gate that quietly shrinks its own population is
/// the failure this design exists to prevent (ADR-0085 principle 6).
pub fn fences(source: &str) -> Result<Scan, String> {
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut collector = Collector::default();
    collector.visit_file(&file);

    let mut scan = Scan {
        fences: Vec::new(),
        multiline_doc_attrs: collector.multiline_doc_attrs,
    };
    scan.multiline_doc_attrs.sort_unstable();
    scan.multiline_doc_attrs.dedup();

    for (doc_block, block) in collector.blocks.iter().enumerate() {
        let mut open: Option<OpenFence> = None;
        for doc in block {
            let line = undent(&doc.text);
            if open.as_ref().is_some_and(|fence| is_closer(line, fence)) {
                if let Some(fence) = open.take() {
                    scan.fences.push(fence.fence);
                }
                continue;
            }

            if let Some(fence) = open.as_mut() {
                let trimmed = line.trim();
                match hidden_code(trimmed) {
                    Some(code) => fence.fence.hidden.push(code.to_string()),
                    None => fence
                        .fence
                        .visible
                        .push(visible_code(trimmed).trim().to_string()),
                }
            } else if let Some((marker, marker_len, info)) = opener(line) {
                open = Some(OpenFence {
                    fence: Fence {
                        line: doc.line,
                        info: info.trim().to_string(),
                        hidden: Vec::new(),
                        visible: Vec::new(),
                        doc_block,
                    },
                    marker,
                    marker_len,
                });
            }
        }
        // An unterminated fence still names a real block; keep it so the vocabulary
        // and companion rules can speak about it rather than dropping it.
        if let Some(fence) = open.take() {
            scan.fences.push(fence.fence);
        }
    }
    scan.fences.sort_by_key(|f| f.line);
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(src: &str) -> Scan {
        fences(src).expect("parses")
    }

    #[test]
    fn a_doc_comment_fence_keys_to_its_opening_line() {
        // `///` desugars to one `#[doc]` attribute per source line, so the
        // opener's span IS the line libtest prints. Probed 2026-08-01.
        let src = "\n/// Docs.\n///\n/// ```compile_fail\n/// let _: i32 = \"x\";\n/// ```\npub struct A;\n";
        let f = &scan(src).fences[0];
        assert_eq!(f.line, 4);
        assert_eq!(f.info, "compile_fail");
    }

    #[test]
    fn module_docs_are_scanned_too() {
        let src = "//! Module.\n//!\n//! ```\n//! let x = 1;\n//! ```\n";
        let f = &scan(src).fences[0];
        assert_eq!(f.line, 3);
        assert_eq!(f.info, "");
    }

    #[test]
    fn hidden_lines_are_separated_and_stripped() {
        let src = "\n/// ```compile_fail\n/// # use foo::Bar;\n/// # let b = Bar;\n/// let _ = b.nope();\n/// ```\npub struct A;\n";
        let f = &scan(src).fences[0];
        assert_eq!(f.hidden, vec!["use foo::Bar;", "let b = Bar;"]);
        assert_eq!(f.visible, vec!["let _ = b.nope();"]);
    }

    #[test]
    fn an_indented_hidden_line_is_still_hidden() {
        // rustdoc's `map_line` trims before testing for `# `, so this line IS part
        // of the compiled fixture. A scanner that tested the merely-undented text
        // would call it visible and leave a real prelude line outside the set the
        // companion rule must match — a silent hole in the whole guarantee.
        let src = "\n/// ```compile_fail\n///   # use foo::Bar;\n/// let _ = Bar;\n/// ```\npub struct A;\n";
        let f = &scan(src).fences[0];
        assert_eq!(f.hidden, vec!["use foo::Bar;"]);
        assert_eq!(f.visible, vec!["let _ = Bar;"]);
    }

    #[test]
    fn a_bare_hash_is_a_hidden_empty_line_not_a_prelude_line() {
        let src = "\n/// ```compile_fail\n/// #\n/// let _ = nope();\n/// ```\npub struct A;\n";
        let f = &scan(src).fences[0];
        assert_eq!(f.hidden, vec![""]);
    }

    #[test]
    fn a_double_hash_is_an_escaped_literal_not_a_hidden_line() {
        let src = "\n/// ```\n/// ## not hidden\n/// ```\npub struct A;\n";
        let f = &scan(src).fences[0];
        assert!(f.hidden.is_empty(), "{:?}", f.hidden);
        assert_eq!(f.visible, vec!["# not hidden"]);
    }

    #[test]
    fn an_attribute_line_is_not_treated_as_an_escape() {
        // `#[derive(…)]` starts with `#` but is neither hidden nor escaped.
        // Stripping it would rewrite a companion's fixture line so it stopped
        // matching the negatives' hidden copy of the very same line — which is
        // exactly how this regressed once.
        let src = "\n/// ```\n/// #[derive(Clone)]\n/// struct S;\n/// ```\npub struct A;\n";
        let f = &scan(src).fences[0];
        assert!(f.hidden.is_empty(), "{:?}", f.hidden);
        assert_eq!(f.visible, vec!["#[derive(Clone)]", "struct S;"]);
    }

    #[test]
    fn a_tilde_fence_is_recognized_as_a_fence() {
        // pulldown-cmark honours `~~~`, so a `~~~text` block would otherwise be
        // invisible in BOTH directions: uncollected by rustdoc and unseen here,
        // never entering the population the vocabulary rule speaks about.
        let src = "\n/// ~~~compile_fail\n/// # use foo::Bar;\n/// let _ = Bar;\n/// ~~~\npub struct A;\n";
        let s = scan(src);
        assert_eq!(s.fences.len(), 1, "{:?}", s.fences);
        assert_eq!(s.fences[0].info, "compile_fail");
    }

    #[test]
    fn fences_in_one_doc_comment_share_a_doc_block() {
        let src = "\n/// ```\n/// let a = 1;\n/// ```\n///\n/// ```compile_fail\n/// let _: i32 = \"x\";\n/// ```\npub struct A;\n\n/// ```\n/// let b = 2;\n/// ```\npub struct B;\n";
        let s = scan(src);
        assert_eq!(s.fences.len(), 3);
        assert_eq!(s.fences[0].doc_block, s.fences[1].doc_block);
        assert_ne!(s.fences[1].doc_block, s.fences[2].doc_block);
    }

    #[test]
    fn fences_inside_nested_items_are_found() {
        let src = "mod m {\n    /// ```\n    /// let x = 1;\n    /// ```\n    pub fn f() {}\n}\n";
        assert_eq!(scan(src).fences.len(), 1);
    }

    #[test]
    fn two_marker_lines_do_not_open_fences() {
        let src = "\n/// ``\n/// let x = 1;\n/// ~~\npub struct A;\n";
        assert!(scan(src).fences.is_empty());
    }

    #[test]
    fn three_spaces_indent_an_opener_but_four_do_not() {
        let src = "\n///    ```\n///    let x = 1;\n///    ```\npub struct A;\n\n///     ```\n/// let y = 2;\n///     ```\npub struct B;\n";
        let s = scan(src);
        assert_eq!(s.fences.len(), 1, "{:?}", s.fences);
        assert_eq!(s.fences[0].line, 2);
        assert_eq!(s.fences[0].visible, vec!["let x = 1;"]);
    }

    #[test]
    fn only_a_long_enough_matching_closer_ends_a_fence() {
        let src =
            "\n/// ``````\n/// `````\n/// ~~~~~~\n/// let x = 1;\n/// ```````\npub struct A;\n";
        let f = &scan(src).fences[0];
        assert_eq!(f.visible, vec!["`````", "~~~~~~", "let x = 1;"]);
    }

    #[test]
    fn trailing_text_does_not_close_but_tab_suffix_does() {
        let src = "\n/// ```\n/// ``` not a closer\n/// let x = 1;\n/// ```\t\n/// after\npub struct A;\n";
        let f = &scan(src).fences[0];
        assert_eq!(f.visible, vec!["``` not a closer", "let x = 1;"]);
    }

    #[test]
    fn backtick_info_with_a_backtick_does_not_open_a_fence() {
        let src = "\n/// ```rust`bad\n/// let x = 1;\npub struct A;\n";
        assert!(scan(src).fences.is_empty());
    }

    #[test]
    fn tilde_info_may_contain_backticks() {
        let src = "\n/// ~~~rust`example\n/// let x = 1;\n/// ~~~\npub struct A;\n";
        let f = &scan(src).fences[0];
        assert_eq!(f.info, "rust`example");
        assert_eq!(f.visible, vec!["let x = 1;"]);
    }

    #[test]
    fn nested_looking_markers_remain_content_until_a_valid_closer() {
        let src = "\n/// ~~~~~\n/// ```\n/// ~~~\n/// let x = 1;\n/// ~~~~~\npub struct A;\n";
        let f = &scan(src).fences[0];
        assert_eq!(f.visible, vec!["```", "~~~", "let x = 1;"]);
    }

    #[test]
    fn an_unclosed_fence_extends_to_the_doc_block_end() {
        let src = "\n/// ```compile_fail\n/// # use foo::Bar;\n/// let _ = Bar;\npub struct A;\n";
        let f = &scan(src).fences[0];
        assert_eq!(f.info, "compile_fail");
        assert_eq!(f.hidden, vec!["use foo::Bar;"]);
        assert_eq!(f.visible, vec!["let _ = Bar;"]);
    }

    #[test]
    fn a_multiline_doc_attribute_is_recorded_not_scanned() {
        // libtest keys a fence inside one of these by attribute-line + markdown
        // offset, which the reconciliation key cannot address.
        let src = "\n#[doc = \"Docs.\\n\\n```\\nlet x = 1;\\n```\\n\"]\npub struct A;\n";
        let s = scan(src);
        assert_eq!(s.multiline_doc_attrs, vec![2]);
        assert!(s.fences.is_empty());
    }

    #[test]
    fn a_single_line_doc_attribute_keys_like_a_slash_comment() {
        // Indistinguishable from `///` and keys correctly, so it is allowed.
        let src =
            "\n#[doc = \" ```\"]\n#[doc = \" let x = 1;\"]\n#[doc = \" ```\"]\npub struct A;\n";
        let s = scan(src);
        assert!(s.multiline_doc_attrs.is_empty());
        assert_eq!(s.fences[0].line, 2);
    }

    #[test]
    fn an_unparseable_file_is_an_error_not_an_empty_scan() {
        // A file the gate cannot read is a file the gate cannot police.
        let err = fences("fn f( {").expect_err("must not parse");
        assert!(err.contains("cannot parse as Rust"), "{err}");
    }
}
