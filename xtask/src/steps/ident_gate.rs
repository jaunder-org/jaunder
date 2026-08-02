//! The shared machinery behind the ident-keyed XSS gates — [`raw-html-door`],
//! [`html-sink`] and [`rendered-html-from-trusted`].
//!
//! Those three gates guard one invariant from three sides (mint trust, inherit
//! trust, spend trust at the DOM), and they were written three times. That is worse
//! than ordinary duplication: a fix to the test-code exemption or the macro-token
//! walk that lands in two copies out of three leaves a gate that still reports
//! green, for the wrong reason — the exact failure ADR-0085 was written about. So
//! the traversal lives here once, and a gate supplies only what is genuinely its
//! own: the roots it scans, the [`Population`] it recognises, and the words it
//! fails in.
//!
//! Two layers:
//!
//! - [`scan`] is the **traversal**: parse, track test-code depth and the enclosing
//!   fn stack, walk macro invocation tokens by hand, and ask the gate's
//!   [`Population`] whether each occurrence is a member.
//! - [`Gate`] is the whole **enumerating gate** on top of it: deny by default,
//!   [`classify`] against the in-source markers, and a [`Report`] supplying the
//!   prose.
//!
//! **Exemptions are markers, not a list** (#778). A site is exempt when the line
//! *immediately above* it carries `// <gate-step>:allow <reason>`. The key is one
//! line, so it cannot absorb a second site the way a fn-keyed entry did; it moves
//! with the code under rename and refactor; and the exempt set is **derived** from
//! the tree rather than declared beside the rule, which removes the whole class of
//! staleness the old multiplicity reconciliation existed to detect.
//!
//! The position is not a matter of taste: a *trailing* marker is relocated by
//! `rustfmt` (below an opening brace) and by `leptosfmt` (in a `view!` body), so
//! only the line above is stable, and only the line above is honored.
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
//! 4. A marker is **trusted, not verified**. The gate checks that a reason exists
//!    and that the marker still points at a site; it can never check that the
//!    reason is true. That is inherent to any written exemption — `cov:ignore` has
//!    it too (ADR-0050 records those as permanent blind spots) — and it is why the
//!    set must stay small enough to re-read.
//! 5. A marked site is exempt regardless of what value flows *into* it. Narrowing
//!    the exemption from a function to a line shrinks that window (class 3 above
//!    is why it cannot be closed) but does not shut it.
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

/// The result of scanning one source: its policed mentions, plus the line ranges
/// of the test code that was skipped.
///
/// The ranges exist for the orphan check alone. Test code is exempt without
/// markers, but a fixture may legitimately carry one anyway; without knowing where
/// the test regions are, such a marker looks exactly like an exemption for a site
/// that no longer exists.
#[derive(Debug, Default)]
pub struct Scan {
    /// Non-test mentions, in line order.
    pub mentions: Vec<Mention>,
    /// 1-based inclusive line ranges of test items.
    pub test_ranges: Vec<(usize, usize)>,
}

impl Scan {
    /// Whether `line` falls inside any test region.
    fn in_test_code(&self, line: usize) -> bool {
        self.test_ranges
            .iter()
            .any(|(start, end)| (*start..=*end).contains(&line))
    }
}

/// Why a mention is not exempt — each variant is a different message, because
/// "you forgot a marker" and "your marker has no reason" need different fixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
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
pub struct Unexempt {
    pub line: usize,
    pub function: String,
    pub why: Why,
}

/// A legitimately marked site — one row of the derived census.
#[derive(Debug, Clone)]
pub struct Marked {
    /// The **site's** line, not the marker's: that is what a reader needs and what
    /// the failure messages already print.
    pub line: usize,
    pub reason: String,
}

/// Every mention of one source, sorted into the three outcomes.
#[derive(Debug, Default)]
pub struct Classified {
    pub unexempt: Vec<Unexempt>,
    pub marked: Vec<Marked>,
    /// Lines carrying this gate's marker whose next line holds no site.
    pub orphans: Vec<usize>,
}

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
    Ok(scan(source, population)?.mentions)
}

/// Every **non-test** mention of `population` in the source, in line order, plus
/// the line ranges of the test code that was skipped. `Err` on a `syn` parse
/// failure (fail-loud). Pure given the source, so gates unit-test through it.
pub fn scan<P: Population>(source: &str, population: &P) -> Result<Scan, String> {
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut scanner = Scanner {
        population,
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

/// Sort every mention into marked / unexempt, and find the markers that cover
/// nothing.
///
/// The marker sits on the line **immediately above** its site — the one position
/// `rustfmt` and `leptosfmt` both preserve (#778). A trailing marker is therefore
/// not an exemption at all: the site sees nothing above it, and the marker itself
/// points at a line with no site, so it fails twice over.
pub fn classify(source: &str, found: &Scan, token: &str) -> Classified {
    let lines: Vec<&str> = source.lines().collect();
    // 1-based line → the marker's reason, for every line carrying this gate's token.
    let marker_at = |line: usize| -> Option<&str> {
        crate::markers::marker_on_line(lines.get(line.checked_sub(1)?)?, token)
    };

    let mut sites_on_line: HashMap<usize, usize> = HashMap::new();
    for m in &found.mentions {
        *sites_on_line.entry(m.line).or_insert(0) += 1;
    }

    let mut out = Classified::default();
    for m in &found.mentions {
        let unexempt = |why| Unexempt {
            line: m.line,
            function: m.function.clone(),
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
    for line in 1..=lines.len() {
        if marker_at(line).is_some()
            && !sites_on_line.contains_key(&(line + 1))
            && !found.in_test_code(line)
        {
            out.orphans.push(line);
        }
    }
    out
}

struct Scanner<'p, P: Population> {
    population: &'p P,
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

impl<P: Population> Scanner<'_, P> {
    /// Note that `item` is test code, so a marker inside it is never an orphan.
    fn record_test_range<T: syn::spanned::Spanned>(&mut self, item: &T) {
        let span = syn::spanned::Spanned::span(item);
        self.test_ranges.push((span.start().line, span.end().line));
    }

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
        syn::visit::visit_item_impl(self, i);
        self.test_depth -= usize::from(test);
    }

    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        let test = is_test_cfg(&i.attrs) || has_test_attr(&i.attrs);
        if test {
            self.record_test_range(i);
        }
        self.test_depth += usize::from(test);
        self.fn_stack.push(i.sig.ident.to_string());
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
    /// The recovery paragraph, ending in the phrase that introduces the derived
    /// census (conventionally "Currently marked:").
    pub recovery: &'static str,
}

/// A complete enumerating gate: the population it reads structurally, the roots it
/// scans, and the words it fails in. The only way out is an in-source marker on the
/// line above the site (#778) — there is no list here to edit.
pub struct Gate<P: Population> {
    /// Step name in the xtask result (`"html-sink"`). Also the marker's token
    /// stem, so the two can never drift apart.
    pub step: &'static str,
    /// Source roots scanned recursively for `.rs` files. A missing root is a hard
    /// failure, so a moved or renamed tree can never quietly disable the guard.
    pub roots: &'static [&'static str],
    pub population: P,
    pub report: Report,
}

impl<P: Population> Gate<P> {
    /// 1-based `(line, enclosing-fn)` of every mention in one source that this
    /// gate's allowlist does not cover.
    ///
    /// Test-only: [`Gate::problems`] parses once and classifies itself, so this is
    /// the single-source convenience the gates' unit tests assert through — and
    /// pairing the parse with the marker rule here means the two halves cannot
    /// drift apart per gate. Orphan markers come back with an empty function name.
    #[cfg(test)]
    pub fn violations(&self, source: &str) -> Result<Vec<(usize, String)>, String> {
        let c = classify(
            source,
            &scan(source, &self.population)?,
            &self.marker_token(),
        );
        let mut out: Vec<(usize, String)> = c
            .unexempt
            .into_iter()
            .map(|u| (u.line, u.function))
            .collect();
        out.extend(c.orphans.into_iter().map(|line| (line, String::new())));
        out.sort();
        Ok(out)
    }

    /// The marker token this gate honors — its step name plus `:allow`. Derived
    /// rather than declared so a gate cannot be renamed out of sync with the
    /// markers that exempt its sites.
    pub fn marker_token(&self) -> String {
        format!("{}:allow", self.step)
    }

    /// The failure detail for every offending mention across the scanned files, or
    /// `None` when every site is marked. A per-file parse failure is surfaced
    /// (never swallowed). Pure given the `(path, source)` pairs, so gates unit-test
    /// it directly.
    ///
    /// On failure the detail ends with the **derived** census — every marked site
    /// the scan found. Unlike the declared allowlist it replaces, that census is
    /// computed from the tree, so it cannot go stale and there is no reconciliation
    /// pass to keep it honest.
    pub fn problems(&self, scanned: &[(String, String)]) -> Option<String> {
        let token = self.marker_token();
        let mut lines = Vec::new();
        let mut census = Vec::new();
        for (path, source) in scanned {
            match scan(source, &self.population) {
                Err(msg) => lines.push(format!(
                    "{path}: {msg} — an unparsed file is invisible to this gate, which is exactly \
                     the blind spot it exists to close. Fix the file or the parser; do not skip it."
                )),
                Ok(found) => {
                    let c = classify(source, &found, &token);
                    for u in c.unexempt {
                        let where_ = if u.function.is_empty() {
                            "at module scope".to_string()
                        } else {
                            format!("in fn `{}`", u.function)
                        };
                        lines.push(match u.why {
                            Why::Unmarked => format!(
                                "{path}:{}: {} {where_} {}",
                                u.line, self.report.subject, self.report.verdict
                            ),
                            Why::NoReason => format!(
                                "{path}:{}: {} {where_} carries a bare `{token}` marker — an \
                                 exemption with no reason is not an exemption; say why this site \
                                 is safe",
                                u.line, self.report.subject
                            ),
                            Why::Shared(n) => format!(
                                "{path}:{}: {n} `{}` sites share this line, so one marker cannot \
                                 justify them — split the line so each carries its own",
                                u.line, self.step
                            ),
                        });
                    }
                    for line in c.orphans {
                        lines.push(format!(
                            "{path}:{line}: `{token}` marker on a line with no `{}` site — a \
                             stale exemption; delete it",
                            self.step
                        ));
                    }
                    census.extend(c.marked.into_iter().map(|m| (path.clone(), m)));
                }
            }
        }

        if lines.is_empty() {
            return None;
        }
        lines.sort();
        lines.push(self.report.recovery.to_string());
        census.sort_by(|a, b| (&a.0, a.1.line).cmp(&(&b.0, b.1.line)));
        for (path, m) in census {
            lines.push(format!("    - {path}:{} — {}", m.line, m.reason));
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
    use super::{mentions, AnyOf};

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

/// The marker rule (#778), tested here rather than three times over: a marker on
/// the line ABOVE a site exempts it, and nothing else does.
#[cfg(test)]
mod marker_tests {
    use super::{classify, scan, AnyOf, Classified, Why};

    const TOKEN: &str = "guard:allow";

    fn classified(src: &str) -> Classified {
        let s = scan(src, &AnyOf(&["GUARDED"])).unwrap();
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
        assert_eq!(c.unexempt[0].function, "a");
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
        let src =
            "// guard:allow first\nfn a() { GUARDED; }\n// guard:allow second\nfn b() { GUARDED; }\n";
        let c = classified(src);
        assert_eq!(
            c.marked.iter().map(|m| m.line).collect::<Vec<_>>(),
            vec![2, 4]
        );
    }
}
