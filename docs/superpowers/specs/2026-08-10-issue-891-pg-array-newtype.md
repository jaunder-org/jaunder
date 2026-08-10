# Issue #891 — `PgHasArrayType` for the ADR-0071 newtype bridge

## Status

Spec — awaiting approval.

## Background

The ADR-0071 sqlx bridge (`macros/src/sqlx_bridge.rs`) emits three impls for
every newtype that is a first-class column: `Type`, `Encode`, `Decode`. It does
**not** emit `PgHasArrayType`, so a _slice_ of a newtype has no Postgres array
encoding — sqlx derives `Type<Postgres>` for `[T]` and `Vec<T>` from
`T: PgHasArrayType`, and without it `.bind(&[FeedEventId])` will not compile as
an array parameter.

The tree works around this by stripping to a raw slice, and
`storage/src/postgres/feed_events.rs:38-49` is explicit that this is a defect,
not a choice:

> Forced, not chosen: … #715 typed `purge_corrupt`'s parameter and routed it
> through here, which added a fourth caller — so this helper now launders _more_
> strips past both gates, not fewer. That is #716's shape (a strip in one
> function, the bind in another), and closing it means giving `IdNewtype` a
> `PgHasArrayType` rather than deleting this helper.

So each new typed call site makes the strip surface larger. ADR-0063 wants the
newtype to survive to the bind; here it cannot.

## Why now

#891 blocks **#876**. That issue's clean design has Postgres perform the whole
post-tag reconcile in a single data-modifying CTE — atomic by construction, so
no transaction, no guard type, no drop hazard — following the precedent at
`storage/src/postgres/feed_events.rs:59-61`. A single-statement reconcile must
receive the desired tag set as a parameter, i.e. as arrays of `Tag` and
`TagLabel`. Without `PgHasArrayType` that forces exactly the strip this issue
exists to remove.

## Design

### One more impl, opt-in per caller

`bridge()` gains a fourth impl, delegating to `type_inner` exactly as `Type`
does:

```rust
#[automatically_derived]
impl ::sqlx::postgres::PgHasArrayType for #name {
    fn array_type_info() -> ::sqlx::postgres::PgTypeInfo {
        <#type_inner as ::sqlx::postgres::PgHasArrayType>::array_type_info()
    }
    fn array_compatible(ty: &::sqlx::postgres::PgTypeInfo) -> bool {
        <#type_inner as ::sqlx::postgres::PgHasArrayType>::array_compatible(ty)
    }
}
```

`type_inner` is the right inner to delegate to: it is what `Type` reports, so
the array type stays consistent with the scalar type by construction.

**Note there is no `where` clause.** An earlier draft of this spec proposed
`where #type_inner: PgHasArrayType` and argued it would make the impl "inert"
for inners that lack it. **That is wrong.** Unlike the other three impls — which
are generic over `DB`, so their bounds are deferred to use sites — this impl is
fully concrete, making the clause a _trivial bound_ that rustc must discharge at
the definition. An unsatisfied one is `E0277` at the impl, not an unusable impl.
The clause would buy nothing and hide the real constraint, so the emission
decision has to be made by the caller instead.

### Which callers opt in

`BridgeSpec` gains a `pg_array: bool`, and `bridge()` emits the impl only when
it is set. sqlx implements `PgHasArrayType` for `i32`, `i64` and `String` (plus
`&T`, `Option<T>`, `Text<T>`) — **not** `u32`, **not** `usize`.

| Caller                    | `type_inner` | `pg_array` | Why                                          |
| ------------------------- | ------------ | ---------- | -------------------------------------------- |
| `StrNewtype` (both forms) | `String`     | **yes**    | `TEXT[]`; #876 needs `&[Tag]`                |
| `IdNewtype`               | `i64`        | **yes**    | `INT8[]`; retires `raw_ids`                  |
| `#[text_enum(sqlx)]`      | `String`     | **yes**    | same inner as `StrNewtype`                   |
| `NumNewtype`              | declared     | no         | inners include `u32`/`usize` — see below     |
| `SqlxBridge`              | field ty     | no         | no demonstrated need; keep the surface small |

`NumNewtype` is not a hypothetical risk, it is a certainty: its bridge is
unconditional (`macros/src/num_newtype.rs:112`, "there is no opt-out"), and
`PageSize` (`common/src/pagination.rs:22`, `u32`), `FeedMinItems` and
`FeedMinDays` (`common/src/feed/settings.rs:12`, `:23`) and the `usize`
retention newtype (`common/src/backup.rs:141`) would each be `E0277` the moment
`common`'s `sqlx` feature is enabled — which `storage/Cargo.toml:12` always
does.

`SqlxBridge`'s two current call sites both resolve to `String`
(`macros/src/sqlx_bridge_derive.rs:40`, `:72`), so enabling it would be _safe
today_ — but a future plain `SqlxBridge` over a non-arrayable field type would
break the build at a distance. Nothing needs it, so it stays off; turning it on
later is a one-line change with a test to justify it.

The flag threads through **seven** production call sites: `id_newtype.rs:102`,
`str_newtype.rs:338` and `:355` (validating and infallible forms),
`text_enum.rs:221`, `num_newtype.rs:110`, and `sqlx_bridge_derive.rs:38` and
`:70` — plus the two `BridgeSpec` literals inside `sqlx_bridge.rs`'s own `tests`
module (`:123`, `:167`).

### No new feature gate

The issue text speculated this "needs to be gated accordingly" because the impl
is Postgres-specific while the rest of the bridge is generic over `DB`. **That
turns out to be unnecessary**, and the spec corrects it: the workspace pins
`sqlx = { version = "0.8", features = ["sqlite", "postgres", …] }`
(`Cargo.toml:70`), so whenever the `sqlx` feature is on, `::sqlx::postgres`
exists. The impl goes inside the existing
`#[cfg(feature = "sqlx")] const _: () = { … }` block and needs nothing further.
The wasm build never enables `sqlx`, so it never sees any of it.

### The proof is deleting the workaround

`raw_ids` (`storage/src/postgres/feed_events.rs:47-49`) is deleted and its five
call sites (`:27`, `:91`, `:102`, `:117`, `:137`) bind `&[FeedEventId]`
directly. That is the real acceptance test: the helper cannot go until the impl
works.

## Acceptance criteria

1. **AC1 — the impl is emitted, opt-in.** `BridgeSpec` carries `pg_array: bool`;
   `bridge()` emits a `PgHasArrayType` impl delegating to `type_inner` **only**
   when set, inside the existing `#[cfg(feature = "sqlx")]` block, with **no
   `where` clause** and no new feature gate. Macro unit tests assert both the
   delegation when on and the absence of the impl when off, in the style of
   `type_impl_delegates_to_type_inner`.

2. **AC2 — the opt-in matrix matches the table above.** `StrNewtype` (both
   forms), `IdNewtype` and `#[text_enum(sqlx)]` set it; `NumNewtype` and
   `SqlxBridge` do not. **Nine** `BridgeSpec` construction sites are updated:
   the seven production ones listed in the Design section, plus the two literals
   in `sqlx_bridge.rs`'s own `tests` module.

3. **AC3 — the non-arrayable newtypes still compile.** `common` builds with its
   `sqlx` feature on. Named as its own criterion because `PageSize`,
   `FeedMinItems`, `FeedMinDays` and the `usize` retention newtype are exactly
   what a uniformly-emitted impl would break, and a green
   `cargo check -p common` is the cheapest possible proof that the opt-in is
   doing its job.

4. **AC4 — `#[automatically_derived]` count is conditional.** The existing test
   `output_is_feature_gated_and_marked_derived`
   (`macros/src/sqlx_bridge.rs:185`) asserts exactly **3** markers. It stays 3
   for a `pg_array: false` spec and becomes 4 for a `true` one — so the test is
   _split or parameterised_ rather than having its number bumped. Called out
   because a bare `3 → 4` edit would silently assert the wrong thing for half
   the callers.

5. **AC5 — id newtype slices bind.** `raw_ids` is deleted from
   `storage/src/postgres/feed_events.rs`, and all five call sites (`:27`, `:91`,
   `:102`, `:117`, `:137`) bind `&[FeedEventId]` directly. The existing
   feed-events tests pass on Postgres.

6. **AC6 — string newtype slices bind too.** A Postgres test binds a slice of a
   `StrNewtype` as an array parameter and reads the rows back. `feed_events`
   only exercises the id case (the only `= ANY(` sites in `storage/` are its
   five), and #876 needs the string case. **Placement and parity:** `storage/`
   tests are backend-parameterised, and this one is necessarily Postgres-only,
   so it uses the `postgres_only` rstest template that already exists for this
   purpose (`storage/src/test_support.rs`) rather than `backends`. A comment
   states why parity does not apply: SQLite has no array type, so there is no
   SQLite behaviour to match.

7. **AC7 — no new strips, no new allowlist.** The `sqlx-newtype-bind` and
   `sqlx-newtype-decode` gate steps stay green with **no** new allowlist
   entries, and the diff adds no new `Vec<i64>`-style unwrapping helper.

8. **AC8 — the docs that count the impls are updated.** `raw_ids`' doc block
   (`storage/src/postgres/feed_events.rs:38-46` — the block; `:47-49` is the fn)
   goes with the function. `macros/src/sqlx_bridge.rs`'s module doc (`:1-23`,
   "The three impls", plus the per-caller table) and `bridge()`'s own doc
   (`:42`, "The three sqlx bridge impls") are updated to describe four, and the
   table gains the `pg_array` column.

9. **AC9 — the coverage gate is green, and this was checked early.** ADR-0050's
   gate is stateless: every uncovered line must be exempted, with no baseline.
   This change adds two generated fn bodies to every opted-in type across
   `common`, of which only a couple will ever execute. Whether llvm-cov
   attributes those regions to the covered `#[derive(…)]` span (harmless) or to
   fresh uncovered lines (gate red, on the order of 100 lines) is **not**
   established. The plan's first implementation task must probe this before the
   rest is built on it.

10. **AC10 — the gate is green.** `cargo xtask validate --no-e2e` passes. The
    diff touches `macros/` and `storage/` only, with no web, server-fn, or
    browser surface; CI runs the full matrix regardless.

## Out of scope

- **#716 itself.** Its named site (`FeedMinItems` at `storage/src/posts.rs`) is
  a _scalar_ `i64` parameter, a different shape from the array strip fixed here.
  This issue removes one mechanism that produces #716-shaped code; it does not
  close #716.
- **#876's reconcile.** This is the unblocking step only.
- Any `SqliteHasArrayType` equivalent — SQLite has no array type and needs none.

## Risks

- **The coverage gate** (AC9) is the main unknown, and the plan probes it first.
- **`SqlxBridge` stays off** even though both current sites would be safe. If a
  future caller wants arrays, the change is one line plus a test — deliberately
  not pre-enabled.

## Verified before writing

Recorded so a later reader does not re-derive them:

- sqlx is **0.8.6**. `PgHasArrayType`
  (`sqlx-postgres-0.8.6/src/types/array.rs:51-56`) has exactly
  `array_type_info() -> PgTypeInfo` and `array_compatible(&PgTypeInfo) -> bool`.
- The bind chain closes: `Type<Postgres> for [T]` (array.rs:94) comes from
  `T: PgHasArrayType`, and `Encode<'q, Postgres> for &[T]` (array.rs:154) from
  `T: Encode + Type` — both already emitted by the bridge. So
  `.bind(&[FeedEventId])` compiles once the impl exists.
- `::sqlx::postgres::PgHasArrayType` resolves (`sqlx-0.8.6/src/lib.rs:55`
  re-exports `sqlx_postgres as postgres`; the trait is at that crate's root).
- No crate narrows sqlx's features: `common`, `storage` and `host` all inherit
  the workspace pin, and `macros` never depends on sqlx at all.

## Correction log

An earlier draft proposed emitting the impl for **every** bridge caller,
contained by `where #type_inner: PgHasArrayType`. Both halves were wrong: the
clause is a _trivial bound_ on a concrete impl, so rustc discharges it at the
definition (`E0277`) rather than deferring it to use sites, and four existing
`NumNewtype`s over `u32`/`usize` would therefore have failed to compile as soon
as `common`'s `sqlx` feature was on. The opt-in table replaces it.
