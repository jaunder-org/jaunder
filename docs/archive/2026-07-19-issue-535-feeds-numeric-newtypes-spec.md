# Spec — issue #535: validated numeric-value newtypes for `feeds.min_items` / `feeds.min_days` (co-lands #464)

- Issue: [#535](https://github.com/jaunder-org/jaunder/issues/535)
- Co-lands: [#464](https://github.com/jaunder-org/jaunder/issues/464) (extract
  the home-grown numeric-value-newtype macro; convert `RetentionCount`)
- Date: 2026-07-19
- Milestone: Domain-value type safety (newtypes)

## Goal

`RetentionCount` (#455) is the only validated integer **value** newtype in the
tree, and its machinery is hand-written. This issue is the deliberate **second
case** that unblocks #464: introduce validated numeric newtypes for the two feed
window settings, and in the same change **extract the shared machinery into a
`macros`-crate derive** and **convert `RetentionCount`** to it. Per the #455 /
#464 rule-of-three, the macro is designed against the concrete invariants in
hand — `RetentionCount` (`usize`, min-1) and the feed settings (`u32`, min-1) —
and generalized (configurable `inner` type + `min`/`max` bounds) so the two
already-filed siblings adopt it unchanged.

## Scope

Three coupled parts, one coherent goal (the numeric-value-newtype mechanism +
its first two adopters):

1. **`#[derive(NumNewtype)]`** in `macros/`.
2. **Convert `RetentionCount`** to it.
3. **`FeedMinItems` / `FeedMinDays`** newtypes threaded through `FeedsConfig`,
   `HybridWindow`, the `site_config` read/write path, and all consumers.

Out of scope (separate filed issues, `blocked-by #464`): media byte-limits
(#536), pagination `PageSize` (#537). The macro is designed so both adopt it
with **no further macro change**, but neither is converted here.

## Decisions (interview outcomes)

- **Feed invariant = min-1.** A "minimum" of 0 items is a no-op and
  `min_days = 0` is a degenerate "no history window"; both feed settings require
  `>= 1`. This gives them a real rejecting invariant (a proper validated-value
  case for #464) and exercises the macro's rejection path. **Minor behavior
  change:** a stored `"0"` (or `set feeds.min_items 0` via CLI) currently parses
  to `0`; it is now rejected — on the read path it falls back to the default (20
  / 30, unchanged `unwrap_or(DEFAULT)` behavior), and at the CLI/setter boundary
  it errors instead of storing degenerate config. No existing test uses 0.
- **Full propagation (ADR-0063 §5).** The newtypes land on **both**
  `FeedsConfig` and `HybridWindow`; raw integers appear only at true edges (the
  `as usize` slice index and `Duration::days` in `window.rs`, the `i64::from`
  SQL bind in `posts.rs`).
- **No enforcement gate.** These are config values with no security/trust
  surface (cf. the i64 IDs, which shipped no gate). The macro's `compile_error!`
  wrong-shape guard is the only structural enforcement.
- **ADR:** amend **ADR-0063** with a "Numeric values" subsection (§2/§3 family),
  documenting the `NumNewtype` trailer and the bound-attribute design — not a
  new ADR (same convention family as `StrNewtype`/`IdNewtype`).

## Part 1 — `#[derive(NumNewtype)]`

New `macros/src/num_newtype.rs`, wired in `macros/src/lib.rs` as a third
`#[proc_macro_derive(NumNewtype, attributes(num_newtype))]`. Reuses
`require_newtype_shape(input, "NumNewtype", "struct X(u32)")`.

### Attribute API

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(inner = u32, min = 1, default = 20)]
pub struct FeedMinItems(u32);
```

- `inner = <ty>` — **required.** The wrapped integer type (`u32`, `usize`,
  `i64`, …). The single tuple field must be exactly this type (checked; mismatch
  → `compile_error!`).
- `min = <lit>` — optional lower bound (inclusive). Absent → no lower bound.
- `max = <lit>` — optional upper bound (inclusive). Absent → no upper bound.
  (Not exercised by this issue's adopters; included because the design must
  carry it for #537's range and because a bound pair is the natural general
  shape. Covered by macros-crate unit tests here.)
- `default = <lit>` — optional. Present → generate `Default`. The value must
  satisfy the bounds (enforced by a `const` bound-assertion in the generated
  `Default`, so an out-of-range default is a **compile error**, mirroring
  `RetentionCount`'s current `const DEFAULT` guard — no runtime `unwrap`).
- `error = "<msg>"` — optional per-type error `Display` message. Absent → a
  generated default message naming the type and its bounds.

Unknown option / missing `inner` / inner-type mismatch → spanned
`compile_error!` (like `str_newtype`'s `parse_opts`).

### Generated trailer

For `struct X(I)` with bounds:

- **A validating constructor + `FromStr`.** `from_str` trims, parses `I` via
  `I::from_str`, then checks the bounds; on any failure returns the generated
  error type `InvalidX` (a unit struct with the `Display` message). This is the
  single chokepoint (mirrors `RetentionCount::from_str`).
- **`get(self) -> I`** (`#[must_use]`) — the inner accessor. (`Copy` inners: by
  value, as `RetentionCount::get` today.)
- **`Display`** — delegates to the inner's `Display`.
- **`Default`** (only when `default` given) — the compile-checked const.
- **Validating serde bridge** — direct `Serialize` (delegates to the inner) /
  `Deserialize` (deserialize the inner, then run the bound check, rejecting
  out-of-range on the wire). Matches `RetentionCount`'s transparent-integer
  serde and `StrNewtype`'s validate-on-deserialize. Always emitted (harmless for
  the feed types, required by `RetentionCount`).

The std derives (`Clone`, `Copy`, `Debug`, `Eq`, `PartialEq`, `Hash`, `Ord`)
stay in the user's `#[derive(...)]` list, per the ADR-0063 convention.

### Error type

The macro generates `struct InvalidX;` (deriving `Debug` via emitted tokens)
with a **hand-generated `Display` + `impl ::std::error::Error`** carrying the
`error` message (or a default). Deliberately **no `thiserror` in generated
code** — the existing `InvalidRetentionCount` uses `thiserror`, but a derive
that emits `::thiserror::Error` would force every future adopter crate (#536
storage/host, #537 web/server) to carry `thiserror`; a direct `Display`/`Error`
pair keeps the macro self-contained (only `::core`/`::std` paths, like the
str/id derives). The `Serialize`/`Deserialize` still reference `::serde`,
matching the existing derives (every adopter already has `serde`).

### Tests (macros crate is coverage-measured)

Unit tests in `macros/src/lib.rs` `tests` (the `syn::parse_quote!` +
`expand(...).to_string().contains(...)` pattern already used for str/id):

- wrong shape → `compile_error!`;
- missing `inner` → `compile_error!`;
- inner-type mismatch (field `u32` but `inner = i64`) → `compile_error!`;
- unknown option → `compile_error!`;
- `min` + `max` + `default` all present → emits `FromStr`, bound check,
  `Default`, serde;
- `max`-only and `min`-only variants emit the corresponding single-sided check.

Plus doctests in the `NumNewtype` rustdoc: a passing fixture
(`#[num_newtype(inner = u32, min = 1, default = 1)] struct Ok(u32);`) exercising
parse-accept, parse-reject-below-min, `Default`, and serde round-trip +
wire-rejection; and a `compile_fail` on a named struct. (Doctests are invisible
to coverage — the runtime error-path coverage comes from the unit tests above,
per the existing note in `lib.rs`.)

## Part 2 — convert `RetentionCount`

`common/src/backup.rs`:

- Replace the hand-written `struct RetentionCount(NonZeroUsize)` + `FromStr` +
  `Display` + `Default` + `InvalidRetentionCount` with:

  ```rust
  #[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
  #[num_newtype(inner = usize, min = 1, default = 7,
      error = "backup retention count must be a whole number of at least 1")]
  pub struct RetentionCount(usize);
  ```

- Inner type changes `NonZeroUsize` → `usize`; the non-zero guarantee is now the
  newtype's own min-1 invariant (only constructible via the validating `FromStr`
  / `Default`). `get()` still returns `usize` — **no consumer change**
  (`prune_backups` etc. already call `.get()`).
- Delete `DEFAULT_BACKUP_RETENTION_COUNT`'s bespoke
  `const DEFAULT: NonZeroUsize` dance (the macro's compile-checked default
  replaces it); keep a `default = 7`.
- The existing `RetentionCount` unit tests (`common/src/backup.rs` `tests`) stay
  and must pass **unchanged in intent**: parse accepts `1`/`  7  `, rejects
  `0`/``/`-1`/`abc`/`1.5`; error message starts `"backup retention
  count"`; `Default::get() ==
  7`; `Display`round-trips; serde serializes`5`→`"5"`, rejects `"0"` on the
  wire. These become the macro's integration proof.
- Serialized/wire shape unchanged (bare integer). No storage migration.

## Part 3 — `FeedMinItems` / `FeedMinDays` + threading

### New types (`common/src/feed/`)

Add to a small module (e.g. `common/src/feed/settings.rs`, re-exported from
`feed/mod.rs`) — kept out of `mod.rs`'s re-export clutter, near `FeedsConfig`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(inner = u32, min = 1, default = 20)]
pub struct FeedMinItems(u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(inner = u32, min = 1, default = 30)]
pub struct FeedMinDays(u32);
```

Unit tests in that module: parse-accept, reject-`0`, `Default::get()` = 20 / 30,
`Display` round-trip, serde round-trip + wire-rejection of `0`. (These + the
macro unit tests + `RetentionCount`'s satisfy the "error/validation paths
covered" acceptance bullet.)

### `FeedsConfig` (`common/src/feed/mod.rs`)

```rust
pub struct FeedsConfig {
    pub min_items: FeedMinItems,
    pub min_days: FeedMinDays,
    pub websub_hub_url: Option<String>,
}
```

`FeedsConfig` currently derives no serde — leave that as-is (the types serde
themselves if it ever gains it; not needed now).

### `HybridWindow` (`common/src/feed/window.rs`)

```rust
pub struct HybridWindow {
    pub min_items: FeedMinItems,
    pub min_days: FeedMinDays,
}
```

- `Default` →
  `{ min_items: FeedMinItems::default(), min_days: FeedMinDays::default() }`
  (still 20 / 30). Stays `#[derive(... Copy ...)]` (u32-backed newtypes are
  `Copy`).
- `cutoff_date`: `Duration::days(i64::from(self.min_days.get()))`.
- `select`: `if i < self.min_items.get() as usize || …`.
- The module's own tests construct
  `HybridWindow { min_items: FeedMinItems::default(), … }` or via a test helper
  (see below) — the raw-`u32` struct literals (`min_items: 20`) become typed
  constructions.

### Storage (`storage/src/site_config.rs`)

- `get_feeds_min_items` / `get_feeds_min_days` return `FeedMinItems` /
  `FeedMinDays`: `.and_then(|v| v.trim().parse().ok()).unwrap_or_default()` (the
  newtype's `FromStr` + `Default`; drop the free-standing
  `DEFAULT_FEEDS_MIN_ITEMS` / `_MIN_DAYS` consts, or keep them only if still
  referenced elsewhere — audit and remove if now dead).
- `get_feeds_config` builds the typed `FeedsConfig` (no `.await?`-shape change).
- `set_feeds_config` writes `config.min_items.to_string()` (unchanged —
  `Display`).
- The `site_config` tests asserting
  `config.min_items == DEFAULT_FEEDS_MIN_ITEMS` / literal `50` / `60` become
  typed comparisons (`== FeedMinItems::default()` /
  `== FeedMinItems::from_str("50")?` via the shared test helper).

### Consumers

- `server/src/feed/regenerate.rs:44-45` —
  `HybridWindow { min_items: feeds.min_items, min_days: feeds.min_days }` now
  moves the newtypes straight through (both are `FeedMin*`); no `.get()` needed
  here.
- `storage/src/posts.rs:1613` —
  `let min_items = i64::from(window.min_items.get());` (edge conversion for the
  SQL bind).
- `server/tests/storage/mod.rs`, `server/src/feed/worker.rs`,
  `server/src/feed/regenerate.rs` test blocks — the
  `HybridWindow { min_items: N, min_days: M }` literals become typed
  constructions via the test helper.

### Test-helper convention

Per the repo's newtype test-helper convention, add
`common::test_support::parse_feed_min_items(&str) -> FeedMinItems` /
`parse_feed_min_days(&str) -> FeedMinDays` to `common/src/test_support.rs`,
siblings of the existing `parse_retention_count(&str)` / `parse_email(&str)` / …
helpers (same `parse_<name>(&str)` shape). Use them at every `cfg(test)`
construction site across `common`, `storage`, and `server/tests` rather than
inline `.parse().unwrap()`. The `HybridWindow { min_items: 20, min_days: 30 }`
literals become `parse_feed_min_items("20")` / `parse_feed_min_days("30")`.

## Acceptance

- `#[derive(NumNewtype)]` exists in `macros/`, with the wrong-shape / bad-option
  `compile_error!` guard; its error + validation paths covered by macros-crate
  unit tests.
- `RetentionCount` and `FeedMinItems`/`FeedMinDays` are all built via the
  derive; no hand-written numeric-newtype trailer remains.
- `FeedsConfig` + `HybridWindow` + the `site_config` path + all consumers carry
  the newtypes; raw ints only at the documented edges.
- Feed settings reject `0`; `RetentionCount` behavior/wire shape unchanged.
- ADR-0063 amended with the numeric-value subsection.
- `cargo xtask validate --no-e2e` clean (both SQLite and Postgres via the
  existing `#[apply(backends)]` site-config tests).

## Risks / notes

- **`thiserror` reachability in generated code.** The generated `InvalidX`
  derives `thiserror::Error`; confirm the path used resolves in every adopter
  crate (`common` has `thiserror`). If a crate could lack it, fall back to a
  hand-written `Display`/`Error` impl in the generated tokens (no external dep).
  Resolve in the plan's first task.
- **Dead default consts.** `DEFAULT_FEEDS_MIN_ITEMS` / `_MIN_DAYS` and
  `DEFAULT_BACKUP_RETENTION_COUNT` may become dead once defaults move into the
  derive; audit and remove (or the `default = N` literal cites them — decide in
  plan).
- **`#[num_newtype]` inner-type check.** The macro must verify the tuple field
  type equals `inner` (a `syn::Type` string compare) so `inner` can't silently
  disagree with the field; covered by a unit test.
- Post-merge, **#464 is satisfied** (macro + two adopters) and should close with
  this PR; #536 / #537 remain, now unblocked (macro exists).
