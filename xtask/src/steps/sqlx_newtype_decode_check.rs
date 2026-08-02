//! The `sqlx-newtype-decode` static check (#715, widened by #728): every sqlx decode
//! under `storage/src` must land in an **approved column type**, or carry a written
//! reason.
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
//! looking perfectly safe.
//!
//! # The rule
//!
//! **Every leaf type of a decode target must be approved, or an [`ALLOWLIST`] entry must
//! name that exact decode.** #715 denied one primitive family, which left every other
//! primitive — and every non-primitive non-newtype — unexamined and recorded nowhere.
//! There is no primitive list here at all: `String`, `bool`, `u32`, `char`, `Uuid` and
//! `NaiveDate` fail for a single reason, that nothing approved them.
//!
//! A type is approved when its declaration carries a bridge-emitting macro
//! ([`BRIDGE_DERIVES`], [`BRIDGE_ATTRIBUTES`]), found by scanning
//! [`DECLARATION_ROOTS`] — so adding a newtype approves it with no gate edit — or when it
//! is listed in [`APPROVED_FOREIGN`].
//!
//! ## Why reading *declaration* spellings is legitimate when reading *violation*
//! spellings is not
//!
//! ADR-0085 forbids deciding violations or exemptions by searching for anticipated
//! spellings. This search is neither, and the difference is the failure direction:
//!
//! - An incomplete **violation** detector is **silent**. The site passes, and a green run
//!   falsely implies it was examined. That is the defect the ADR exists to prevent.
//! - An incomplete **approval** detector is **loud**. An unrecognised declaration form
//!   means the type is not approved, so every decode into it fails and the author is told.
//!
//! This one fails closed. [`macro_enumeration_problems`] then makes the noise legible:
//! a forgotten family is one message naming the macro, not thirty unrelated failures.
//!
//! Approval means "declared with a bridge-capable macro", **not** "has a bridge". A
//! `#[str_newtype(secret)]` or `no_sqlx` type carries `StrNewtype` and emits none, so it is
//! approved here while being undecodable in fact — harmless, since the compiler rejects a
//! decode into a type with no `Decode` impl, but do not read approval as proof of a bridge.
//! (`#[text_enum]`'s bridge is opt-in via an `sqlx` flag the gate *can* read, so there the
//! answer is exact.)
//!
//! ## Composites are approved by delegation
//!
//! A `#[derive(FromRow)]` struct or tuple alias declared under **[`POLICED_ROOT`]** passes
//! as a target. That is not a hole: every field and element is **separately policed at the
//! declaration**, which is where the newtype belongs — a second population, not a promise.
//!
//! Hence the scoping, and note it is *narrower* than the approve-set's. A bridge-carrying
//! type is approved wherever it is declared, because the bridge is the whole claim. A
//! composite is approved because its fields were checked — and that check runs under the
//! policed root only, so a composite declared in `common/src` or `host/src` has had no
//! field examined and stays unrecognised.
//! A composite with a *hand-written* `FromRow` (`ClaimedRow`) has no policed fields either,
//! so it takes an allowlist entry accounting for its parts.
//!
//! `Result<T, E>` recurses into `T` only — the error arm is never decoded from a column.
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
//! every tuple alias under the root. It is what stops a future
//! `struct PostRow { revision_id: i64 }` from decoding an id into a primitive
//! invisibly — and it is the check that *backs* composite delegation above: `PostRow` is
//! an approved target because these twelve fields were each examined, not instead of.
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
//! One class that *used* to be listed here is not any more. A `.get`/`try_get` in
//! **struct-literal field position** was called safe on the grounds that "the destination
//! field's declared type pins the decode, and that declaration is itself policed as a
//! declared target". That holds for a `#[derive(FromRow)]` struct and is false for a plain
//! one, whose fields are policed by nothing — `storage`'s own `FeedEventRecord` and
//! `ColumnInfo` were both invisible that way (#728). `syn` cannot follow a field to its
//! struct's definition, so the gate does not guess: it **fails**, and the author writes the
//! type at the call.
//!
//! Both are recorded here so the boundary is inherited by the next audit rather than
//! rediscovered.
//!
//! The **over-bite** is the mirror of that boundary, and it is no longer latent: an
//! unascribed `.get(…)` on something that is *not* a row — a `HashMap`, a
//! `SiteConfigStorage` — inside a function whose return type is unapproved is recorded,
//! because rule 3 supplies the target. Widening the population under #728 made this live:
//! `smtp.rs`'s four `load_smtp_config` reads and three in `test_support.rs` are config-store
//! lookups, not row reads, and they carry `not-a-decode-target` entries. Telling them apart
//! by receiver name would be exactly the pattern search this gate forbids.
//!
//! [`Scanner::visit_item_type`] polices *every* tuple alias under the root, whether or not
//! it is a `query_as` target — it cannot tell, and guessing is what the enumerate-don't-search
//! rule forbids. That reach had one instance, `helpers.rs`'s `UserRecordParts`, a
//! function-parameter tuple that was never decoded into; #777 removed it by making the type a
//! named struct, which it wanted to be anyway (its two adjacent `bool`s transposed silently).
//! **The tuple-alias over-bite currently has no instance** — the reach remains, and the next
//! function-parameter tuple alias declared under the root will land here again.
//!
//! # What this gate does not claim
//!
//! Type identity is not column correspondence. It can prove a target is **a** domain type;
//! it can never prove it is the **right** one. Two adjacent `DateTime<Utc>` columns
//! transpose invisibly and compile — `SessionRow`, `InviteRow` and `CacheTuple` each hold
//! such a pair. That needs a different mechanism and is tracked in #751.
//!
//! # Roots
//!
//! **Policed:** `storage/src` only. The two `server/tests/storage/mod.rs` decodes #715
//! typed are **not** policed: a regression there surfaces as a failing test, not as a
//! production transposition, and widening the root would drag every test `COUNT(*)` into
//! the allowlist for no safety gain.
//!
//! **Scanned for declarations:** [`DECLARATION_ROOTS`] — wider, because the types a
//! `storage` decode targets are declared elsewhere. A file missed there would shrink what
//! the gate *accepts*, which changes the rule rather than the population, so it fails the
//! same way an unparseable policed file does.

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
    /// A flag or counter whose primitive **is** the whole domain — a `bool` with two
    /// meaningful states, an integer retry count. A newtype would wrap nothing.
    ///
    /// Distinct from [`Category::CountOrExists`], which is about a *query shape*: reading
    /// `email_verified` is not a cardinality probe, and filing it under one would dilute
    /// exactly the by-rationale reading these categories exist for.
    FlagOrCounter,
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
    /// A row target whose `FromRow` is **hand-written**, so delegation cannot back it:
    /// the gate polices a derived struct's fields and a tuple alias's elements, and a
    /// hand-written impl has neither. Its parts are accounted for in the reason instead.
    HandWrittenFromRow,
    /// **Residue, not a verdict.** This should be a domain type; the fix is a vertical
    /// tracked elsewhere. The reason must name the issue.
    DeferredNewtype,
}

impl Category {
    /// Every variant, so the failure footer can group in a stable, total order without a
    /// `HashMap` iteration order or a hand-kept second list.
    const ALL: &'static [Self] = &[
        Self::CountOrExists,
        Self::FlagOrCounter,
        Self::SchemaIntrospection,
        Self::OpaquePayload,
        Self::DeliberateLossy,
        Self::NotADecodeTarget,
        Self::TestScaffolding,
        Self::HandWrittenFromRow,
        Self::DeferredNewtype,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::CountOrExists => "count-or-exists",
            Self::FlagOrCounter => "flag-or-counter",
            Self::SchemaIntrospection => "schema-introspection",
            Self::OpaquePayload => "opaque-payload",
            Self::DeliberateLossy => "deliberate-lossy",
            Self::NotADecodeTarget => "not-a-decode-target",
            Self::TestScaffolding => "test-scaffolding",
            Self::HandWrittenFromRow => "hand-written-fromrow",
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
/// **No entry here may name a decode that yields a domain value.** That is the whole
/// point: this list is the complete population of legitimate untyped decodes under the root,
/// and anything not on it is a failure.
const ALLOWLIST: &[Allowed] = &[
    // ---- schema introspection: names and definitions out of the DB's own catalog ----
    Allowed {
        file: "postgres/backup.rs",
        function: "existing_export_tables",
        target: "String",
        what: "\"table_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a table name from information_schema — a catalog identifier, not a domain value",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "repair_sequences",
        target: "String",
        what: "\"table_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a table name read from information_schema and spliced back into DDL — a \
                 catalog identifier the domain model has no type for",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "repair_sequences",
        target: "String",
        what: "\"column_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a column name from information_schema",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "columns",
        target: "String",
        what: "\"column_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a column name from information_schema, into the plain ColumnInfo struct",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "columns",
        target: "String",
        what: "\"udt_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a Postgres type name from information_schema",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "export_table",
        target: "String",
        what: "0",
        count: 1,
        category: Category::OpaquePayload,
        reason: "the row rendered as JSON by the query itself — an opaque payload this layer \
                 never interprets",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "schema_checksum",
        target: "String",
        what: "\"table_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a table name from information_schema, hashed into the schema fingerprint",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "schema_checksum",
        target: "String",
        what: "\"column_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a column name from information_schema, hashed into the schema fingerprint",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "schema_checksum",
        target: "String",
        what: "\"udt_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a Postgres type name from information_schema, hashed into the fingerprint",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "schema_checksum",
        target: "String",
        what: "\"is_nullable\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "information_schema's YES/NO nullability flag — a catalog string, not a bool \
                 column",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "existing_export_tables",
        target: "String",
        what: "\"name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a table name from sqlite_master, the dialect twin of the Postgres read",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "columns",
        target: "String",
        what: "\"name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a column name from PRAGMA table_info, into the plain ColumnInfo struct",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "columns",
        target: "String",
        what: "\"type\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a SQLite declared column type from PRAGMA table_info",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "export_table",
        target: "String",
        what: "0",
        count: 1,
        category: Category::OpaquePayload,
        reason: "the row rendered as JSON by the query itself, the twin of the Postgres export",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "schema_checksum",
        target: "String",
        what: "\"name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "an object name from sqlite_master, hashed into the schema fingerprint",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "schema_checksum",
        target: "String",
        what: "\"sql\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "the stored DDL text from sqlite_master, hashed into the schema fingerprint",
    },
    Allowed {
        file: "postgres/mod.rs",
        function: "database_is_empty",
        target: "String",
        what: "\"SELECTtable_nameFROMinformation_schema.tables\\WHEREtable_schema='public'ANDtable_type='BASETABLE'\\ANDtable_name<>'_sqlx_migrations'\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "table names enumerated to decide emptiness",
    },
    Allowed {
        file: "sqlite/mod.rs",
        function: "database_is_empty",
        target: "String",
        what: "\"SELECTnameFROMsqlite_master\\WHEREtype='table'ANDnameNOTLIKE'sqlite_%'ANDname<>'_sqlx_migrations'\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "table names enumerated to decide emptiness, the SQLite twin",
    },
    // ---- cardinality probes ----
    Allowed {
        file: "postgres/mod.rs",
        function: "database_is_empty",
        target: "bool",
        what: "&format!(\"SELECTEXISTS(SELECT1FROM{}LIMIT1)\",crate::sql::quote_identifier(&table))",
        count: 1,
        category: Category::CountOrExists,
        reason: "SELECT EXISTS(…) emptiness probe; the SQLite twin decodes i64 (no bool there)",
    },
    Allowed {
        file: "postgres/posts.rs",
        function: "tag_post",
        target: "bool",
        what: "\"SELECTCOUNT(*)>0FROMpostsWHEREpost_id=$1\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "post-exists check before tagging",
    },
    Allowed {
        file: "sqlite/posts.rs",
        function: "tag_post",
        target: "bool",
        what: "\"SELECTCOUNT(*)>0FROMpostsWHEREpost_id=$1\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "post-exists check before tagging, the dialect twin",
    },
    Allowed {
        file: "posts.rs",
        function: "list_posts_by_tag",
        target: "bool",
        what: "\"SELECTCOUNT(*)>0FROMtagsWHEREtag_slug=$1\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "tag-existence check, so an unknown tag is a 404 rather than an empty list",
    },
    Allowed {
        file: "posts.rs",
        function: "list_user_posts_by_tag",
        target: "bool",
        what: "\"SELECTCOUNT(*)>0FROMtagsWHEREtag_slug=$1\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "the same tag-existence check on the per-user listing",
    },
    Allowed {
        file: "postgres/teardown.rs",
        function: "database_exists",
        target: "bool",
        what: "\"SELECTEXISTS(SELECT1FROMpg_databaseWHEREdatname=$1)\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "database-exists probe before a teardown DROP",
    },
    // ---- deliberately lossy / opaque ----
    Allowed {
        file: "helpers.rs",
        function: "",
        target: "String",
        what: "SessionRow.3",
        count: 1,
        category: Category::DeliberateLossy,
        reason: "the session label is stored lossily (SessionLabel::from_lossy truncates), so \
                 the column holds less than the domain type claims — decoding into it would \
                 assert an invariant the data does not carry (#728 names this site explicitly)",
    },
    Allowed {
        file: "feed_cache.rs",
        function: "",
        target: "String",
        what: "CacheTuple.1",
        count: 1,
        category: Category::OpaquePayload,
        reason: "the cached feed body — rendered RSS/Atom/JSON this layer stores and serves \
                 verbatim, never inspects. Note the same tuple's feed_url and content_type \
                 DO decode into FeedPath/ContentType",
    },
    Allowed {
        file: "helpers.rs",
        function: "",
        target: "String",
        what: "tags",
        count: 1,
        category: Category::OpaquePayload,
        reason: "PostRow's tags column is the JSON aggregate built by TAGS_SUBQUERY, parsed \
                 by build_post_record — the one column of that row that is not a domain type",
    },
    Allowed {
        file: "feed_events.rs",
        function: "",
        target: "Option<String>",
        what: "last_error",
        count: 1,
        category: Category::OpaquePayload,
        reason: "free-text error detail from a failed regeneration attempt; no shape to type",
    },
    Allowed {
        file: "feed_events.rs",
        function: "",
        target: "i32",
        what: "attempts",
        count: 1,
        category: Category::FlagOrCounter,
        reason: "retry counter for the claim-lease backoff — an integer compared against a \
                 max, with no identity of its own",
    },
    // ---- flags on row tuples ----
    Allowed {
        file: "helpers.rs",
        function: "",
        target: "bool",
        what: "UserRow.7",
        count: 1,
        category: Category::FlagOrCounter,
        reason: "email_verified — a two-state flag whose meaning is exhausted by the bool; \
                 there is no wider domain for a newtype to carry",
    },
    Allowed {
        file: "helpers.rs",
        function: "",
        target: "bool",
        what: "UserRow.8",
        count: 1,
        category: Category::FlagOrCounter,
        reason: "is_operator — the same two-state shape; the authorization *decision* is a \
                 domain concept, but the stored bit is not",
    },
    Allowed {
        file: "users.rs",
        function: "authenticate",
        target: "(UserId,Username,Option<DisplayName>,Option<Bio>,DateTime<Utc>,Option<DateTime<Utc>>,StoredPasswordHash,Option<Email>,bool,bool,)",
        count: 1,
        what: "\"SELECTuser_id,username,display_name,bio,created_at,last_authenticated_at,password_hash,email,email_verified,is_operatorFROMusersWHEREusername=$1\"",
        category: Category::FlagOrCounter,
        reason: "email_verified and is_operator — the same two-state flags the helpers.rs \
                 entries describe, and the only unapproved leaves left here. The \
                 password_hash element was this entry's deferred-newtype residue (#693) and \
                 is now StoredPasswordHash, decoding through its bridge",
    },
    // ---- config values: #687 owns the key half, nothing owns the value half ----
    Allowed {
        file: "site_config.rs",
        function: "get",
        target: "(String,)",
        what: "\"SELECTvalueFROMsite_configWHEREkey=$1\"",
        count: 1,
        category: Category::OpaquePayload,
        reason: "a site-config value is deliberately polymorphic text (a URL, a port, a token) \
                 parsed by each key's own getter. #687 types the KEY half; this entry \
                 survives it, because the value half stays String by design",
    },
    Allowed {
        file: "site_config.rs",
        function: "list",
        target: "(String,String)",
        what: "\"SELECTkey,valueFROMsite_configORDERBYkey\"",
        count: 1,
        category: Category::DeferredNewtype,
        reason: "two adjacent Strings — a real transposition hazard, and the only one in this \
                 file. #687's SiteConfigKey types the first element and removes it; the \
                 second stays String per the entry above",
    },
    Allowed {
        file: "site_config.rs",
        function: "delete",
        target: "(String,)",
        what: "\"DELETEFROMsite_configWHEREkey=$1RETURNINGkey\"",
        count: 1,
        category: Category::DeferredNewtype,
        reason: "the RETURNING key echoes back the config key — #687's SiteConfigKey territory",
    },
    Allowed {
        file: "user_config.rs",
        function: "get",
        target: "(String,)",
        what: "\"SELECTvalueFROMuser_configWHEREuser_id=$1ANDkey=$2\"",
        count: 1,
        category: Category::OpaquePayload,
        reason: "a per-user config value, polymorphic text like its site-config sibling",
    },
    Allowed {
        file: "smtp.rs",
        function: "load_smtp_config",
        target: "Result<Option<SmtpConfig>,SmtpConfigError>",
        what: "\"smtp.host\"",
        count: 1,
        category: Category::NotADecodeTarget,
        reason: "SiteConfigStorage::get, not a row read — the gate takes the target from the \
                 enclosing fn return because the call writes no type, and cannot tell this \
                 receiver from an sqlx row",
    },
    Allowed {
        file: "smtp.rs",
        function: "load_smtp_config",
        target: "Result<Option<SmtpConfig>,SmtpConfigError>",
        what: "\"smtp.port\"",
        count: 1,
        category: Category::NotADecodeTarget,
        reason: "SiteConfigStorage::get, not a row read",
    },
    Allowed {
        file: "smtp.rs",
        function: "load_smtp_config",
        target: "Result<Option<SmtpConfig>,SmtpConfigError>",
        what: "\"smtp.tls_mode\"",
        count: 1,
        category: Category::NotADecodeTarget,
        reason: "SiteConfigStorage::get, not a row read",
    },
    Allowed {
        file: "smtp.rs",
        function: "load_smtp_config",
        target: "Result<Option<SmtpConfig>,SmtpConfigError>",
        what: "\"smtp.sender\"",
        count: 1,
        category: Category::NotADecodeTarget,
        reason: "SiteConfigStorage::get, not a row read",
    },
    // ---- subscriptions ----
    Allowed {
        file: "subscriptions.rs",
        function: "list_subscribers",
        target: "(SubscriptionId,ChannelId,String,DateTime<Utc>)",
        what: "DB::LIST_ACTIVE_SUBSCRIBERS",
        count: 1,
        category: Category::DeferredNewtype,
        reason: "subscriber_ref is a channel-scoped opaque reference — a domain value with no \
                 type. Deferred to #750: the fix spans the admission seam, the ChannelId \
                 pairing and the wire DTOs, not this decode",
    },
    // ---- the claim wrapper ----
    Allowed {
        file: "postgres/feed_events.rs",
        function: "claim_pending_batch",
        target: "ClaimedRow",
        what: "\"WITHeligibleAS(\\SELECTidFROMfeed_events\\WHERE(status='pending'ANDnext_attempt_at<=$1)\\OR(status='claimed'ANDclaimed_at<$2)\\ORDERBYnext_attempt_atASC\\LIMIT$3\\FORUPDATESKIPLOCKED\\)\\UPDATEfeed_eventsSETstatus='claimed',claimed_at=$1\\WHEREidIN(SELECTidFROMeligible)\\RETURNINGid,feed_url,status,attempts,last_error,next_attempt_at,claimed_at,\\created_at,regenerated_at,pinged_at\"",
        count: 1,
        category: Category::HandWrittenFromRow,
        reason: "ClaimedRow's FromRow is hand-written (it must divert a corrupt feed_url to \
                 the purge list), so delegation cannot back it. Its parts are accounted for: \
                 FeedEventRecord is a policed FromRow struct and FeedEventId is a bridge type",
    },
    Allowed {
        file: "sqlite/feed_events.rs",
        function: "claim_pending_batch",
        target: "ClaimedRow",
        what: "\"UPDATEfeed_eventsSETstatus='claimed',claimed_at=$1\\WHEREidIN(\\SELECTidFROMfeed_events\\WHERE(status='pending'ANDnext_attempt_at<=$2)\\OR(status='claimed'ANDclaimed_at<$3)\\ORDERBYnext_attempt_atASC\\LIMIT$4\\)\\RETURNINGid,feed_url,status,attempts,last_error,next_attempt_at,claimed_at,\\created_at,regenerated_at,pinged_at\"",
        count: 1,
        category: Category::HandWrittenFromRow,
        reason: "the dialect twin of the Postgres claim; same wrapper, same accounting",
    },
    // ---- test scaffolding ----
    Allowed {
        file: "test_support.rs",
        function: "string_triples",
        target: "Result<Vec<(String,String,String)>,sqlx::Error>",
        what: "sql",
        count: 2,
        category: Category::TestScaffolding,
        reason: "a generic test row helper; the SQL is a runtime &str and the shape comes from \
                 the fn return. The two dialect arms are byte-identical",
    },
    Allowed {
        file: "test_support.rs",
        function: "ensure_template_db",
        target: "bool",
        what: "\"SELECTEXISTS(SELECT1FROMpg_databaseWHEREdatname=$1)\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "database-exists probe in the Postgres test harness",
    },
    Allowed {
        file: "test_support.rs",
        function: "get",
        target: "sqlx::Result<Option<String>>",
        what: "key",
        count: 1,
        category: Category::NotADecodeTarget,
        reason: "a HashMap::get in the in-memory SiteConfigStorage fake — not a row read; the \
                 gate takes the target from the fn return and cannot tell the receiver apart",
    },
    Allowed {
        file: "test_support.rs",
        function: "get_smtp_credentials",
        target: "sqlx::Result<crate::smtp::SmtpCredentials>",
        what: "\"smtp.username\"",
        count: 1,
        category: Category::NotADecodeTarget,
        reason: "SiteConfigStorage::get in the test harness, not a row read",
    },
    Allowed {
        file: "test_support.rs",
        function: "get_smtp_credentials",
        target: "sqlx::Result<crate::smtp::SmtpCredentials>",
        what: "\"smtp.password\"",
        count: 1,
        category: Category::NotADecodeTarget,
        reason: "SiteConfigStorage::get in the test harness, not a row read",
    },
    // ---- surviving i64-family entries from #715 ----
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
    /// The leaf types that are not approved — the reason this site is in the report.
    /// Message only; matching keys on the four fields above.
    unapproved: Vec<String>,
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

/// Roots scanned for *declarations*, to build the approve-set. Wider than
/// [`POLICED_ROOT`]: the domain types a `storage` decode targets are declared in `common`.
///
/// A missing or unparseable file here fails the gate, exactly as under the policed root —
/// the approve-set is what makes a decode legal, so a shrunken one is a gate that has
/// quietly changed its own rule.
/// `host/src` is here because the *validated* halves of a few split newtypes live there,
/// server-only and serde-free — `host::invite::InviteCode` is the one storage decodes
/// today, and it is exactly the type `common`'s wasm-facing `ProfferedInviteCode` is not.
/// Missing it would have cost a spurious allowlist entry for a properly typed decode.
const DECLARATION_ROOTS: &[&str] = &["common/src", "host/src", "storage/src"];

/// Generic containers walked *through* to reach leaves. Anything else is a leaf that must
/// itself be approved, so an unrecognised wrapper fails closed.
const CONTAINERS: &[&str] = &["Vec", "Option", "Box", "Cow", "Arc", "Rc"];

/// Foreign types that are legitimate column targets but are declared outside this repo, so
/// no declaration scan can find them.
///
/// The only hand-maintained part of the approve-set, and small precisely because the ~35
/// domain types derive automatically. Each entry is a deliberate statement that decoding a
/// column straight into this type is right.
const APPROVED_FOREIGN: &[(&str, &str)] = &[(
    "DateTime",
    "chrono timestamps — the correct target for every temporal column; note this is also \
     the residual ADR-0085 records, since two adjacent DateTime columns transpose \
     invisibly (#751)",
)];

/// The types a decode may land in: domain types with a bridge, plus the composites whose
/// parts this gate polices separately.
#[derive(Default)]
pub(crate) struct ApproveSet {
    /// Types declared with a bridge-emitting macro, plus [`APPROVED_FOREIGN`].
    approved: std::collections::HashSet<String>,
    /// `#[derive(FromRow)]` structs and tuple aliases declared under a scanned root.
    ///
    /// Approved **by delegation**: every field and element is itself policed, at the
    /// declaration, by [`Scanner::visit_item_struct`] / [`Scanner::visit_item_type`]. This
    /// is a second policed population, not a promise — which is why the delegation is
    /// scoped to composites declared under a root this gate actually reads. One declared
    /// elsewhere has had no field checked, so it stays unrecognised and fails.
    composites: std::collections::HashSet<String>,
}

/// Whether `attrs` carry a bridge-emitting macro.
///
/// Two shapes, and the rule differs between them (#746). A **derive** from
/// [`BRIDGE_DERIVES`] approves on its presence alone: whether it actually emits the bridge
/// depends on the type's own options (`no_sqlx`, `secret`), which is not a static property
/// — so this over-approves, harmlessly, since the compiler rejects a decode into a type
/// with no `Decode` impl. A **`#[text_enum]` attribute** carries an explicit `sqlx` flag,
/// so the gate reads it and is exact: `TargetKind` has it, `Channel` does not.
fn has_bridge(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        let last = a.path().segments.last().map(|s| s.ident.to_string());
        match last.as_deref() {
            Some("derive") => a.meta.require_list().is_ok_and(|l| {
                l.tokens.clone().into_iter().any(|t| match t {
                    proc_macro2::TokenTree::Ident(i) => {
                        BRIDGE_DERIVES.contains(&i.to_string().as_str())
                    }
                    _ => false,
                })
            }),
            Some(name) if BRIDGE_ATTRIBUTES.contains(&name) => {
                a.meta.require_list().is_ok_and(|l| {
                    l.tokens
                        .clone()
                        .into_iter()
                        .any(|t| matches!(&t, proc_macro2::TokenTree::Ident(i) if i == "sqlx"))
                })
            }
            _ => false,
        }
    })
}

/// Which kind of root a scanned declaration file sits under.
///
/// Named rather than a `bool` because the two call sites read very differently: at the
/// scan it is derived from the root, but in a test `collect_declarations(src, true, …)`
/// says nothing about *what* is true.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Root {
    /// Under [`POLICED_ROOT`] — decode sites here are checked, so composites declared here
    /// can be approved by delegation.
    Policed,
    /// Scanned for declarations only. Bridge-carrying types still count; composites do
    /// not, because nothing here polices their fields.
    DeclarationsOnly,
}

/// Whether `attrs` derive `sqlx::FromRow`.
fn is_from_row(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|a| a.path().is_ident("derive") && render(&a.meta).contains("FromRow"))
}

/// Folds one file's declarations into `set`.
///
/// `policed` says whether this file sits under [`POLICED_ROOT`], and it gates **only** the
/// composites. A bridge-carrying type is approved wherever it is declared — the bridge is
/// the whole claim. A composite is approved because its fields are checked, and that check
/// runs under the policed root alone; collecting composites from the wider declaration
/// roots would approve a `FromRow` struct or tuple alias with **zero fields examined**,
/// which is precisely the unbacked promise delegation must not make.
///
/// Only top-level `file.items` are read: a declaration inside an inline `mod` is not seen.
/// That direction is safe — an unseen declaration is an unapproved type, so the gate bites
/// rather than waving something through — but it is a boundary, not an oversight.
fn collect_declarations(source: &str, root: Root, set: &mut ApproveSet) -> Result<(), String> {
    let policed = root == Root::Policed;
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    for item in &file.items {
        match item {
            syn::Item::Struct(s) if has_bridge(&s.attrs) => {
                set.approved.insert(s.ident.to_string());
            }
            syn::Item::Enum(e) if has_bridge(&e.attrs) => {
                set.approved.insert(e.ident.to_string());
            }
            syn::Item::Struct(s) if policed && is_from_row(&s.attrs) => {
                set.composites.insert(s.ident.to_string());
            }
            syn::Item::Type(t) if policed && matches!(&*t.ty, syn::Type::Tuple(_)) => {
                set.composites.insert(t.ident.to_string());
            }
            _ => {}
        }
    }
    Ok(())
}

/// Every leaf of `ty` that is not an approved column type — empty when the decode is fine.
///
/// `Result<T, E>` recurses into `T` **only**: the error arm is never decoded from a column,
/// so asking `BackupError` to be an approved column type would be nonsense.
fn unapproved_leaves(ty: &syn::Type, set: &ApproveSet) -> Vec<String> {
    match ty {
        syn::Type::Path(p) => {
            let Some(last) = p.path.segments.last() else {
                return Vec::new();
            };
            let name = last.ident.to_string();
            let args: Vec<&syn::Type> = match &last.arguments {
                syn::PathArguments::AngleBracketed(ab) => ab
                    .args
                    .iter()
                    .filter_map(|a| match a {
                        syn::GenericArgument::Type(t) => Some(t),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            if name == "Result" {
                return args
                    .first()
                    .map(|t| unapproved_leaves(t, set))
                    .unwrap_or_default();
            }
            if CONTAINERS.contains(&name.as_str()) {
                return args
                    .iter()
                    .flat_map(|t| unapproved_leaves(t, set))
                    .collect();
            }
            if set.approved.contains(&name) || set.composites.contains(&name) {
                Vec::new()
            } else {
                vec![name]
            }
        }
        syn::Type::Tuple(t) => t
            .elems
            .iter()
            .flat_map(|e| unapproved_leaves(e, set))
            .collect(),
        syn::Type::Reference(r) => unapproved_leaves(&r.elem, set),
        syn::Type::Paren(p) => unapproved_leaves(&p.elem, set),
        syn::Type::Group(g) => unapproved_leaves(&g.elem, set),
        syn::Type::Slice(s) => unapproved_leaves(&s.elem, set),
        syn::Type::Array(a) => unapproved_leaves(&a.elem, set),
        _ => Vec::new(),
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
struct Scanner<'a> {
    /// The types a decode may legally land in.
    approve: &'a ApproveSet,
    out: Vec<DecodeSite>,
    /// `(line, column-argument)` of each turbofish-less `.get`/`try_get` in struct-literal
    /// field position — a hard failure, not a decode record.
    unreadable_fields: Vec<(usize, String)>,
    /// Spans of those calls, so [`Scanner::record`] can decline them rather than pinning
    /// them to whatever `let` or `fn` return happens to enclose the struct literal.
    field_positions: std::collections::HashSet<(usize, usize)>,
    function: String,
    fn_ret: Option<syn::Type>,
    let_ty: Option<syn::Type>,
}

impl Scanner<'_> {
    fn new(
        field_positions: std::collections::HashSet<(usize, usize)>,
        approve: &ApproveSet,
    ) -> Scanner<'_> {
        Scanner {
            out: Vec::new(),
            unreadable_fields: Vec::new(),
            field_positions,
            approve,
            function: String::new(),
            fn_ret: None,
            let_ty: None,
        }
    }

    /// Records one decode with the nearest declared target, if that target is in the
    /// has an unapproved leaf. `turbofish` wins, then the enclosing `let`, then the `fn`
    /// return.
    fn record(&mut self, turbofish: Option<syn::Type>, what: String, span: proc_macro2::Span) {
        let target = turbofish
            .or_else(|| self.let_ty.clone())
            .or_else(|| self.fn_ret.clone());
        let Some(target) = target else {
            // Unreadable: no turbofish, no ascription, no fn return. Out of population
            // by construction — see the module doc.
            return;
        };
        let unapproved = unapproved_leaves(&target, self.approve);
        if unapproved.is_empty() {
            return;
        }
        self.out.push(DecodeSite {
            function: self.function.clone(),
            target: render(&target),
            what,
            unapproved,
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

/// Strips the wrappers that can sit between a struct-literal field and the call that
/// produces its value, so `name: row.try_get("c")?` is recognised as field position.
///
/// **The peel set is deliberately short and closed**: `?`, `.await`, and parens/groups.
/// Anything else — `.unwrap()`, a cast, a nested call — is *not* field position and this
/// rule does not reach it. A longer peel would be guesswork about which expressions
/// "really" mean the field, and the whole design rests on the gate never guessing.
fn peel_to_call(expr: &syn::Expr) -> &syn::Expr {
    match expr {
        syn::Expr::Try(e) => peel_to_call(&e.expr),
        syn::Expr::Await(e) => peel_to_call(&e.base),
        syn::Expr::Paren(e) => peel_to_call(&e.expr),
        syn::Expr::Group(e) => peel_to_call(&e.expr),
        other => other,
    }
}

/// `(line, column)` of every turbofish-less `.get`/`try_get` sitting in struct-literal
/// field position.
///
/// This is the class #715's module doc wrongly called safe: it claimed the destination
/// field's declared type pins such a decode "and that declaration is itself policed as a
/// declared target". True for a `#[derive(FromRow)]` struct; false for a plain one, whose
/// fields are policed by nothing. `syn` cannot follow the struct name to its definition —
/// it may be in another file or another crate — so the gate does not try. It refuses to be
/// blind instead: write the type at the call, and rule 1 reads it.
fn unreadable_field_positions(file: &syn::File) -> std::collections::HashSet<(usize, usize)> {
    struct Finder(std::collections::HashSet<(usize, usize)>);
    impl<'ast> syn::visit::Visit<'ast> for Finder {
        fn visit_expr_struct(&mut self, i: &'ast syn::ExprStruct) {
            for f in &i.fields {
                if let syn::Expr::MethodCall(m) = peel_to_call(&f.expr) {
                    if target_index(&m.method.to_string()).is_some() && m.turbofish.is_none() {
                        let s = m.method.span().start();
                        self.0.insert((s.line, s.column));
                    }
                }
            }
            syn::visit::visit_expr_struct(self, i);
        }
    }
    let mut finder = Finder(std::collections::HashSet::new());
    syn::visit::Visit::visit_file(&mut finder, file);
    finder.0
}

impl<'ast> syn::visit::Visit<'ast> for Scanner<'_> {
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
            let span = i.method.span().start();
            let what = i.args.first().map(render).unwrap_or_default();
            if self.field_positions.contains(&(span.line, span.column)) {
                // Field position with no turbofish: a hard failure, and NOT a decode
                // record. Falling through to `record` would pin it to the enclosing `fn`
                // return — the struct literal's own type, or worse an unrelated one — and
                // report a target the author never wrote.
                self.unreadable_fields.push((span.line, what));
            } else {
                let turbofish = i
                    .turbofish
                    .as_ref()
                    .and_then(|t| nth_type_of(t.args.iter(), idx));
                self.record(turbofish, what, i.method.span());
            }
        }
        syn::visit::visit_expr_method_call(self, i);
    }

    /// A `#[derive(FromRow)]` struct is a declared decode target: each field is a
    /// column decode, and the field's type is where the newtype belongs.
    fn visit_item_struct(&mut self, i: &'ast syn::ItemStruct) {
        // Same predicate `collect_declarations` uses to decide delegation — they must not
        // drift, or a struct could be approved as a composite while its fields go
        // unpoliced.
        if is_from_row(&i.attrs) {
            for f in &i.fields {
                let unapproved = unapproved_leaves(&f.ty, self.approve);
                if !unapproved.is_empty() {
                    self.out.push(DecodeSite {
                        function: String::new(),
                        target: render(&f.ty),
                        what: f
                            .ident
                            .as_ref()
                            .map(std::string::ToString::to_string)
                            .unwrap_or_default(),
                        unapproved,
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
                let unapproved = unapproved_leaves(elem, self.approve);
                if !unapproved.is_empty() {
                    self.out.push(DecodeSite {
                        function: String::new(),
                        target: render(elem),
                        what: format!("{}.{n}", i.ident),
                        unapproved,
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
fn decodes(source: &str, approve: &ApproveSet) -> Result<FileScan, String> {
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut scanner = Scanner::new(unreadable_field_positions(&file), approve);
    syn::visit::Visit::visit_file(&mut scanner, &file);
    Ok(FileScan {
        sites: scanner.out,
        unreadable_fields: scanner.unreadable_fields,
    })
}

/// One file's scan: the decode records, and the field-position calls whose target is not
/// written anywhere the gate can read.
struct FileScan {
    sites: Vec<DecodeSite>,
    /// `(line, column-argument)` per failure.
    unreadable_fields: Vec<(usize, String)>,
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
pub(crate) fn problems(scanned: &[(String, String)], approve: &ApproveSet) -> Option<String> {
    let mut found: Vec<(String, DecodeSite)> = Vec::new();
    let mut lines = Vec::new();
    for (path, source) in scanned {
        match decodes(source, approve) {
            Ok(scan) => {
                found.extend(scan.sites.into_iter().map(|d| (path.clone(), d)));
                for (line, what) in scan.unreadable_fields {
                    lines.push(format!(
                        "{path}:{line}: `{what}` decodes into a struct-literal field with no type \
                         written at the call. Add a turbofish — `row.try_get::<T, _>({what})` — so \
                         this gate can read the target. It will not follow the field to the \
                         struct's definition: that declaration is only policed when the struct \
                         derives `FromRow`, and for a plain struct nothing checks it at all."
                    ));
                }
            }
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
                "{path}:{}: `{}` decodes into `{}`, whose leaf type(s) {} are not approved column \
                 types. If the column holds a domain value, decode it straight into its type — the \
                 ADR-0071 bridge makes `query_scalar::<_, PostId>` work — and delete any hand \
                 re-wrap. If it is genuinely untyped, add an ALLOWLIST entry with a written \
                 reason. This gate reads no SQL, so it cannot tell which; that judgement is yours \
                 to record.",
                d.line,
                d.what,
                d.target,
                d.unapproved.join(", ")
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
         hold domain values, and deliberately so, because every audit that searched for the \
         id-ish spelling missed the sites spelled another way (#686, #715). So a decode passes \
         only when every leaf of its target is an APPROVED type — one declared with a \
         bridge-emitting macro, or a composite whose fields this gate polices — and every other \
         decode is either typed or listed below. Currently exempt, by rationale:"
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

    // The approve-set is built from a WIDER set of roots than the policed one: a
    // `storage` decode targets types declared in `common`. Same read-and-parse discipline
    // — a file missed here would silently shrink what the gate accepts, which changes the
    // rule rather than the population, and is worse.
    let mut approve = ApproveSet::default();
    // Delegation is only sound where field policing runs, and that link is a *string*
    // match between the two consts. Check it rather than assume it: a `DECLARATION_ROOTS`
    // that spells the policed root differently would silently stop collecting composites.
    // That direction fails closed (every composite target becomes unrecognised and the
    // gate goes loudly red), so this is about naming the cause, not preventing a silent
    // hole.
    if !DECLARATION_ROOTS.contains(&POLICED_ROOT) {
        unreadable.push(format!(
            "DECLARATION_ROOTS does not contain POLICED_ROOT ({POLICED_ROOT}) — composite \
             delegation is scoped by matching the two, so nothing would be approved by \
             delegation and every row-struct target would fail as unrecognised."
        ));
    }
    for root in DECLARATION_ROOTS {
        let kind = if *root == POLICED_ROOT {
            Root::Policed
        } else {
            Root::DeclarationsOnly
        };
        match files::with_extension(Path::new(root), "rs") {
            Ok(decls) => {
                for p in &decls {
                    let path = p.display().to_string();
                    match std::fs::read_to_string(p) {
                        Ok(s) => {
                            if let Err(e) = collect_declarations(&s, kind, &mut approve) {
                                unreadable.push(format!(
                                    "{path}: {e} — this gate's approved-type set is built from \
                                     the declarations here, so an unparsed file shrinks what it \
                                     accepts."
                                ));
                            }
                        }
                        Err(e) => unreadable.push(format!(
                            "{path}: cannot read: {e} — a declaration file this gate cannot read \
                             is an approve-set it cannot trust."
                        )),
                    }
                }
            }
            Err(e) => unreadable.push(format!("cannot scan declaration root {root}: {e}")),
        }
    }
    approve
        .approved
        .extend(APPROVED_FOREIGN.iter().map(|(n, _)| (*n).to_string()));

    let detail = match (problems(&scanned, &approve), unreadable.is_empty()) {
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

    /// A synthetic approve-set, so the pure tests never touch the filesystem.
    ///
    /// The names here stand in for what the real declaration scan finds: `Slug`/`PostId`
    /// for bridge-carrying domain types, `DateTime` for [`APPROVED_FOREIGN`], and
    /// `PostRow`/`CacheTuple` for composites approved by delegation.
    fn approve() -> ApproveSet {
        let names = |ns: &[&str]| ns.iter().map(|s| (*s).to_string()).collect();
        ApproveSet {
            approved: names(&[
                "Slug",
                "PostId",
                "UserId",
                "AudienceId",
                "Email",
                "FeedPath",
                "TagId",
                "Tag",
                "DateTime",
            ]),
            composites: names(&["PostRow", "CacheTuple"]),
        }
    }

    /// Targets of every decode the scanner found, for terse assertions.
    fn targets(src: &str) -> Vec<String> {
        decodes(src, &approve())
            .expect("parses")
            .sites
            .into_iter()
            .map(|d| d.target)
            .collect()
    }

    /// Line numbers of the turbofish-less struct-literal field decodes in `src`.
    fn field_failures(src: &str) -> Vec<usize> {
        decodes(src, &approve())
            .expect("parses")
            .unreadable_fields
            .into_iter()
            .map(|(line, _)| line)
            .collect()
    }

    /// [`problems`] against the synthetic approve-set.
    fn problems_of(scanned: &[(String, String)]) -> Option<String> {
        problems(scanned, &approve())
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
    fn every_unapproved_target_is_collected_with_no_special_casing() {
        // This replaces `bool_and_string_targets_are_not_collected`, which asserted the
        // opposite. Under #715's rule those were out of population — invisible, and
        // recorded nowhere. That invisibility is the defect #728 exists to close, so they
        // are now in population and need a written reason like anything else.
        //
        // Note there is no primitive list anywhere in the rule: `bool`, `String`, `i64`,
        // `u32`, `char` and `Uuid` fail for one reason — nothing approved them.
        for target in [
            "bool",
            "String",
            "i64",
            "u32",
            "char",
            "f64",
            "Uuid",
            "NaiveDate",
        ] {
            let src = format!(
                r#"fn f() {{ sqlx::query_scalar::<_, {target}>("SELECT c").fetch_one(p); }}"#
            );
            assert_eq!(targets(&src), vec![target.to_string()], "for {target}");
        }
    }

    #[test]
    fn slice_and_array_leaves_are_reached() {
        // `Type::Slice`/`Type::Array` were absent from #715's recursion, so a `&[u8]`
        // target slipped through even though `u8` is no more approved than `i64`.
        for target in ["&[u8]", "[u8; 32]"] {
            let src = format!(r#"fn f() {{ let v: {target} = r.get("c"); }}"#);
            assert_eq!(targets(&src).len(), 1, "for {target}");
        }
    }

    #[test]
    fn a_composite_is_approved_by_delegation_but_its_parts_are_not() {
        // The property that keeps delegation honest. `PostRow` passes as a target — its
        // fields are policed at the declaration, which is where the newtype belongs — but
        // that approval does NOT extend to a primitive field of it.
        let approved_target =
            r#"fn f() { sqlx::query_as::<_, PostRow>("SELECT *").fetch_one(p); }"#;
        assert!(targets(approved_target).is_empty());

        let leaky_field = r#"
            #[derive(sqlx::FromRow)]
            struct PostRow { slug: Slug, tags: String }
        "#;
        assert_eq!(targets(leaky_field), vec!["String"]);
    }

    #[test]
    fn a_composite_the_gate_never_read_is_not_approved() {
        // Delegation is a second policed population, not a promise. A composite the gate
        // never read has had no field checked, so approving it would be an unbacked claim.
        let src = r#"fn f() { sqlx::query_as::<_, SomeForeignRow>("SELECT *").fetch_one(p); }"#;
        assert_eq!(targets(src), vec!["SomeForeignRow"]);
    }

    #[test]
    fn composites_are_collected_only_from_the_policed_root() {
        // The scoping that keeps delegation backed. A `FromRow` struct or tuple alias
        // declared in `common/src` is NOT approved: its fields are never examined, because
        // field policing runs under `storage/src` alone. Approving it would be the unbacked
        // promise delegation exists to avoid — and this is the direction that fails open,
        // so nothing else would catch it.
        let src = "
            #[derive(sqlx::FromRow)]
            struct ForeignRow { a: Slug }
            type ForeignTuple = (Slug, PostId);
        ";
        let mut policed = ApproveSet::default();
        collect_declarations(src, Root::Policed, &mut policed).expect("parses");
        assert!(policed.composites.contains("ForeignRow"));
        assert!(policed.composites.contains("ForeignTuple"));

        let mut declaration_only = ApproveSet::default();
        collect_declarations(src, Root::DeclarationsOnly, &mut declaration_only).expect("parses");
        assert!(
            declaration_only.composites.is_empty(),
            "a composite outside the policed root has had no field checked"
        );
    }

    #[test]
    fn bridge_types_are_collected_from_every_declaration_root() {
        // The other half of the asymmetry: a bridge-carrying type is approved wherever it
        // is declared, because the bridge itself is the claim — no second check needed.
        // `host::invite::InviteCode` is decoded by storage and lives outside `common`.
        let src = "
            #[derive(Clone, StrNewtype)]
            pub struct InviteCode(String);
        ";
        let mut outside = ApproveSet::default();
        collect_declarations(src, Root::DeclarationsOnly, &mut outside).expect("parses");
        assert!(outside.approved.contains("InviteCode"));
    }

    #[test]
    fn a_result_error_arm_is_never_a_decode_target() {
        // `Result<T, E>`: the error arm is not decoded from a column, so an unapproved
        // `E` must not fail the decode. Every fn-return site in the tree hits this.
        let src = r#"
            fn f() -> Result<Slug, BackupError> {
                sqlx::query_scalar("SELECT slug").fetch_one(p)
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

    // ---- struct-literal field position: the gate refuses to be blind ----

    #[test]
    fn an_unturbofished_struct_literal_field_is_a_failure() {
        // This test replaces `struct_literal_row_get_is_not_collected`, which asserted the
        // opposite on the strength of a claim that only holds for `#[derive(FromRow)]`
        // structs: that the destination field's declaration polices the decode. `Rec` here
        // is a plain struct, so nothing polices it — the exact shape that hid
        // `FeedEventRecord.attempts` and `ColumnInfo.name` (#728).
        let src = r#"fn f() { Rec { id: r.get("id"), attempts: r.get("attempts") }; }"#;
        assert_eq!(field_failures(src).len(), 2);
        // …and it is NOT also recorded as a decode against some enclosing type.
        assert!(targets(src).is_empty());
    }

    #[test]
    fn a_turbofished_struct_literal_field_is_read_normally() {
        let src = r#"fn f() { Rec { id: r.get::<i64, _>("id") }; }"#;
        assert!(field_failures(src).is_empty());
        assert_eq!(targets(src), vec!["i64"]);
    }

    #[test]
    fn the_peel_set_is_exactly_try_await_and_parens() {
        // The live sites are `name: row.try_get("c")?`, so `?` must peel. `.await` and
        // parens travel with it. Everything else is deliberately out of reach — a longer
        // peel would be guesswork about which expressions "really" mean the field.
        for src in [
            r#"fn f() { Rec { a: r.try_get("c")? }; }"#,
            r#"fn f() { Rec { a: r.get("c").await }; }"#,
            r#"fn f() { Rec { a: (r.get("c")) }; }"#,
        ] {
            assert_eq!(field_failures(src).len(), 1, "must bite: {src}");
        }
        // `.unwrap()` puts the call outside field position: the field's value is the
        // `unwrap` call, not the `get`.
        let src = r#"fn f() { Rec { a: r.get("c").unwrap() }; }"#;
        assert!(field_failures(src).is_empty(), "must not bite: {src}");
    }

    #[test]
    fn field_position_does_not_reach_a_nested_argument() {
        // A `get` buried in an argument is not the field's value.
        let src = r#"fn f() { Rec { a: helper(r.get("c")) }; }"#;
        assert!(field_failures(src).is_empty());
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
        let two_detail = problems_of(&two).unwrap_or_default();
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
        let detail = problems_of(&three).expect("a third identical decode must fail");
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
        let detail = problems_of(&[("storage/src/test_support.rs".to_string(), src.to_string())])
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
        let detail = problems_of(&[("storage/src/users.rs".to_string(), src.to_string())])
            .expect("a bare i64 id decode must fail");
        assert!(detail.contains("storage/src/users.rs"), "{detail}");
        assert!(detail.contains("decodes into `i64`"), "{detail}");
    }

    #[test]
    fn a_stale_entry_with_no_matching_site_is_reported() {
        // An allowlist that stops tracking the tree has quietly become a region
        // exemption, so a vanished site is a failure too, not a free pass.
        let detail = problems_of(&[("storage/src/users.rs".to_string(), String::new())])
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
        let detail = problems_of(&[("storage/src/broken.rs".to_string(), "fn f( {{{".to_string())])
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
            unapproved: vec!["i64".to_string()],
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
    fn leaf_recursion_reaches_through_wrappers_without_over_matching() {
        // Replaces `is_i64_family_recurses_without_over_matching`. Same shapes, new rule:
        // the question is no longer "is any leaf i64" but "is any leaf unapproved".
        let set = approve();
        let ty: syn::Type = syn::parse_quote!(Vec<(String, Option<PostId>)>);
        assert_eq!(unapproved_leaves(&ty, &set), vec!["String".to_string()]);
        let ty: syn::Type = syn::parse_quote!(Vec<(Slug, DateTime<Utc>)>);
        assert!(unapproved_leaves(&ty, &set).is_empty());
        let ty: syn::Type = syn::parse_quote!(Option<AudienceId>);
        assert!(unapproved_leaves(&ty, &set).is_empty());
    }
}
