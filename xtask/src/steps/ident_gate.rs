//! The shared machinery behind the ident-keyed XSS gates — [`raw-html-door`],
//! [`html-sink`] and [`rendered-html-from-trusted`].
//!
//! Those three gates guard one invariant from three sides (mint trust, inherit
//! trust, spend trust at the DOM), and they were written three times. That is worse
//! than ordinary duplication: a fix to the test-code exemption or the macro-token
//! walk that lands in two copies out of three leaves a gate that still reports
//! green, for the wrong reason — the exact failure ADR-0085 was written about. So
//! the traversal lives here once, and a gate supplies only what is genuinely its
//! own: the roots it scans, the [`Population`] it recognises, its allowlist, and
//! the words it fails in.
//!
//! Two layers, because the three gates do not share the same allowlist model:
//!
//! - [`mentions`] is the **scan**: parse, track test-code depth and the enclosing
//!   fn stack, walk macro invocation tokens by hand, and ask the gate's
//!   [`Population`] whether each occurrence is a member. Every gate uses this.
//! - [`Gate`] is the whole **enumerating gate** on top of it: deny by default,
//!   allowlist entries scoped to a top-level fn *with a multiplicity*
//!   ([`Allowed`]), tree-wide reconciliation of every entry, and a [`Report`]
//!   supplying the prose. `raw-html-door` and `html-sink` are each one `Gate`;
//!   `rendered-html-from-trusted` uses [`mentions`] and [`run_scan`] with its own
//!   count-less allowlist (#778 tracks giving it multiplicities).
//!
//! **Unreadable classes inherent to this scan** (ADR-0085's honesty obligation;
//! each gate states the ones specific to *its* idents on top of these):
//!
//! 1. A `use … as` rename, or a re-export under another name, evades ident matching
//!    — `syn` has no name resolution.
//! 2. Tokens inside an *attribute* macro's argument list are not walked; only
//!    [`syn::Macro`] invocations are. Macro **expansions** are never seen either,
//!    which is deliberate: only author-written tokens are in the population.
//! 3. There is no call graph, so a member reached through a helper is attributed to
//!    the helper, not to the caller that supplied the untrusted value. The scan can
//!    detect; it cannot attribute.
//! 4. An allowlist entry is keyed by enclosing fn **name**, not by file, so a
//!    same-named fn in another file matches the entry. [`Gate::problems`]'s
//!    tree-wide multiplicity reconciliation catches the resulting count drift, but
//!    the per-file report will not name it as a shadow.
//!
//! A `syn` parse failure is a **hard error** everywhere (ADR-0085 principle 6): a
//! file we cannot walk could hide a member, and a gate that quietly shrinks its own
//! population reports green for the one reason it must never report green.
//!
//! [`raw-html-door`]: crate::steps::raw_html_door_check
//! [`html-sink`]: crate::steps::html_sink_check
//! [`rendered-html-from-trusted`]: crate::steps::rendered_html_from_trusted_check

use std::collections::HashMap;
use std::path::Path;

use crate::files;
use crate::result::{CommandResult, StepResult};

/// One occurrence of a population member: where it is, and what encloses it.
#[derive(Debug, Clone)]
pub struct Mention {
    /// 1-based source line.
    pub line: usize,
    /// Nearest enclosing fn name; empty at module scope.
    pub function: String,
    /// Whether that fn is top-level (`fn_stack.len() == 1`). Only a top-level fn can
    /// match an allowlist entry, so a nested fn shadowing an allowed name cannot
    /// borrow its exemption.
    pub top_level: bool,
}

/// What a gate counts as a member of its population — the one question the scan
/// cannot answer for itself (ADR-0085 principle 1: the population is read
/// structurally, from what the AST says, never from a pattern believed to
/// characterise violations).
///
/// All three hooks are required rather than defaulted: a gate must say what it does
/// *not* look at, because "I never implemented that hook" and "that construct is
/// outside my population" are the same silence otherwise.
pub trait Population {
    /// A bare ident in ordinary (non-macro) code, at any position — a call, a field,
    /// a path segment, a bare reference.
    fn ident(&self, id: &proc_macro2::Ident) -> bool;

    /// A path expression in ordinary code (`Type::assoc`, `.map(Type::assoc)`), for
    /// a gate whose membership depends on the path's *qualifier* and so cannot be
    /// decided from the leaf ident alone.
    ///
    /// A gate that matches on [`Population::ident`] must return `false` here: `syn`
    /// descends from a path into its segment idents, so answering `true` to both
    /// would record the same site twice.
    fn expr_path(&self, path: &syn::Path) -> bool;

    /// An ident inside a macro invocation's tokens. `trees[idx]` is `id`; the flat
    /// sibling stream is passed so a gate can read positional context (tokens carry
    /// no path structure, so `Type::assoc` reads as `Ident : : Ident`).
    fn macro_ident(
        &self,
        id: &proc_macro2::Ident,
        trees: &[proc_macro2::TokenTree],
        idx: usize,
    ) -> bool;
}

/// The population of a gate keyed purely by ident: an occurrence of any of these
/// names, in ordinary code or inside macro tokens, wherever it appears.
///
/// Matching the ident rather than a call shape is what keeps such a gate an
/// enumeration instead of a search for the spelling someone anticipated — a builder
/// call, a struct field and a bare reference are all inside the population rather
/// than silently outside it (ADR-0085 principle 3).
pub struct AnyOf(pub &'static [&'static str]);

impl Population for AnyOf {
    fn ident(&self, id: &proc_macro2::Ident) -> bool {
        self.0.iter().any(|name| id == *name)
    }

    /// Nothing: the segment idents of a path are reached by [`Population::ident`].
    fn expr_path(&self, _path: &syn::Path) -> bool {
        false
    }

    fn macro_ident(
        &self,
        id: &proc_macro2::Ident,
        _trees: &[proc_macro2::TokenTree],
        _idx: usize,
    ) -> bool {
        self.ident(id)
    }
}

/// Every **non-test** mention of `population` in the source, in line order. `Err` on
/// a `syn` parse failure (fail-loud). Pure given the source, so gates unit-test
/// through it directly.
pub fn mentions<P: Population>(source: &str, population: &P) -> Result<Vec<Mention>, String> {
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut scanner = Scanner {
        population,
        test_depth: 0,
        fn_stack: Vec::new(),
        hits: Vec::new(),
    };
    syn::visit::visit_file(&mut scanner, &file);
    scanner.hits.sort_by_key(|m| m.line);
    Ok(scanner.hits)
}

struct Scanner<'p, P: Population> {
    population: &'p P,
    /// >0 while inside a `#[cfg(test)]`/`#[test]` item — members there are exempt.
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

impl<P: Population> Scanner<'_, P> {
    /// Record a mention on `line`, unless it is test code.
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
    /// record every member. `syn` never parses these tokens, so nothing found here
    /// can duplicate a hit already found in the AST — and comments are not tokens,
    /// so prose mentioning a guarded name cannot false-positive.
    fn walk_macro_tokens(&mut self, tokens: &proc_macro2::TokenStream) {
        let population = self.population;
        let trees: Vec<proc_macro2::TokenTree> = tokens.clone().into_iter().collect();
        for (idx, tt) in trees.iter().enumerate() {
            match tt {
                proc_macro2::TokenTree::Group(g) => self.walk_macro_tokens(&g.stream()),
                proc_macro2::TokenTree::Ident(id) if population.macro_ident(id, &trees, idx) => {
                    self.record(id.span().start().line);
                }
                _ => {}
            }
        }
    }
}

impl<'ast, P: Population> syn::visit::Visit<'ast> for Scanner<'_, P> {
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

    /// A `use` declaration is the one construct outside every gate's population: it
    /// names something but reaches, mints and spends nothing — what it enables is
    /// its own ident occurrence, and that occurrence *is* in the population. So
    /// skipping the tree loses no site, and it keeps a door costing **one** allowlist
    /// entry rather than one for the door and one for the `use` that reached it. (An
    /// import with no call site cannot survive anyway: clippy denies an unused
    /// import.)
    fn visit_item_use(&mut self, _i: &'ast syn::ItemUse) {}

    fn visit_ident(&mut self, i: &'ast proc_macro2::Ident) {
        if self.population.ident(i) {
            self.record(i.span().start().line);
        }
    }

    fn visit_expr_path(&mut self, i: &'ast syn::ExprPath) {
        if self.population.expr_path(&i.path) {
            self.record(syn::spanned::Spanned::span(&i.path).start().line);
        }
        syn::visit::visit_expr_path(self, i);
    }

    /// `syn` stops at a macro invocation's boundary, so the tokens are walked by
    /// hand — the render layer is `html!`/`view!` bodies, so this is where the
    /// interesting sites live.
    fn visit_macro(&mut self, i: &'ast syn::Macro) {
        self.walk_macro_tokens(&i.tokens);
        syn::visit::visit_macro(self, i);
    }
}

/// A population member permitted in production code, keyed by its enclosing
/// **top-level** function plus how many identical sites that key covers.
///
/// **The count is load-bearing, not decoration.** A bare function-scoped exemption
/// is a region exemption in disguise: a second site added inside the allowed fn
/// would pass silently, which is the precise defect ADR-0085 principle 4 forbids
/// (and which #778 records against `rendered-html-from-trusted`'s `ALLOWED_FNS`).
/// Declaring the multiplicity means gaining one more is a mismatch and a failure.
pub struct Allowed {
    /// Enclosing top-level function name.
    pub function: &'static str,
    /// How many sites this entry covers, tree-wide.
    pub count: usize,
    /// Why the construct is legitimate there.
    pub reason: &'static str,
}

/// The mentions `allowlist` does not cover: everything outside an allowlisted
/// top-level fn, plus everything **beyond** an entry's declared multiplicity (the
/// later sites in line order, so the first `count` keep the exemption they were
/// written for).
pub fn unjustified(found: &[Mention], allowlist: &[Allowed]) -> Vec<(usize, String)> {
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

/// The prose a [`Gate`] fails in. A gate's diagnosis is most of its value — the
/// reader has to learn what they tripped and what to do instead — so the wording
/// stays with the gate rather than being generalised into something that fits every
/// gate and helps at none.
pub struct Report {
    /// What was found, as the sentence's subject: `` "`PreEscaped`" ``, "an
    /// unescaped-HTML sink". Rendered as `{subject} {where} {verdict}`.
    pub subject: &'static str,
    /// Why it fails, following the `in fn \`x\`` / `at module scope` phrase.
    pub verdict: &'static str,
    /// What the entries count, for the reconciliation line: "sink(s)",
    /// "raw door(s)".
    pub noun: &'static str,
    /// The instruction when an entry's count has fallen to zero: "The sink is gone —
    /// delete the entry."
    pub vanished: &'static str,
    /// The recovery paragraph, ending in the phrase that introduces the allowlist
    /// dump (conventionally "Currently exempt:").
    pub recovery: &'static str,
}

/// A complete enumerating gate: the population it reads structurally, the allowlist
/// that is the only way out, the roots it scans, and the words it fails in.
pub struct Gate<P: Population> {
    /// Step name in the xtask result (`"html-sink"`).
    pub step: &'static str,
    /// Source roots scanned recursively for `.rs` files. A missing root is a hard
    /// failure, so a moved or renamed tree can never quietly disable the guard.
    pub roots: &'static [&'static str],
    pub population: P,
    pub allowlist: &'static [Allowed],
    pub report: Report,
}

impl<P: Population> Gate<P> {
    /// 1-based `(line, enclosing-fn)` of every mention in one source that this
    /// gate's allowlist does not cover.
    ///
    /// Test-only: [`Gate::problems`] parses once and applies [`unjustified`] itself,
    /// so this is the single-source convenience the gates' unit tests assert
    /// through — and pairing the parse with the allowlist here means the two halves
    /// of the rule cannot drift apart per gate.
    #[cfg(test)]
    pub fn violations(&self, source: &str) -> Result<Vec<(usize, String)>, String> {
        Ok(unjustified(
            &mentions(source, &self.population)?,
            self.allowlist,
        ))
    }

    /// The failure detail for every offending mention across the scanned files, or
    /// `None` when the tree matches the allowlist exactly. A per-file parse failure
    /// is surfaced (never swallowed). Pure given the `(path, source)` pairs, so
    /// gates unit-test it directly.
    pub fn problems(&self, scanned: &[(String, String)]) -> Option<String> {
        let mut lines = Vec::new();
        let mut found: Vec<(String, Mention)> = Vec::new();
        for (path, source) in scanned {
            match mentions(source, &self.population) {
                Err(msg) => lines.push(format!(
                    "{path}: {msg} — an unparsed file is invisible to this gate, which is exactly \
                     the blind spot it exists to close. Fix the file or the parser; do not skip it."
                )),
                Ok(ms) => {
                    for (ln, enclosing) in unjustified(&ms, self.allowlist) {
                        let where_ = if enclosing.is_empty() {
                            "at module scope".to_string()
                        } else {
                            format!("in fn `{enclosing}`")
                        };
                        lines.push(format!(
                            "{path}:{ln}: {} {where_} {}",
                            self.report.subject, self.report.verdict
                        ));
                    }
                    found.extend(ms.into_iter().map(|m| (path.clone(), m)));
                }
            }
        }

        // Stale or drifted entries: an allowlist that stops tracking the tree has
        // silently become a region exemption. This is also what catches a *second*
        // file growing a same-named fn — the per-file pass would hand it the entry's
        // exemption, but the tree-wide total no longer matches.
        for e in self.allowlist {
            let seen = found
                .iter()
                .filter(|(_, m)| m.top_level && m.function == e.function)
                .count();
            if seen != e.count {
                lines.push(format!(
                    "fn `{}`: allowlist entry declares {} {}, the tree has {}. {}",
                    e.function,
                    e.count,
                    self.report.noun,
                    seen,
                    if seen == 0 {
                        self.report.vanished
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
        lines.push(self.report.recovery.to_string());
        for a in self.allowlist {
            lines.push(format!(
                "    - fn `{}` ×{}: {}",
                a.function, a.count, a.reason
            ));
        }
        Some(lines.join("\n"))
    }
}

/// Read every `.rs` file under each of `roots`, hand the `(path, source)` pairs to
/// `problems`, and push the resulting step.
///
/// A missing root, and a file that cannot be read, are both hard failures: a gate
/// that quietly shrinks its own population reports green for the one reason it must
/// never report green (ADR-0085 principle 6).
pub fn run_scan(
    result: &mut CommandResult,
    step: &'static str,
    roots: &'static [&'static str],
    problems: impl Fn(&[(String, String)]) -> Option<String>,
) {
    let mut files = Vec::new();
    for root in roots {
        match files::with_extension(Path::new(root), "rs") {
            Ok(found) => files.extend(found),
            Err(e) => {
                result.push(StepResult::fail(step).detail(format!("cannot scan {root}: {e}")));
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
        (true, None) => StepResult::ok(step),
        (_, prob) => {
            read_errors.extend(prob);
            StepResult::fail(step).detail(read_errors.join("\n"))
        }
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::{mentions, unjustified, Allowed, AnyOf, Mention};

    /// The multiplicity rule reads the allowlist it is handed; nothing about it is
    /// baked into any gate's const. This is the shared half of ADR-0085 principle 4,
    /// so it is tested here rather than three times over.
    fn at(line: usize, function: &str, top_level: bool) -> Mention {
        Mention {
            line,
            function: function.to_string(),
            top_level,
        }
    }

    const ONE_ALLOWED: &[Allowed] = &[Allowed {
        function: "allowed",
        count: 1,
        reason: "a test fixture",
    }];

    #[test]
    fn an_entry_covers_its_declared_count_and_no_more() {
        let found = vec![at(1, "allowed", true), at(2, "allowed", true)];
        // The FIRST site keeps the exemption it was written for; the second is the
        // silent absorption the count exists to refuse.
        assert_eq!(
            unjustified(&found, ONE_ALLOWED),
            vec![(2, "allowed".to_string())]
        );
    }

    #[test]
    fn a_zero_count_entry_exempts_nothing() {
        const RETIRED: &[Allowed] = &[Allowed {
            function: "allowed",
            count: 0,
            reason: "a hypothetical list from which the site has been retired",
        }];
        assert_eq!(unjustified(&[at(1, "allowed", true)], RETIRED).len(), 1);
    }

    /// A nested fn shadowing an allowed name must not borrow the entry's exemption —
    /// the allowlist is pinned to a *top-level* fn.
    #[test]
    fn a_non_top_level_fn_cannot_borrow_the_entry() {
        assert_eq!(
            unjustified(&[at(1, "allowed", false)], ONE_ALLOWED),
            vec![(1, "allowed".to_string())]
        );
    }

    #[test]
    fn an_unlisted_fn_is_never_covered() {
        assert_eq!(
            unjustified(&[at(1, "other", true)], ONE_ALLOWED),
            vec![(1, "other".to_string())]
        );
    }

    /// The scan reports mentions in line order regardless of traversal order, which
    /// is what makes "the first `count` keep the exemption" a statement about the
    /// source rather than about `syn`'s walk.
    #[test]
    fn mentions_come_back_in_line_order() {
        let src = "fn a() { GUARDED; }\nfn b() { let _ = m! { GUARDED }; }\nfn c() { GUARDED; }\n";
        let found = mentions(src, &AnyOf(&["GUARDED"])).unwrap();
        assert_eq!(
            found.iter().map(|m| m.line).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }
}
