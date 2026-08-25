//! The shared machinery behind the ident-keyed XSS gates — [`raw-html-door`],
//! [`html-sink`] and [`rendered-html-from-trusted`].
//!
//! Those three gates guard one invariant from three sides (mint trust, inherit
//! trust, spend trust at the DOM), and they were written three times. That is worse
//! than ordinary duplication: a fix to the test-code exemption or the macro-token
//! walk that lands in two copies out of three leaves a gate that still reports
//! green, for the wrong reason — the exact failure ADR-0085 was written about. So
//! the traversal lives here once, and a gate supplies only what is genuinely its
//! own: the roots it scans, the [`population`] it recognises, and the words it
//! fails in.
//!
//! Two layers:
//!
//! - [`scan`] is the **traversal**: parse, track test-code depth and the enclosing
//!   fn stack, walk macro invocation tokens by hand, and ask whether each occurrence
//!   is in the gate's [`population`].
//! - [`Gate`] is the whole **enumerating gate** on top of it: deny by default,
//!   [`classify`] against the in-source markers, and a [`Report`] supplying the
//!   prose.
//!
//! **A gate reads idents everywhere, by construction.** A population is a set of
//! names, and membership is the same question in ordinary code and inside macro
//! tokens — there is no per-gate hook, so there is no hook a gate can silently fail
//! to implement ("say what you do not look at", #803).
//!
//! **Where the ident is not the whole question, the qualifier decides** (#790). A gate
//! whose population is an associated fn name another type may legitimately share sets
//! [`Gate::owner`]; the walker then resolves each site's qualifier and **suppresses**
//! the ones that belong to some other type. Two properties make that safe rather than
//! a hole:
//!
//! - **[`visit_ident`] stays the sole recorder.** Resolution only ever suppresses. A
//!   `fn` ident is not a [`syn::Path`], nor is a method-call ident or a macro token, so
//!   recording from a path hook would silently drop every definition site — including
//!   the guarded door's own. It also means a site cannot be counted twice, and that
//!   `owner: None` scans with no suppression at all: the suppression set is simply
//!   empty.
//! - **Unresolvable means in-population.** A qualifier the gate cannot pin — glob
//!   import, generic parameter, unqualified call, macro body — stays policed. Obscuring
//!   a qualifier buys a gate failure, not an exemption.
//!
//! Deciding membership this way is **structural**: it identifies the door rather than
//! exempting a site from it, so ADR-0085 principle 3 is not in play. #778 conflated the
//! two and deleted a qualifier check as a pattern exemption, which left the codebase
//! carrying markers on a provably harmless population. See
//! `docs/adr/0110-gate-population-membership-is-structural.md`.
//!
//! Macro bodies are deliberately **not** resolved — [`walk_macro_tokens`] sees a flat
//! token stream, and under the rule above not resolving is fail-closed. A
//! path-qualifier read three tokens to the left remains an available seam, since
//! `walk_macro_tokens` already materialises the flat sibling stream (the index is
//! one `.enumerate()` away).
//!
//! [`visit_ident`]: syn::visit::Visit::visit_ident
//!
//! **Exemptions are markers, not a list** (#778). A site is exempt when the line
//! *immediately above* it carries `// <gate-step>:allow <reason>`. The key is one
//! line, so it cannot absorb a second site the way a fn-keyed entry did; it moves
//! with the code under rename and refactor; and the exempt set is **derived** from
//! the tree rather than declared beside the rule, which removes the whole class of
//! staleness a declared list creates.
//!
//! The position is not a matter of taste: a *trailing* marker is relocated by
//! `rustfmt` (below an opening brace) and by `leptosfmt` (in a `view!` body), so
//! only the line above is stable, and only the line above is honored.
//!
//! **Unreadable classes inherent to this scan** (ADR-0085's honesty obligation;
//! each gate states the ones specific to *its* idents on top of these):
//!
//! 1. **Only for an owner-configured gate** (see [`Gate::owner`]), three ways a
//!    qualifier can mislead resolution, all fail-**open** (#790):
//!    a rename of a rename — [`owner_aliases`] harvests a single
//!    `use …Owner as X`, so a rename *of that rename* in a third module evades;
//!    a renaming re-export living **outside** the gate's roots, which is never
//!    harvested at all, so a use site inside them resolves to the alias's own name
//!    and is suppressed; and a free `fn` nested inside another type's `impl` method
//!    body, which the enclosing-`impl` lookup attributes to that type. None has a
//!    live instance; the first two are why a gate's roots must cover every tree it
//!    claims to police. For a gate with no owner there is nothing to resolve, and a
//!    `use … as` rename simply evades ident matching outright — `syn` has no name
//!    resolution, and before #790 that was this class's whole content.
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
//! 6. The test-code exemption is decided by a **substring**, not by parsing the
//!    `cfg` predicate: [`is_test_cfg`] asks whether the attribute's tokens mention
//!    `test` and not `not`. So `#[cfg(feature = "test-utils")]` reads as test code
//!    and its members are dropped from the population entirely — no marker owed,
//!    no census row. Nothing under the policed roots currently matches *and*
//!    encloses a gate ident, so there is no live hole; it is recorded because a
//!    **pattern** on an attribute's text is deciding an exemption, which is what
//!    ADR-0085 principle 3 forbids — and unlike deciding *membership* from a
//!    resolved qualifier (#790, which is structural), this really is an exemption
//!    granted by pattern. The marker work also made it load-bearing in a second
//!    place (`test_ranges`, which suppresses orphan reports). Parsing the predicate
//!    would close it.
//!
//! A `syn` parse failure is a **hard error** everywhere (ADR-0085 principle 6): a
//! file we cannot walk could hide a member, and a gate that quietly shrinks its own
//! population reports green for the one reason it must never report green.
//!
//! [`population`]: Gate::population
//! [`raw-html-door`]: crate::steps::raw_html_door_check
//! [`html-sink`]: crate::steps::html_sink_check
//! [`rendered-html-from-trusted`]: crate::steps::rendered_html_from_trusted_check

use std::collections::BTreeSet;
use std::collections::HashMap;

use crate::result::CommandResult;
use crate::steps::scan::run_source_scan;

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
    pub context: MentionContext,
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
    pub context: MentionContext,
}

/// The source context attached to a population mention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MentionContext {
    /// No enclosing function or field owner.
    Module,
    /// Nearest enclosing fn name.
    Function(String),
    /// Direct struct field, rendered as `Struct.field`.
    Field(String),
    /// Explicit row decoder, rendered as `fn.method`.
    RowDecode(String),
}

impl MentionContext {
    #[cfg(test)]
    pub fn legacy_label(&self) -> String {
        match self {
            Self::Module => String::new(),
            Self::Function(name) | Self::Field(name) | Self::RowDecode(name) => name.clone(),
        }
    }
}

fn mention_where(context: &MentionContext) -> String {
    match context {
        MentionContext::Module => "at module scope".to_string(),
        MentionContext::Function(name) => format!("in fn `{name}`"),
        MentionContext::Field(name) => format!("at field `{name}`"),
        MentionContext::RowDecode(name) => format!("at row decoder `{name}`"),
    }
}

/// Every ident that can denote `owner` anywhere in the scanned tree.
///
/// A renaming re-export in one module (`pub use crate::render::RenderedHtml as Doc;`)
/// makes `Doc::from_trusted` in *another* module a site on the owner's door, so a gate
/// that resolved qualifiers per-file alone would miss it (#790).
///
/// Deliberately **over-approximates**: an ident lands here on a name match alone, so a
/// `type ContentType = RenderedHtml;` anywhere in policed code would pull genuine
/// `ContentType` sites into the population. That is the fail-closed direction — an
/// over-large owner set costs a marker, an under-large one loses an XSS door.
///
/// The harvest is only as wide as the caller's roots: a rename living outside them is
/// invisible, which is why a gate's roots must cover every tree it claims to police.
///
/// A `syn` parse failure is skipped rather than fatal. This is a widening pass, and
/// [`scan`] already hard-errors on an unparseable file, so a second error path here would
/// only duplicate that one.
pub fn owner_aliases(sources: &[(String, String)], owner: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    set.insert(owner.to_string());
    for (_, source) in sources {
        let Ok(file) = syn::parse_file(source) else {
            continue;
        };
        collect_owner_aliases_in(&file.items, owner, &mut set);
    }
    set
}

/// Harvest owner aliases from a list of items, **recursing into inline modules**.
///
/// Recursing is not tidiness, it closes a fail-open. Miss a
/// `mod inner { pub use …Owner as Doc; }` here and `Doc` never enters the owner set —
/// while the file that then writes `use crate::a::inner::Doc;` *does* bind `Doc`, so
/// [`Resolver::membership`] reads `Doc::from_trusted` as another type and suppresses a
/// real door. Widening this pass can only move sites into the population, so recursing is
/// the safe direction; recursing [`Resolver::for_file`] without this would be the unsafe
/// one.
fn collect_owner_aliases_in(items: &[syn::Item], owner: &str, set: &mut BTreeSet<String>) {
    for item in items {
        match item {
            syn::Item::Use(u) => collect_owner_renames(&u.tree, owner, set),
            syn::Item::Type(t) if type_name(&t.ty).is_some_and(|id| id == owner) => {
                set.insert(t.ident.to_string());
            }
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    collect_owner_aliases_in(inner, owner, set);
                }
            }
            _ => {}
        }
    }
}

/// Whose door a policed site belongs to.
///
/// Named for the question it answers rather than for [`Gate::owner`], which is a type
/// *name* — this is a verdict about one site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Membership {
    /// The qualifier denotes the gate's owner type — the real door.
    Door,
    /// The qualifier denotes some other, named type. Not this door; no marker owed.
    OtherType,
    /// The qualifier could not be determined, so the site stays in the population.
    ///
    /// This is what keeps resolution from failing open: obscuring a qualifier buys a
    /// gate failure, not an exemption (#790).
    Unknown,
}

/// One file's answer to "what type does this bare ident denote?".
///
/// Only the two things a syntactic pass can know: what the file imports, and what it
/// defines. Everything else is [`Membership::Unknown`].
pub struct Resolver {
    /// Idents bound by a non-glob `use`, mapped to the final segment of their path.
    imported: BTreeSet<String>,
    /// Type names defined in this file — `struct`, `enum`, `union`, `type`.
    defined: BTreeSet<String>,
}

impl Resolver {
    /// Collect one file's `use` bindings and type definitions.
    pub fn for_file(file: &syn::File) -> Self {
        let mut imported = BTreeSet::new();
        let mut defined = BTreeSet::new();
        for item in &file.items {
            match item {
                syn::Item::Use(u) => collect_bound_names(&u.tree, &mut imported),
                syn::Item::Struct(s) => {
                    defined.insert(s.ident.to_string());
                }
                syn::Item::Enum(e) => {
                    defined.insert(e.ident.to_string());
                }
                syn::Item::Union(u) => {
                    defined.insert(u.ident.to_string());
                }
                syn::Item::Type(t) => {
                    defined.insert(t.ident.to_string());
                }
                _ => {}
            }
        }
        Self { imported, defined }
    }

    /// Classify a path whose leaf is a policed ident.
    ///
    /// `impl_self` is the enclosing `impl`'s self-type name, so `Self::` resolves.
    ///
    /// The owner set is consulted **first**, so a renamed owner is recognised before any
    /// other reading of the same ident. Getting that order wrong is what would let a
    /// cross-file rename (`use …Owner as Doc;` elsewhere, `use crate::a::Doc;` here)
    /// resolve as another type and be suppressed.
    pub fn membership(
        &self,
        path: &syn::Path,
        owners: &BTreeSet<String>,
        impl_self: Option<&str>,
    ) -> Membership {
        let segments: Vec<&syn::Ident> = path.segments.iter().map(|s| &s.ident).collect();
        // The leaf is the policed ident; the segment before it names the type.
        let Some(qualifier) = segments.len().checked_sub(2).map(|i| segments[i]) else {
            // A single-segment path is an unqualified call — nothing to resolve.
            return Membership::Unknown;
        };
        let name = qualifier.to_string();
        if owners.contains(&name) {
            return Membership::Door;
        }
        if name == "Self" {
            return match impl_self {
                Some(ty) if owners.contains(ty) => Membership::Door,
                Some(_) => Membership::OtherType,
                None => Membership::Unknown,
            };
        }
        // A multi-segment path spells the type out, so it resolves by construction.
        if segments.len() > 2 || self.imported.contains(&name) || self.defined.contains(&name) {
            return Membership::OtherType;
        }
        Membership::Unknown
    }

    /// Classify a direct struct-field type against an owner set.
    ///
    /// This is deliberately shallower than Rust type resolution. A plain path whose
    /// leaf is a known owner alias is the guarded type. A plain single-ident path
    /// that is neither imported nor locally defined is unknown and therefore remains
    /// guarded. Containers, borrowed types and qualified non-owner paths are outside
    /// the direct-field population.
    pub fn direct_type_membership(&self, ty: &syn::Type, owners: &BTreeSet<String>) -> Membership {
        let syn::Type::Path(p) = ty else {
            return Membership::OtherType;
        };
        if p.qself.is_some() {
            return Membership::OtherType;
        }
        let Some(final_segment) = p.path.segments.last() else {
            return Membership::Unknown;
        };
        if !matches!(final_segment.arguments, syn::PathArguments::None) {
            return Membership::OtherType;
        }

        let name = final_segment.ident.to_string();
        if owners.contains(&name) {
            return Membership::Door;
        }
        if is_known_non_owner_direct_type(&name) {
            return Membership::OtherType;
        }
        if p.path.segments.len() > 1
            || self.imported.contains(&name)
            || self.defined.contains(&name)
        {
            return Membership::OtherType;
        }
        Membership::Unknown
    }
}

fn is_known_non_owner_direct_type(name: &str) -> bool {
    matches!(
        name,
        "String"
            | "str"
            | "bool"
            | "char"
            | "f32"
            | "f64"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
    )
}

/// Every ident a non-glob `use` tree brings into scope, by the name it is bound to.
fn collect_bound_names(tree: &syn::UseTree, out: &mut BTreeSet<String>) {
    match tree {
        syn::UseTree::Path(p) => collect_bound_names(&p.tree, out),
        syn::UseTree::Group(g) => {
            for t in &g.items {
                collect_bound_names(t, out);
            }
        }
        syn::UseTree::Name(n) => {
            out.insert(n.ident.to_string());
        }
        syn::UseTree::Rename(r) => {
            out.insert(r.rename.to_string());
        }
        syn::UseTree::Glob(_) => {}
    }
}

/// Walk a `use` tree, recording the new name of any `… as X` that renames `owner`.
fn collect_owner_renames(tree: &syn::UseTree, owner: &str, set: &mut BTreeSet<String>) {
    match tree {
        syn::UseTree::Path(p) => collect_owner_renames(&p.tree, owner, set),
        syn::UseTree::Group(g) => {
            for t in &g.items {
                collect_owner_renames(t, owner, set);
            }
        }
        syn::UseTree::Rename(r) if r.ident == owner => {
            set.insert(r.rename.to_string());
        }
        syn::UseTree::Rename(_) | syn::UseTree::Name(_) | syn::UseTree::Glob(_) => {}
    }
}

/// The final path segment of a type, when it is a plain path — the type's own name.
pub(crate) fn type_name(ty: &syn::Type) -> Option<&syn::Ident> {
    match ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| &s.ident),
        _ => None,
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
pub fn scan(
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

/// Sort every mention into marked / unexempt, and find the markers that cover
/// nothing.
///
/// The marker sits on the line **immediately above** its site — the one position
/// `rustfmt` and `leptosfmt` both preserve (#778). A trailing marker is therefore
/// not an exemption at all: the site sees nothing above it, and the marker itself
/// points at a line with no site, so it fails twice over.
pub fn classify(source: &str, found: &Scan, token: &str) -> Classified {
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
pub(crate) fn is_test_cfg(attrs: &[syn::Attribute]) -> bool {
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
pub(crate) fn has_test_attr(attrs: &[syn::Attribute]) -> bool {
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
    /// Marker lookup happens later, in [`classify`], because whether a marker
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
            .push(type_name(&i.self_ty).map(ToString::to_string));
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
pub struct Gate {
    /// Step name in the xtask result (`"html-sink"`). Also the marker's token
    /// stem, so the two can never drift apart.
    pub step: &'static str,
    /// Source roots scanned recursively for `.rs` files. A missing root is a hard
    /// failure, so a moved or renamed tree can never quietly disable the guard.
    pub roots: &'static [&'static str],
    /// The names this gate polices — its population, read structurally from what the
    /// AST says, never from a pattern believed to characterise violations (ADR-0085
    /// principle 1). An occurrence of any of these idents is a member, in ordinary
    /// code or inside macro tokens, wherever it appears.
    ///
    /// Matching the ident rather than a call shape is what keeps such a gate an
    /// enumeration instead of a search for the spelling someone anticipated — a
    /// builder call, a struct field and a bare reference are all inside the
    /// population rather than silently outside it (ADR-0085 principle 3).
    pub population: &'static [&'static str],
    /// The type whose door this gate guards, when the population is an **associated fn
    /// name** another type may legitimately share (#790).
    ///
    /// With `Some(ty)`, a site whose qualifier resolves to a type other than `ty` is not
    /// this gate's door and owes no marker; a qualifier that cannot be resolved stays in
    /// the population, so the narrowing never fails open. Deciding membership this way is
    /// **structural** — it identifies the door rather than exempting a site from it, so
    /// ADR-0085 principle 3 is not in play.
    ///
    /// `None` polices the bare ident wherever it appears. That is right for a population
    /// that is a type (`PreEscaped`) or a method reached through `.` (`set_inner_html`),
    /// where there is no qualifier to read.
    pub owner: Option<&'static str>,
    pub report: Report,
}

impl Gate {
    /// 1-based `(line, enclosing-fn)` of every mention in one source that this
    /// gate's markers do not cover, plus every orphan marker (empty fn name).
    ///
    /// Test-only: [`Gate::problems`] parses once and classifies itself, so this is
    /// the single-source convenience the gates' unit tests assert through — and
    /// pairing the parse with the marker rule here means the two halves cannot
    /// drift apart per gate. Orphan markers come back with an empty function name.
    #[cfg(test)]
    pub fn violations(&self, source: &str) -> Result<Vec<(usize, String)>, String> {
        // Single-file owner set: a fixture is the whole tree as far as this helper is
        // concerned, so a rename it declares is honored and one it does not is not.
        let aliases = self
            .owner
            .map(|ty| owner_aliases(&[(String::new(), source.to_string())], ty));
        let owner = self.owner.zip(aliases.as_ref());
        let c = classify(
            source,
            &scan(source, self.population, owner)?,
            &self.marker_token(),
        );
        let mut out: Vec<(usize, String)> = c
            .unexempt
            .into_iter()
            .map(|u| (u.line, u.context.legacy_label()))
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
        // Harvested once, across every scanned file, before any classification: a
        // renaming re-export in one module decides membership in another (#790, D2).
        let aliases = self.owner.map(|ty| owner_aliases(scanned, ty));
        let owner = self.owner.zip(aliases.as_ref());
        let mut lines = Vec::new();
        let mut census = Vec::new();
        for (path, source) in scanned {
            match scan(source, self.population, owner) {
                Err(msg) => lines.push(format!(
                    "{path}: {msg} — an unparsed file is invisible to this gate, which is exactly \
                     the blind spot it exists to close. Fix the file or the parser; do not skip it."
                )),
                Ok(found) => {
                    let c = classify(source, &found, &token);
                    for u in c.unexempt {
                        let where_ = mention_where(&u.context);
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
pub fn run_scan(
    result: &mut CommandResult,
    step: &'static str,
    roots: &'static [&'static str],
    problems: impl FnOnce(&[(String, String)]) -> Option<String>,
) {
    run_source_scan(result, step, roots, problems);
}

#[cfg(test)]
mod tests {
    use super::{owner_aliases, scan};

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

    fn src(text: &str) -> (String, String) {
        ("a.rs".to_string(), text.to_string())
    }

    #[test]
    fn the_owner_is_always_in_its_own_alias_set() {
        let set = owner_aliases(&[], "Owner");
        assert_eq!(
            set.len(),
            1,
            "an empty tree yields the owner alone: {set:?}"
        );
        assert!(set.contains("Owner"));
    }

    #[test]
    fn a_renaming_use_of_the_owner_contributes_its_new_name() {
        let set = owner_aliases(&[src("use crate::render::Owner as Doc;\n")], "Owner");
        assert!(set.contains("Doc"), "a renamed import can denote the owner");
    }

    #[test]
    fn a_type_alias_to_the_owner_contributes_its_name() {
        assert!(owner_aliases(&[src("type Html = Owner;\n")], "Owner").contains("Html"));
    }

    #[test]
    fn a_nested_use_group_still_yields_the_rename() {
        let set = owner_aliases(
            &[src("use crate::render::{Sanitizer, Owner as Doc};\n")],
            "Owner",
        );
        assert!(set.contains("Doc"));
    }

    #[test]
    fn unrelated_renames_and_aliases_are_ignored() {
        let set = owner_aliases(
            &[src(
                "use crate::media::ContentType as Ct;\ntype Bytes = Vec<u8>;\n",
            )],
            "Owner",
        );
        assert_eq!(set.len(), 1, "only the owner itself: {set:?}");
    }

    #[test]
    fn a_plain_non_renaming_import_of_the_owner_adds_nothing() {
        let set = owner_aliases(&[src("use crate::render::Owner;\n")], "Owner");
        assert_eq!(set.len(), 1, "already the owner's own name: {set:?}");
    }

    #[test]
    fn the_harvest_spans_files_and_is_order_independent() {
        let a = (
            "a.rs".to_string(),
            "use crate::render::Owner as Doc;\n".to_string(),
        );
        let b = ("b.rs".to_string(), "type Html = Owner;\n".to_string());
        let forward = owner_aliases(&[a.clone(), b.clone()], "Owner");
        let backward = owner_aliases(&[b, a], "Owner");
        assert_eq!(forward, backward);
        assert!(forward.contains("Doc") && forward.contains("Html"));
    }

    #[test]
    fn an_unparseable_file_is_skipped_rather_than_panicking() {
        assert_eq!(owner_aliases(&[src("fn (((")], "Owner").len(), 1);
    }

    /// A rename inside an inline module must be harvested. Missing it is fail-**open**:
    /// the importing file binds the alias, so the resolver would read the door as another
    /// type and suppress it. Found by the whole-branch standards review on #790.
    #[test]
    fn a_rename_inside_an_inline_module_is_harvested() {
        let set = owner_aliases(
            &[src("mod inner { pub use crate::render::Owner as Doc; }\n")],
            "Owner",
        );
        assert!(
            set.contains("Doc"),
            "nested renames must widen the set: {set:?}"
        );
    }

    #[test]
    fn a_type_alias_inside_an_inline_module_is_harvested() {
        let set = owner_aliases(&[src("mod inner { pub type Html = Owner; }\n")], "Owner");
        assert!(set.contains("Html"), "{set:?}");
    }

    #[test]
    fn nesting_is_harvested_to_any_depth() {
        let set = owner_aliases(
            &[src(
                "mod a { mod b { pub use crate::render::Owner as Deep; } }\n",
            )],
            "Owner",
        );
        assert!(set.contains("Deep"), "{set:?}");
    }
}

/// Qualifier resolution (#790): the rule is "prove this is not the owner's door, or
/// leave it in the population", so every branch that returns [`Membership::Unknown`]
/// matters as much as the ones that resolve.
#[cfg(test)]
mod resolver_tests {
    use std::collections::BTreeSet;

    use super::{Membership, Resolver};

    /// The first path in `file` whose **last** segment is `leaf`, in visit order.
    ///
    /// Returns a single-segment path for an unqualified call — "unqualified" is a verdict
    /// the resolver produces, not an absence. `use` items are skipped, or a fixture's own
    /// import would be found before its call site.
    fn first_policed_path(file: &syn::File, leaf: &str) -> Option<syn::Path> {
        struct Find<'a> {
            leaf: &'a str,
            found: Option<syn::Path>,
        }
        impl<'ast> syn::visit::Visit<'ast> for Find<'_> {
            fn visit_item_use(&mut self, _: &'ast syn::ItemUse) {}
            fn visit_path(&mut self, p: &'ast syn::Path) {
                if self.found.is_none() && p.segments.last().is_some_and(|s| s.ident == self.leaf) {
                    self.found = Some(p.clone());
                }
                syn::visit::visit_path(self, p);
            }
        }
        let mut find = Find { leaf, found: None };
        syn::visit::visit_file(&mut find, file);
        find.found
    }

    fn resolve(src: &str, owners: &[&str], impl_self: Option<&str>) -> Membership {
        let file: syn::File = syn::parse_str(src).expect("fixture parses");
        let set: BTreeSet<String> = owners.iter().map(|s| (*s).to_string()).collect();
        let path = first_policed_path(&file, "from_trusted").expect("fixture has a site");
        Resolver::for_file(&file).membership(&path, &set, impl_self)
    }

    #[test]
    fn a_bare_owner_qualifier_is_the_door() {
        let src = "fn f() { Owner::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::Door);
    }

    #[test]
    fn a_renamed_owner_qualifier_is_the_door() {
        // The #778 hole, closed by resolution rather than by over-approximation.
        let src = "use crate::render::Owner as Doc;\nfn f() { Doc::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner", "Doc"], None), Membership::Door);
    }

    #[test]
    fn a_fully_qualified_owner_path_is_still_the_door() {
        // Fails OPEN if ">2 segments" is read as "not the door".
        let src = "fn f() { crate::render::Owner::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::Door);
    }

    #[test]
    fn a_multi_segment_path_names_its_type_and_needs_no_import() {
        let src = "fn f() { crate::media::ContentType::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
    }

    #[test]
    fn self_inside_the_owners_impl_is_the_door() {
        let src = "fn f() { Self::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], Some("Owner")), Membership::Door);
    }

    #[test]
    fn self_inside_another_impl_is_not_the_door() {
        let src = "fn f() { Self::from_trusted(x); }\n";
        assert_eq!(
            resolve(src, &["Owner"], Some("ContentType")),
            Membership::OtherType
        );
    }

    #[test]
    fn self_with_no_enclosing_impl_is_unknown() {
        let src = "fn f() { Self::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::Unknown);
    }

    #[test]
    fn a_qualifier_defined_in_this_file_resolves_to_itself() {
        let src = "struct ContentType(String);\nfn f() { ContentType::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
    }

    #[test]
    fn a_qualifier_imported_by_a_flat_use_resolves() {
        let src = "use crate::media::ContentType;\nfn f() { ContentType::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
    }

    #[test]
    fn a_qualifier_imported_by_a_nested_use_group_resolves() {
        // The form `common/src/feed/feed_path.rs:7` actually uses.
        let src = "use crate::{media::ContentType, tag::Tag};\nfn f() { ContentType::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
    }

    #[test]
    fn an_in_file_type_alias_resolves_without_the_owner_set() {
        // The alias is NOT seeded into `owners`, so this exercises the in-file branch
        // rather than short-circuiting on the owner set.
        let src = "type Ct = ContentType;\nfn f() { Ct::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
    }

    #[test]
    fn an_unbound_bare_qualifier_is_unknown() {
        let src = "use foo::*;\nfn f() { Mystery::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::Unknown);
    }

    #[test]
    fn a_generic_parameter_qualifier_is_unknown() {
        let src = "fn f<T>() { T::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::Unknown);
    }

    #[test]
    fn an_unqualified_call_is_unknown() {
        let src = "fn f() { from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::Unknown);
    }
}

/// Resolution wired through the walker (#790): the same rule as `resolver_tests`, but
/// reached the way a gate reaches it, so `impl_stack`, the suppression set and the
/// definition-site path are all exercised.
#[cfg(test)]
mod owned_scan_tests {
    use super::{Classified, Why, classify, owner_aliases, scan};

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

/// The marker rule (#778), tested here rather than three times over: a marker on
/// the line ABOVE a site exempts it, and nothing else does.
#[cfg(test)]
mod marker_tests {
    use super::{Classified, Why, classify, scan};

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
