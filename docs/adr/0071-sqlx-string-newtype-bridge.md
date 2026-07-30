# ADR-0071: Transparent sqlx bridge for domain newtypes

- Status: accepted
- Date: 2026-07-20
- Issue: [#438](https://github.com/jaunder-org/jaunder/issues/438),
  [#686](https://github.com/jaunder-org/jaunder/issues/686) (extended to
  `IdNewtype`/`NumNewtype`)

## Context

String domain newtypes (`Username`, `Slug`, `TokenHash`, `InviteCode`, …)
crossed the `sqlx` boundary as bare strings: every bind was `.bind(x.as_ref())`
(stripping the newtype to `&str`), and every read decoded a `String` and
hand-re-parsed it back into the newtype. That hand re-parse was fallible
boilerplate — e.g. `build_invite_record` carried two `cov:ignore` lines for an
"unreachable" error arm — and a newtype was not a first-class DB column type. It
also meant a value's type-safety was lost at the storage edge and manually
reconstructed, a place for the newtype invariant to silently drift.

ADR-0063 already solved the analogous problem for numeric id newtypes with a
transparent `serde` bridge emitted by the `StrNewtype`/id derives. The DB
boundary is the same shape of problem, one layer down.

**#686 extended the decision to the other two families**, which this ADR
originally left out. `IdNewtype` and `NumNewtype` had no bridge, so ids and
bounded numerics were stuck in exactly the pre-#438 shape: ~29 row/tuple
positions declared as bare `i64` and re-wrapped by hand after the read, and 114
`.bind(i64::from(x))` strips before the write. For an **id** that is worse than
for a string: an id newtype's whole purpose is the transposition guarantee
(ADR-0063 §2), and a query returning adjacent bare `i64` `post_id`/`tag_id`
columns hands that guarantee straight back — the SELECT's column order becomes
the only thing pairing them. Nothing about the original reasoning was
string-specific; the bridge was simply written for the family that needed it
first.

Two constraints shape the solution:

- **`common` is target-agnostic (wasm).** The value types live in `common`,
  which the CSR/wasm build compiles and which must never pull `sqlx` (a native,
  non-wasm dependency). `InviteCode` lives in `host` (server-only).
- **`Encode` is a capability, not just a conversion.** Implementing
  `sqlx::Encode` for a type means "this value may be written to the database as
  its raw string." For a few types that is precisely the wrong capability: a
  plaintext `Password` is never stored (only its hash), and a `RawToken` must be
  hashed to a `TokenHash` before it touches a table.

## Decision

The `StrNewtype` derive emits a **transparent, feature-gated sqlx bridge** —
generic `sqlx::Type` + `Encode` + `Decode`, `impl<DB: sqlx::Database>`
delegating to the inner `String` (one impl covers SQLite and Postgres, both
TEXT) — **on by default**, gated behind a `#[cfg(feature = "sqlx")]` that the
wasm build never enables. This mirrors the serde bridge's own shape (ADR-0063):
default-on, `secret` drops it, a secret re-adds it explicitly.

- **Emission rule** (parsed in the derive's `parse_opts`):
  - non-`secret` type → bridge **emitted by default** (no annotation);
  - `secret` → bridge **dropped** (a secret is not storable by default —
    `.bind(password)` will not compile);
  - `secret, sqlx` → **re-adds** the bridge to a secret that genuinely is stored
    (`InviteCode`);
  - `no_sqlx` → the one opt-**out** for a non-`secret` must-not-store type
    (`RawToken`). This is the single place sqlx diverges from serde, justified
    because `Encode` carries a storability semantic `Serialize` does not.
- **`Decode` validates.** For a validating newtype, `Decode` routes the decoded
  string through `FromStr` — keeping the integrity guard the old hand re-parse
  gave (a corrupted/migrated column is rejected, surfacing as
  `sqlx::Error::ColumnDecode`), not silently admitted. For an `infallible`
  newtype it wraps via `From<String>`.
- **Feature isolation.** `common` gains an optional `sqlx` dependency + `sqlx`
  feature; `storage` enables `common/sqlx`. Cargo feature unification is
  per-target-per-graph: `storage` is only ever in the host build, and `web`'s
  CSR/wasm build pulls `common` + `macros` without the feature, so the optional
  dep never compiles for wasm32 — the same isolation that already keeps
  `host`/`storage`/`sqlx` out of the CSR build. A
  `#[cfg(all(target_arch = "wasm32", feature = "sqlx"))] compile_error!` in
  `common` makes any future violation fail loudly.
- **An `xtask` enforcement gate (`sqlx-newtype-bind`)** scans `storage/src` and
  fails on the newtype-stripping bind idioms — the stringly ones
  (`.bind(_.as_ref())`, `.bind(&*_)`, `.bind(&**_)`), since #686 the numeric one
  (`.bind(i64::from(_))`), and since #696 the **hoisted** form of that
  (`let x = i64::from(y); … .bind(x)`, which evaded a scan that only looked
  after `.bind(`) — so a newtype cannot silently be bound as a bare primitive
  again. Its substring-matched allowlist holds **only** two typed
  `Option<_>::as_ref()` binds; **nothing numeric is exempt** (#696 removed the
  last two, below). (#502 retired the `RenderedHtml` entry by giving that type a
  hand-written write-only bridge — see Consequences.)

  **What the gate still cannot see** is a strip laundered through a function
  parameter: the conversion in one function, the `.bind` in another, where
  nothing remains to detect. It is tracked as **#716**.

  This was originally recorded here as "a real limit of a line-based scan rather
  than an oversight". **#715 supersedes that framing.** The gate decides a
  violation by searching for three strip spellings, so anything spelled a fourth
  way — including via a parameter — passes green; and its substring allowlist
  exempts every matching line under the root rather than one site. Both are
  departures from the decision recorded in
  `docs/adr/0085-static-type-safety-gates-enumerate.md`, and #716 now carries
  the scope of rebuilding this gate to enumerate.

- **A second `xtask` gate (`sqlx-newtype-decode`, #715)** covers the other
  direction. The bridges above make a column decode straight into its newtype,
  but nothing checked that it did, so ids kept arriving as bare `i64` and being
  re-wrapped by hand. It parses `storage/src` with `syn` and fails on **every**
  decode target in the `i64` family — across `query_scalar`, `query_as`,
  `let`-ascription, `row.get`, and declared `FromRow`/tuple-alias targets —
  unless an allowlist entry names that exact decode, its multiplicity, and a
  written reason. It inspects no SQL, deliberately: deciding by column name or
  by `COUNT(` would be the same pattern search that let three earlier audits
  report done while residue remained.

**The `IdNewtype` and `NumNewtype` bridges (#686)** have the same
`Type`/`Encode` shape, delegating to the declared inner integer rather than
`String`, and differ from the string bridge — and from each other — only in
`Decode`:

- **`IdNewtype`'s `Decode` is an infallible wrap.** An id has no value invariant
  beyond "is an integer" (ADR-0063 §2), so there is nothing to validate; it
  deliberately does **not** route through the generated `FromStr`, which is a
  non-validating delegate rather than a chokepoint.
- **`NumNewtype`'s `Decode` re-runs the bound**, via the generated
  `TryFrom<inner>` — the same chokepoint `FromStr` and the serde bridge use.
  Skipping it would leave the column a hole in an invariant enforced everywhere
  else. This is a **behaviour change**: an out-of-range stored value is now a
  `ColumnDecode` error rather than a silently-admitted value. The bridge is
  parameterized on the declared `inner`, not hardcoded to `i64`.
- **Neither has an opt-out.** `StrNewtype` needs `secret`/`no_sqlx`/`sqlx`
  because `Encode` carries a storability semantic that genuinely differs per
  type (a plaintext `Password` is never stored; a `RawToken` must be hashed
  first). No id or bounded numeric has such a value — an id is not a credential.
  Add a flag when one appears, not before.

**The residue this left, and how it was closed.** #686 finished with two
`.bind(i64::from(…))` shapes it could not sweep, because they were not newtype
strips: `limit` was a bare `u32` and `PageOffset`'s `inner` was `u32`, and sqlx
implements no Postgres `Encode` for unsigned types, so the widening was forced
by the driver rather than by a missing bridge. They became two documented
allowlist entries.

**#696 removed both by removing their cause** — giving those values `i64`-backed
newtypes with _declared_ bounds (`RowLimit`, `min = 1`; `PageOffset`, `min = 0`)
rather than bounds implied by an unsigned primitive that the boundary discards
anyway. The general rule is recorded in ADR-0063 §2: an unsigned `inner` is not
a substitute for a declared `min` on a value that crosses this boundary. So the
numeric half of the gate is now absolute rather than absolute-with-footnotes.

## Consequences

- Every derive-based newtype — string, id, or bounded numeric — is a first-class
  DB column type: `.bind(newtype)` binds directly and `query_as` decodes
  straight into the newtype. New stored newtypes are DB-ready with no annotation
  and **cannot silently miss the bridge** — the gate enforces it.
- A row tuple can name what its positions mean.
  `query_as::<_, (PostId, TagId, …)>` makes a swapped destructuring a compile
  error, where two adjacent bare `i64`s made it invisible — the transposition
  guarantee now reaches the SQL boundary instead of stopping one layer above it.
- The cost is paid in **where-clauses**. Every generic
  `impl<DB> …Storage for …Store<DB>` restates its row tuples as `FromRow` bounds
  (ADR-0019: supertrait where-clauses don't propagate), so retyping a tuple
  means changing the `query_as` turbofish **and** the bound; changing only one
  removes the `FromRow` impl for the new shape and the error surfaces on the
  _other_ columns. The same applies to nullable binds: sqlx implements `Encode`
  for `Option<T>` per concrete database (`impl_encode_for_option!`), never
  blanket over a generic `DB`, so each `Option<Newtype>` bind must be restated
  too.
- The fallible hand re-parses and their `cov:ignore` debt are retired; several
  record builders became infallible.
- Type-safety is preserved end-to-end across the storage edge; a corrupt column
  is a `ColumnDecode` error at read, not a silently-admitted invalid value. For
  `NumNewtype` that is new behaviour: an out-of-range integer column that was
  previously admitted (or rejected only where someone remembered a hand
  `try_from`) is now rejected uniformly, at the column.
- `secret` **string** types stay bridge-less by default — the derive is now the
  single place that decides storability, and a plaintext secret cannot be bound
  to a query by accident. Ids and bounded numerics have no such case, so their
  bridges are unconditional; the asymmetry is deliberate, not an oversight.
- Commits us to: the `common` optional-`sqlx`-feature seam (kept off for wasm by
  the `compile_error!` guard and the wasm-clippy gate); the `sqlx-newtype-bind`
  gate and its allowlist (two typed `Option<_>::as_ref()` binds after #502 and
  #696 — nothing numeric).
- Rules out per-type hand-written sqlx impls (orphan-rule-bound and duplicative)
  and a storage-side wrapper type (second-class, conversion at every edge) — for
  derive-eligible newtypes. The lone sanctioned exception is `RenderedHtml`
  (#502): a provenance type whose carve-outs (no `FromStr`) rule the derive out,
  so it gets a hand-written bridge. That bridge was **write-only**
  (`Type`+`Encode`, no `Decode`) for as long as a `Decode` would have laundered
  an untrusted column into trusted unescaped HTML; **#445 moved sanitization
  onto the type**, which removed that objection, so it now has a `Decode` too
  and its column no longer needs the gated `from_trusted` rebuild on read
  (ADR-0079).
