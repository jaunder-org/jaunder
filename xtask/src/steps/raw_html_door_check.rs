//! The `raw-html-door` static check (#333): pins `PreEscaped` — maud's raw,
//! unescaped-markup constructor — to an enumerated allowlist of production sites.
//!
//! `web`'s render layer carries trusted HTML in one type, `web::html::Markup`, and
//! `Markup` is the only thing maud will splice into an `html!` without escaping it.
//! That makes the compiler, not a scanner, the thing stopping a hand-built string
//! from reaching the DOM — **except** at `PreEscaped`, which mints trust out of an
//! arbitrary `String`. It is the render layer's counterpart to
//! [`crate::steps::rendered_html_from_trusted_check`]'s `from_trusted`: one door
//! that *asserts* an invariant nothing checked.
//!
//! **Population** (read structurally, ADR-0085 principle 1): every `PreEscaped`
//! ident under [`POLICED_ROOTS`], in ordinary code **and inside macro token
//! streams** — the render layer is `html!` bodies, so a gate blind to macro tokens
//! would be blind to the whole layer. Every member fails unless an [`ALLOWLIST`]
//! entry names its enclosing top-level fn *and* the entry's declared multiplicity
//! still covers it, so a second door added inside an already-allowed fn is a
//! failure rather than a silent absorption (ADR-0085 principle 4).
//!
//! The one construct outside the population is a `use` declaration. An import names
//! the constructor but cannot mint anything — the mint is the call, which is its own
//! ident occurrence and is in the population. So excluding imports loses no site,
//! and it keeps `Markup::from_rendered_html` costing **one** entry rather than one
//! for the door and one for the `use` that reached it. (An import with no call site
//! cannot survive anyway: clippy denies an unused import.)
//!
//! Because the scan sees author-written macro **invocation** tokens and never
//! expansions, maud's own internal `PreEscaped` use inside the expanded `html!` is
//! invisible here and needs no exemption.
//!
//! Test/fixture code (anything under a `#[cfg(test)]` module/impl/fn, or a
//! `#[test]`/`#[rstest]` fn) is exempt — fixtures legitimately mint raw markup.
//!
//! **Unreadable classes** (ADR-0085's honesty obligation): (1) a `use maud::PreEscaped
//! as Raw` rename evades ident matching, as does a re-exported alias under any other
//! name — `syn` has no name resolution. (2) Tokens inside an *attribute* macro's
//! argument list are not walked, only `syn::Macro` invocations. (3) The allowlist is
//! keyed by enclosing fn name, not by file, so a same-named fn in another file
//! matches the entry — the tree-wide multiplicity reconciliation in [`problems`]
//! catches the resulting count drift, but the per-file report will not name it as a
//! shadow. All are as visible in review as editing the allowlist. A `syn` parse
//! failure is a **hard error** (a file we cannot walk could hide a door — a false
//! pass), matching [`crate::steps::rendered_html_from_trusted_check`].

use std::collections::HashMap;
use std::path::Path;

use crate::files;
use crate::result::{CommandResult, StepResult};

/// Source roots scanned recursively for `.rs` files — the same production `src`
/// trees [`crate::steps::rendered_html_from_trusted_check`] polices, not the
/// `tests/` integration crates (whose fixtures mint freely). The door only makes
/// sense in `web` today, but the population is defined by the tree, not by where we
/// expect the door to appear: a `PreEscaped` sprouting in `server` or `csr` must
/// fail rather than sit outside the gate's reach.
const POLICED_ROOTS: &[&str] = &[
    "common/src",
    "host/src",
    "storage/src",
    "web/src",
    "server/src",
    "csr/src",
    "macros/src",
];

/// The constructor ident this guard pins.
const DOOR: &str = "PreEscaped";

/// A raw door permitted in production code, keyed by its enclosing **top-level**
/// function plus how many identical sites that key covers.
///
/// **The count is load-bearing, not decoration.** A bare function-scoped exemption
/// is a region exemption in disguise: a second `PreEscaped` added inside the allowed
/// fn would pass silently, which is the precise defect ADR-0085 principle 4 forbids
/// (and which #778 records against `rendered-html-from-trusted`'s `ALLOWED_FNS`).
/// Declaring the multiplicity means gaining one more is a mismatch and a failure.
struct Allowed {
    /// Enclosing top-level function name.
    function: &'static str,
    /// How many `PreEscaped` sites this entry covers, tree-wide.
    count: usize,
    /// Why raw, unescaped markup is legitimate there.
    reason: &'static str,
}

/// Every production raw door, each with its reason.
///
/// One entry, and the design intends it to stay that way: `Markup` is the render
/// layer's trusted carrier and composes into `html!` unescaped by construction, so
/// ordinary markup never needs a door. Only a value whose safety was *established*
/// elsewhere does.
const ALLOWLIST: &[Allowed] = &[
    // `web/src/html.rs` — `Markup::from_rendered_html`, the crate's single raw door,
    // carrying the `// XSS SAFETY:` comment beside it. It re-wraps a
    // `common::render::RenderedHtml`, whose invariant `RenderedHtml::sanitize`
    // established by scrubbing (ADR-0079); this only inherits it, which is why the
    // value may be emitted unescaped. Everything else in `web` builds markup with
    // `html!`, where maud escapes text for us.
    Allowed {
        function: "from_rendered_html",
        count: 1,
        reason: "re-wraps a RenderedHtml whose safety sanitization established (ADR-0079)",
    },
];

/// One `PreEscaped` mention: where it is, and what encloses it.
#[derive(Debug, Clone)]
struct Mention {
    /// 1-based source line.
    line: usize,
    /// Nearest enclosing fn name; empty at module scope.
    function: String,
    /// Whether that fn is top-level (`fn_stack.len() == 1`). Only a top-level fn can
    /// match an allowlist entry, so a nested fn shadowing an allowed name cannot
    /// borrow its exemption.
    top_level: bool,
}

/// Every **non-test** `PreEscaped` mention in the source, in line order. `Err` on a
/// `syn` parse failure (fail-loud). Pure given the source, so it is unit-tested
/// directly.
fn mentions(source: &str) -> Result<Vec<Mention>, String> {
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut scanner = Scanner {
        test_depth: 0,
        fn_stack: Vec::new(),
        hits: Vec::new(),
    };
    syn::visit::visit_file(&mut scanner, &file);
    scanner.hits.sort_by_key(|m| m.line);
    Ok(scanner.hits)
}

/// 1-based `(line, enclosing-fn)` of every mention the real [`ALLOWLIST`] does not
/// cover. Test-only: [`problems`] parses once and applies [`unjustified`] itself, so
/// this is the single-source convenience the unit tests assert through.
#[cfg(test)]
fn violations(source: &str) -> Result<Vec<(usize, String)>, String> {
    Ok(unjustified(&mentions(source)?, ALLOWLIST))
}

/// [`violations`] against a caller-supplied allowlist. The allowlist is an argument
/// rather than a value baked into the matching path, so the multiplicity rule can be
/// exercised independently of whatever the real list happens to hold today.
#[cfg(test)]
fn violations_with(source: &str, allowlist: &[Allowed]) -> Result<Vec<(usize, String)>, String> {
    Ok(unjustified(&mentions(source)?, allowlist))
}

/// The mentions `allowlist` does not cover: everything outside an allowlisted
/// top-level fn, plus everything **beyond** an entry's declared multiplicity (the
/// later sites in line order, so the first `count` keep the exemption they were
/// written for).
fn unjustified(found: &[Mention], allowlist: &[Allowed]) -> Vec<(usize, String)> {
    let mut used: HashMap<&str, usize> = HashMap::new();
    let mut out = Vec::new();
    for m in found {
        let entry = m
            .top_level
            .then(|| allowlist.iter().find(|a| a.function == m.function))
            .flatten();
        match entry {
            Some(a) => {
                let seen = used.entry(a.function).or_insert(0);
                *seen += 1;
                if *seen > a.count {
                    out.push((m.line, m.function.clone()));
                }
            }
            None => out.push((m.line, m.function.clone())),
        }
    }
    out
}

struct Scanner {
    /// >0 while inside a `#[cfg(test)]`/`#[test]` item — doors there are exempt.
    test_depth: usize,
    /// Names of the enclosing functions; the last is the nearest.
    fn_stack: Vec<String>,
    hits: Vec<Mention>,
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

impl Scanner {
    /// Record a door mention on `line`, unless it is test code.
    ///
    /// Allowlisting happens later, in [`unjustified`], because a multiplicity can
    /// only be judged once every site is in hand.
    fn record(&mut self, line: usize) {
        if self.test_depth > 0 {
            return;
        }
        self.hits.push(Mention {
            line,
            function: self.fn_stack.last().cloned().unwrap_or_default(),
            top_level: self.fn_stack.len() == 1,
        });
    }

    /// Walk a macro invocation's tokens, recursing through nested `Group`s, and
    /// record every `PreEscaped` ident. `syn` never parses these tokens, so nothing
    /// found here can duplicate a hit already found in the AST — and comments are
    /// not tokens, so prose mentioning the door cannot false-positive.
    fn walk_macro_tokens(&mut self, tokens: &proc_macro2::TokenStream) {
        for tt in tokens.clone() {
            match tt {
                proc_macro2::TokenTree::Group(g) => self.walk_macro_tokens(&g.stream()),
                proc_macro2::TokenTree::Ident(id) if id == DOOR => {
                    self.record(id.span().start().line);
                }
                _ => {}
            }
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for Scanner {
    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        let test = is_test_cfg(&i.attrs);
        self.test_depth += usize::from(test);
        syn::visit::visit_item_mod(self, i);
        self.test_depth -= usize::from(test);
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        let test = is_test_cfg(&i.attrs);
        self.test_depth += usize::from(test);
        syn::visit::visit_item_impl(self, i);
        self.test_depth -= usize::from(test);
    }

    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        let test = is_test_cfg(&i.attrs) || has_test_attr(&i.attrs);
        self.test_depth += usize::from(test);
        self.fn_stack.push(i.sig.ident.to_string());
        syn::visit::visit_item_fn(self, i);
        self.fn_stack.pop();
        self.test_depth -= usize::from(test);
    }

    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        let test = is_test_cfg(&i.attrs) || has_test_attr(&i.attrs);
        self.test_depth += usize::from(test);
        self.fn_stack.push(i.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, i);
        self.fn_stack.pop();
        self.test_depth -= usize::from(test);
    }

    /// A `use` declaration is the one construct outside the population: it names the
    /// constructor but mints nothing, and the call it enables is its own ident
    /// occurrence. Skipping the tree entirely (rather than descending) is what keeps
    /// the door costing one allowlist entry instead of two.
    fn visit_item_use(&mut self, _i: &'ast syn::ItemUse) {}

    /// The population is otherwise the **ident**, wherever it appears — a call, a
    /// bare reference, a path segment. Matching the ident rather than a call shape is
    /// what keeps this an enumeration instead of a search for the spelling someone
    /// anticipated.
    fn visit_ident(&mut self, i: &'ast proc_macro2::Ident) {
        if i == DOOR {
            self.record(i.span().start().line);
        }
    }

    /// `syn` stops at a macro invocation's boundary, so the tokens are walked by
    /// hand — the render layer is `html!` bodies, so this is where the door lives.
    fn visit_macro(&mut self, i: &'ast syn::Macro) {
        self.walk_macro_tokens(&i.tokens);
        syn::visit::visit_macro(self, i);
    }
}

/// The failure detail for every offending mention across the scanned files, or
/// `None` when the tree matches the allowlist exactly. A per-file parse failure is
/// surfaced (never swallowed). Pure given the `(path, source)` pairs, so it is
/// unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    let mut found: Vec<(String, Mention)> = Vec::new();
    for (path, source) in scanned {
        match mentions(source) {
            Err(msg) => lines.push(format!(
                "{path}: {msg} — an unparsed file is invisible to this gate, which is exactly the \
                 blind spot it exists to close. Fix the file or the parser; do not skip it."
            )),
            Ok(ms) => {
                for (ln, enclosing) in unjustified(&ms, ALLOWLIST) {
                    let where_ = if enclosing.is_empty() {
                        "at module scope".to_string()
                    } else {
                        format!("in fn `{enclosing}`")
                    };
                    lines.push(format!(
                        "{path}:{ln}: `PreEscaped` {where_} is not an allowlisted raw-HTML door — \
                         markup minted here reaches the DOM unescaped (XSS) (#333)"
                    ));
                }
                found.extend(ms.into_iter().map(|m| (path.clone(), m)));
            }
        }
    }

    // Stale or drifted entries: an allowlist that stops tracking the tree has
    // silently become a region exemption. This is also what catches a *second* file
    // growing a same-named fn — the per-file pass would hand it the entry's
    // exemption, but the tree-wide total no longer matches.
    for e in ALLOWLIST {
        let seen = found
            .iter()
            .filter(|(_, m)| m.top_level && m.function == e.function)
            .count();
        if seen != e.count {
            lines.push(format!(
                "fn `{}`: allowlist entry declares {} raw door(s), the tree has {}. {}",
                e.function,
                e.count,
                seen,
                if seen == 0 {
                    "The door is gone — delete the entry."
                } else {
                    "Re-justify each site, then update the count."
                }
            ));
        }
    }

    if lines.is_empty() {
        return None;
    }
    lines.sort();
    lines.push(
        "  recovery: `PreEscaped` asserts trust rather than establishing it. Trusted markup \
         already has a carrier — build it with `html!` and wrap it in `Markup`, which composes \
         unescaped by construction and needs no door. The only value that legitimately needs the \
         raw door is a `RenderedHtml`, whose safety `RenderedHtml::sanitize` established; reach \
         it through `Markup::from_rendered_html`. If this really is a new door, add an ALLOWLIST \
         entry with its multiplicity and a written reason in \
         xtask/src/steps/raw_html_door_check.rs. Currently exempt:"
            .to_string(),
    );
    for a in ALLOWLIST {
        lines.push(format!(
            "    - fn `{}` ×{}: {}",
            a.function, a.count, a.reason
        ));
    }
    Some(lines.join("\n"))
}

/// Scan every Rust file under each [`POLICED_ROOTS`] and push the result step. A
/// missing root is a hard failure, so a moved/renamed tree can never quietly disable
/// the guard.
pub fn run(result: &mut CommandResult) {
    let mut files = Vec::new();
    for root in POLICED_ROOTS {
        match files::with_extension(Path::new(root), "rs") {
            Ok(found) => files.extend(found),
            Err(e) => {
                result.push(
                    StepResult::fail("raw-html-door").detail(format!("cannot scan {root}: {e}")),
                );
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
        (true, None) => StepResult::ok("raw-html-door"),
        (_, prob) => {
            read_errors.extend(prob);
            StepResult::fail("raw-html-door").detail(read_errors.join("\n"))
        }
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::{problems, violations, violations_with, Allowed};

    /// The crate's one door, as `web/src/html.rs` writes it — the `use` that reaches
    /// the constructor plus the single call. Both name `PreEscaped`; only the call is
    /// in the population, which is what makes ONE entry the right cost.
    const THE_DOOR: &str = r#"
        use maud::{PreEscaped, Render};

        impl Markup {
            pub fn from_rendered_html(html: &RenderedHtml) -> Self {
                // XSS SAFETY: inherited from sanitization (ADR-0079).
                Self(PreEscaped(html.as_ref()).into_string())
            }
        }
    "#;

    #[test]
    fn the_allowed_door_at_its_declared_multiplicity_passes() {
        assert_eq!(violations(THE_DOOR).unwrap(), vec![]);
    }

    /// The rule reads the allowlist it is handed; nothing about the multiplicity
    /// logic is baked into the const. Same source, a list that has retired the door,
    /// and the call becomes a violation.
    #[test]
    fn the_multiplicity_rule_follows_the_allowlist_it_is_given() {
        const NO_DOORS: &[Allowed] = &[Allowed {
            function: "from_rendered_html",
            count: 0,
            reason: "a hypothetical list from which the door has been retired",
        }];
        assert_eq!(violations_with(THE_DOOR, NO_DOORS).unwrap().len(), 1);
    }

    /// ADR-0085 principle 4: the entry is scoped to a site with a multiplicity, so a
    /// SECOND door inside the same allowed fn is a violation, not absorbed.
    #[test]
    fn a_second_door_inside_the_allowed_fn_is_a_violation() {
        let src = r#"
            impl Markup {
                pub fn from_rendered_html(html: &RenderedHtml) -> Self {
                    let _ = PreEscaped("<b>".to_string());
                    Self(PreEscaped(html.to_string()))
                }
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    /// A nested fn shadowing an allowed name must not borrow the entry's exemption —
    /// the allowlist is pinned to a *top-level* fn.
    #[test]
    fn a_nested_fn_shadowing_the_allowed_name_is_still_flagged() {
        let src = r#"
            fn outer(html: &RenderedHtml) -> Markup {
                fn from_rendered_html(html: &RenderedHtml) -> Markup {
                    Markup(PreEscaped(html.to_string()))
                }
                from_rendered_html(html)
            }
        "#;
        let hits = violations(src).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, "from_rendered_html");
    }

    /// The whole reason this gate exists: the render layer is macro bodies now.
    #[test]
    fn a_door_inside_an_html_macro_body_is_detected() {
        let src = r#"
            fn render_thing(s: &str) -> Markup {
                Markup::new(html! { div { (PreEscaped(s)) } })
            }
        "#;
        let hits = violations(src).unwrap();
        assert_eq!(hits.len(), 1, "macro body must be scanned: {hits:?}");
        assert_eq!(hits[0].1, "render_thing");
    }

    #[test]
    fn a_door_nested_deeper_in_macro_groups_is_detected() {
        let src = r#"
            fn render_thing(s: &str) -> Markup {
                Markup::new(html! { div { @if true { (PreEscaped(s)) } } })
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn a_mention_in_a_comment_is_not_a_token_and_does_not_trip() {
        let src = r#"
            /// See `PreEscaped` for the raw door.
            fn render_thing() -> Markup { Markup::empty() }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    /// An import names the constructor but mints nothing, so it is outside the
    /// population — the call it enables is its own occurrence and is not. Being
    /// inside would cost the one real door a second allowlist entry for its `use`.
    #[test]
    fn an_import_alone_is_outside_the_population() {
        let src = "use maud::PreEscaped;\n";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_door_in_a_cfg_test_module_is_exempt() {
        let src = r#"
            #[cfg(test)]
            mod tests {
                fn fixture() -> Markup {
                    Markup(PreEscaped("<p>x</p>".to_string()))
                }
            }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_door_in_a_test_fn_is_exempt() {
        let src = r#"
            #[test]
            fn t() {
                let _ = html! { (PreEscaped("<p>x</p>")) };
            }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_cfg_not_test_production_fn_is_scanned() {
        let src = r#"
            #[cfg(not(test))]
            fn prod_only(s: &str) -> Markup {
                Markup(PreEscaped(s.to_string()))
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn a_door_at_module_scope_is_flagged() {
        let src = "static X: () = { let _ = PreEscaped(\"<p>x</p>\"); };\n";
        let hits = violations(src).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, "");
    }

    #[test]
    fn unparseable_source_is_a_hard_error() {
        assert!(violations("fn broken( {").is_err());
    }

    /// The tree as it stands: one door, in the one fn the allowlist names.
    fn the_real_tree() -> Vec<(String, String)> {
        vec![("web/src/html.rs".to_string(), THE_DOOR.to_string())]
    }

    #[test]
    fn problems_is_none_for_the_one_declared_door() {
        assert_eq!(problems(&the_real_tree()), None);
    }

    /// The cross-file hole a fn-name key would otherwise leave: a *second* file grows
    /// a fn named `from_rendered_html` with a door, and the per-file pass hands it the
    /// entry's exemption. The tree-wide reconciliation is what refuses it.
    #[test]
    fn a_same_named_fn_in_another_file_breaks_the_declared_multiplicity() {
        let mut scanned = the_real_tree();
        scanned.push((
            "web/src/posts/render.rs".to_string(),
            "fn from_rendered_html(s: &str) -> Markup { Markup(PreEscaped(s)) }\n".to_string(),
        ));
        let detail = problems(&scanned).expect("a problem");
        assert!(
            detail.contains("declares 1 raw door(s), the tree has 2"),
            "{detail}"
        );
    }

    /// A door that disappears leaves a stale entry, which is an exemption nobody is
    /// re-justifying. Deleting the entry is part of deleting the door.
    #[test]
    fn a_vanished_door_leaves_a_stale_entry_that_fails() {
        let detail = problems(&[]).expect("a problem");
        assert!(
            detail.contains("The door is gone — delete the entry."),
            "{detail}"
        );
    }

    #[test]
    fn problems_reports_file_line_and_recovery() {
        let mut scanned = the_real_tree();
        scanned.push((
            "web/src/x.rs".to_string(),
            "fn sneaky(s: &str) -> Markup { Markup(PreEscaped(s.to_string())) }\n".to_string(),
        ));
        let detail = problems(&scanned).expect("a problem");
        assert!(detail.contains("web/src/x.rs:1"));
        assert!(detail.contains("not an allowlisted raw-HTML door"));
        assert!(detail.contains("ALLOWLIST"));
        assert!(detail.contains("fn `from_rendered_html` ×1"));
    }

    #[test]
    fn problems_surfaces_a_parse_failure_with_the_file() {
        let scanned = vec![("web/src/broken.rs".to_string(), "fn ( {{{".to_string())];
        let detail = problems(&scanned).expect("a hard error");
        assert!(detail.contains("web/src/broken.rs"));
        assert!(detail.contains("cannot parse"));
    }
}
