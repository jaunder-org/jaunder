//! The `sqlx-newtype-decode` static check (#715, widened by #728): every structurally
//! readable SQLx decode under `storage/src` must land in declaration-backed column types.
//!
//! The sibling `sqlx-newtype-bind` polices *binds*. Nothing policed *decodes*, so
//! `query_scalar::<_, i64>` on a `RETURNING post_id` was invisible to it and to the
//! audits that preceded it.
//!
//! # The rule
//!
//! **Every leaf type of a decode target must be approved.** There is no primitive list
//! and no per-site exception: `String`, `bool`, `u32`, `char`, `Uuid`, and `NaiveDate`
//! fail for the single reason that nothing approved them. Intentional persisted values
//! use explicit role-specific types rather than a primitive escape hatch.
//!
//! A type is approved when its declaration carries a bridge-emitting macro
//! ([`BRIDGE_DERIVES`], [`BRIDGE_ATTRIBUTES`]), found by scanning
//! [`DECLARATION_ROOTS`], or when it is listed in [`APPROVED_FOREIGN`]. Adding a
//! newtype therefore approves it with no gate edit.
//!
//! ## Why reading *declaration* spellings is legitimate
//!
//! ADR-0085 forbids deciding violations by searching for anticipated spellings. This
//! declaration scan is different because it fails closed: an incomplete violation detector
//! is silent, while an unrecognised declaration leaves its type unapproved and makes every
//! decode into it fail. [`macro_enumeration_problems`] makes a forgotten macro family
//! legible as one message naming that macro.
//!
//! Approval means "declared with a bridge-capable macro", **not** "has a bridge". A
//! `#[str_newtype(secret)]` or `no_sqlx` type carries `StrNewtype` and emits none, so it is
//! approved here while being undecodable in fact — harmless, since the compiler rejects a
//! decode into a type with no `Decode` impl. (`#[text_enum]`'s bridge is opt-in via an
//! `sqlx` flag the gate can read, so there the answer is exact.)
//!
//! ## Composites are approved by delegation
//!
//! A `#[derive(FromRow)]` struct or tuple alias declared under [`POLICED_ROOT`] passes as a
//! target only because every field and element is separately policed at its declaration.
//! A custom row policy decodes a fully policed intermediate row and then converts it.
//!
//! A hand-written `sqlx::FromRow` is approved only after every matching implementation for
//! its simple self type passes a narrow syntactic proof. Its `from_row` body must be flat
//! `let` statements followed by `Ok(Self { … })`; every use of its `row` parameter must be
//! the direct receiver of `row.try_get::<ConcreteType, _>(one_column_index)?`. There are no
//! aliases, shadowing, UFCS, helper flow, untyped gets, alternate access, delegation, nested
//! scopes/items, macros, or attributes on handwritten decoder nodes in that grammar. A `let`
//! that never mentions `row` may transform an already-decoded binding. Every other
//! hand-written decoder remains unapproved.
//!
//! A bridge-carrying type is approved wherever declared because the bridge is the whole
//! claim. A derived composite is approved only under the policed root, where the field check
//! runs. `Result<T, E>` recurses into `T` only — the error arm is never decoded from a column.
//!
//! # The population — decode targets whose type is written down
//!
//! `syn` has no type inference, so one record is made for every decode call
//! (`query_scalar`, `query_as`, `get`, `try_get`) whose target is the nearest declared type:
//!
//! 1. a turbofish on the call itself — `query_scalar::<_, PostId>(…)`;
//! 2. else the enclosing `let`'s ascription — `let id: PostId = query_scalar(…)`;
//! 3. else the enclosing function or trait-default-method return type.
//!
//! A `let` or `fn` covering several calls yields one record per call. Separately, declared
//! decode targets are policed per field: a `#[derive(FromRow)]` struct's fields and a tuple
//! alias's elements. This backs composite delegation rather than replacing it.
//!
//! # What this gate cannot read, stated rather than papered over
//!
//! - **A `.get`/`try_get` with neither turbofish nor ascription.** `syn` cannot tell
//!   `sqlx::Row::get` from `serde_json::Map::get`, and receiver-name heuristics would both
//!   violate ADR-0085 and miss real sites.
//! - **A decode whose type is pinned only by later use** — an unascribed `let` whose value is
//!   later pushed into a typed collection.
//!
//! A `.get`/`try_get` in a **struct-literal field position** is not an exception to those
//! limits. The gate cannot reliably follow that field to its declaration, so it fails and
//! requires a type at the call.
//!
//! The structural population deliberately over-bites: an unascribed `.get(…)` on something
//! that is not a row inside a function whose return type is unapproved is recorded too.
//! Distinguishing receivers would be the forbidden heuristic. Likewise, every tuple alias
//! under the root is policed because `syn` cannot prove which aliases are `query_as` targets.
//!
//! # What this gate does not claim
//!
//! Type identity is not column correspondence. It can prove a target is **a** domain or
//! persisted-role type; it can never prove it is the **right** one. It reads no SQL and does
//! not infer a target from later use.
//!
//! # Roots
//!
//! **Policed:** `storage/src` only. **Scanned for declarations:**
//! [`DECLARATION_ROOTS`] — wider, because storage decode targets are declared elsewhere.
//! An unreadable or unparseable input under either set is a hard failure rather than a
//! population shrink.

mod approve_set;
mod macros_audit;
mod report;
mod scan;

pub use report::run;
