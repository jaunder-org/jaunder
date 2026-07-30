//! The `sqlx-newtype-decode` static check (#715): every sqlx decode under
//! `storage/src` that lands in the `i64` family must be justified.
//!
//! The sibling `sqlx-newtype-bind` polices *binds*. Nothing policed *decodes*, so
//! `query_scalar::<_, i64>` on a `RETURNING post_id` was invisible to it and to the
//! three audits that preceded it — each of which searched for the one spelling its
//! author had in mind and reported done (#686's field-name pass missed five tuple
//! sites; its tuple pass then missed every `query_scalar`).
//!
//! **This gate enumerates; it does not search** (see the "enumerate, don't search"
//! ADR). It reads **no SQL**: it does not look for `*_id` to decide something is an
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
    /// Why this decode legitimately yields a primitive.
    reason: &'static str,
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
        reason: "COUNT(*) of live SQLite tables, checked against the backup manifest",
    },
    Allowed {
        file: "backup.rs",
        function: "backup_covers_every_table_or_deliberately_excludes_it",
        target: "i64",
        what: "\"SELECTCOUNT(*)FROMinformation_schema.tables\\WHEREtable_schema='public'ANDtable_type='BASETABLE'\"",
        count: 1,
        reason: "COUNT(*) of live Postgres tables, the dialect twin of the SQLite arm above",
    },
    Allowed {
        file: "backup.rs",
        function: "database_is_empty_ignores_only_seeded_lookups",
        target: "i64",
        what: "&format!(\"SELECTCOUNT(*)FROM{table}\")",
        count: 2,
        reason: "COUNT(*) per seeded lookup table; the two dialect arms are byte-identical",
    },
    Allowed {
        file: "sqlite/mod.rs",
        function: "database_is_empty",
        target: "i64",
        what: "&format!(\"SELECTEXISTS(SELECT1FROM{}LIMIT1)\",crate::sql::quote_identifier(&table))",
        count: 1,
        reason: "SELECT EXISTS(…) decoded as i64 — SQLite has no bool; the Postgres twin decodes bool",
    },
    Allowed {
        file: "postgres/schema.rs",
        function: "every_foreign_key_is_deferrable",
        target: "i64",
        what: "\"SELECTCOUNT(*)FROMpg_constraint\\WHEREcontype='f'ANDconnamespace='public'::regnamespace\\ANDNOTcondeferrable\"",
        count: 1,
        reason: "COUNT(*) of non-deferrable FK constraints",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "schema_version",
        target: "Option<i64>",
        what: "\"SELECTMAX(version)FROM_sqlx_migrations\"",
        count: 1,
        reason: "MAX(version) migration version, NULL on an empty migrations table",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "schema_version",
        target: "Option<i64>",
        what: "\"SELECTMAX(version)FROM_sqlx_migrations\"",
        count: 1,
        reason: "MAX(version) migration version, the dialect twin of the Postgres one",
    },
    Allowed {
        file: "test_support.rs",
        function: "scalar_i64",
        target: "Result<i64,sqlx::Error>",
        what: "sql",
        count: 2,
        reason: "Generic test scalar helper; SQL is a runtime &str and the type comes from the fn return",
    },
    Allowed {
        file: "subscriptions.rs",
        function: "is_subscriber",
        target: "(i64,)",
        what: "DB::IS_ACTIVE_SUBSCRIBER",
        count: 1,
        reason: "Existence flag, not an id — subscriptions.rs's own bound comment says so",
    },
    Allowed {
        file: "sqlite/feed_events.rs",
        function: "claim_pending_batch",
        target: "i64",
        what: "\"attempts\"",
        count: 1,
        reason: "attempts retry counter, narrowed to i32 for the record field",
    },
];

/// One decode site: where it is, and what it decodes into.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Decode {
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

/// The `n`th type argument of an angle-bracketed turbofish, if present.
fn nth_type_arg(args: &syn::PathArguments, n: usize) -> Option<syn::Type> {
    let syn::PathArguments::AngleBracketed(ab) = args else {
        return None;
    };
    ab.args
        .iter()
        .filter_map(|a| match a {
            syn::GenericArgument::Type(t) => Some(t.clone()),
            _ => None,
        })
        .nth(n)
}

/// Walks a file collecting [`Decode`]s, carrying the enclosing `fn` name/return type
/// and the enclosing `let` ascription so each call can take its **nearest** declared
/// type.
struct Scanner {
    out: Vec<Decode>,
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
        self.out.push(Decode {
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
                .and_then(|t| {
                    t.args
                        .iter()
                        .filter_map(|a| match a {
                            syn::GenericArgument::Type(ty) => Some(ty.clone()),
                            _ => None,
                        })
                        .nth(idx)
                })
                .or(None);
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
                    self.out.push(Decode {
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
                    self.out.push(Decode {
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
fn decodes(source: &str) -> Result<Vec<Decode>, String> {
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut scanner = Scanner::new();
    syn::visit::Visit::visit_file(&mut scanner, &file);
    Ok(scanner.out)
}

/// Whether `path` ends with the allowlist entry's `file` suffix, at a path boundary
/// so `mod.rs` cannot match `sqlite/mod.rs`'s entry and vice versa.
fn file_matches(path: &str, suffix: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized == suffix
        || normalized.ends_with(&format!("/{suffix}"))
        || normalized
            .strip_prefix(POLICED_ROOT)
            .is_some_and(|rest| rest.trim_start_matches('/') == suffix)
}

/// Whether `entry` names `decode` in `path`.
fn entry_matches(entry: &Allowed, path: &str, decode: &Decode) -> bool {
    file_matches(path, entry.file)
        && entry.function == decode.function
        && entry.target == decode.target
        && entry.what == decode.what
}

/// The failure detail for every unjustified decode and every allowlist entry whose
/// declared count no longer matches the tree, or `None` when the population is exactly
/// accounted for. Pure given the `(path, source)` pairs, so it is unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut found: Vec<(String, Decode)> = Vec::new();
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

    if lines.is_empty() {
        return None;
    }
    lines.push(
        "  recovery: this gate enumerates rather than searching — it has no idea which columns \
         are ids, and deliberately so, because every audit that searched for the id-ish spelling \
         missed the sites spelled another way (#686, #715). Every i64-family decode is therefore \
         either typed or listed. Currently exempt:"
            .to_string(),
    );
    for a in ALLOWLIST {
        lines.push(format!(
            "    - {}::{} `{}` ×{}: {}",
            a.file, a.function, a.target, a.count, a.reason
        ));
    }
    Some(lines.join("\n"))
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
    let scanned: Vec<(String, String)> = files
        .iter()
        .filter_map(|p| {
            std::fs::read_to_string(p)
                .ok()
                .map(|s| (p.display().to_string(), s))
        })
        .collect();
    let step = match problems(&scanned) {
        None => StepResult::ok("sqlx-newtype-decode"),
        Some(detail) => StepResult::fail("sqlx-newtype-decode").detail(detail),
    };
    result.push(step);
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
    fn an_unparseable_file_is_a_failure_not_a_skip() {
        let detail = problems(&[("storage/src/broken.rs".to_string(), "fn f( {{{".to_string())])
            .expect("an unparsed file must fail");
        assert!(detail.contains("invisible to this gate"), "{detail}");
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
