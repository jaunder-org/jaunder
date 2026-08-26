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
//! A hand-written `sqlx::FromRow` is approved only after every matching implementation for
//! its simple self type passes a narrow syntactic proof. Its `from_row` body must be flat `let`
//! statements followed by `Ok(Self { … })`; every use of its `row` parameter must be the direct
//! receiver of `row.try_get::<ConcreteType, _>(one_column_index)?`. There are no aliases,
//! shadowing, UFCS, helper flow, untyped gets, alternate access, delegation, nested
//! scopes/items, macros, or attributes on the handwritten decoder nodes in that grammar. A `let`
//! that never mentions `row` may transform an already-decoded binding (such as PostRecord's tags
//! JSON). The scanner still polices every direct typed get. Every other hand-written decoder needs
//! an exact, counted [`ALLOWLIST`] entry.
//!
//! Hence the scoping, and note it is *narrower* than the approve-set's. A bridge-carrying
//! type is approved wherever it is declared, because the bridge is the whole claim. A
//! derived composite is approved because its fields or elements were checked — and that check
//! runs under the policed root only, so a composite declared in `common/src` or `host/src`
//! has had neither examined and stays unrecognised.
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
//! 3. else the enclosing function or trait-default-method return type —
//!    `scalar_i64(…) -> Result<i64, _>`.
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
//! `smtp.rs`'s four `load_smtp_config` reads are config-store
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
//! it can never prove it is the **right** one. #751 removed this gate's named adjacent
//! timestamp residuals by replacing the affected tuple decodes with named row structs and
//! distinct timestamp role types; the gate still does not read SQL column correspondence.
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

mod allowlist;
mod approve_set;
mod macros_audit;
mod report;
mod scan;

pub use report::run;
