# Spec — #588: `PageOffset` newtype for media listing

Issue: [#588](https://github.com/jaunder-org/jaunder/issues/588) — milestone #13
(Domain-value type safety). Family: #537/#556 (`PageSize` — the other half of
the pair), #464 (numeric newtype macro), #583 (permalink triple — same
transposability criterion), ADR-0063.

## Problem

Media listing carries a **transposable same-typed argument pair**:
`MediaStorage::list_media(user_id, source, limit: u32, offset: u32)`
(`storage/src/media.rs:82`, and its single generic impl). Two adjacent bare
`u32`s can be swapped at any call site — it compiles clean and silently returns
the wrong page. At the wire, `list_my_media` (`web/src/media/api.rs:82`) already
types the limit (`Option<PageSize>`, #556) but the offset is a raw
`Option<u32>`, and the storage call erases the limit via `.value()`, re-creating
the bare pair.

## Decisions

1. **`PageOffset` is an unbounded `NumNewtype` over `u32`** in
   `common::pagination`, beside `PageSize`. Unlike `PageSize`'s `1..=50`,
   `offset` has **no range invariant** — the full `u32` domain is valid. The
   newtype's job is **misuse prevention** (the transposability criterion, #583),
   not validation. `#[num_newtype(inner = u32, default = 0, error = "…")]` with
   no `min`/`max`/`clamp`. The macro's `FromStr`/serde still reject a
   non-integer or negative value (the only error path); `default = 0` supports
   `unwrap_or_default()` and `PageOffset::default()`.
2. **Type `offset` only; `limit` stays bare `u32`.** Typing one of a same-typed
   pair is sufficient to de-transpose it — a typed + a raw argument can no
   longer swap silently. The storage `limit` deliberately stays `u32`: that is
   the #537 fetch-limit erasure (posts pass `page_size + 1` for the has-more
   probe), out of scope here.
3. **No ADR-0065 client validation.** `offset` is programmatic pagination
   (computed by the media listing UI), not a user-facing validated form field —
   there is no `ValidatedInput` for it (contrast #581's destination). PageOffset
   being unbounded, there is no invalid in-range value a caller could enter;
   serde rejects a non-integer/negative on the wire.
4. **The storage `list_media` impl is a single generic one**
   (`storage/src/media.rs:240`, over `DB`), not a per-dialect ADR-0019 split —
   so this is one signature + one impl body, not a dual-backend change.

## Design

### `common::pagination` — the newtype

```rust
/// A pagination offset (0-based row offset into a listing). The full `u32` domain is valid —
/// unlike [`PageSize`], there is no range bound; the type exists to de-transpose the
/// `(limit, offset)` pair on the media-listing path (#588), not to validate a range.
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(inner = u32, default = 0, error = "page offset must be a whole number")]
pub struct PageOffset(u32);
```

The generated trailer supplies `value()` + `From<Self> for u32`, `FromStr`/serde
(transparent integer; rejects non-integer/negative), `Display`, and `Default`
(0).

### `common::test_support`

- `parse_page_offset(s: &str) -> PageOffset` beside `parse_page_size`.

### `storage/src/media.rs` — trait + impl

- `MediaStorage::list_media(&self, user_id, source, limit: u32, offset: PageOffset)`
  (trait `:82`, impl `:240`). `limit` unchanged.
- Both query binds (`:263`, `:276`): `.bind(i64::from(offset))` →
  `.bind(i64::from(offset.value()))`.

### `web/src/media/api.rs`

- `list_my_media(source, limit: Option<PageSize>, offset: Option<PageOffset>)`
  (`:85`).
- The storage call (`:96`) `offset.unwrap_or(0)` → `offset.unwrap_or_default()`
  (Option<PageOffset> → PageOffset, default 0).

### `web/src/media/component.rs`

- The one caller (`:207`)
  `list_my_media(None, Some(PageSize::default()), Some(0))` →
  `…, Some(PageOffset::default()))` (import `PageOffset`; preserves the explicit
  offset-0 first-page load).

### Test sweep (`cfg(test)` construction)

- `.list_media(…, 10, 0)` offset literals → `parse_page_offset("0")`:
  `storage/src/media.rs:539,591`;
  `server/tests/storage/mod.rs:6951,7139,7186,7194`. The `10` (limit) stays a
  bare literal.
- `common::pagination` unit test for `PageOffset` (mirrors the `PageSize`
  suite): `value()`/ `From<Self>`, `FromStr` accepts a large `u32` and rejects
  `"abc"`/`"-1"`/`"1.5"` with the domain message, `Default` = 0, serde
  round-trip + wire-rejection of a non-integer, and the `parse_page_offset`
  fixture. **Also exercise the generated `TryFrom<u32>`** (an always-`Ok` body
  for the unbounded type — `PageOffset::try_from(7u32)` → `Ok`), since
  `PageOffset` is the first fully-unbounded `NumNewtype` and that generated
  region is otherwise uncovered.

## Acceptance criteria

1. `PageOffset` exists in `common::pagination` with the ADR-0063 numeric
   trailer, no range bound; `"0"`/`"4294967295"` parse, `"-1"`/`"abc"` reject
   with `"page offset must be a whole number"`; serde is a bare integer
   round-trip and rejects a non-integer on deserialize;
   `PageOffset::default().value() == 0`.
2. No signature on the media-listing path keeps `limit` and `offset` as two bare
   integers: `MediaStorage::list_media`'s `offset` is `PageOffset` and
   `list_my_media`'s is `Option<PageOffset>` (its `limit` remains
   `PageSize`/`u32`).
3. `parse_page_offset` exists in `common::test_support`; every `cfg(test)`
   `list_media` offset argument is a typed `PageOffset`.
4. The media component still loads the first page (offset 0); `list_my_media_*`
   web tests and the `list_media_*` storage tests stay green.
5. `cargo xtask validate --no-e2e` clean (coverage includes the `PageOffset`
   error path).

## Verification

- Unit: the `PageOffset` suite in `common::pagination`.
- Host/integration: existing `list_media_*` (storage) and `list_my_media_*`
  (web) dual-backend tests, with the offset literals now typed.
- No browser surface changes (offset is not a form field); no e2e required.
