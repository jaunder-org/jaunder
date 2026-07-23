# `PageOffset` newtype (#588) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

Spec:
[`docs/superpowers/specs/2026-07-23-issue-588-page-offset.md`](../specs/2026-07-23-issue-588-page-offset.md).
Issue: [#588](https://github.com/jaunder-org/jaunder/issues/588).

**Goal:** De-transpose the media-listing `(limit, offset)` pair by typing the
offset as an unbounded `PageOffset` newtype over `u32`.

**Architecture:** A `NumNewtype` in `common::pagination` beside `PageSize`, but
with no range bound (misuse-prevention only). Threaded through
`MediaStorage::list_media` (offset only — `limit` stays bare `u32` per the #537
erasure) and the `list_my_media` wire arg. No web-form or e2e surface (offset is
programmatic pagination).

**Tech Stack:** Rust, `macros::NumNewtype` (#464/ADR-0063), `sqlx`, dual-backend
storage tests.

## Review header

**Scope (in):** `common::pagination` (the newtype), `common::test_support`
(`parse_page_offset`), `storage/src/media.rs` (trait + single generic impl + 2
binds + 2 tests), `web/src/media/api.rs` (wire arg),
`web/src/media/component.rs:207` (caller), `server/tests/storage/mod.rs` (4
offset literals).

**Scope (out):** the storage `limit: u32` (the #537 fetch-limit erasure — spec
decision 2); ADR-0065 client validation (offset isn't a form field — decision
3); any range bound (unbounded — decision 1).

**Tasks:**

1. `PageOffset` unbounded `NumNewtype` in `common::pagination` +
   `parse_page_offset` helper + unit tests.
2. Thread `offset: PageOffset` through `list_media` (trait+impl+binds),
   `list_my_media` (wire), the component caller, and the `cfg(test)` offset
   literals.

**Key risks/decisions:**

- **First fully-unbounded `NumNewtype`** — the generated `TryFrom<u32>` is an
  always-`Ok` body with no in-tree precedent for coverage; Task 1 exercises it
  explicitly.
- The type change (Task 2) is one atomic compile unit: trait, generic impl, wire
  arg, caller, and every `cfg(test)` offset literal move together.

## Global Constraints

- **Unbounded newtype**: no `min`/`max`/`clamp`; `default = 0`;
  `error = "page offset must be a whole number"`.
- **No `Co-Authored-By` trailer** on commits.
- **Test construction** via `common::test_support::parse_page_offset` — never an
  inline `.parse()`/`TryFrom` at a fixture site.
- **Per-commit gate:** the pre-commit hook runs `cargo xtask check`; run it
  first so it passes clean (**jaunder-commit**).
- **Storage tests are dual-backend** (`#[apply(backends)]`) — the existing
  `list_media_*` tests already are; keep them so.

---

### Task 1: `PageOffset` newtype + test helper

**Files:**

- Modify: `common/src/pagination.rs` (add `PageOffset` beside `PageSize`)
- Modify: `common/src/test_support.rs` (add `parse_page_offset` beside
  `parse_page_size`)

**Interfaces:**

- Consumes: `macros::NumNewtype` (already imported in `pagination.rs`).
- Produces:
  - `common::pagination::PageOffset` —
    `#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)] #[num_newtype(inner = u32, default = 0, error = "page offset must be a whole number")] pub struct PageOffset(u32);`.
    Trailer: `value()`, `From<Self> for u32`, `TryFrom<u32>` (always-`Ok`),
    `FromStr`/serde (transparent integer; rejects non-integer/negative),
    `Display`, `Default` (0), and the generated `InvalidPageOffset` error.
  - `common::test_support::parse_page_offset(s: &str) -> PageOffset`.

- [ ] **Step 1: Write the failing tests** — append a `PageOffset` test to
      `common/src/pagination.rs`'s `#[cfg(test)] mod tests` (mirror the
      `page_size_surface` shape):

```rust
#[test]
fn page_offset_surface() {
    // value()/From<Self>, trim, and the full u32 domain is valid (no upper bound).
    assert_eq!("0".parse::<PageOffset>().map(u32::from).ok(), Some(0));
    assert_eq!(
        "  4294967295  ".parse::<PageOffset>().map(PageOffset::value).ok(),
        Some(u32::MAX)
    );
    // FromStr rejects non-integers / negatives (the only error path)...
    for bad in ["abc", "-1", "1.5"] {
        assert!(bad.parse::<PageOffset>().is_err(), "{bad} should reject");
    }
    // ...with the domain message.
    assert!("abc"
        .parse::<PageOffset>()
        .err()
        .is_some_and(|e| e.to_string().starts_with("page offset")));
    // Default is 0 and Display round-trips.
    let d = PageOffset::default();
    assert_eq!(d.value(), 0);
    assert_eq!(d.to_string().parse::<PageOffset>().ok(), Some(d));
    // serde: bare integer, round-trip, wire-rejection of a non-integer.
    assert_eq!(serde_json::to_string(&d).ok(), Some("0".to_owned()));
    assert_eq!(
        serde_json::from_str::<PageOffset>("42").map(u32::from).ok(),
        Some(42)
    );
    assert!(serde_json::from_str::<PageOffset>("\"x\"").is_err());
    // The generated TryFrom<u32> (always-Ok for the unbounded type) — exercise the region.
    assert_eq!(PageOffset::try_from(7u32).map(u32::from), Ok(7));
    // The shared test-support fixture builds a valid PageOffset (its single door).
    assert_eq!(crate::test_support::parse_page_offset("5").value(), 5);
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p common pagination::tests::page_offset` Expected: FAIL
— `PageOffset` / `parse_page_offset` not defined (compile error).

- [ ] **Step 3: Implement**

In `common/src/pagination.rs`, add the `PageOffset` struct to the signature
above with a doc comment (the full text is in the spec's "the newtype" section).
In `common/src/test_support.rs`, add `use crate::pagination::PageOffset;`
(extend the existing `pagination` import) and:

```rust
/// Parse `s` into a [`PageOffset`] for tests — the single place a test offset literal is
/// parsed, so a malformed fixture fails loudly and the parse isn't re-spelled at every
/// media-listing call site.
///
/// # Panics
///
/// Panics if `s` is not a `u32` (non-integer or negative).
#[must_use]
pub fn parse_page_offset(s: &str) -> PageOffset {
    s.parse().expect("valid test page offset")
}
```

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p common pagination::tests::page_offset` Expected:
PASS.

- [ ] **Step 5: Commit**

```bash
git add common/src/pagination.rs common/src/test_support.rs
git commit -m "feat(common): unbounded PageOffset NumNewtype in common::pagination (#588)"
```

Run `cargo xtask check` first (**jaunder-commit**) — confirms the
fully-unbounded expansion compiles and the `PageOffset` region (incl. `TryFrom`)
is covered.

---

### Task 2: Thread `PageOffset` through the media-listing path

The atomic type change — trait, impl, wire arg, caller, and `cfg(test)` literals
move as one compile unit. No new behavior (the newtype is tested in Task 1); the
existing `list_media_*` and `list_my_media_*` suites are the guard.

**Files:**

- Modify: `storage/src/media.rs` (trait `:87`, impl `:245`, binds `:263`/`:276`,
  tests `:539`/`:591`)
- Modify: `web/src/media/api.rs` (`:85`, `:96`)
- Modify: `web/src/media/component.rs` (`:207`)
- Modify: `server/tests/storage/mod.rs` (`:6951`, `:7139`, `:7186`, `:7194`)

**Interfaces:**

- Consumes: `common::pagination::PageOffset`,
  `common::test_support::parse_page_offset` (Task 1).
- Produces:
  - `MediaStorage::list_media(&self, user_id, source, limit: u32, offset: PageOffset)`.
  - `web::media::list_my_media(source, limit: Option<PageSize>, offset: Option<PageOffset>)`.

- [ ] **Step 1: Change the trait + impl signatures and binds**
      (`storage/src/media.rs`)
  - Trait (`:82-88`) and impl (`:240-246`): `offset: u32` →
    `offset: PageOffset`. `limit: u32` unchanged. Import `PageOffset` (extend
    the `common::…` use, or `use common::pagination::PageOffset;`).
  - Both query binds (`:263`, `:276`): `.bind(i64::from(offset))` →
    `.bind(i64::from(offset.value()))`.

- [ ] **Step 2: Update the wire arg + caller**
  - `web/src/media/api.rs:85`: `offset: Option<u32>` →
    `offset: Option<PageOffset>` (import `PageOffset` from
    `common::pagination`). Line `:96`: `offset.unwrap_or(0)` →
    `offset.unwrap_or_default()`.
  - `web/src/media/component.rs:207`: `Some(0)` → `Some(PageOffset::default())`
    (import `PageOffset`).

- [ ] **Step 3: Sweep the `cfg(test)` offset literals**
  - `storage/src/media.rs:539`, `:591`: `.list_media(user_id, None, 10, 0)` →
    `…, 10, parse_page_offset("0"))` (import `parse_page_offset`).
  - `server/tests/storage/mod.rs:6951`, `:7139`, `:7186`, `:7194`: same — the
    `0` offset → `parse_page_offset("0")` (import `parse_page_offset`; `10`
    limit unchanged).

- [ ] **Step 4: Run the gate, verify green**

Run: `cargo xtask check` (the per-task iterate gate; it runs the Nix coverage
step, so it satisfies AC5's coverage requirement — the exact
`cargo xtask validate --no-e2e` the AC cites runs later at ship). Expected: PASS
— `common` (newtype), `storage` (`list_media_*` dual-backend), `web` (compiles),
and `server` integration (`list_my_media_*`) all green with the typed offset.
Also: `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` and
`cargo check -p web --all-features --all-targets` (the default check skips
server-gated web).

- [ ] **Step 5: Commit**

```bash
git add storage/src/media.rs web/src/media/api.rs web/src/media/component.rs \
  server/tests/storage/mod.rs
git commit -m "refactor(storage,web): type media-listing offset as PageOffset (#588)"
```
