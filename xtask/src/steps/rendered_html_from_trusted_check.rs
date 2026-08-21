//! The `rendered-html-from-trusted` static check (#398, extended #445, #778,
//! #701): pins every `from_trusted` in production code — above all
//! `RenderedHtml::from_trusted`, the *inheriting* door of the
//! `common::render::RenderedHtml` newtype — and every direct production
//! `RenderedHtml` struct field.
//!
//! `RenderedHtml` marks HTML that is safe to emit **unescaped** into the DOM
//! (`inner_html`). Two doors carry that invariant, and they mean different things:
//! `sanitize` **establishes** it by scrubbing against an allowlist (the door for
//! anything from outside jaunder — a rendered post body via `render()`, an ingested
//! feed entry, any future inbound producer), while `from_trusted` merely
//! **inherits** it, asserting a value we already sanitized has round-tripped through
//! our own storage or wire.
//!
//! Because `from_trusted` is `pub` and cross-crate, Rust visibility cannot confine
//! it: a future inbound path could launder a stranger's HTML into "trusted" and
//! compile clean, reopening the #445 stored-XSS hole. This gate makes that a
//! host-side failure — every **non-test** `from_trusted` mention must carry a
//! marker, so choosing the wrong door breaks the build instead of silently
//! shipping. (`sanitize` needs no gate: it is safe wherever it is called.)
//!
//! **Population** (read structurally, ADR-0085 principle 1): every `from_trusted`
//! under [`POLICED_ROOTS`] whose qualifier is **`RenderedHtml`'s**, plus every one
//! whose qualifier cannot be determined — in ordinary code and inside macro token
//! streams — and every direct struct field whose type is `RenderedHtml`,
//! path-qualified `...::RenderedHtml`, a harvested owner alias, or an unresolved
//! single-ident type that the scanner cannot prove is something else (#701).
//!
//! Deciding *membership* is structural — it identifies the door — and is not the
//! same act as *exempting* a site from it (ADR-0085 principle 3 governs only the
//! latter; see `docs/adr/0110-gate-population-membership-is-structural.md`, #790).
//! The gate resolves the qualifier by reading what the AST plainly says, never by
//! pattern-matching a name list — a list fails **open** asymmetrically (an aliased
//! qualifier like `use RenderedHtml as ContentType` hands out the exemption) — and
//! it **fails closed** on anything it cannot resolve.
//!
//! Two consequences follow:
//!
//! - **Another type's door owes nothing** once the gate can see whose door it is.
//!   `ContentType::from_trusted` carries no marker. A qualifier the gate *cannot*
//!   resolve — glob import, generic parameter, unqualified call, macro body — stays
//!   in the population, so obscuring one buys a failure, not an exemption.
//! - **Another type's field owes nothing** once the gate can see what the type is.
//!   `field: OtherType` carries no marker when `OtherType` is imported, locally
//!   defined, fully qualified, or a known Rust scalar/prelude type. A direct
//!   single-ident field type the scanner cannot resolve stays in the population.
//! - A `from_trusted` **definition** is in the population when it sits in
//!   `impl RenderedHtml` — `syn` visits a fn's own `sig.ident` — so `pub fn
//!   from_trusted` carries a marker saying it is the door itself. A definition in
//!   another type's `impl` does not.
//!
//! Every member fails unless the line **immediately above** it carries a
//! `// rendered-html-from-trusted:allow <reason>` marker. The marker rule and the
//! derived census are [`crate::steps::ident_gate`]. This module owns the combined
//! population because field markers and call-site markers share the same token: the
//! two populations must be classified together or each population would misread the
//! other's legitimate markers as stale.
//!
//! Test/fixture code (anything under a `#[cfg(test)]` module/fn, or a `#[test]`/
//! `#[rstest]` fn) is exempt — fixtures legitimately mint `RenderedHtml` to stand
//! in for rendered output.
//!
//! **Macro bodies are scanned** (#333). `syn` itself does not descend into a macro
//! invocation, so the shared scan walks [`syn::Macro`]'s `.tokens` directly,
//! recursing through nested `Group`s — `web`'s render layer is built out of
//! `html!`/`view!` bodies, which is exactly where the unescaped sink lives, so
//! skipping macros would skip the ordinary case.
//!
//! **Unreadable classes** (ADR-0085's honesty obligation) specific to this gate:
//! resolution reads names, not types, so it can be misled by a chain of renames. The
//! three call-site ways, all fail-**open**, are enumerated as class 1 in
//! [`crate::steps::ident_gate`] — a rename of a rename, a renaming re-export outside
//! the roots, and a free `fn` nested inside another type's `impl`. None has a live
//! instance. Field scanning is intentionally direct only: `&RenderedHtml`,
//! `Option<RenderedHtml>`, `Vec<RenderedHtml>` and `Box<RenderedHtml>` are not in the
//! population. That is a documented out-of-scope class for #701 rather than implied
//! coverage. The field scanner can also over-include a direct field inside an inline
//! module whose non-`RenderedHtml` type is defined only in that same inline module;
//! that fails closed. Each class is strictly narrower than the blind spot #778
//! removed, which handed out a tree-wide exemption for one aliased qualifier. The
//! classes inherent to the shared scan (the unwalked attribute-macro tokens, the
//! absent call graph, and that a marker is trusted rather than verified) are also
//! stated there. A `syn` parse failure is a **hard error** (a file we cannot walk
//! could hide a spurious door or field — a false pass), matching
//! [`crate::steps::server_fn_registrar_check`].

use std::collections::BTreeSet;

use crate::result::CommandResult;
use crate::steps::ident_gate::{
    self, Gate, Marked, Membership, Mention, Report, Scan, Unexempt, Why,
};
use syn::spanned::Spanned;

/// Source roots scanned recursively for `.rs` files — production `src` trees, not
/// the `tests/` integration crates (whose fixtures mint freely).
const POLICED_ROOTS: &[&str] = &[
    "common/src",
    "host/src",
    "storage/src",
    "web/src",
    "server/src",
    "csr/src",
    "macros/src",
];

/// The associated-fn ident this guard pins.
const DOORS: &[&str] = &["from_trusted"];

const FIELD_CONTEXT_PREFIX: &str = "field:";

/// The gate: population, roots and prose. Exemptions are in-source markers on the
/// line above each door (#778), so there is no list here.
///
/// The prose can name `RenderedHtml` again (#790). Between #778 and #790 it had to name
/// the bare **ident**, because the population then held `ContentType::from_trusted` too
/// and a verdict asserting "a raw string minted here is emitted unescaped" would have
/// been false there — a gate that fails with an inaccurate reason teaches the wrong
/// lesson at the exact moment someone is reading it. Resolving the qualifier removed
/// that population, so the verdict can be specific again.
///
/// It still has to cover the **unresolvable** case, which is the one kind of site that
/// reaches a human without being known to be this door.
const GATE: Gate = Gate {
    step: "rendered-html-from-trusted",
    roots: POLICED_ROOTS,
    population: DOORS,
    // The qualifier decides membership (#790): a `from_trusted` on another type is not
    // this door and owes nothing, while one whose qualifier cannot be resolved stays in.
    owner: Some("RenderedHtml"),
    report: Report {
        subject: "a `from_trusted` door",
        verdict: "is not marked — either it is `RenderedHtml`'s door, which is what lets HTML \
                  reach the DOM unescaped (XSS) (#398), or its qualifier could not be resolved, \
                  so this gate cannot tell whose door it is (#790)",
        recovery: "  recovery: `from_trusted` only *inherits* safety — it may reconstruct a value we \
                   already sanitized and round-tripped through our own store or wire. If the HTML \
                   comes from OUTSIDE jaunder (an ingested feed entry, a remote channel, any \
                   inbound producer), it must go through `RenderedHtml::sanitize`, which \
                   *establishes* safety by scrubbing; for a rendered post body that means \
                   `render()`. If this is a DIFFERENT type's `from_trusted`, the gate ignores it \
                   once it can see that: name the type so the qualifier resolves (an import, an \
                   in-file definition, or the full path) rather than reaching for a marker. \
                   Otherwise put the reason in a \
                   `// rendered-html-from-trusted:allow <reason>` comment on the line IMMEDIATELY \
                   ABOVE the site — not trailing it, which the formatters move. Currently marked:",
    },
};

/// 1-based `(line, enclosing-fn)` of every unmarked mention, plus every orphan
/// marker (empty fn name). Test-only: [`problems`] parses once and classifies
/// itself, so this is the single-source convenience the unit tests assert through.
#[cfg(test)]
fn violations(source: &str) -> Result<Vec<(usize, String)>, String> {
    let aliases = ident_gate::owner_aliases(&[(String::new(), source.to_string())], "RenderedHtml");
    let c = ident_gate::classify(
        source,
        &combined_scan(source, Some(("RenderedHtml", &aliases)))?,
        &GATE.marker_token(),
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

/// The failure detail for every offending mention across the scanned files, or
/// `None` when every door is marked.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let token = GATE.marker_token();
    let aliases = ident_gate::owner_aliases(scanned, "RenderedHtml");
    let mut lines = Vec::new();
    let mut census = Vec::new();
    for (path, source) in scanned {
        match combined_scan(source, Some(("RenderedHtml", &aliases))) {
            Err(msg) => lines.push(format!(
                "{path}: {msg} — an unparsed file is invisible to this gate, which is exactly \
                 the blind spot it exists to close. Fix the file or the parser; do not skip it."
            )),
            Ok(found) => {
                let c = ident_gate::classify(source, &found, &token);
                for u in c.unexempt {
                    lines.push(format_unexempt(path, &token, &u));
                }
                for line in c.orphans {
                    lines.push(format!(
                        "{path}:{line}: `{token}` marker on a line with no `{}` site — a stale \
                         exemption; delete it",
                        GATE.step
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
    lines.push(GATE.report.recovery.to_string());
    census.sort_by(|a, b| (&a.0, a.1.line).cmp(&(&b.0, b.1.line)));
    for (path, m) in census {
        lines.push(format_marked(&path, &m));
    }
    Some(lines.join("\n"))
}

/// Scan every Rust file under each [`POLICED_ROOTS`] and push the result step. A
/// missing root is a hard failure, so a moved/renamed tree can never quietly
/// disable the guard.
pub fn run(result: &mut CommandResult) {
    ident_gate::run_scan(result, GATE.step, GATE.roots, problems);
}

fn combined_scan(source: &str, owner: Option<(&str, &BTreeSet<String>)>) -> Result<Scan, String> {
    let mut found = ident_gate::scan(source, DOORS, owner)?;
    if let Some((_, owners)) = owner {
        let fields = field_scan(source, owners)?;
        found.mentions.extend(fields.mentions);
        found.test_ranges.extend(fields.test_ranges);
        found.mentions.sort_by_key(|m| m.line);
    }
    Ok(found)
}

fn field_scan(source: &str, owners: &BTreeSet<String>) -> Result<Scan, String> {
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut scanner = FieldScanner {
        owners,
        resolver: ident_gate::Resolver::for_file(&file),
        test_depth: 0,
        struct_stack: Vec::new(),
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

fn format_unexempt(path: &str, token: &str, u: &Unexempt) -> String {
    match field_context(&u.function) {
        Some(context) => format_field_unexempt(path, token, u, context),
        None => format_door_unexempt(path, token, u),
    }
}

fn format_door_unexempt(path: &str, token: &str, u: &Unexempt) -> String {
    let where_ = if u.function.is_empty() {
        "at module scope".to_string()
    } else {
        format!("in fn `{}`", u.function)
    };
    match u.why {
        Why::Unmarked => format!(
            "{path}:{}: {} {where_} {}",
            u.line, GATE.report.subject, GATE.report.verdict
        ),
        Why::NoReason => format!(
            "{path}:{}: {} {where_} carries a bare `{token}` marker — an exemption with no \
             reason is not an exemption; say why this site is safe",
            u.line, GATE.report.subject
        ),
        Why::Shared(n) => format_shared(path, u.line, n),
    }
}

fn format_field_unexempt(path: &str, token: &str, u: &Unexempt, context: &str) -> String {
    match u.why {
        Why::Unmarked => format!(
            "{path}:{}: a `RenderedHtml` field at `{context}` is not marked — direct \
             `RenderedHtml` fields carry trusted HTML across storage, domain, or wire boundaries \
             and must explain why that trust is legitimate (#701)",
            u.line
        ),
        Why::NoReason => format!(
            "{path}:{}: a `RenderedHtml` field at `{context}` carries a bare `{token}` marker — \
             an exemption with no reason is not an exemption; say why this field is safe",
            u.line
        ),
        Why::Shared(n) => format_shared(path, u.line, n),
    }
}

fn format_shared(path: &str, line: usize, n: usize) -> String {
    format!(
        "{path}:{line}: {n} `{}` sites share this line, so one marker cannot justify them — split \
         the line so each carries its own",
        GATE.step
    )
}

fn format_marked(path: &str, m: &Marked) -> String {
    format!("    - {path}:{} — {}", m.line, m.reason)
}

fn field_context(function: &str) -> Option<&str> {
    function.strip_prefix(FIELD_CONTEXT_PREFIX)
}

struct FieldScanner<'a> {
    owners: &'a BTreeSet<String>,
    resolver: ident_gate::Resolver,
    test_depth: usize,
    struct_stack: Vec<String>,
    hits: Vec<Mention>,
    test_ranges: Vec<(usize, usize)>,
}

impl FieldScanner<'_> {
    fn record_test_range<T: Spanned>(&mut self, item: &T) {
        let span = item.span();
        self.test_ranges.push((span.start().line, span.end().line));
    }

    fn record_field(&mut self, field: &syn::Field) {
        if self.test_depth > 0 {
            return;
        }
        match self.resolver.direct_type_membership(&field.ty, self.owners) {
            Membership::Door | Membership::Unknown => {}
            Membership::OtherType => return,
        }
        let line = field
            .ident
            .as_ref()
            .map_or_else(|| field.ty.span().start().line, |id| id.span().start().line);
        let field_name = field
            .ident
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "<unnamed>".to_string());
        let owner = self
            .struct_stack
            .last()
            .cloned()
            .unwrap_or_else(|| "<unknown>".to_string());
        self.hits.push(Mention {
            line,
            function: format!("{FIELD_CONTEXT_PREFIX}{owner}.{field_name}"),
        });
    }
}

impl<'ast> syn::visit::Visit<'ast> for FieldScanner<'_> {
    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        let test = ident_gate::is_test_cfg(&i.attrs);
        if test {
            self.record_test_range(i);
        }
        self.test_depth += usize::from(test);
        syn::visit::visit_item_mod(self, i);
        self.test_depth -= usize::from(test);
    }

    fn visit_item_struct(&mut self, i: &'ast syn::ItemStruct) {
        let test = ident_gate::is_test_cfg(&i.attrs);
        if test {
            self.record_test_range(i);
        }
        self.test_depth += usize::from(test);
        self.struct_stack.push(i.ident.to_string());
        syn::visit::visit_item_struct(self, i);
        self.struct_stack.pop();
        self.test_depth -= usize::from(test);
    }

    fn visit_field(&mut self, i: &'ast syn::Field) {
        let test = ident_gate::is_test_cfg(&i.attrs);
        if test {
            self.record_test_range(i);
        }
        self.test_depth += usize::from(test);
        self.record_field(i);
        self.test_depth -= usize::from(test);
    }
}

#[cfg(test)]
mod tests {
    use super::{problems, violations};

    #[test]
    fn a_marked_door_passes() {
        let src = "fn deserialize_rendered_html(s: String) -> RenderedHtml {\n    // rendered-html-from-trusted:allow wire DTO our own server serialized (#445)\n    RenderedHtml::from_trusted(s)\n}\n";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    /// The fn name bought the old exemption; it buys nothing now.
    #[test]
    fn a_formerly_allowlisted_fn_name_grants_nothing() {
        let src = "fn deserialize_rendered_html(s: String) -> RenderedHtml { RenderedHtml::from_trusted(s) }\n";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn call_in_a_non_allowlisted_fn_is_flagged() {
        let src = "\
fn sneaky(raw: String) -> RenderedHtml {
    RenderedHtml::from_trusted(raw)
}
";
        assert_eq!(violations(src).unwrap(), vec![(2, "sneaky".to_string())]);
    }

    #[test]
    fn an_inbound_shaped_fn_using_from_trusted_is_flagged() {
        // The #445 shape this guard exists to stop: a future inbound producer
        // (feed ingestion, #282) reaching for the *inheriting* door on HTML that
        // came from a stranger's server. `RenderedHtml::sanitize` is the only
        // correct door for outside data, and this is what makes choosing wrong a
        // build failure rather than a silent stored-XSS hole.
        let src = "\
fn ingest_feed_entry(remote_html: String) -> RenderedHtml {
    RenderedHtml::from_trusted(remote_html)
}
";
        assert_eq!(
            violations(src).unwrap(),
            vec![(2, "ingest_feed_entry".to_string())]
        );
    }

    #[test]
    fn map_reference_in_a_non_allowlisted_fn_is_flagged() {
        let src = "\
fn sneaky(raw: String) -> RenderedHtml {
    Some(raw).map(RenderedHtml::from_trusted).unwrap()
}
";
        assert_eq!(violations(src).unwrap(), vec![(2, "sneaky".to_string())]);
    }

    /// The payoff of #790: another type's door owes **nothing** once the gate can see
    /// whose door it is. `ContentType` is defined right here, so the qualifier resolves.
    #[test]
    fn a_resolvable_content_type_door_needs_no_marker() {
        let src = "\
struct ContentType(String);
fn detect(n: &str) -> ContentType { ContentType::from_trusted(n) }
";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    /// The same call with the type merely *named* — no import, no definition — is
    /// unresolvable, so it stays in the population. This is what keeps the narrowing from
    /// failing open: the gate flags what it cannot rule out (#790 D1).
    #[test]
    fn an_unresolvable_qualifier_is_still_flagged() {
        let src = "fn detect(n: &str) -> ContentType { ContentType::from_trusted(n) }\n";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    /// And the unresolvable case can still be discharged with a marker, exactly as
    /// before — resolution adds a way to be clean, it does not remove one.
    #[test]
    fn a_marked_unresolvable_door_passes() {
        let src = "fn detect(n: &str) -> ContentType {\n    // rendered-html-from-trusted:allow mints a media type, never HTML (#584)\n    ContentType::from_trusted(n)\n}\n";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    /// A rename of the owner still lands in the population — the #778 fail-open
    /// (`use RenderedHtml as ContentType`), now closed by resolution rather than by
    /// refusing to read the qualifier at all.
    #[test]
    fn a_renamed_owner_is_in_the_population() {
        let src = "\
use crate::render::RenderedHtml as ContentType;
fn sneaky(raw: String) -> ContentType { ContentType::from_trusted(raw) }
";
        assert_eq!(violations(src).unwrap(), vec![(2, "sneaky".to_string())]);
    }

    #[test]
    fn a_from_trusted_on_an_unresolvable_unrelated_type_is_still_flagged() {
        let src = "\
fn sneaky(raw: String) -> Widget {
    Widget::from_trusted(raw)
}
";
        assert_eq!(violations(src).unwrap(), vec![(2, "sneaky".to_string())]);
    }

    #[test]
    fn a_from_trusted_on_a_resolvable_unrelated_type_is_ignored() {
        let src = "\
use crate::widget::Widget;
fn fine(raw: String) -> Widget {
    Widget::from_trusted(raw)
}
";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    /// Replaces `the_definition_site_has_no_path_mention`. Now that membership is the
    /// bare ident wherever it occurs, the door's own declaration is in the population —
    /// a deliberate behavior change (#778), failing closed. `visit_impl_item_fn` pushes
    /// the fn name before the signature's ident is visited, so the mention's enclosing
    /// fn is the door itself.
    #[test]
    fn the_definition_site_is_in_the_population() {
        let src = "\
impl RenderedHtml {
    pub fn from_trusted(html: impl Into<String>) -> Self {
        Self(html.into())
    }
}
";
        assert_eq!(
            violations(src).unwrap(),
            vec![(2, "from_trusted".to_string())]
        );
    }

    #[test]
    fn a_marked_definition_site_passes() {
        let src = "\
impl RenderedHtml {
    // rendered-html-from-trusted:allow the door's own definition; the gate pins its uses
    pub fn from_trusted(html: impl Into<String>) -> Self {
        Self(html.into())
    }
}
";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_bare_marker_fails() {
        let src = "fn f(raw: String) -> RenderedHtml {\n    // rendered-html-from-trusted:allow\n    RenderedHtml::from_trusted(raw)\n}\n";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn an_orphan_marker_fails() {
        let src = "// rendered-html-from-trusted:allow stale\nfn f() { harmless(); }\n";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn an_unmarked_rendered_html_field_is_flagged() {
        let src = "\
struct PostRow {
    rendered_html: RenderedHtml,
}
";
        assert_eq!(
            violations(src).unwrap(),
            vec![(2, "field:PostRow.rendered_html".to_string())]
        );
    }

    #[test]
    fn a_marked_rendered_html_field_passes() {
        let src = "\
struct PostRow {
    // rendered-html-from-trusted:allow storage row decodes sanitized rendered_html (#701)
    rendered_html: RenderedHtml,
}
";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_bare_field_marker_fails() {
        let src = "\
struct PostRow {
    // rendered-html-from-trusted:allow
    rendered_html: RenderedHtml,
}
";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn a_field_marker_is_not_an_orphan_without_a_door_call() {
        let src = "\
struct PostRow {
    // rendered-html-from-trusted:allow storage row decodes sanitized rendered_html (#701)
    rendered_html: RenderedHtml,
}
";
        assert_eq!(
            problems(&[("storage/src/helpers.rs".to_string(), src.to_string())]),
            None
        );
    }

    #[test]
    fn a_door_marker_is_not_an_orphan_without_a_field() {
        let src = "\
fn f(raw: String) -> RenderedHtml {
    // rendered-html-from-trusted:allow storage wire rebuild (#701)
    RenderedHtml::from_trusted(raw)
}
";
        assert_eq!(
            problems(&[("common/src/render.rs".to_string(), src.to_string())]),
            None
        );
    }

    #[test]
    fn shared_line_counts_fields_and_door_calls_together() {
        let src = "\
// rendered-html-from-trusted:allow one reason cannot cover two trust sites (#701)
struct Row { rendered_html: RenderedHtml } fn f(raw: String) { RenderedHtml::from_trusted(raw); }
";
        assert_eq!(violations(src).unwrap().len(), 2);
    }

    #[test]
    fn field_in_a_cfg_test_module_is_exempt() {
        let src = "\
#[cfg(test)]
mod tests {
    struct PostRow {
        rendered_html: RenderedHtml,
    }
}
";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn field_spellings_that_resolve_to_rendered_html_are_flagged() {
        let cases = [
            "struct Row { rendered_html: RenderedHtml }\n",
            "struct Row { rendered_html: common::render::RenderedHtml }\n",
            "use common::render::RenderedHtml as Html;\nstruct Row { rendered_html: Html }\n",
            "type Html = RenderedHtml;\nstruct Row { rendered_html: Html }\n",
        ];
        for src in cases {
            assert_eq!(violations(src).unwrap().len(), 1, "{src}");
        }
    }

    #[test]
    fn borrowed_and_container_rendered_html_fields_are_out_of_scope() {
        let src = "\
struct Row<'a> {
    borrowed: &'a RenderedHtml,
    optional: Option<RenderedHtml>,
    many: Vec<RenderedHtml>,
    boxed: Box<RenderedHtml>,
}
";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn unrelated_resolvable_field_types_are_ignored() {
        let src = "\
struct RenderedHtmlish;
struct Other;
struct Row {
    a: RenderedHtmlish,
    b: Other,
    c: String,
}
";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn unresolved_direct_field_type_fails_closed() {
        let src = "\
struct Row {
    rendered_html: MaybeHtml,
}
";
        assert_eq!(
            violations(src).unwrap(),
            vec![(2, "field:Row.rendered_html".to_string())]
        );
    }

    #[test]
    fn marked_unresolved_direct_field_type_passes() {
        let src = "\
struct Row {
    // rendered-html-from-trusted:allow unresolved alias is known at this call site (#701)
    rendered_html: MaybeHtml,
}
";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn inline_module_local_type_is_conservatively_overincluded() {
        let src = "\
mod m {
    struct Other;
    struct Row {
        value: Other,
    }
}
";
        assert_eq!(
            violations(src).unwrap(),
            vec![(4, "field:Row.value".to_string())]
        );
    }

    #[test]
    fn call_in_a_cfg_test_module_is_exempt() {
        let src = "\
#[cfg(test)]
mod tests {
    fn fixture() -> RenderedHtml {
        RenderedHtml::from_trusted(\"<p>x</p>\")
    }
}
";
        assert!(violations(src).unwrap().is_empty());
    }

    #[test]
    fn a_cfg_not_test_production_fn_is_scanned() {
        let src = "\
#[cfg(not(test))]
fn prod_only(raw: String) -> RenderedHtml {
    RenderedHtml::from_trusted(raw)
}
";
        assert_eq!(violations(src).unwrap(), vec![(3, "prod_only".to_string())]);
    }

    #[test]
    fn call_in_a_test_fn_is_exempt() {
        let src = "\
#[test]
fn t() {
    let _ = RenderedHtml::from_trusted(\"<p>x</p>\");
}
";
        assert!(violations(src).unwrap().is_empty());
    }

    #[test]
    fn module_scope_call_is_flagged() {
        // Not inside any fn — no enclosing fn to name.
        let src = "static X: () = { RenderedHtml::from_trusted(\"<p>x</p>\"); };\n";
        let v = violations(src).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].1, "");
    }

    /// #333: the render layer is macro bodies now, so the limitation this gate
    /// documented ("syn does not descend into macro bodies") is no longer
    /// acceptable. A `from_trusted` inside a `view!` must be seen.
    #[test]
    fn from_trusted_inside_a_macro_body_is_detected() {
        let src = r#"
            fn sneaky(s: &str) -> AnyView {
                view! { <div inner_html=RenderedHtml::from_trusted(s).to_string()></div> }.into_any()
            }
        "#;
        let hits = violations(src).unwrap();
        assert_eq!(hits.len(), 1, "macro body must be scanned: {hits:?}");
        assert_eq!(hits[0].1, "sneaky");
    }

    #[test]
    fn from_trusted_nested_deeper_in_macro_groups_is_detected() {
        // The token walk recurses through nested `Group`s, so depth is not a hiding
        // place — a `view!` body is groups all the way down.
        let src = r#"
            fn sneaky(s: &str) -> AnyView {
                view! { <div>{ move || RenderedHtml::from_trusted(s).to_string() }</div> }.into_any()
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn a_marked_site_inside_a_macro_body_passes() {
        let src = r#"
            fn seeded(s: &str) -> AnyView {
                // rendered-html-from-trusted:allow wire DTO our own server serialized (#445)
                view! { <div inner_html=RenderedHtml::from_trusted(s).to_string()></div> }.into_any()
            }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_bare_from_trusted_inside_a_macro_body_is_flagged() {
        // No qualifier at all (a `use`-imported or aliased door) — the leaf is the
        // population, so it is guarded regardless of how it was reached.
        let src = r#"
            fn sneaky(s: &str) -> AnyView {
                view! { <div inner_html=from_trusted(s)></div> }.into_any()
            }
        "#;
        assert_eq!(violations(src).unwrap(), vec![(3, "sneaky".to_string())]);
    }

    #[test]
    fn a_macro_body_in_a_test_fn_is_exempt() {
        let src = r#"
            #[test]
            fn t() {
                let _ = view! { <div inner_html=RenderedHtml::from_trusted("x")></div> };
            }
        "#;
        assert!(violations(src).unwrap().is_empty());
    }

    #[test]
    fn parse_failure_is_an_error() {
        assert!(violations("fn broken( {{{ not valid").is_err());
    }

    /// The verdict fires at **unresolvable-qualifier** sites as well as at the door's
    /// own, so it must not make claims that are false there: that *this* site is
    /// `RenderedHtml`'s door, or that a string minted here reaches the DOM. Since #790
    /// the verdict may name `RenderedHtml` again, but only disjunctively — "either it is
    /// `RenderedHtml`'s door … or its qualifier could not be resolved" — which is true at
    /// both kinds of site. Naming the type while explaining why the gate exists is fine
    /// and stays; the assertion is about what the message claims *of this site*, not
    /// about which words appear in it.
    ///
    /// Checked on the violation line alone; the recovery paragraph discusses
    /// `RenderedHtml` at length and should.
    #[test]
    fn the_verdict_claims_nothing_false_at_an_unresolvable_site() {
        let scanned = vec![(
            "common/src/media.rs".to_string(),
            "fn detect(n: &str) -> ContentType { ContentType::from_trusted(n) }\n".to_string(),
        )];
        let detail = problems(&scanned).expect("a problem");
        let verdict = detail.lines().next().expect("a violation line");
        assert!(
            !verdict.contains("RenderedHtml::from_trusted"),
            "must not call this site RenderedHtml's door: {verdict}"
        );
        assert!(
            !verdict.contains("minted here"),
            "must not claim this site mints unescaped HTML: {verdict}"
        );
        assert!(
            !verdict.contains("trusted-rebuild door"),
            "the pre-#778 wording described a population this gate no longer has: {verdict}"
        );
        assert!(verdict.contains("from_trusted"), "{verdict}");
    }

    #[test]
    fn problems_reports_file_line_and_recovery() {
        let scanned = vec![(
            "web/src/x.rs".to_string(),
            "fn sneaky(raw: String) -> RenderedHtml { RenderedHtml::from_trusted(raw) }\n"
                .to_string(),
        )];
        let detail = problems(&scanned).expect("a problem");
        assert!(detail.contains("web/src/x.rs:1"));
        assert!(detail.contains("is not marked"));
        assert!(detail.contains("rendered-html-from-trusted:allow"));
    }

    #[test]
    fn problems_is_none_for_a_fully_marked_tree() {
        let scanned = vec![
            (
                "common/src/render.rs".to_string(),
                "fn deserialize_rendered_html(s: String) -> RenderedHtml {\n    // rendered-html-from-trusted:allow wire DTO (#445)\n    RenderedHtml::from_trusted(s)\n}\n"
                    .to_string(),
            ),
            (
                "storage/src/posts.rs".to_string(),
                "#[cfg(test)]\nmod t {\n  fn f() { let _ = RenderedHtml::from_trusted(\"x\"); }\n}\n"
                    .to_string(),
            ),
        ];
        assert_eq!(problems(&scanned), None);
    }

    #[test]
    fn problems_surfaces_a_parse_failure_with_the_file() {
        let scanned = vec![("common/src/broken.rs".to_string(), "fn ( {{{".to_string())];
        let detail = problems(&scanned).expect("a hard error");
        assert!(detail.contains("common/src/broken.rs"));
        assert!(detail.contains("cannot parse"));
    }
}
