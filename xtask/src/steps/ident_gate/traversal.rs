use std::collections::BTreeSet;

use super::resolution::{Membership, Resolver};

/// The result of scanning one source: its policed mentions, plus the line ranges
/// of the test code that was skipped.
///
/// The ranges exist for the orphan check alone. Test code is exempt without
/// markers, but a fixture may legitimately carry one anyway; without knowing where
/// the test regions are, such a marker looks exactly like an exemption for a site
/// that no longer exists.
#[derive(Debug, Default)]
pub(super) struct Scan {
    /// Non-test mentions, in line order.
    pub(super) mentions: Vec<Mention>,
    /// 1-based inclusive line ranges of test items.
    test_ranges: Vec<(usize, usize)>,
}

impl Scan {
    /// Whether `line` falls inside any test region.
    pub(super) fn in_test_code(&self, line: usize) -> bool {
        self.test_ranges
            .iter()
            .any(|(start, end)| (*start..=*end).contains(&line))
    }
}

/// One occurrence of a population member: where it is, and what encloses it.
#[derive(Debug, Clone)]
pub(super) struct Mention {
    /// 1-based source line.
    pub(super) line: usize,
    pub(super) context: MentionContext,
}

/// The source context attached to a population mention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MentionContext {
    /// No enclosing function or field owner.
    Module,
    /// Nearest enclosing fn name.
    Function(String),
}

impl MentionContext {
    #[cfg(test)]
    pub(super) fn legacy_label(&self) -> String {
        match self {
            Self::Module => String::new(),
            Self::Function(name) => name.clone(),
        }
    }
}

/// Every **non-test** mention of `population` in the source, in line order, plus
/// the line ranges of the test code that was skipped. `Err` on a `syn` parse
/// failure (fail-loud). Pure given the source, so gates unit-test through it.
/// `owner` opts into qualifier resolution (#790): the owner type's name paired with the
/// idents that can denote it. They travel as one argument because they are meaningless
/// apart — the set always contains the owner, and it is only consulted when an owner is
/// given. `None` polices the bare ident wherever it appears, which is the pre-#790
/// behaviour and what the two sibling gates keep.
pub(super) fn scan(
    source: &str,
    population: &[&str],
    owner: Option<(&str, &BTreeSet<String>)>,
) -> Result<Scan, String> {
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut scanner = Scanner {
        population,
        owner,
        resolver: Resolver::for_file(&file),
        impl_stack: Vec::new(),
        suppressed: std::collections::HashSet::new(),
        test_depth: 0,
        fn_stack: Vec::new(),
        hits: Vec::new(),
        test_ranges: Vec::new(),
    };
    syn::visit::visit_file(&mut scanner, &file);
    scanner.hits.sort_by_key(|m| m.line);
    Ok(Scan {
        mentions: scanner.hits,
        test_ranges: scanner.test_ranges,
    })
}

struct Scanner<'p> {
    population: &'p [&'p str],
    /// The gate's owner type and the idents that can denote it (#790). `None` leaves
    /// every occurrence in the population, which is the pre-#790 behaviour and the one
    /// the two sibling gates keep.
    ///
    /// One field carrying both, because they are meaningless apart: with no owner there
    /// is no alias set to hold.
    owner: Option<(&'p str, &'p BTreeSet<String>)>,
    /// Per-file `use` bindings and type definitions, for resolving a bare qualifier.
    resolver: Resolver,
    /// Self-types of the enclosing `impl` blocks; `None` for a non-path self-type.
    /// The last is the nearest, so `Self::` and definition sites read `last()`.
    impl_stack: Vec<Option<String>>,
    /// Spans of idents that resolution has determined belong to **another** type.
    ///
    /// Resolution suppresses; it never records. `visit_ident` stays the sole recorder,
    /// which is what keeps definition sites (a `fn` ident is not a `syn::Path`),
    /// method-call idents and macro tokens in the population, and what makes
    /// `owner: None` byte-identical to the pre-#790 scan — the set is simply empty.
    suppressed: std::collections::HashSet<(usize, usize)>,
    /// >0 while inside a `#[cfg(test)]`/`#[test]` item — members there are exempt.
    test_depth: usize,
    /// Names of the enclosing functions; the last is the nearest.
    fn_stack: Vec<String>,
    hits: Vec<Mention>,
    /// 1-based inclusive line ranges of the test items encountered.
    test_ranges: Vec<(usize, usize)>,
}

/// Whether an attribute list carries a test-enabling `#[cfg(test)]` (incl.
/// `cfg(all(test, …))` / `cfg(any(test, …))`). Pragmatic token scan: the attr is
/// `cfg`, its tokens mention `test`, and are not negated (`not(...)`). The
/// `not`-guard biases the rare `cfg(all(not(x), test))` toward being **scanned** (a
/// safe false-positive) rather than letting a production-only `cfg(not(test))` slip
/// through unscanned.
fn is_test_cfg(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| match &a.meta {
        syn::Meta::List(ml) if ml.path.is_ident("cfg") => {
            let toks = ml.tokens.to_string();
            toks.contains("test") && !toks.contains("not")
        }
        _ => false,
    })
}

/// Whether an attribute list carries a test-harness attribute (`#[test]`,
/// `#[tokio::test]`, `#[rstest]`). Belt-and-suspenders for a test fn that is not
/// wrapped in a `#[cfg(test)]` module.
fn has_test_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path()
            .segments
            .last()
            .is_some_and(|s| s.ident == "test" || s.ident == "rstest")
    })
}

impl Scanner<'_> {
    /// Whether `id` is one of the names this gate polices.
    fn is_member(&self, id: &proc_macro2::Ident) -> bool {
        self.population.iter().any(|name| id == *name)
    }

    /// Mark an ident as belonging to another type, so `visit_ident` skips it.
    ///
    /// Must be called **before** delegating to `syn::visit::visit_*`: `syn` visits a
    /// parent before its children, and that ordering is the whole reason one pass
    /// suffices.
    fn suppress(&mut self, id: &proc_macro2::Ident) {
        let at = id.span().start();
        self.suppressed.insert((at.line, at.column));
    }

    /// Whether resolution has ruled this occurrence out of the population.
    fn is_suppressed(&self, id: &proc_macro2::Ident) -> bool {
        let at = id.span().start();
        self.suppressed.contains(&(at.line, at.column))
    }

    /// Classify a definition site: `fn from_trusted` belongs to whichever `impl`
    /// encloses it.
    ///
    /// A definition in another type's `impl` is suppressed. The owner's own `impl`, and a
    /// free fn with no enclosing `impl`, are both left in the population — the first
    /// because it *is* the door, the second because there is nothing to rule out.
    fn suppress_foreign_definition(&mut self, ident: &proc_macro2::Ident) {
        let Some((_, owners)) = self.owner else {
            return;
        };
        if !self.is_member(ident) {
            return;
        }
        let enclosing = self.impl_stack.last().and_then(Option::as_deref);
        if enclosing.is_some_and(|ty| !owners.contains(ty)) {
            self.suppress(ident);
        }
    }

    /// Note that `item` is test code, so a marker inside it is never an orphan.
    fn record_test_range<T: syn::spanned::Spanned>(&mut self, item: &T) {
        let span = syn::spanned::Spanned::span(item);
        self.test_ranges.push((span.start().line, span.end().line));
    }

    /// Record a mention on `line`, unless it is test code.
    ///
    /// Marker lookup happens later, in [`super::marker_policy::classify`], because whether a marker
    /// covers a site depends on how many sites share its line.
    fn record(&mut self, line: usize) {
        if self.test_depth > 0 {
            return;
        }
        self.hits.push(Mention {
            line,
            context: self
                .fn_stack
                .last()
                .cloned()
                .map(MentionContext::Function)
                .unwrap_or(MentionContext::Module),
        });
    }

    /// Walk a macro invocation's tokens, recursing through nested `Group`s, and
    /// record every member. `syn` never parses these tokens, so nothing found here
    /// can duplicate a hit already found in the AST — and comments are not tokens,
    /// so prose mentioning a guarded name cannot false-positive.
    fn walk_macro_tokens(&mut self, tokens: &proc_macro2::TokenStream) {
        // Materialised rather than iterated lazily: membership needs only the ident
        // today, but the flat sibling stream is the seam a positional-context gate
        // would read (see the module doc), and that stays one `.enumerate()` away.
        let trees: Vec<proc_macro2::TokenTree> = tokens.clone().into_iter().collect();
        for tt in &trees {
            match tt {
                proc_macro2::TokenTree::Group(g) => self.walk_macro_tokens(&g.stream()),
                proc_macro2::TokenTree::Ident(id) if self.is_member(id) => {
                    self.record(id.span().start().line);
                }
                _ => {}
            }
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for Scanner<'_> {
    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        let test = is_test_cfg(&i.attrs);
        if test {
            self.record_test_range(i);
        }
        self.test_depth += usize::from(test);
        syn::visit::visit_item_mod(self, i);
        self.test_depth -= usize::from(test);
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        let test = is_test_cfg(&i.attrs);
        if test {
            self.record_test_range(i);
        }
        self.test_depth += usize::from(test);
        self.impl_stack
            .push(super::resolution::type_name(&i.self_ty).map(ToString::to_string));
        syn::visit::visit_item_impl(self, i);
        self.impl_stack.pop();
        self.test_depth -= usize::from(test);
    }

    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        let test = is_test_cfg(&i.attrs) || has_test_attr(&i.attrs);
        if test {
            self.record_test_range(i);
        }
        self.test_depth += usize::from(test);
        self.fn_stack.push(i.sig.ident.to_string());
        self.suppress_foreign_definition(&i.sig.ident);
        syn::visit::visit_item_fn(self, i);
        self.fn_stack.pop();
        self.test_depth -= usize::from(test);
    }

    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        let test = is_test_cfg(&i.attrs) || has_test_attr(&i.attrs);
        if test {
            self.record_test_range(i);
        }
        self.test_depth += usize::from(test);
        self.fn_stack.push(i.sig.ident.to_string());
        self.suppress_foreign_definition(&i.sig.ident);
        syn::visit::visit_impl_item_fn(self, i);
        self.fn_stack.pop();
        self.test_depth -= usize::from(test);
    }

    /// Resolve a policed path's qualifier and suppress it when it names another type.
    ///
    /// This is the only place a qualifier is read. It **suppresses, never records** — see
    /// [`Scanner::suppressed`] — and it does nothing at all when no owner is configured,
    /// which is what keeps the sibling gates on the pre-#790 behaviour.
    fn visit_path(&mut self, i: &'ast syn::Path) {
        if let Some((_, owners)) = self.owner
            && let Some(leaf) = i.segments.last().map(|s| &s.ident)
            && self.is_member(leaf)
        {
            let enclosing = self.impl_stack.last().and_then(Option::as_deref);
            if self.resolver.membership(i, owners, enclosing) == Membership::OtherType {
                let leaf = leaf.clone();
                self.suppress(&leaf);
            }
        }
        syn::visit::visit_path(self, i);
    }

    /// A `use` declaration is the one construct outside every gate's population: it
    /// names something but reaches, mints and spends nothing — what it enables is
    /// its own ident occurrence, and that occurrence *is* in the population. So
    /// skipping the tree loses no site, and it keeps a door costing **one** marker
    /// rather than one for the door and one for the `use` that reached it. (An
    /// import with no call site cannot survive anyway: clippy denies an unused
    /// import.)
    fn visit_item_use(&mut self, _i: &'ast syn::ItemUse) {}

    fn visit_ident(&mut self, i: &'ast proc_macro2::Ident) {
        if self.is_member(i) && !self.is_suppressed(i) {
            self.record(i.span().start().line);
        }
    }

    /// `syn` stops at a macro invocation's boundary, so the tokens are walked by
    /// hand — the render layer is `html!`/`view!` bodies, so this is where the
    /// interesting sites live.
    fn visit_macro(&mut self, i: &'ast syn::Macro) {
        self.walk_macro_tokens(&i.tokens);
        syn::visit::visit_macro(self, i);
    }
}

#[cfg(test)]
mod tests {
    use super::scan;

    /// The scan reports mentions in line order regardless of traversal order, which
    /// is what lets the marker rule be a statement about the source rather than
    /// about `syn`'s walk.
    #[test]
    fn mentions_come_back_in_line_order() {
        let src = "fn a() { GUARDED; }\nfn b() { let _ = m! { GUARDED }; }\nfn c() { GUARDED; }\n";
        let found = scan(src, &["GUARDED"], None).unwrap().mentions;
        assert_eq!(
            found.iter().map(|m| m.line).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }
}

/// Resolution wired through the walker (#790): the same rule as `resolver_tests`, but
/// reached the way a gate reaches it, so `impl_stack`, the suppression set and the
/// definition-site path are all exercised.
#[cfg(test)]
mod owned_scan_tests {
    use super::super::marker_policy::{Classified, Why, classify};
    use super::super::resolution::owner_aliases;
    use super::scan;

    const TOKEN: &str = "guard:allow";

    fn classified_owned(src: &str) -> Classified {
        let owners = owner_aliases(&[("t.rs".to_string(), src.to_string())], "Owner");
        let s = scan(src, &["from_trusted"], Some(("Owner", &owners))).unwrap();
        classify(src, &s, TOKEN)
    }

    fn classified_unowned(src: &str) -> Classified {
        let s = scan(src, &["from_trusted"], None).unwrap();
        classify(src, &s, TOKEN)
    }

    #[test]
    fn an_owner_qualified_site_still_needs_a_marker() {
        let c = classified_owned("fn a() { Owner::from_trusted(x); }\n");
        assert_eq!(c.unexempt.len(), 1);
    }

    #[test]
    fn another_types_site_needs_no_marker() {
        let c = classified_owned("struct ContentType;\nfn a() { ContentType::from_trusted(x); }\n");
        assert!(c.unexempt.is_empty(), "not this door: {:?}", c.unexempt);
        assert!(c.marked.is_empty(), "and it earns no census entry either");
    }

    #[test]
    fn self_in_the_owners_impl_needs_a_marker() {
        // Exercises `impl_stack`, which the resolver tests cannot reach.
        let c = classified_owned("impl Owner { fn f() { Self::from_trusted(x); } }\n");
        assert_eq!(c.unexempt.len(), 1);
    }

    #[test]
    fn self_in_another_impl_needs_no_marker() {
        let c = classified_owned("impl ContentType { fn f() { Self::from_trusted(x); } }\n");
        assert!(c.unexempt.is_empty(), "{:?}", c.unexempt);
    }

    #[test]
    fn the_owners_definition_site_needs_a_marker() {
        // A `fn` ident is not a Path: passes only because the fn visitors participate.
        let c = classified_owned("impl Owner { fn from_trusted(v: V) -> Self { v } }\n");
        assert_eq!(c.unexempt.len(), 1);
    }

    #[test]
    fn another_types_definition_site_needs_no_marker() {
        let c = classified_owned("impl ContentType { fn from_trusted(v: V) -> Self { v } }\n");
        assert!(c.unexempt.is_empty(), "{:?}", c.unexempt);
    }

    #[test]
    fn a_free_module_scope_definition_is_flagged() {
        let c = classified_owned("fn from_trusted(v: V) -> V { v }\n");
        assert_eq!(c.unexempt.len(), 1, "no impl, so no owner to rule out");
    }

    #[test]
    fn an_unqualified_call_is_flagged() {
        let c = classified_owned("fn a() { from_trusted(x); }\n");
        assert_eq!(c.unexempt.len(), 1);
    }

    #[test]
    fn an_unresolvable_qualifier_is_flagged() {
        let c = classified_owned("use foo::*;\nfn a() { Mystery::from_trusted(x); }\n");
        assert_eq!(c.unexempt.len(), 1);
    }

    #[test]
    fn another_types_site_in_a_macro_body_is_still_flagged() {
        // D4: macro bodies are not resolved, so they stay in the population.
        let c = classified_owned(
            "struct ContentType;\nfn a() { let _ = view! { ContentType::from_trusted(x) }; }\n",
        );
        assert_eq!(c.unexempt.len(), 1);
    }

    #[test]
    fn a_marker_over_a_now_ignored_site_is_an_orphan() {
        // The mirror of the marker deletions in production code.
        let c = classified_owned(
            "struct ContentType;\n// guard:allow stale\nfn a() { ContentType::from_trusted(x); }\n",
        );
        assert_eq!(c.orphans, vec![2]);
    }

    #[test]
    fn without_an_owner_every_site_is_in_the_population() {
        // AC1: the sibling gates' behaviour is unchanged.
        let c =
            classified_unowned("struct ContentType;\nfn a() { ContentType::from_trusted(x); }\n");
        assert_eq!(c.unexempt.len(), 1);
    }

    #[test]
    fn a_site_is_recorded_exactly_once() {
        // AC1a: resolution suppresses, never records. The count is what pins it —
        // double-recording on an unmarked line yields two `Unmarked` entries, not `Shared`.
        let c = classified_owned("fn a() { Owner::from_trusted(x); }\n");
        assert_eq!(c.unexempt.len(), 1, "recorded once, not once per hook");
        assert!(matches!(c.unexempt[0].why, Why::Unmarked));
    }

    /// The end-to-end form of the fail-open the standards review found on #790: the alias
    /// is declared inside an inline module in one file and imported in another. Before the
    /// harvest recursed into `mod`, `Doc` was absent from the owner set while the importing
    /// file still bound it — so a real door resolved as another type and was suppressed,
    /// with no marker owed and no census row.
    #[test]
    fn an_owner_alias_declared_inside_a_module_still_puts_a_site_in_the_population() {
        let reexport = (
            "a.rs".to_string(),
            "mod inner { pub use crate::render::Owner as Doc; }\n".to_string(),
        );
        let site_src = "use crate::a::inner::Doc;\nfn f() { Doc::from_trusted(x); }\n";
        let site = ("b.rs".to_string(), site_src.to_string());
        let owners = owner_aliases(&[reexport, site], "Owner");
        let s = scan(site_src, &["from_trusted"], Some(("Owner", &owners))).unwrap();
        assert_eq!(
            classify(site_src, &s, TOKEN).unexempt.len(),
            1,
            "a real door must not be suppressed by an unharvested nested alias"
        );
    }

    #[test]
    fn an_owner_alias_from_another_file_puts_a_site_in_the_population() {
        // D2's whole reason for existing: must fail if the harvest goes per-file.
        let reexport = (
            "a.rs".to_string(),
            "pub use crate::render::Owner as Doc;\n".to_string(),
        );
        let site_src = "use crate::a::Doc;\nfn f() { Doc::from_trusted(x); }\n";
        let site = ("b.rs".to_string(), site_src.to_string());
        let owners = owner_aliases(&[reexport, site], "Owner");
        let s = scan(site_src, &["from_trusted"], Some(("Owner", &owners))).unwrap();
        assert_eq!(classify(site_src, &s, TOKEN).unexempt.len(), 1);
    }
}
