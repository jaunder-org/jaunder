//! The `sqlx-newtype-decode` static check (#715): every sqlx decode under
//! `storage/src` that lands in the `i64` family must be justified.
//!
//! The sibling `sqlx-newtype-bind` polices *binds*. Nothing policed *decodes*, so
//! `query_scalar::<_, i64>` on a `RETURNING post_id` was invisible to it and to the
//! three audits that preceded it — each of which searched for the one spelling its
//! author had in mind and reported done (#686's field-name pass missed five tuple
//! sites; its tuple pass then missed every `query_scalar`).
//!
//! **This gate enumerates; it does not search**
//! (`docs/adr/0085-static-type-safety-gates-enumerate.md`). It reads **no SQL**: it
//! does not look for `*_id` to decide something is an
//! id, and it does not look for `COUNT(` to decide something is a count. Both are
//! pattern searches, and either one hands the blind spot straight back —
//! `SELECT post_id FROM t WHERE (SELECT COUNT(*) …) > 0` defeats the second while
//! looking perfectly safe. Instead: **every** `i64`-family decode target is a failure
//! unless an [`ALLOWLIST`] entry names that exact decode. A construct the gate has
//! never seen fails *because it recognised nothing*, which is the only claim a static
//! check can honestly make.
//!
//! # The population — decode targets whose type is written down
//!
//! `syn` has no type inference, so the population is defined by *where the type is
//! declared*. One record per decode call (`query_scalar`, `query_as`, `get`,
//! `try_get`), whose target is the **nearest declared type**:
//!
//! 1. a turbofish on the call itself — `query_scalar::<_, i64>(…)`;
//! 2. else the enclosing `let`'s ascription — `let id: i64 = query_scalar(…)`;
//! 3. else the enclosing `fn`'s return type — `scalar_i64(…) -> Result<i64, _>`.
//!
//! Precedence is load-bearing, not tidiness. `postgres/backup.rs`'s `schema_version`
//! is a `-> Result<i64, _>` fn whose body is `query_scalar::<_, Option<i64>>(…)?`, so
//! rules 1 and 3 both fire; recording both would make the allowlist's declared counts
//! unmatchable and the gate would fail on a clean tree.
//!
//! A `let` or `fn` covering several calls yields one record **each** — `backup.rs`'s
//! table counts are two `query_scalar`s under one `let live_count: i64 = match {…}`.
//!
//! Separately, **declared decode targets** are policed per field: a
//! `#[derive(FromRow)]` struct's fields and a tuple `type` alias's elements. `syn`
//! cannot tell a `query_as` target alias from any other tuple alias, so this polices
//! every tuple alias under the root — today that is only `feed_cache.rs`'s
//! `CacheTuple`, so the reach costs nothing. It is what stops a future
//! `struct PostRow { revision_id: i64 }` from decoding an id into a primitive
//! invisibly.
//!
//! # What this gate cannot read, stated rather than papered over
//!
//! - **A `.get`/`try_get` with neither turbofish nor ascription.** `syn` cannot tell
//!   `sqlx::Row::get` from `serde_json::Map::get`, and both live under the root —
//!   `postgres/backup.rs` and `sqlite/backup.rs` each bind a JSON map value that way.
//!   Keying on the receiver name (`row` vs `r`) to separate them would be exactly the
//!   pattern search this gate forbids, **and** would miss the real sites.
//! - **A decode whose type is pinned only by later use** — an unascribed `let` whose
//!   value is later pushed into a `Vec<i64>`.
//!
//! Neither occurs as an id decode today. They are recorded here so the boundary is
//! inherited by the next audit rather than rediscovered.
//!
//! The boundary is visible in the tree, and the shape is worth recognising: the two
//! feed-events dialects decode the same `attempts` column, and only one is policed.
//! SQLite ascribes `let attempts: i64`, so it is in population and carries an
//! allowlist entry; Postgres reads it unascribed straight into a struct field, so the
//! field's declared type pins it and the call is invisible here. Same act, two
//! spellings, one policed — which is the honest cost of a population defined by where
//! the type is written down, not a gap to paper over with a heuristic.
//!
//! The mirror of that boundary is a **latent over-bite**: an unascribed `.get(…)` on
//! something that is not a row — a `HashMap`, a JSON map — inside a function whose
//! return type is in the `i64` family would be recorded, because rule 3 supplies the
//! target. No such site exists today. If one appears, the fix is a turbofish or an
//! ascription at the call, not a receiver-name heuristic here.
//!
//! # Root
//!
//! `storage/src` only. The two `server/tests/storage/mod.rs` decodes #715 typed are
//! **not** policed: a regression there surfaces as a failing test, not as a
//! production transposition, and widening the root would drag every test `COUNT(*)`
//! into the allowlist for no safety gain.

use std::path::Path;

use quote::ToTokens;
use syn::spanned::Spanned;

use crate::files;
use crate::result::{CommandResult, StepResult};

/// Source root scanned recursively for `.rs` files.
const POLICED_ROOT: &str = "storage/src";

/// The derive crate whose `#[proc_macro_derive]`s this gate must account for.
const MACROS_LIB: &str = "macros/src/lib.rs";

/// Derives that emit the shared sqlx bridge (`macros/src/sqlx_bridge.rs::bridge`), so a
/// type carrying one is a legitimate decode target.
///
/// **This is the gate's model of the newtype families, and a wrong model fails closed** —
/// a family missing here means every decode into those types is unrecognised, so the gate
/// bites rather than waving them through. That is the whole reason reading *declaration*
/// spellings is legitimate under ADR-0085 while reading *violation* spellings is not: an
/// incomplete approval detector is loud, an incomplete violation detector is silent.
///
/// Failing closed is safe but noisy — a forgotten family would produce dozens of confusing
/// failures at once. [`macro_enumeration_problems`] turns that into a single clear message.
const BRIDGE_DERIVES: &[&str] = &["StrNewtype", "IdNewtype", "NumNewtype", "SqlxBridge"];

/// **Attribute** macros that emit the bridge. A bridge-emitting macro need not be a
/// derive: `#[macros::text_enum(sqlx, …)]` (#746) replaces the whole strum + parse-error +
/// serde stack for a closed string enum, and emits the sqlx bridge when asked.
///
/// Enumerated separately because the *approval* rule differs, not just the spelling. The
/// bridge is **opt-in** here — `#[text_enum(…)]` without `sqlx` emits no `Decode`, and
/// several enums are declared exactly that way (`Channel`, `SubscriptionStatus`,
/// `TargetKind`, `AudienceBase` are FK-normalized and bind a `&'static str` instead). So a
/// type carrying this attribute is approved **only when the `sqlx` flag is present**.
///
/// Worth noting the asymmetry: for the derives, "does it emit a bridge?" is *not* a static
/// property (`StrNewtype` suppresses it under `no_sqlx`/`secret`), so they are approved on
/// the derive alone and the module doc records the resulting over-approval. Here the flag
/// is right there in the attribute, so the gate can be exact for free.
const BRIDGE_ATTRIBUTES: &[&str] = &["text_enum"];

/// Macros in the same crate that deliberately emit **no** sqlx bridge, and why.
///
/// It exists so adding a non-bridge macro is a deliberate one-line statement rather than a
/// silent omission — [`macro_enumeration_problems`] requires every macro to be in one list
/// or the other.
const NON_BRIDGE_MACROS: &[(&str, &str)] = &[(
    "server",
    "the #[server] server-fn attribute (ADR-0016); nothing to do with column types",
)];

/// Every macro `source` exports — `#[proc_macro_derive(Name)]` and
/// `#[proc_macro_attribute]` alike — or the parse error.
///
/// **Both kinds, because a bridge-emitting macro need not be a derive.** #746 shipped
/// `#[macros::text_enum(sqlx, …)]` as an attribute, and a gate that enumerated only
/// derives would have declared itself complete while the newest bridge family was
/// invisible to it — the exact self-blindness this check exists to prevent.
///
/// Deliberately **not** "which macros reach `sqlx_bridge::bridge()`". That is not a
/// property `syn` can decide: the call is hops deep through module-shadowing local
/// functions, and for `StrNewtype` it is conditional on the derive's own attributes
/// (`no_sqlx` / `secret` suppress it), so "reaches `bridge()`" is not static at all.
/// Enumerating the declarations and forcing each into one of two lists gets the same
/// guarantee from something that can actually be read.
fn declared_macros(source: &str) -> Result<Vec<String>, String> {
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut out = Vec::new();
    for item in &file.items {
        let syn::Item::Fn(f) = item else { continue };
        for attr in &f.attrs {
            if attr.path().is_ident("proc_macro_attribute") {
                // An attribute macro is named by the function it decorates.
                out.push(f.sig.ident.to_string());
            } else if attr.path().is_ident("proc_macro_derive") {
                // `#[proc_macro_derive(Name)]` or `#[proc_macro_derive(Name, attributes(..))]`
                // — the derive's name is the first ident in the list either way.
                if let Ok(list) = attr.meta.require_list() {
                    if let Some(name) = list.tokens.clone().into_iter().find_map(|t| match t {
                        proc_macro2::TokenTree::Ident(i) => Some(i.to_string()),
                        _ => None,
                    }) {
                        out.push(name);
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Every macro name the gate claims to know, bridge-emitting or not.
fn known_macros() -> Vec<&'static str> {
    BRIDGE_DERIVES
        .iter()
        .chain(BRIDGE_ATTRIBUTES)
        .copied()
        .chain(NON_BRIDGE_MACROS.iter().map(|(n, _)| *n))
        .collect()
}

/// Failures where the gate's macro lists and the macro crate disagree, in either
/// direction.
///
/// A macro the gate has never heard of is the dangerous case (its types silently stop
/// being approved); a listed macro that no longer exists is the stale case (the model has
/// drifted). Both are one clear message here instead of a scatter of decode failures.
fn macro_enumeration_problems(source: &str) -> Vec<String> {
    let declared = match declared_macros(source) {
        Ok(d) => d,
        Err(e) => {
            return vec![format!(
                "{MACROS_LIB}: {e} — this gate's approved-type set is derived from the macros \
                 declared here, so a file it cannot parse silently shrinks what it approves."
            )]
        }
    };
    let mut lines = Vec::new();
    for name in &declared {
        if !known_macros().contains(&name.as_str()) {
            lines.push(format!(
                "{MACROS_LIB}: `{name}` is declared but this gate does not know it. If it emits \
                 the sqlx bridge, add it to BRIDGE_DERIVES or BRIDGE_ATTRIBUTES so types carrying \
                 it are approved decode targets; if it does not, add it to NON_BRIDGE_MACROS with \
                 a reason. Leaving it out is not neutral — every decode into a `{name}` type would \
                 fail as unrecognised."
            ));
        }
    }
    for name in BRIDGE_DERIVES.iter().chain(BRIDGE_ATTRIBUTES) {
        if !declared.iter().any(|d| d == name) {
            lines.push(format!(
                "{MACROS_LIB}: `{name}` is listed as bridge-emitting but is no longer declared \
                 there. Delete it — a stale entry means this gate is approving types on the \
                 strength of a macro that does not exist."
            ));
        }
    }
    for (name, _) in NON_BRIDGE_MACROS {
        if !declared.iter().any(|d| d == name) {
            lines.push(format!(
                "{MACROS_LIB}: NON_BRIDGE_MACROS lists `{name}`, which is no longer declared \
                 there. Delete it."
            ));
        }
    }
    lines
}

/// A decode exempt from the guard, keyed by (file, function, target, what) — all
/// reflow-stable, none positional — plus how many identical sites that key covers.
///
/// **The count is load-bearing, not decoration.** `sqlx-newtype-bind`'s substring
/// needles exempt "every matching line under the policed root, not one site" (its own
/// doc says so), which is a region-scoped exemption: a new violation inside the reach
/// passes silently. The population here really does contain byte-identical decode
/// pairs that no key can separate — two `COUNT(*) FROM {table}` calls in one `match`,
/// two `query_scalar(sql)` arms in one helper. Declaring the multiplicity means
/// gaining a third is a mismatch and a failure, not a silent absorption.
struct Allowed {
    /// Path suffix under [`POLICED_ROOT`], e.g. `backup.rs` or `sqlite/mod.rs`.
    file: &'static str,
    /// Enclosing function name.
    function: &'static str,
    /// Rendered decode target, whitespace-stripped, e.g. `i64` or `Option<i64>`.
    target: &'static str,
    /// Rendered first argument of the decode call, whitespace-stripped — the SQL
    /// literal, the column name, or the expression that produced it. A **key only**:
    /// nothing in the rule branches on it.
    what: &'static str,
    /// How many identical decodes this entry covers.
    count: usize,
    /// What kind of exemption this is. Grouping only — see [`Category`].
    category: Category,
    /// Why this decode legitimately yields a primitive.
    reason: &'static str,
}

/// What kind of exemption an [`Allowed`] entry is, so the failure output can be read by
/// rationale instead of by file.
///
/// **Nothing in the matching rule or the count check branches on this.** It exists because
/// the allowlist's whole value is that a human reads it, and a flat list where a third of
/// the entries are variations of "a name out of `information_schema`" is a list people skim
/// — which is how a region exemption sneaks in wearing a dozen costumes. An enum rather
/// than a string so a typo is a compile error and the grouping order is total.
///
/// [`Category::DeferredNewtype`] is the one that carries an obligation: it means "this
/// *should* be a domain type and is not yet", so its reason must name a tracking issue —
/// enforced in [`problems`]. That makes the allowlist a worklist rather than a graveyard:
/// `DeferredNewtype` entries are the remaining unwrapped storage values, and the gate's own
/// staleness check deletes each one as it gets typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Category {
    /// `COUNT(*)`, `SELECT EXISTS(…)`, and other cardinality probes.
    CountOrExists,
    /// Names and versions read out of the database's own catalog.
    SchemaIntrospection,
    /// A blob the storage layer deliberately does not interpret — raw JSON, a cached
    /// response body.
    OpaquePayload,
    /// A value that is *deliberately* stored lossily, so the domain type would claim more
    /// than the column holds.
    DeliberateLossy,
    /// Not an sqlx decode at all — the gate's population is defined structurally, so it
    /// reaches a few constructs that are not row reads. See the module doc's over-bite
    /// note.
    NotADecodeTarget,
    /// Test scaffolding whose type comes from a generic helper's signature.
    TestScaffolding,
    /// **Residue, not a verdict.** This should be a domain type; the fix is a vertical
    /// tracked elsewhere. The reason must name the issue.
    DeferredNewtype,
}

impl Category {
    /// Every variant, so the failure footer can group in a stable, total order without a
    /// `HashMap` iteration order or a hand-kept second list.
    const ALL: &'static [Self] = &[
        Self::CountOrExists,
        Self::SchemaIntrospection,
        Self::OpaquePayload,
        Self::DeliberateLossy,
        Self::NotADecodeTarget,
        Self::TestScaffolding,
        Self::DeferredNewtype,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::CountOrExists => "count-or-exists",
            Self::SchemaIntrospection => "schema-introspection",
            Self::OpaquePayload => "opaque-payload",
            Self::DeliberateLossy => "deliberate-lossy",
            Self::NotADecodeTarget => "not-a-decode-target",
            Self::TestScaffolding => "test-scaffolding",
            Self::DeferredNewtype => "deferred-newtype",
        }
    }
}

/// Whether `reason` names a tracking issue (`#` followed by at least one digit).
///
/// Only [`Category::DeferredNewtype`] requires one. A deferred entry whose reason names no
/// issue is a TODO with no owner — the shape that turns an allowlist into a graveyard.
fn names_an_issue(reason: &str) -> bool {
    reason
        .split('#')
        .skip(1)
        .any(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// Every decode that is genuinely primitive, each with its reason.
///
/// **No entry here may name a decode that yields an id.** That is the whole point:
/// this list is the complete population of legitimate `i64` decodes under the root,
/// and anything not on it is a failure.
const ALLOWLIST: &[Allowed] = &[
    Allowed {
        file: "backup.rs",
        function: "backup_covers_every_table_or_deliberately_excludes_it",
        target: "i64",
        what: "\"SELECTCOUNT(*)FROMsqlite_masterWHEREtype='table'ANDnameNOTLIKE'sqlite_%'\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "COUNT(*) of live SQLite tables, checked against the backup manifest",
    },
    Allowed {
        file: "backup.rs",
        function: "backup_covers_every_table_or_deliberately_excludes_it",
        target: "i64",
        what: "\"SELECTCOUNT(*)FROMinformation_schema.tables\\WHEREtable_schema='public'ANDtable_type='BASETABLE'\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "COUNT(*) of live Postgres tables, the dialect twin of the SQLite arm above",
    },
    Allowed {
        file: "backup.rs",
        function: "database_is_empty_ignores_only_seeded_lookups",
        target: "i64",
        what: "&format!(\"SELECTCOUNT(*)FROM{table}\")",
        count: 2,
        category: Category::CountOrExists,
        reason: "COUNT(*) per seeded lookup table; the two dialect arms are byte-identical",
    },
    Allowed {
        file: "sqlite/mod.rs",
        function: "database_is_empty",
        target: "i64",
        what: "&format!(\"SELECTEXISTS(SELECT1FROM{}LIMIT1)\",crate::sql::quote_identifier(&table))",
        count: 1,
        category: Category::CountOrExists,
        reason: "SELECT EXISTS(…) decoded as i64 — SQLite has no bool; the Postgres twin decodes bool",
    },
    Allowed {
        file: "postgres/schema.rs",
        function: "every_foreign_key_is_deferrable",
        target: "i64",
        what: "\"SELECTCOUNT(*)FROMpg_constraint\\WHEREcontype='f'ANDconnamespace='public'::regnamespace\\ANDNOTcondeferrable\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "COUNT(*) of non-deferrable FK constraints",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "schema_version",
        target: "Option<i64>",
        what: "\"SELECTMAX(version)FROM_sqlx_migrations\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "MAX(version) migration version, NULL on an empty migrations table",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "schema_version",
        target: "Option<i64>",
        what: "\"SELECTMAX(version)FROM_sqlx_migrations\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "MAX(version) migration version, the dialect twin of the Postgres one",
    },
    Allowed {
        file: "test_support.rs",
        function: "scalar_i64",
        target: "Result<i64,sqlx::Error>",
        what: "sql",
        count: 2,
        category: Category::TestScaffolding,
        reason: "Generic test scalar helper; SQL is a runtime &str and the type comes from the fn return",
    },
    Allowed {
        file: "subscriptions.rs",
        function: "is_subscriber",
        target: "(i64,)",
        what: "DB::IS_ACTIVE_SUBSCRIBER",
        count: 1,
        category: Category::CountOrExists,
        reason: "Existence flag, not an id — subscriptions.rs's own bound comment says so",
    },
    Allowed {
        file: "sqlite/feed_events.rs",
        function: "claim_pending_batch",
        target: "i64",
        what: "\"attempts\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "attempts retry counter, narrowed to i32 for the record field",
    },
];

/// One decode site: where it is, and what it decodes into.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DecodeSite {
    /// Enclosing function name; empty at item level (a declared target type).
    function: String,
    /// Rendered decode target, whitespace-stripped.
    target: String,
    /// Rendered first argument / field name. A key and a message, never a decision.
    what: String,
    line: usize,
}

/// Renders `t` to source text with all whitespace removed, so the key is stable
/// against rustfmt reflow and `syn`'s token spacing (`Option < i64 >` → `Option<i64>`).
fn render<T: ToTokens>(t: &T) -> String {
    t.to_token_stream()
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Whether `ty` resolves to the `i64` family, recursing through `Vec`, `Option`,
/// `Result`, references, and tuples — so `Vec<(String, Option<i64>)>` is in
/// population and `Vec<(String, DateTime<Utc>)>` is not.
///
/// Pure, so it is unit-tested directly.
fn is_i64_family(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Path(p) => {
            let Some(last) = p.path.segments.last() else {
                return false;
            };
            if last.ident == "i64" {
                return true;
            }
            match &last.arguments {
                syn::PathArguments::AngleBracketed(args) => args.args.iter().any(|a| match a {
                    syn::GenericArgument::Type(t) => is_i64_family(t),
                    _ => false,
                }),
                _ => false,
            }
        }
        syn::Type::Tuple(t) => t.elems.iter().any(is_i64_family),
        syn::Type::Reference(r) => is_i64_family(&r.elem),
        syn::Type::Paren(p) => is_i64_family(&p.elem),
        syn::Type::Group(g) => is_i64_family(&g.elem),
        _ => false,
    }
}

/// The decode methods this gate reads, and where their target type sits in a
/// turbofish. `query_scalar::<DB, T>` puts it second; `get::<T, I>` puts it first.
fn target_index(name: &str) -> Option<usize> {
    match name {
        "query_scalar" | "query_as" => Some(1),
        "get" | "try_get" => Some(0),
        _ => None,
    }
}

/// The `n`th *type* argument in a generic-argument list, skipping lifetimes and
/// consts. Shared by the free-call path (`query_scalar::<_, T>`) and the method path
/// (`row.get::<T, _>`), which spell their turbofish differently but index it the same.
fn nth_type_of<'a>(
    args: impl Iterator<Item = &'a syn::GenericArgument>,
    n: usize,
) -> Option<syn::Type> {
    args.filter_map(|a| match a {
        syn::GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    })
    .nth(n)
}

/// The `n`th type argument of a path's angle-bracketed turbofish, if present.
fn nth_type_arg(args: &syn::PathArguments, n: usize) -> Option<syn::Type> {
    let syn::PathArguments::AngleBracketed(ab) = args else {
        return None;
    };
    nth_type_of(ab.args.iter(), n)
}

/// Walks a file collecting [`DecodeSite`]s, carrying the enclosing `fn` name/return type
/// and the enclosing `let` ascription so each call can take its **nearest** declared
/// type.
struct Scanner {
    out: Vec<DecodeSite>,
    function: String,
    fn_ret: Option<syn::Type>,
    let_ty: Option<syn::Type>,
}

impl Scanner {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            function: String::new(),
            fn_ret: None,
            let_ty: None,
        }
    }

    /// Records one decode with the nearest declared target, if that target is in the
    /// `i64` family. `turbofish` wins, then the enclosing `let`, then the `fn` return.
    fn record(&mut self, turbofish: Option<syn::Type>, what: String, span: proc_macro2::Span) {
        let target = turbofish
            .or_else(|| self.let_ty.clone())
            .or_else(|| self.fn_ret.clone());
        let Some(target) = target else {
            // Unreadable: no turbofish, no ascription, no fn return. Out of population
            // by construction — see the module doc.
            return;
        };
        if !is_i64_family(&target) {
            return;
        }
        self.out.push(DecodeSite {
            function: self.function.clone(),
            target: render(&target),
            what,
            line: span.start().line,
        });
    }

    fn visit_block_with(&mut self, name: &str, ret: Option<syn::Type>, block: &syn::Block) {
        let prev_name = std::mem::replace(&mut self.function, name.to_string());
        let prev_ret = std::mem::replace(&mut self.fn_ret, ret);
        let prev_let = self.let_ty.take();
        syn::visit::Visit::visit_block(self, block);
        self.function = prev_name;
        self.fn_ret = prev_ret;
        self.let_ty = prev_let;
    }
}

/// The declared return type of `sig`, or `None` for `-> ()`.
fn return_type(sig: &syn::Signature) -> Option<syn::Type> {
    match &sig.output {
        syn::ReturnType::Type(_, t) => Some((**t).clone()),
        syn::ReturnType::Default => None,
    }
}

impl<'ast> syn::visit::Visit<'ast> for Scanner {
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        let ret = return_type(&i.sig);
        self.visit_block_with(&i.sig.ident.to_string(), ret, &i.block);
    }

    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        let ret = return_type(&i.sig);
        self.visit_block_with(&i.sig.ident.to_string(), ret, &i.block);
    }

    fn visit_local(&mut self, i: &'ast syn::Local) {
        let ascribed = match &i.pat {
            syn::Pat::Type(pt) => Some((*pt.ty).clone()),
            _ => None,
        };
        let prev = std::mem::replace(&mut self.let_ty, ascribed);
        if let Some(init) = &i.init {
            syn::visit::Visit::visit_local_init(self, init);
        }
        self.let_ty = prev;
    }

    fn visit_expr_call(&mut self, i: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*i.func {
            if let Some(last) = p.path.segments.last() {
                let name = last.ident.to_string();
                if let Some(idx) = target_index(&name) {
                    let turbofish = nth_type_arg(&last.arguments, idx);
                    let what = i.args.first().map(render).unwrap_or_default();
                    self.record(turbofish, what, last.ident.span());
                }
            }
        }
        syn::visit::visit_expr_call(self, i);
    }

    fn visit_expr_method_call(&mut self, i: &'ast syn::ExprMethodCall) {
        let name = i.method.to_string();
        if let Some(idx) = target_index(&name) {
            let turbofish = i
                .turbofish
                .as_ref()
                .and_then(|t| nth_type_of(t.args.iter(), idx));
            let what = i.args.first().map(render).unwrap_or_default();
            self.record(turbofish, what, i.method.span());
        }
        syn::visit::visit_expr_method_call(self, i);
    }

    /// A `#[derive(FromRow)]` struct is a declared decode target: each field is a
    /// column decode, and the field's type is where the newtype belongs.
    fn visit_item_struct(&mut self, i: &'ast syn::ItemStruct) {
        let is_from_row = i
            .attrs
            .iter()
            .any(|a| a.path().is_ident("derive") && render(&a.meta).contains("FromRow"));
        if is_from_row {
            for f in &i.fields {
                if is_i64_family(&f.ty) {
                    self.out.push(DecodeSite {
                        function: String::new(),
                        target: render(&f.ty),
                        what: f
                            .ident
                            .as_ref()
                            .map(std::string::ToString::to_string)
                            .unwrap_or_default(),
                        line: f.ty.span().start().line,
                    });
                }
            }
        }
        syn::visit::visit_item_struct(self, i);
    }

    /// A tuple `type` alias is a declared `query_as` target. `syn` cannot tell one
    /// from any other tuple alias, so every tuple alias under the root is policed —
    /// see the module doc.
    fn visit_item_type(&mut self, i: &'ast syn::ItemType) {
        if let syn::Type::Tuple(t) = &*i.ty {
            for (n, elem) in t.elems.iter().enumerate() {
                if is_i64_family(elem) {
                    self.out.push(DecodeSite {
                        function: String::new(),
                        target: render(elem),
                        what: format!("{}.{n}", i.ident),
                        line: elem.span().start().line,
                    });
                }
            }
        }
        syn::visit::visit_item_type(self, i);
    }
}

/// Every `i64`-family decode in `source`, or the parse error.
///
/// A file that will not parse is **not** silently skipped: an unparsed file is a file
/// the gate cannot see, and a gate that quietly shrinks its own population is the
/// failure this whole design exists to prevent. Pure, so it is unit-tested directly.
fn decodes(source: &str) -> Result<Vec<DecodeSite>, String> {
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut scanner = Scanner::new();
    syn::visit::Visit::visit_file(&mut scanner, &file);
    Ok(scanner.out)
}

/// Whether `path` is exactly `POLICED_ROOT/relative`.
///
/// **Exact, not a suffix match.** Three files under the root are named `backup.rs`,
/// and two of them declare a function called `schema_version`, so a suffix match would
/// let the `backup.rs` entry reach decodes in `postgres/backup.rs` — a region-scoped
/// exemption of exactly the kind the site-scoping rule exists to stop. The other key
/// fields happen to keep it honest today; that is luck, not design.
fn file_matches(path: &str, relative: &str) -> bool {
    path.strip_prefix(POLICED_ROOT)
        .map(|rest| rest.trim_start_matches('/'))
        == Some(relative)
}

/// Whether `entry` names `decode` in `path`.
fn entry_matches(entry: &Allowed, path: &str, decode: &DecodeSite) -> bool {
    file_matches(path, entry.file)
        && entry.function == decode.function
        && entry.target == decode.target
        && entry.what == decode.what
}

/// The failure detail for every unjustified decode and every allowlist entry whose
/// declared count no longer matches the tree, or `None` when the population is exactly
/// accounted for. Pure given the `(path, source)` pairs, so it is unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut found: Vec<(String, DecodeSite)> = Vec::new();
    let mut lines = Vec::new();
    for (path, source) in scanned {
        match decodes(source) {
            Ok(ds) => found.extend(ds.into_iter().map(|d| (path.clone(), d))),
            Err(e) => lines.push(format!(
                "{path}: {e} — an unparsed file is invisible to this gate, which is exactly the \
                 blind spot it exists to close. Fix the file or the parser; do not skip it."
            )),
        }
    }

    // Unjustified decodes: nothing in the allowlist names them.
    for (path, d) in &found {
        if !ALLOWLIST.iter().any(|e| entry_matches(e, path, d)) {
            lines.push(format!(
                "{path}:{}: `{}` decodes into `{}` — an sqlx decode in the `i64` family with no \
                 allowlist entry. If this column is an id, decode it into its id newtype directly \
                 (the ADR-0071 bridge makes `query_scalar::<_, PostId>` work) and delete the \
                 hand re-wrap. If it is genuinely a primitive, add an ALLOWLIST entry with a \
                 written reason. This gate reads no SQL, so it cannot tell which — that judgement \
                 is yours to record.",
                d.line, d.what, d.target
            ));
        }
    }

    // Stale or drifted entries: an allowlist that stops tracking the tree is an
    // allowlist that has silently become a region exemption.
    for e in ALLOWLIST {
        let seen = found
            .iter()
            .filter(|(path, d)| entry_matches(e, path, d))
            .count();
        if seen != e.count {
            lines.push(format!(
                "{}::{}: allowlist entry for `{}` declares {} site(s), the tree has {}. {}",
                e.file,
                e.function,
                e.target,
                e.count,
                seen,
                if seen == 0 {
                    "The decode is gone — delete the entry."
                } else {
                    "Re-justify each site, then update the count."
                }
            ));
        }
    }

    lines.extend(allowlist_self_problems(ALLOWLIST));

    if lines.is_empty() {
        return None;
    }
    lines.push(
        "  recovery: this gate enumerates rather than searching — it has no idea which columns \
         are ids, and deliberately so, because every audit that searched for the id-ish spelling \
         missed the sites spelled another way (#686, #715). Every i64-family decode is therefore \
         either typed or listed. Currently exempt, by rationale:"
            .to_string(),
    );
    for category in Category::ALL {
        let mut group = ALLOWLIST
            .iter()
            .filter(|a| a.category == *category)
            .peekable();
        if group.peek().is_none() {
            continue;
        }
        lines.push(format!("    [{}]", category.label()));
        for a in group {
            lines.push(format!(
                "      - {}::{} `{}` ×{}: {}",
                a.file, a.function, a.target, a.count, a.reason
            ));
        }
    }
    Some(lines.join("\n"))
}

/// Faults in an allowlist itself, independent of the tree.
///
/// A gate that polices the source but not its own exemption list is blind in the one place
/// it can least afford to be — the same rule as failing on an unparseable file, applied
/// inward (ADR-0085 principle 6).
///
/// Takes the list rather than reading [`ALLOWLIST`] directly so the tests drive *this*
/// function with synthetic entries instead of re-implementing the rule beside it.
fn allowlist_self_problems(allowlist: &[Allowed]) -> Vec<String> {
    let mut lines = Vec::new();

    // Duplicate keys. Matching is `.any(…)` and the count check is per-entry, so two
    // entries with the same key each declaring 1 would BOTH pass while double-covering a
    // single site — and deleting the decode would then need two edits to go green, which
    // is exactly how a stale exemption survives.
    for (i, a) in allowlist.iter().enumerate() {
        if let Some(dup) = allowlist[..i].iter().find(|b| {
            (b.file, b.function, b.target, b.what) == (a.file, a.function, a.target, a.what)
        }) {
            lines.push(format!(
                "{}::{} `{}`: two allowlist entries share one key ({} and {}). Merge them into \
                 one entry and state the combined multiplicity in `count` — two entries covering \
                 one site can never go stale together.",
                a.file, a.function, a.target, dup.reason, a.reason
            ));
        }
    }

    // A deferred entry with no issue is a TODO with no owner.
    for a in allowlist {
        if a.category == Category::DeferredNewtype && !names_an_issue(a.reason) {
            lines.push(format!(
                "{}::{} `{}`: a `deferred-newtype` entry must name the issue tracking the fix \
                 (e.g. \"…, deferred to #750\"). Without one this is not deferred work, it is an \
                 exemption with a sympathetic label.",
                a.file, a.function, a.target
            ));
        }
    }

    lines
}

/// Scan every Rust file under [`POLICED_ROOT`] and push the result step. A missing
/// root is a hard failure, so a moved or renamed tree can never quietly disable the
/// guard.
pub fn run(result: &mut CommandResult) {
    let files = match files::with_extension(Path::new(POLICED_ROOT), "rs") {
        Ok(files) => files,
        Err(e) => {
            result.push(
                StepResult::fail("sqlx-newtype-decode")
                    .detail(format!("cannot scan {POLICED_ROOT}: {e}")),
            );
            return;
        }
    };
    // A file that cannot be READ is as invisible as one that cannot be PARSED, so it
    // fails the same way. `read_to_string(p).ok()` would have dropped it from the
    // population silently — the precise failure this gate exists to prevent, committed
    // by the gate itself.
    let mut scanned: Vec<(String, String)> = Vec::with_capacity(files.len());
    let mut unreadable = Vec::new();
    for p in &files {
        let path = p.display().to_string();
        match std::fs::read_to_string(p) {
            Ok(s) => scanned.push((path, s)),
            Err(e) => unreadable.push(format!(
                "{path}: cannot read: {e} — an unread file is invisible to this gate, so it \
                 fails rather than shrinking the population."
            )),
        }
    }

    // The derive crate is read the same way and fails the same way: this gate's model of
    // the newtype families comes from it, so a file it cannot read is a model it cannot
    // check.
    match std::fs::read_to_string(MACROS_LIB) {
        Ok(s) => unreadable.extend(macro_enumeration_problems(&s)),
        Err(e) => unreadable.push(format!(
            "{MACROS_LIB}: cannot read: {e} — this gate's approved-type set is derived from the \
             derives declared there, so it fails rather than assuming its own list is current."
        )),
    }

    let detail = match (problems(&scanned), unreadable.is_empty()) {
        (None, true) => {
            result.push(StepResult::ok("sqlx-newtype-decode"));
            return;
        }
        (found, _) => {
            let mut lines = unreadable;
            lines.extend(found);
            lines.join("\n")
        }
    };
    result.push(StepResult::fail("sqlx-newtype-decode").detail(detail));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Targets of every decode the scanner found, for terse assertions.
    fn targets(src: &str) -> Vec<String> {
        decodes(src)
            .expect("parses")
            .into_iter()
            .map(|d| d.target)
            .collect()
    }

    // ---- the population: each of the three call-site type positions bites ----

    #[test]
    fn turbofish_target_is_collected() {
        let src = r#"fn f() { sqlx::query_scalar::<_, i64>("SELECT a").fetch_one(p); }"#;
        assert_eq!(targets(src), vec!["i64"]);
    }

    #[test]
    fn ascribed_let_target_is_collected() {
        let src = r#"fn f() { let id: i64 = sqlx::query_scalar("SELECT a").fetch_one(p); }"#;
        assert_eq!(targets(src), vec!["i64"]);
    }

    #[test]
    fn vec_of_tuple_with_option_i64_is_collected() {
        // Site #9's shape (`posts.rs`'s `pa.audience_id`): the type is on the `let`,
        // wrapped in a `Vec` and a tuple. #686's audit required a turbofish and so
        // could not see it. A gate that misses this misses the site it was built for.
        let src = r#"
            fn f() {
                let rows: Vec<(String, Option<i64>)> = sqlx::query_as("SELECT a, b").fetch_all(p);
            }
        "#;
        assert_eq!(targets(src), vec!["Vec<(String,Option<i64>)>"]);
    }

    #[test]
    fn ascribed_row_get_is_collected() {
        // Both feed-events mappers had this shape before #715 swept them, so only a
        // synthetic source can still prove the gate bites here.
        let src = r#"fn f() { let id: i64 = r.get("id"); }"#;
        assert_eq!(targets(src), vec!["i64"]);
    }

    #[test]
    fn fn_return_type_covers_every_arm() {
        // `test_support.rs`'s `scalar_i64`: one fn return type, a decode in EACH arm.
        // Allowlist entry `scalar_i64 ×2` depends on this yielding two records, not one.
        let src = r#"
            async fn scalar_i64(&self, sql: &str) -> Result<i64, sqlx::Error> {
                match self {
                    A(pool) => sqlx::query_scalar(sql).fetch_one(pool).await,
                    B(pool) => sqlx::query_scalar(sql).fetch_one(pool).await,
                }
            }
        "#;
        assert_eq!(targets(src).len(), 2);
    }

    #[test]
    fn one_let_over_two_calls_yields_two_records() {
        // `backup.rs`'s table count: one `let live_count: i64 = match {…}` over two
        // dialect arms. Collapsing them to one record would make the counts unmatchable.
        let src = r#"
            fn f() {
                let live_count: i64 = match pool {
                    A(p) => sqlx::query_scalar("SELECT COUNT(*) FROM a").fetch_one(p).await?,
                    B(p) => sqlx::query_scalar("SELECT COUNT(*) FROM b").fetch_one(p).await?,
                };
            }
        "#;
        assert_eq!(targets(src).len(), 2);
    }

    #[test]
    fn turbofish_wins_over_the_enclosing_fn_return() {
        // `postgres/backup.rs`'s `schema_version` is `-> Result<i64, _>` around a
        // `query_scalar::<_, Option<i64>>`. Both positions fire; precedence must pick
        // the turbofish and record ONE site, or the seed allowlist can never match and
        // the gate fails on a clean tree.
        let src = r#"
            async fn schema_version(c: &mut C) -> Result<i64, BackupError> {
                Ok(sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(version) FROM m")
                    .fetch_one(c).await?.unwrap_or_default())
            }
        "#;
        assert_eq!(targets(src), vec!["Option<i64>"]);
    }

    #[test]
    fn ascription_wins_over_the_enclosing_fn_return() {
        let src = r#"
            fn f() -> Result<i64, E> {
                let n: Option<i64> = sqlx::query_scalar("SELECT MAX(v) FROM m").fetch_one(c)?;
                Ok(n.unwrap_or_default())
            }
        "#;
        assert_eq!(targets(src), vec!["Option<i64>"]);
    }

    // ---- declared decode targets ----

    #[test]
    fn from_row_struct_field_is_collected() {
        let src = r#"
            #[derive(sqlx::FromRow)]
            struct R { revision_id: i64, slug: Slug }
        "#;
        assert_eq!(targets(src), vec!["i64"]);
    }

    #[test]
    fn tuple_alias_element_is_collected() {
        let src = "type CacheTuple = (FeedPath, i64, DateTime<Utc>);";
        assert_eq!(targets(src), vec!["i64"]);
    }

    #[test]
    fn a_plain_struct_without_from_row_is_not_collected() {
        let src = "struct NotARow { count: i64 }";
        assert!(targets(src).is_empty());
    }

    // ---- the gate must not over-bite ----

    #[test]
    fn a_typed_decode_is_not_collected() {
        let src = r#"fn f() { sqlx::query_scalar::<_, PostId>("SELECT post_id").fetch_one(p); }"#;
        assert!(targets(src).is_empty());
    }

    #[test]
    fn bool_and_string_targets_are_not_collected() {
        // The out-of-population classes: `COUNT(*) > 0` flags and table-name reads are
        // never in the population at all, so they need no allowlist entry.
        let src = r#"
            fn f() {
                let ok: bool = sqlx::query_scalar("SELECT COUNT(*) > 0 FROM t").fetch_one(p);
                let names = sqlx::query_scalar::<_, String>("SELECT name FROM t").fetch_all(p);
            }
        "#;
        assert!(targets(src).is_empty());
    }

    #[test]
    fn a_typed_tuple_decode_is_not_collected() {
        // #686's `query_as` tuples stay green for free — covering them here means they
        // can never silently regress.
        let src = r#"fn f() { sqlx::query_as::<_, (UserId, Email)>("SELECT a, b").fetch_one(p); }"#;
        assert!(targets(src).is_empty());
    }

    #[test]
    fn struct_literal_row_get_is_not_collected() {
        // The destination field's declared type pins the decode, and that declaration
        // is itself policed as a declared target — so the invariant lives on the
        // struct, where the newtype belongs.
        let src = r#"fn f() { Rec { id: r.get("id"), attempts: r.get("attempts") }; }"#;
        assert!(targets(src).is_empty());
    }

    #[test]
    fn an_unascribed_get_is_not_collected() {
        // `syn` cannot tell `sqlx::Row::get` from `serde_json::Map::get`, and both live
        // under the policed root. Guessing by receiver name would be the pattern search
        // this gate forbids — so this class is out of population and documented.
        let src = r#"fn f() { let value = row.get(&column.name).ok_or_else(|| e)?; }"#;
        assert!(targets(src).is_empty());
    }

    #[test]
    fn a_hoist_does_not_leak_into_the_next_function() {
        // Scope discipline: an ascription in one fn must not type a decode in the next.
        let src = r#"
            fn a() { let n: i64 = sqlx::query_scalar("SELECT COUNT(*)").fetch_one(p); }
            fn b() { sqlx::query_scalar("SELECT slug").fetch_one(p); }
        "#;
        assert_eq!(targets(src).len(), 1);
    }

    // ---- the allowlist is site-scoped, and its count is load-bearing ----

    /// A source with `n` identical `COUNT(*)` decodes inside `scalar_i64`, matching the
    /// shape of the real `test_support.rs` entry.
    fn identical_sites(n: usize) -> String {
        let arms: String = (0..n)
            .map(|_| "P(pool) => sqlx::query_scalar(sql).fetch_one(pool).await,\n".to_string())
            .collect();
        format!(
            "async fn scalar_i64(&self, sql: &str) -> Result<i64, sqlx::Error> {{ match self {{ {arms} }} }}"
        )
    }

    #[test]
    fn an_entry_count_of_two_passes_on_two_and_fails_on_three() {
        // The property that stops an entry becoming a region exemption. The real
        // `test_support.rs` entry declares 2; a third identical decode must NOT be
        // silently absorbed by it.
        //
        // Scanning one file in isolation legitimately makes the other nine entries
        // stale, so the assertion is scoped to this entry's own message rather than to
        // `problems` returning `None`.
        let two = vec![(
            "storage/src/test_support.rs".to_string(),
            identical_sites(2),
        )];
        let two_detail = problems(&two).unwrap_or_default();
        // Match the failure phrasing, not the bare key — the recovery footer lists
        // every entry by key, including this one.
        assert!(
            !two_detail.contains("test_support.rs::scalar_i64: allowlist entry"),
            "two sites match the declared count, so this entry must not complain: {two_detail}"
        );

        let three = vec![(
            "storage/src/test_support.rs".to_string(),
            identical_sites(3),
        )];
        let detail = problems(&three).expect("a third identical decode must fail");
        assert!(
            detail.contains("declares 2 site(s), the tree has 3"),
            "{detail}"
        );
    }

    #[test]
    fn an_entry_exempts_only_the_decode_it_names() {
        // A different `i64` decode in the same allowlisted function is still a failure —
        // the entry covers one decode, never a region.
        let src = "async fn scalar_i64(&self, sql: &str) -> Result<i64, sqlx::Error> { \
                   match self { \
                   P(pool) => sqlx::query_scalar(sql).fetch_one(pool).await, \
                   Q(pool) => sqlx::query_scalar(sql).fetch_one(pool).await, \
                   R(pool) => sqlx::query_scalar(\"SELECT owner_id FROM t\").fetch_one(pool).await, \
                   } }";
        let detail = problems(&[("storage/src/test_support.rs".to_string(), src.to_string())])
            .expect("the unlisted sibling decode must fail");
        assert!(detail.contains("SELECTowner_idFROMt"), "{detail}");
    }

    #[test]
    fn an_unallowlisted_id_decode_is_flagged() {
        // The headline case: reverting any swept site must fail the gate.
        let src = r#"
            impl S {
                async fn create(&self) -> Result<UserId, E> {
                    let id = sqlx::query_scalar::<_, i64>("INSERT INTO users RETURNING user_id")
                        .fetch_one(&self.pool).await?;
                    Ok(UserId::from(id))
                }
            }
        "#;
        let detail = problems(&[("storage/src/users.rs".to_string(), src.to_string())])
            .expect("a bare i64 id decode must fail");
        assert!(detail.contains("storage/src/users.rs"), "{detail}");
        assert!(detail.contains("decodes into `i64`"), "{detail}");
    }

    #[test]
    fn a_stale_entry_with_no_matching_site_is_reported() {
        // An allowlist that stops tracking the tree has quietly become a region
        // exemption, so a vanished site is a failure too, not a free pass.
        let detail = problems(&[("storage/src/users.rs".to_string(), String::new())])
            .expect("every entry is now stale");
        assert!(
            detail.contains("The decode is gone — delete the entry."),
            "{detail}"
        );
    }

    #[test]
    fn an_entry_does_not_reach_a_same_named_file_in_a_subdirectory() {
        // `backup.rs`, `postgres/backup.rs` and `sqlite/backup.rs` all exist, and the
        // last two both declare `schema_version`. A suffix match would let one entry
        // exempt decodes in a sibling file — a region exemption by the back door.
        assert!(file_matches("storage/src/backup.rs", "backup.rs"));
        assert!(!file_matches("storage/src/postgres/backup.rs", "backup.rs"));
        assert!(file_matches(
            "storage/src/postgres/backup.rs",
            "postgres/backup.rs"
        ));
        assert!(!file_matches("storage/src/sqlite/mod.rs", "mod.rs"));
    }

    #[test]
    fn an_unparseable_file_is_a_failure_not_a_skip() {
        let detail = problems(&[("storage/src/broken.rs".to_string(), "fn f( {{{".to_string())])
            .expect("an unparsed file must fail");
        assert!(detail.contains("invisible to this gate"), "{detail}");
    }

    // ---- the gate's model of the macros crate polices itself ----

    /// A macros crate declaring exactly `derives` and `attributes`.
    fn macros_lib_with(derives: &[&str], attributes: &[&str]) -> String {
        let d = derives.iter().map(|n| {
            format!("#[proc_macro_derive({n}, attributes(x))]\npub fn {n}_d(item: TokenStream) -> TokenStream {{ item }}\n")
        });
        let a = attributes.iter().map(|n| {
            format!("#[proc_macro_attribute]\npub fn {n}(a: TokenStream, i: TokenStream) -> TokenStream {{ i }}\n")
        });
        d.chain(a).collect()
    }

    /// The macros crate exactly as the gate currently models it.
    fn macros_lib_as_modelled() -> String {
        let attrs: Vec<&str> = BRIDGE_ATTRIBUTES
            .iter()
            .copied()
            .chain(NON_BRIDGE_MACROS.iter().map(|(n, _)| *n))
            .collect();
        macros_lib_with(BRIDGE_DERIVES, &attrs)
    }

    #[test]
    fn the_shipped_macro_lists_match_the_macros_crate() {
        // The real enforcement is in `run`, which reads macros/src/lib.rs on every gate
        // invocation. Pinning the model here means a drift shows up as a message about the
        // macro lists rather than as thirty unrelated decode failures.
        assert!(
            macro_enumeration_problems(&macros_lib_as_modelled()).is_empty(),
            "{:?}",
            macro_enumeration_problems(&macros_lib_as_modelled())
        );
    }

    #[test]
    fn a_macro_the_gate_has_never_heard_of_is_one_clear_failure() {
        // #746's `SqlxBridge` arrived exactly this way, and the gate caught it on the
        // first run after the rebase. Failing closed is correct but noisy; this message is
        // what makes the cause obvious.
        let mut derives = BRIDGE_DERIVES.to_vec();
        derives.push("NewFamily");
        let attrs: Vec<&str> = BRIDGE_ATTRIBUTES
            .iter()
            .copied()
            .chain(NON_BRIDGE_MACROS.iter().map(|(n, _)| *n))
            .collect();
        let problems = macro_enumeration_problems(&macros_lib_with(&derives, &attrs));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("NewFamily"), "{problems:?}");
        assert!(
            problems[0].contains("BRIDGE_DERIVES") && problems[0].contains("BRIDGE_ATTRIBUTES"),
            "the message must name both fixes: {problems:?}"
        );
    }

    #[test]
    fn an_unknown_attribute_macro_is_caught_too() {
        // The hole #746 would have opened: `text_enum` is an ATTRIBUTE, not a derive, so a
        // check that enumerated only derives would have declared itself complete while the
        // newest bridge family was invisible to it.
        let attrs: Vec<&str> = BRIDGE_ATTRIBUTES
            .iter()
            .copied()
            .chain(NON_BRIDGE_MACROS.iter().map(|(n, _)| *n))
            .chain(std::iter::once("new_attr"))
            .collect();
        let problems = macro_enumeration_problems(&macros_lib_with(BRIDGE_DERIVES, &attrs));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("new_attr"), "{problems:?}");
    }

    #[test]
    fn a_listed_macro_that_no_longer_exists_is_a_failure() {
        // The stale direction: the gate would otherwise keep approving types on the
        // strength of a macro that has been deleted.
        let attrs: Vec<&str> = BRIDGE_ATTRIBUTES
            .iter()
            .copied()
            .chain(NON_BRIDGE_MACROS.iter().map(|(n, _)| *n))
            .collect();
        let fewer = &BRIDGE_DERIVES[..BRIDGE_DERIVES.len() - 1];
        let problems = macro_enumeration_problems(&macros_lib_with(fewer, &attrs));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains(BRIDGE_DERIVES[BRIDGE_DERIVES.len() - 1]),
            "{problems:?}"
        );
    }

    #[test]
    fn an_unparseable_macros_crate_is_a_failure_not_a_skip() {
        let problems = macro_enumeration_problems("pub fn f( {{{");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("silently shrinks"), "{problems:?}");
    }

    #[test]
    fn macro_names_are_read_from_every_declaration_spelling() {
        // A derive with and without an `attributes(..)` trailer (the name is the first
        // ident either way), and an attribute macro (named by its function).
        let src = "#[proc_macro_derive(IdNewtype)]\npub fn a(i: TokenStream) -> TokenStream { i }\n\
                   #[proc_macro_derive(StrNewtype, attributes(str_newtype))]\npub fn b(i: TokenStream) -> TokenStream { i }\n\
                   #[proc_macro_attribute]\npub fn text_enum(a: TokenStream, i: TokenStream) -> TokenStream { i }\n";
        let mut got = declared_macros(src).expect("parses");
        got.sort();
        assert_eq!(
            got,
            vec![
                "IdNewtype".to_string(),
                "StrNewtype".to_string(),
                "text_enum".to_string()
            ]
        );
    }

    // ---- the allowlist polices itself ----

    /// An entry with everything but the field under test held fixed.
    fn entry(what: &'static str, category: Category, reason: &'static str) -> Allowed {
        Allowed {
            file: "users.rs",
            function: "f",
            target: "i64",
            what,
            count: 1,
            category,
            reason,
        }
    }

    #[test]
    fn the_category_field_changes_nothing_about_matching_or_counting() {
        // A8's falsifiable form. "No code path branches on `category`" cannot be asserted
        // from a test, but its observable consequence can: two entries that differ ONLY in
        // category must behave identically for both the match and the duplicate check.
        let a = entry("\"SELECTCOUNT(*)\"", Category::CountOrExists, "a count");
        let b = entry("\"SELECTCOUNT(*)\"", Category::OpaquePayload, "a count");
        let site = DecodeSite {
            function: "f".to_string(),
            target: "i64".to_string(),
            what: "\"SELECTCOUNT(*)\"".to_string(),
            line: 1,
        };
        let path = "storage/src/users.rs";
        assert_eq!(
            entry_matches(&a, path, &site),
            entry_matches(&b, path, &site),
            "category must not affect whether an entry covers a site"
        );
        // …and differing only in category does NOT make two entries distinct, so it cannot
        // be used to sneak a second entry past the duplicate check.
        assert_eq!(
            allowlist_self_problems(&[a, b]).len(),
            1,
            "same key, still a duplicate"
        );
    }

    #[test]
    fn category_drives_only_the_deferred_obligation() {
        // The precise claim: `category` is inert for matching and counting, but it is NOT
        // decoration — `DeferredNewtype` alone carries the name-your-issue obligation.
        let ok = entry("\"a\"", Category::CountOrExists, "a count");
        assert!(allowlist_self_problems(std::slice::from_ref(&ok)).is_empty());
        let deferred = entry("\"a\"", Category::DeferredNewtype, "should be a newtype");
        assert_eq!(
            allowlist_self_problems(std::slice::from_ref(&deferred)).len(),
            1,
            "a deferred entry naming no issue must fail"
        );
    }

    #[test]
    fn two_entries_with_one_key_are_a_failure() {
        let a = entry("\"SELECTCOUNT(*)\"", Category::CountOrExists, "first");
        let b = entry("\"SELECTCOUNT(*)\"", Category::CountOrExists, "second");
        assert_eq!(allowlist_self_problems(&[a, b]).len(), 1);
    }

    #[test]
    fn distinct_keys_are_not_duplicates() {
        let a = entry("\"SELECTCOUNT(*)\"", Category::CountOrExists, "first");
        let b = entry("\"SELECTMAX(v)\"", Category::CountOrExists, "second");
        assert!(allowlist_self_problems(&[a, b]).is_empty());
    }

    #[test]
    fn the_shipped_allowlist_has_no_self_faults() {
        // The self-checks run on every gate invocation, so a bad entry would fail the gate
        // on a clean tree. Pin it here too, where the message is about the allowlist rather
        // than about whatever else was failing.
        assert!(
            allowlist_self_problems(ALLOWLIST).is_empty(),
            "{:?}",
            allowlist_self_problems(ALLOWLIST)
        );
    }

    #[test]
    fn a_deferred_newtype_entry_must_name_its_issue() {
        assert!(names_an_issue(
            "subscriber_ref should be a newtype, deferred to #750"
        ));
        assert!(!names_an_issue(
            "subscriber_ref should be a newtype one day"
        ));
        // A bare `#` with no number is a false positive waiting to happen — a reason that
        // says "the # column" must not count as a tracking reference.
        assert!(!names_an_issue("the # column is opaque"));
    }

    #[test]
    fn is_i64_family_recurses_without_over_matching() {
        let ty: syn::Type = syn::parse_quote!(Vec<(String, Option<i64>)>);
        assert!(is_i64_family(&ty));
        let ty: syn::Type = syn::parse_quote!(Vec<(String, DateTime<Utc>)>);
        assert!(!is_i64_family(&ty));
        let ty: syn::Type = syn::parse_quote!(Option<AudienceId>);
        assert!(!is_i64_family(&ty));
    }
}
