# Spec — #499: model `AudienceSelection.base` as an `AudienceBase` enum

- **Issue:** jaunder-org/jaunder#499
- **Milestone:** Domain-value type safety (newtypes)
- **Date:** 2026-07-17

## Problem

`AudienceSelection.base` (`web/src/posts/mod.rs:84`) is a `String` holding one
of three known values — `"public"`, `"subscribers"`, `"private"` — compared and
constructed via string literals on both ends of the wire (~10 sites). An invalid
`base` is representable in the type and is only caught by a `_ => …`
fall-through in the server-side match. This spec makes an invalid base
**unrepresentable** past the DOM edge by introducing a three-variant enum.

## Change

### 1. New `AudienceBase` enum (`common/src/visibility.rs`)

A three-variant enum `AudienceBase { Public, Subscribers, Private }`, defined
via a **new `serde` arm of the existing `str_enum!` macro** so its serialization
routes through the same `as_str`/`TryFrom<&str>` literals as the DOM edge — one
source of truth for the strings `"public"`/`"subscribers"`/`"private"`.

- The `serde` arm is **additive**: it expands the normal `str_enum!` body
  (giving `as_str`, `TryFrom<&str>`, `Display`, and the standard derives)
  **and** emits `serde::Serialize` (via `as_str` → `serialize_str`) and
  `serde::Deserialize` (via `String` → `TryFrom<&str>`, mapping an unknown
  string to `serde::de::Error::unknown_variant`). The three existing invocations
  (`Channel`, `SubscriptionStatus`, `TargetKind`) keep using the non-serde arm
  and gain no serde.
- `AudienceBase` gets a hand-written `impl Default` returning `Private` (the
  macro cannot choose a default variant). `Private` is author-only — the safe,
  non-widening default, faithful to today's empty-string → author-only
  fall-through.
- `TargetKind` is **not** reused: it has no author-only/`Private` variant.

### 2. `AudienceSelection.base: String` → `AudienceBase` (`web/src/posts/mod.rs`)

- Field type changes; the struct keeps its existing derives, including `Default`
  (now satisfied by `AudienceBase: Default`).
- `audience_selection_to_targets` (`:100`): the `match selection.base.as_str()`
  becomes an **exhaustive** `match selection.base` over the three variants —
  `Public => Some(Public)`, `Subscribers => Some(Subscribers)`,
  `Private => None` (author-only). No `_` arm.
- `targets_to_audience_selection` (`:140`): the `&str` accumulator becomes an
  `AudienceBase`, initialised to `AudienceBase::Private`, assigned
  `AudienceBase::Public` / `AudienceBase::Subscribers` as those targets are
  seen; the final `base.to_string()` becomes the enum value directly.

### 3. DOM edge (`web/src/pages/ui.rs`)

- `AudiencePicker` `on:change` (`:477`): parse the `<select>` value once via
  `AudienceBase::try_from(event_target_value(&ev).as_str())`. On `Ok`, assign
  `sel.base`; on the (unreachable, closed-world `<select>`) `Err`, leave the
  selection **unchanged** — never silently widen.
- The `base_options` array (`:463`) — currently the production string literals
  `["public", "subscribers", "private"]` — becomes an array of `AudienceBase`
  variants
  (`[AudienceBase::Public, AudienceBase::Subscribers, AudienceBase::Private]`).
  The `<option>` construction (`:481`–`:495`) uses `base.as_str()` for `value=`
  and compares variants for `selected=` (`:489`): `selection.get().base == base`
  (`AudienceBase` is `Copy + Eq`). `base_labels` (`:464`) are display captions,
  not base values, and stay as-is.
- The "disable named while Private" check (`:532`) compares
  `selection.get().base == AudienceBase::Private`, not a string literal.
- Editor default constructions (`ui.rs:588`, `posts.rs:643`) use
  `AudienceBase::Public`.

### 4. Tests

- The `selection(base, named)` unit helper (`web/src/posts/mod.rs:705`) takes
  `AudienceBase` instead of `&str`.
- The `"nonsense"` unrecognized-base assertion (`:790`) is **removed** — an
  invalid base is no longer constructible (this is the point of the change).
- The two integration assertions (`server/tests/web/web_posts.rs:3479`, `:3528`)
  that read `selection.base` compare against `AudienceBase::Public`; the test
  module adds `use common::visibility::AudienceBase` (`web_posts.rs:10`
  currently imports only `AudienceSelection`).

### 5. Coverage of the new `common` code (`common/src/visibility.rs` is gate-measured)

Keeping `Default` and generating `Display` on the new enum creates regions that
need explicit exercise (ADR-0050 stateless gate):

- **`AudienceBase::default()`** is only reachable via the (dead) struct
  `Default`, so it is otherwise uncovered. A unit test asserts
  `AudienceBase::default() == AudienceBase::Private`, covering the impl **and**
  locking the safe-default choice.
- **Macro-generated `Display`** for `AudienceBase` loses its only production
  caller (the old `base.to_string()`). Extend the existing
  `display_matches_as_str` / round-trip tests (`visibility.rs:129`, `:118`) to
  include `AudienceBase`, matching how the three existing `str_enum!` outputs
  are covered.

## Non-goals

- No xtask gate and no ADR (this is a domain enum, not a security newtype; the
  exhaustive `match` is the enforcement).
- No change to `AudienceTarget`, storage, the wire endpoint shapes, or the
  `Public`/`Subscribers`/`Named` union semantics.
- No change to the observable wire strings
  (`"public"`/`"subscribers"`/`"private"`) or the DOM `<option value>` strings.

## Acceptance criteria

Each is observable so a conformance review can tell delivered from not.

1. **Enum exists, single-source serde.** `common::visibility::AudienceBase` has
   variants `Public`, `Subscribers`, `Private`, is produced by a `serde` arm of
   `str_enum!`, and `impl Default for AudienceBase` returns `Private`. Unit
   tests assert: (a) `serde_json::to_string(&AudienceBase::V)` is exactly
   `"\"public\""`, `"\"subscribers\""`, `"\"private\""` for the three variants;
   (b) `serde_json::from_str::<AudienceBase>` round-trips each and **rejects**
   an unknown string (e.g. `"\"bogus\""`) with an error; (c)
   `AudienceBase::default() == AudienceBase::Private`; and (d)
   `Display`/`as_str`/`TryFrom` for `AudienceBase` are exercised (folded into
   the existing `str_enum!` coverage tests).
2. **Field is typed.** `AudienceSelection.base` is of type `AudienceBase` (not
   `String`).
3. **No stray literals.** No `"public"`, `"subscribers"`, or `"private"` string
   literal for an audience base appears in **production** code outside the
   `str_enum!(serde AudienceBase {…})` definition. In particular
   `web/src/posts/mod.rs`, the `AudiencePicker` comparison/construction sites,
   and the `base_options` array (`ui.rs:463`) contain no such literal. Test and
   e2e code that drives the `<select>` by option value (e.g.
   `selectOption("#audience-base", "public")`) is exempt.
4. **Exhaustive server mapping.** `audience_selection_to_targets` matches
   `AudienceBase` exhaustively with no `_` wildcard arm; `Private` yields
   author-only (empty targets), `Public`/`Subscribers` yield their target
   unioned with `named`.
5. **Invalid base unrepresentable.** The two `&str → AudienceBase` doors — the
   DOM edge (`ui.rs:477`) and the serde `Deserialize` impl — both reject unknown
   strings: the DOM edge leaves the prior selection unchanged, and `Deserialize`
   errors. There is no code path by which an `AudienceSelection` with a base
   outside the three variants can be constructed and reach the server mapping.
6. **Behaviour preserved.** All existing audience round-trip semantics hold:
   `Public`/`Subscribers` union with `named`; `Private` drops `named` and yields
   author-only; absent selection still defaults to `[Public]`; persisted targets
   → selection → targets round-trips (existing
   `targets_round_trip_through_selection` assertions pass, adapted to the enum).
   The e2e `visibility.spec.ts` flows are untouched and pass.
7. **Gate green.** `cargo xtask validate --no-e2e` passes (static + clippy +
   coverage), and the new/changed lines are covered — including the serde
   `Serialize`/`Deserialize` (both happy and reject paths),
   `AudienceBase::default()`, the macro `Display`/`as_str`/`TryFrom`, and the
   exhaustive mapping arms.
