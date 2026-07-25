# Spec — validated `PermalinkDate` for the loose y/m/d triples (#583)

**Issue:** jaunder-org/jaunder#583 · **Milestone:** #13 (Domain-value type safety) ·
**Family:** #504 (`SoftPath` route parsing), #17/#457 (numeric sweep), ADR-0063/0065/0072.

## Problem

The permalink calendar date travels as three loose ints (`i32`/`u32`/`u32`) with no range
typing at both ends — the web `#[server]` boundary (`get_post`), its client-side router
mirror (`parse_permalink_params`), and storage (`fetch_post_record` /
`find_draft_by_permalink_for_user` / the raw `storage::PermalinkDate` struct). One concept
(a calendar date) whose validity invariant is enforced ad-hoc (a lone
`NaiveDate::from_ymd_opt` in `fetch_post_record`) rather than by construction, and
transposable at every loose-arg site.

## Decisions (from the design interview)

1. **`common::time::PermalinkDate(chrono::NaiveDate)`** — a calendar-date newtype mirroring
   the sibling `common::time::UtcInstant` (ADR-0072): serde-transparent (newtype struct →
   wire form `"YYYY-MM-DD"`, decode validated by `NaiveDate`), `value() -> NaiveDate`,
   `From<NaiveDate>`, `Display` (ISO `YYYY-MM-DD`), and the fallible int door
   `from_ymd(i32, u32, u32) -> Option<Self>`. Validity is **by construction** — a
   `NaiveDate` cannot hold an impossible date. Lives in `common` (not `storage`) because it
   crosses the `#[server]` boundary and compiles to wasm.
2. **The URL is unchanged** — `/~user/YYYY/MM/DD/slug`, three date segments. The type is
   *assembled from* the three parsed segments (`from_ymd`), not parsed from one string; the
   wire form (serde) is the single ISO string.
3. **Collapse the loose triple to one typed value** (ADR-0065 typed-wire-arg + client
   validation): `get_post` and the storage fns take one `PermalinkDate`; the client
   assembles + validates it before dispatch, exactly as it already does for
   `Username`/`Slug`.
4. **Uniform soft-404 for an invalid date** (preserves + completes #504's intent). Today
   the date segments aren't soft-parsed: a non-numeric segment → axum **400**, an
   impossible-but-numeric date (`2026/13/40`) → storage validation error → **500** in the
   projector (a latent bug). Once the date is `SoftPath`-able, **any** invalid date →
   `None` → SPA shell (server) / client-404 (client), uniform with username/slug. This
   changes only the bad-date paths (an improvement); the happy path and the existing
   soft-404 tests (unknown post, bad username) are unchanged.
5. **No new ADR, no new gate.** A straight application of the `UtcInstant`/ADR-0072
   date-newtype pattern + ADR-0063/0065; the soft-404 fix is local behavior, not
   architecture.

## Design

### The type — `common/src/time.rs` (beside `UtcInstant`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PermalinkDate(NaiveDate);

impl PermalinkDate {
    /// The one fallible construction door: an impossible date (bad month/day, etc.)
    /// yields `None`. Used to assemble the type from the URL's three segments.
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Option<Self> {
        NaiveDate::from_ymd_opt(year, month, day).map(Self)
    }
    pub fn value(self) -> NaiveDate { self.0 }
}
// From<NaiveDate>, Display (delegates to NaiveDate's ISO YYYY-MM-DD).
```

- Newtype-struct serde is transparent → wire `"2026-01-02"`; deserialize rejects an invalid
  date (a corrupt wire value is a decode error, not a silent bad date). `chrono` (with
  `serde`) is already a `common` dependency.
- Test-support: `common::test_support::permalink_date(i32, u32, u32) -> PermalinkDate`
  (built via `from_ymd(...).expect(...)`), added in the task that first uses it (per the
  coverage-gate lesson — an unused test helper reads as uncovered).

### Web `#[server]` boundary — `web/src/posts/api.rs`

- `get_post(username: Username, year: i32, month: u32, day: u32, slug: Slug)` →
  `get_post(username: Username, date: PermalinkDate, slug: Slug)`. Body threads `date` to
  `fetch_post_record` / `find_draft_by_permalink_for_user` (no y/m/d).

### Client parse + resource — `web/src/posts/parse.rs`, `web/src/posts/component.rs`

- `parse_permalink_params(...) -> (Option<Username>, i32, u32, u32, Option<Slug>)` →
  `(Option<Username>, Option<PermalinkDate>, Option<Slug>)`: parse the three segments to
  ints, then `PermalinkDate::from_ymd(y, m, d)` — a non-numeric segment **or** an impossible
  date yields `None` (replacing today's silent `0` default).
- `PostPage` resource: the fetch key `(username, year, month, day, slug, refetch)` →
  `(Option<Username>, Option<PermalinkDate>, Option<Slug>, refetch)`; the resource body adds
  a `date == None` short-circuit to `WebError::validation("Invalid permalink")` (client-404)
  alongside the existing `username`/`slug` `None` arms; then dispatches
  `get_post(username, date, slug)`.

### Projector soft-404 — `server/src/projector/mod.rs`

- `PermalinkPath = (SoftPath<Username>, i32, u32, u32, SoftPath<Slug>)` →
  `(SoftPath<Username>, SoftPath<i32>, SoftPath<u32>, SoftPath<u32>, SoftPath<Slug>)`. The
  handler assembles `PermalinkDate::from_ymd` from the three soft-parsed ints; the existing
  `let (Some(username), Some(slug)) = … else { shell }` becomes
  `let (Some(username), Some(date), Some(slug)) = …` — so a non-numeric or impossible date
  falls to `shell_response` (soft-404), not 400/500. Then `fetch_post_record(..., date, ...)`.

### Storage — `storage/src/posts.rs` (+ dialect files)

- Delete the raw `storage::PermalinkDate { year, month, day }` struct and add
  `pub use common::time::PermalinkDate;` to `storage`'s public surface, so existing
  `storage::PermalinkDate` references (the atompub + storage integration tests) keep
  resolving without an import churn.
- `fetch_post_record`, `find_draft_by_permalink_for_user`, and the
  `PostStorage::get_post_by_permalink` trait method take `PermalinkDate`. **Remove** the
  ad-hoc `NaiveDate::from_ymd_opt` validation in `fetch_post_record` — the type now
  guarantees validity (every caller — the `#[server]` fn via serde-decode, the projector via
  `from_ymd` — hands it a valid value).
- `get_post_by_permalink` binds the date string via `Display` (`date` → `"YYYY-MM-DD"`); the
  dialect `PERMALINK_DATE_CLAUSE` SQL (`date(published_at) = $3`) is **unchanged**.
- `find_draft_by_permalink_for_user` compares via `date.value()` (its in-Rust draft match,
  `created_at.date_naive() == date.value()`).

### Compile-forced tails (enumerated so the plan's grep sweep catches them)

The tuple-struct makes today's `PermalinkDate { year, month, day }` literal a compile error,
and the signature changes break loose-int calls. Beyond the production paths above, these
**test/fixture** sites construct the raw triple and must move to `permalink_date(...)` /
typed args (sweep `server/tests/{atompub,storage}` and `storage/src/posts.rs`, not just
`web`+`storage` production):

- `server/tests/atompub/atompub_posts.rs:1814` — `storage::PermalinkDate { … }`.
- `server/tests/storage/mod.rs` — `PermalinkDate` import (:21) + literals at :1896, :1912,
  :1934, :5349, :5372.
- `storage/src/posts.rs` in-file tests — loose-int calls: `fetch_post_record(…, y, m, d, …)`
  (:2844, :2858), `find_draft_by_permalink_for_user(…, y, m, d, …)` (:3205, :3211), mock
  (:3263). These derive y/m/d from `record.created_at.year()/month()/day()` → rebuild via
  `permalink_date(…)` or `PermalinkDate::from(record.created_at.date_naive())`.

Also: the `web/src/posts/component.rs:905` `PermalinkFetchKey` tuple alias shrinks (5→3
value elements + refetch); the `web/src/posts/parse.rs:13-19` doc comment ("fall back to
`0`") updates to the `None` semantics; `common/src/time.rs` adds `use chrono::NaiveDate`.

## Acceptance criteria

1. **Type & validation.** `common::time::PermalinkDate` exists with the surface above. Unit
   tests: `from_ymd` accepts a real date and returns `None` for impossible ones (month 0/13,
   day 0/30-in-Feb, etc.); serde serializes as `"YYYY-MM-DD"` and rejects an invalid string
   on deserialize; `Display`/`value()` round-trip. Coverage clean (common is measured).
2. **Boundary collapsed.** `get_post` takes one `PermalinkDate`; `parse_permalink_params`
   returns `Option<PermalinkDate>`; storage fns and the `get_post_by_permalink` trait method
   take `PermalinkDate`; **no loose `(i32, u32, u32)` permalink triple remains** on these
   paths. The `storage::PermalinkDate` raw struct is gone (re-exported from `common`).
   Grep-checkable, sweeping production **and** tests — `web/src`, `storage/src`,
   `server/src`, `server/tests/{projector,storage,atompub}` — for `PermalinkDate {`
   literals and loose-int `fetch_post_record`/`find_draft_by_permalink_for_user` calls.
3. **Client validates before dispatch.** The `PostPage` resource short-circuits an invalid
   date to a client-404 (`None` arm) before any `get_post` round-trip; the
   `unparseable_date_segments_default_to_zero` parse test is replaced by one asserting an
   invalid date → `None`.
4. **Uniform soft-404.** A non-numeric **or** impossible date at the projector serves the
   SPA shell (200), not 400/500. The soft-404 branch lives in the `permalink` handler's
   `let (Some, Some, Some) = … else { shell }` (the `from_ymd → None` arm), which is only
   reachable through the **router** — so the new test is a **router-level `oneshot`** in
   `server/tests/projector/mod.rs` (mirroring `permalink_unknown_serves_spa_shell`), pinning
   `GET /~a/2026/13/40/slug` → 200 shell. (The `permalink_response` unit seam is downstream
   of date assembly and cannot observe this — do not use it.) The existing soft-404 tests
   (`permalink_unknown_serves_spa_shell`, `permalink_invalid_segment_serves_shell`) and the
   happy-path permalink test are unchanged and pass.
5. **Storage behavior unchanged (valid dates).** Existing `get_post_by_permalink` storage
   tests pass with fixtures rebuilt via `permalink_date(...)`; the SQL date-match is
   byte-identical. A storage/parse test pins that an impossible triple no longer reaches a
   query (it's unrepresentable / `None`).
6. **Gates green.** `cargo xtask validate --no-e2e` clean; wasm-clippy clean (the type is in
   the wasm bundle); the permalink e2e tests pass at ship.

## Out of scope

- Any change to the permalink **URL structure** (stays `/~user/YYYY/MM/DD/slug`).
- Changing the SQL date-match semantics (still `date(published_at) = <date>`).
- A weak/again-typed *time-of-day* concept — this is a calendar date only.
- #587 / #417 and other milestone-#13 issues.
