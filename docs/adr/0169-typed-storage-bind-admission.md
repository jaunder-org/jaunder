# ADR-0169: Typed storage bind admission

- Status: accepted
- Date: 2026-08-31
- Issue: [#1146](https://github.com/jaunder-org/jaunder/issues/1146)

## Context

The SQLx bridges from [ADR-0071](0071-sqlx-string-newtype-bridge.md) make a
newtype representable by a database, but `Encode`/`Type` alone do not decide
which values may enter a storage query. The former `sqlx-newtype-bind` check
looked for selected stripping spellings and carried a substring allowlist. It
could not prevent a value being stripped before a helper boundary, and it did
not govern every SQLx value-admission API.

Storage needs a compile-time admission boundary that is independent of a
backend's SQLx capabilities. Values with application semantics remain their
shared domain types through application inputs, storage traits, records,
helpers, and binds. Values that exist only to represent persistence facts remain
storage-owned roles: catalog identifiers, stored payloads and configuration,
backup values, restore cells, counts, and column-specific corrupt fixtures.
Those roles are constructed inside the storage seam and do not introduce
ubiquitous product vocabulary. In particular, dynamic backup/restore retains a
closed `RestoreBindValue` representation and its schema-driven dispatch rather
than acquiring a raw-bind exception.

## Decision

`storage::sql` owns a sealed `StorageBind` registry. Its approved leaves are
explicit repository domain and persistence-role types; it has no blanket
primitive or foreign-representation approval. Approval is backend-independent:
SQLx's existing `Encode` and `Type` bounds still determine whether an approved
value is representable for the chosen database and lifetime. References,
`Option<T>`, `Vec<T>`, and slices preserve approval only when their leaf `T` is
approved. This permits existing PostgreSQL slice-array binds through SQLx's
existing `PgHasArrayType` capability without inventing a SQLite array
abstraction.

The only normal storage value-admission APIs are native SQLx extension traits:

- `bind_storage(value)` on `Query`, `QueryAs`, and `QueryScalar`;
- `push_storage_bind(value)` on `QueryBuilder` and `Separated`.

Each delegates directly once to the native SQLx method, retaining native query
and builder types and their execution/fetch APIs, without allocation,
conversion, cloning, retained query state, or runtime branching. The registry
approves a type's admission to storage, not a value's correspondence to an SQL
placeholder: exact helper and storage-trait signatures, plus query review,
retain that responsibility. It performs no SQL-text or SQL-column inference.

There are no marker, site, module, dialect, administration, or test escapes; no
prebuilt arguments; no `push_bind_unseparated`; and no SQLx query macros under
`storage/src`. Test-only fixture roles are explicitly registered behind
`cfg(test)` or `cfg(any(test, feature = "test-support"))`, so test-support code
is governed by the same typed seam rather than granted a primitive escape. The
migration is a clean cutover: every storage caller uses one of the extension
methods, old raw binds and `sqlx-newtype-bind:allow` markers are removed, and no
compatibility path remains.

The registry's compile contracts prove representative approved domain,
persistence-role, reference, optional, and collection values compile at the
seam, and prove representative strings, integers, booleans, timestamps, JSON,
and bytes do not. Wrong-role substitution is prevented by exact helper and
storage-trait types; the generic registry deliberately does not claim
placeholder identity.

### Residual raw-admission detector

The `sqlx-newtype-bind` static gate is defense in depth, not the normal
admission mechanism. It parses every Rust file under `storage/src`, including
inline tests and test-support code, and fails closed when its root or an input
cannot be read or parsed. It rejects these source-visible raw admission doors:

- methods named `bind`, `try_bind`, `push_bind`, `push_bind_unseparated`, and
  `with_arguments`, including SQLx UFCS forms;
- `Arguments::add`, `Arguments`/`IntoArguments` implementations, direct native
  argument construction, and native argument aliases;
- `query_with`, `query_as_with`, `query_scalar_with`, `__query_with_result`, and
  `__query_scalar_with_result`, including imported aliases; and
- SQLx query and query-file macros.

Its only permitted raw calls are the exact five direct `self.<raw>(value)`
delegations in the typed extension implementations: `bind_storage` delegates to
`bind` for `Query`, `QueryAs`, and `QueryScalar`, and `push_storage_bind`
delegates to `push_bind` for `QueryBuilder` and `Separated`. It tracks local
imports and aliases structurally and treats uncertain method receivers
conservatively, so ambiguity fails rather than becoming an exemption.

This detector is intentionally source analysis, not Rust compilation analysis:
it has no rustc type resolution, call graph, visibility into arbitrary
proc-macro expansion, or SQL-column understanding. The query-macro prohibition
closes the known expansion route through native `Arguments::add` and hidden
result constructors; conservative syntax failures cover unresolved source
shapes. Its bounded claim is therefore that source-visible raw admission syntax
is rejected, while compile-time registry approval is the primary admission
proof. This conforms to [ADR-0085](0085-static-type-safety-gates-enumerate.md)
and its structural-membership refinement
[ADR-0110](0110-gate-population-membership-is-structural.md).

## Consequences

Adding a legitimate bind leaf requires choosing its existing domain type or
naming its exact persistence role, then explicitly registering it. A primitive
representation cannot be laundered through a helper and admitted later. Storage
keeps SQLx's native builder/query ergonomics and backend-specific array support,
while the sealed registry makes the value boundary visible to the compiler.

The residual detector must evolve whenever SQLx exposes a new source-visible
admission door or a Rust syntax form changes its structural model. It remains
honest about the analysis it cannot perform instead of inferring safety from SQL
text, later type use, or macro expansion.
