# Spec — `PageSize` range newtype for pagination (issue #537)

Status: proposed · Issue: jaunder-org/jaunder#537 · Family: #464 (macro), #455
(`RetentionCount`), #536 (`MaxFileSize`/`UserQuota`), ADR-0063.

## Problem

Pagination page-size is a bare `u32` with a `1..=50` range invariant re-asserted
by hand (`limit.unwrap_or(50).clamp(1, 50)`) at ~7 call sites across the web
`posts` surface and the AtomPub `posts` handler, plus a standalone
`const PAGE_SIZE: u32 = 50` and the AtomPub `DEFAULT_PAGE_SIZE = 25` /
`MAX_PAGE_SIZE = 50` literals. The bound is duplicated as literals and clamps; a
stray call could omit the clamp and pass an out-of-range size to a query.

## Solution

Introduce a `PageSize` validated range-newtype over `u32` with a `1..=50`
invariant, built via the `#[derive(NumNewtype)]` macro (#464), and adopt it at
every page-size site so the bound is defined **once** in the type.

The macro's existing doors (rejecting `FromStr`/serde, one compile-checked
`Default`) cannot express page size's two per-context defaults (web 50,
AtomPub 25) nor the AtomPub clamp semantics. So the macro gains one opt-in range
affordance, and each surface adopts `PageSize` in the way that preserves its
current behavior.

### Design decisions (resolved)

1. **Home.** `PageSize` lives in a new `common::pagination` module
   (`common/src/pagination.rs`), matching the one-module-per-domain-value
   pattern (`common::backup::RetentionCount`, `common::media::MaxFileSize`). No
   facade re-export; consumers import `common::pagination::PageSize`.

2. **Invariant + web default.**
   `#[num_newtype(inner = u32, min = 1, max = 50, default = 50, clamp)]`. The
   `1`/`50` bound and the web default (50) live only in the type. Accessor is
   `.value()` (per ADR-0063; the macro's raw accessor).

3. **Macro `clamp` affordance (new).** A `clamp` flag on `#[num_newtype(...)]`,
   valid only when both `min` and `max` are set, emits:
   - `pub const MIN: <inner>` and `pub const MAX: <inner>`, and
   - `#[must_use] pub const fn clamped(v: <inner>) -> Self` — coerces `v` into
     `[MIN, MAX]` and is therefore infallible and always in-range.

   `clamped` is a validated door (it cannot produce an out-of-range value), so
   it does not weaken the newtype's invariant. It is opt-in so other numeric
   newtypes (e.g. `RetentionCount`) do not silently gain clamp coercion.

4. **AtomPub `?limit=` — clamp preserved (owner-approved flatten).** The public
   `CollectionPaging.limit` query field stays `Option<u32>` (a deliberate
   flatten of a serialization surface, expressly approved on the issue per
   #470), so out-of-range values continue to **clamp** into `1..=50` rather than
   reject with a 400. Construction routes through `PageSize::clamped(...)`; the
   AtomPub default-25 becomes `PageSize::clamped(25)` (const, bound-safe). The
   `DEFAULT_PAGE_SIZE`/`MAX_PAGE_SIZE` literals are removed — 50 comes from the
   type, 25 is AtomPub's documented policy default.

5. **Web `#[server]` args — typed; contract tightens for out-of-range.** The web
   pagination wire args and internal fetchers take `Option<PageSize>`;
   `page_size = limit.unwrap_or_default()` (= 50). The `.clamp(1, 50)`
   disappears because a `PageSize` is in-range by construction. This **does**
   change the web endpoints' wire contract: an out-of-range `limit` that
   previously _clamped_ now _rejects_ (the serde bridge fails deserialization) —
   the deliberate ADR-0065 posture. It is behavior-preserving in practice only
   because every first-party client sends the constant page size; unlike AtomPub
   (decision 4), the web endpoints are not a public clamp surface.
   `const PAGE_SIZE: u32 = 50` is deleted; client callers pass
   `Some(PageSize::default())`.

6. **`has_more` probe arithmetic preserved.** The `+1` fetch-limit probe and the
   `len()`/`truncate()` comparisons operate on the raw integer via
   `page_size.value()` (`page_size.value().saturating_add(1)`,
   `page_size.value() as usize`; AtomPub `limit.value() + 1`). No probe logic
   changes.

7. **ADR.** ADR-0063 (proposed) is amended to document the range `clamp`
   affordance (`const MIN`/`MAX` + `const fn clamped`) as part of the numeric
   newtype convention. No new ADR.

## Acceptance criteria

Each is stated so a conformance review can tell delivered from not.

- **AC1 — Type exists.** `common::pagination::PageSize` is a
  `#[derive(NumNewtype)]` newtype declared
  `#[num_newtype(inner = u32, min = 1, max = 50, default = 50, clamp)]`;
  `common/src/lib.rs` declares `pub mod pagination`.
- **AC2 — Macro clamp support.** With `min` + `max` + `clamp`, `NumNewtype`
  emits `PageSize::MIN == 1`, `PageSize::MAX == 50`, and a
  `const fn clamped(u32) -> PageSize` where `clamped(0) == clamped via MIN`
  (value 1), `clamped(999)` has value 50, and `clamped(25)` has value 25. The
  `clamp` flag without both bounds is a compile error.
- **AC3 — Bound defined once.** No residual open-coded page-size
  `.clamp(1, 50)`, `unwrap_or(50)` page default, `MAX_PAGE_SIZE`, or
  `const PAGE_SIZE: u32` literal remains; each is sourced from `PageSize`
  (`unwrap_or_default`/`clamped`/`MAX`). (AtomPub's `25` policy default remains,
  as `PageSize::clamped(25)`.)
- **AC4 — Web sites adopted (complete enumeration).** Every page-size site on
  the web posts surface takes/passes `PageSize`. This is exhaustive; the
  conformance check is "no `Option<u32>` `limit` arg or `Some(50)`/`Some(<int>)`
  page-size literal remains on any of these":
  - **Internal fetchers** (`web/src/posts/listing.rs`): `fetch_user_posts`,
    `fetch_local_timeline`, `fetch_posts_by_tag`, `fetch_user_posts_by_tag`, and
    the inline `list_home_feed` body — arg becomes `Option<PageSize>`, clamp
    removed.
  - **`#[server]` wire fns**: `list_user_posts`, `list_local_timeline`,
    `list_home_feed`, `list_posts_by_tag`, `list_user_posts_by_tag`
    (`listing.rs`); `list_drafts` (`web/src/posts/mod.rs`) — arg becomes
    `Option<PageSize>`.
  - **Client callers** pass `Some(PageSize::default())`:
    `web/src/pages/timeline.rs`, `web/src/pages/home.rs`,
    `web/src/pages/cockpit.rs`, and **all** sites in `web/src/pages/posts.rs` —
    both the direct server-fn calls (`list_user_posts`, `list_drafts`,
    `list_posts_by_tag`, `list_user_posts_by_tag`) and the generated
    action-struct `limit: Some(50)` field inits used for infinite-scroll.
  - **Server-side (projector) callers** (`server/src/projector/mod.rs`) pass
    `Some(PageSize::default())` into the fetchers: the `fetch_local_timeline`,
    `fetch_user_posts`, `fetch_posts_by_tag`, `fetch_user_posts_by_tag` calls.
  - `const PAGE_SIZE: u32 = 50` (`web/src/pages/timeline.rs`) is deleted along
    with its imports.
- **AC5 — AtomPub adopted, behavior unchanged, with regression.**
  `collection_get` computes its page size via `PageSize::clamped(...)` with
  default 25; a request with `?limit=999` still returns at most 50 items and
  `?limit=0` still returns at least 1 (no new 400). An **automated** AtomPub
  test asserts the out-of-range clamp (there is none today — `?limit=1` is the
  only covered case), so AC5 is verifiable in CI rather than only by manual
  spot-check.
- **AC6 — `has_more` probe preserved.** Where a site over-fetches to detect a
  next page (the `+1` probe and `len()`/`truncate()` comparisons), the
  arithmetic is unchanged, expressed on the raw integer via `PageSize::value()`
  (`.value().saturating_add(1)`, `.value() as usize`; AtomPub `.value() + 1`).
  Sites with no probe (`list_drafts` passes the page size straight through)
  simply use `.value()`.
- **AC7 — Test-helper convention.**
  `common::test_support::parse_page_size(&str) -> PageSize` exists (routing
  through `FromStr`), and every `cfg(test)` **`PageSize`-typed Rust value** is
  built through it or `PageSize::default()` — no inline `.parse().unwrap()` or
  per-module helper. (This does not touch integer `limit=<n>` values in
  wire/query-string test fixtures, nor the raw `u32` limits handed to
  storage-trait methods, which stay integers.)
- **AC8 — Validation/error coverage.** The macros crate has a unit test covering
  the `clamp`-flag codegen path (and the missing-bound-with-clamp error path);
  `PageSize`'s own tests cover `value()`, `From`, `FromStr` accept + reject (0,
  51), `Default`, the serde bridge accept + reject, `MIN`/`MAX`, and `clamped`
  (below → MIN, above → MAX, in-range → identity), following the generic
  `assert_*_newtype::<T>()` helper convention.
- **AC9 — ADR + macro docs.** ADR-0063 documents the `clamp` range affordance.
  The macro's stale doc comments that name a `get()` accessor
  (`macros/src/num_newtype.rs`, `macros/src/lib.rs`) are corrected to `value()`
  in the same change (per the docs-track-late-API-changes convention).
- **AC10 — Gate green.** `cargo xtask validate --no-e2e` passes.

## Out of scope / separable

- No change to cursor/keyset paging logic, storage query signatures (they keep
  raw `u32` limits — `PageSize` is unwrapped via `.value()` at the storage
  boundary), or the AtomPub feed format.
- The macro affordance is the minimal `clamp` flag; no broader macro redesign.
- **`web/src/media/api.rs` `list_my_media` (`limit.unwrap_or(50)`) is
  excluded.** It is a media-surface (#536) default, **not** a `1..=50` clamp
  site — there is no upper bound enforced today, so typing it `PageSize` would
  newly impose `max = 50` and reject/clamp larger media requests: a behavior
  change outside this issue's "page-size 1..=50 case" scope. Whether media
  pagination should gain a bounded page size is a separable question; the plan's
  first task may file it as a follow-up (media-pagination bound) rather than
  fold it in here.

## Verification

`cargo xtask validate --no-e2e` (AC10) — includes the new automated AtomPub
clamp regression (`?limit=999` → ≤ 50, `?limit=0` → ≥ 1; AC5) and the
`PageSize`/macro unit tests (AC8). Web listing behavior is exercised by the
existing web integration suite (AC4/AC6).
