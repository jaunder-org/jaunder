# `PermalinkDate` newtype (#583) Implementation Plan

> **For agentic workers:** Execute task-by-task with **jaunder-iterate** (delegating a task
> to a subagent via **jaunder-dispatch** when useful). Checkbox (`- [ ]`) tracking.

**Spec:** [`docs/superpowers/specs/2026-07-24-issue-583-permalink-date.md`](../specs/2026-07-24-issue-583-permalink-date.md) — the "what/why"; this is the "how."

**Goal:** Introduce `common::time::PermalinkDate(NaiveDate)` and thread it through the web
`#[server]` boundary, the client router parse, the projector (uniform soft-404), and
storage — collapsing every loose `(i32, u32, u32)` permalink triple to one validated value.

**Architecture:** A calendar-date newtype mirroring `common::time::UtcInstant` (serde-
transparent, validity by construction via `NaiveDate`). The signature change ripples across
crates atomically, so after the additive type (Task 1) the threading lands as one coherent
commit (Task 2).

**Tech Stack:** Rust, `chrono` (already in `common`), Leptos/wasm (`web`), axum + `SoftPath`
(`server`), `sqlx` dual-backend (`storage`).

## Global Constraints

- **No `Co-Authored-By` trailer.** Pre-commit hook runs full `cargo xtask check` (fmt +
  clippy + **wasm-clippy** + Nix coverage/tests, both backends) — run `cargo xtask check`
  before each commit so it passes clean (**jaunder-commit**).
- **DB-test env:** storage/server tests need the env `cargo xtask check` sets up; bare
  `cargo nextest` ConnectionRefused's on Postgres. `cargo xtask check` is authoritative;
  targeted `cargo nextest run -p <crate>` gives compile + the pure/SQLite path.
- **URL unchanged** — `/~user/YYYY/MM/DD/slug`, three date segments. `PermalinkDate` is
  assembled from the three via `from_ymd`; the wire form is one ISO string (serde).
- **No new ADR, no new gate** (straight `UtcInstant`/ADR-0072 application).

**Behavior change (spec Decision 4):** an invalid permalink date (non-numeric **or**
impossible) → SPA shell (server) / client-404, replacing today's 400 / latent 500.

---

## Review header

**Scope — in:** the `PermalinkDate` type + `permalink_date` test helper; `get_post` +
`parse_permalink_params` + the `PostPage` resource; the projector `SoftPath` date assembly;
storage fns + trait method + the raw-struct deletion/re-export; every compile-forced
test/fixture site; the new impossible-date soft-404 test.

**Scope — out:** the permalink URL structure; the SQL date-match semantics; #587/#417.

**Tasks:**

1. `common::time::PermalinkDate(NaiveDate)` + unit tests (`permalink_date` helper deferred
   to Task 2, its first user — per the coverage-gate lesson).
2. Thread it end-to-end (atomic, cross-crate): storage (struct→re-export, fn sigs, drop the
   `from_ymd_opt` guard, helper), web (`get_post`, `parse_permalink_params`, `PostPage`
   resource), server projector (`SoftPath` assembly → soft-404), all fixtures + the new
   impossible-date router test.

**Key risks / decisions:**

- **Task 2 is one atomic cross-crate commit.** Changing `fetch_post_record`/
  `get_post_by_permalink`/`get_post` signatures breaks web + server + storage tests
  simultaneously; no clean split exists without transitional scaffolding (rejected — churn).
- **Soft-404 seam is the router**, not the `permalink_response` unit fn — the new test is a
  router `oneshot` (spec AC4).
- **Removing the `from_ymd_opt` guard** is safe: both production callers (`get_post` via
  serde-decode, projector via `from_ymd`) hand a by-construction-valid value (verified in
  the spec soundness review).

---

## Task 1: `common::time::PermalinkDate`

**Files:**

- Modify: `common/src/time.rs` (add the type beside `UtcInstant`; add `use chrono::NaiveDate`)

**Interfaces:**

- Consumes: `chrono::NaiveDate` (serde feature already on).
- Produces (relied on by Task 2):
  - `common::time::PermalinkDate` — `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)] struct PermalinkDate(NaiveDate)`
  - `PermalinkDate::from_ymd(year: i32, month: u32, day: u32) -> Option<PermalinkDate>`
  - `PermalinkDate::value(self) -> NaiveDate`
  - `impl From<NaiveDate> for PermalinkDate`, `impl Display` (ISO `YYYY-MM-DD`)

- [ ] **Step 1: Write the failing tests** in `common/src/time.rs` (`#[cfg(test)] mod tests`, beside the `UtcInstant` tests):

```rust
#[test]
fn permalink_date_from_ymd_accepts_a_real_date() {
    assert!(PermalinkDate::from_ymd(2026, 1, 2).is_some());
}

#[test]
fn permalink_date_from_ymd_rejects_impossible_dates() {
    for (y, m, d) in [(2026, 13, 1), (2026, 0, 1), (2026, 1, 0), (2026, 2, 30), (2026, 4, 31)] {
        assert!(PermalinkDate::from_ymd(y, m, d).is_none(), "{y}-{m}-{d} must reject");
    }
}

#[test]
fn permalink_date_serde_is_transparent_iso_and_rejects_invalid() {
    let pd = PermalinkDate::from_ymd(2026, 1, 2).unwrap();
    assert_eq!(serde_json::to_string(&pd).unwrap(), "\"2026-01-02\"");
    assert_eq!(serde_json::from_str::<PermalinkDate>("\"2026-01-02\"").unwrap(), pd);
    assert!(serde_json::from_str::<PermalinkDate>("\"2026-13-40\"").is_err());
}

#[test]
fn permalink_date_display_and_value_round_trip() {
    let pd = PermalinkDate::from_ymd(2026, 1, 2).unwrap();
    assert_eq!(pd.to_string(), "2026-01-02");
    let nd = chrono::NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
    assert_eq!(pd.value(), nd);
    assert_eq!(PermalinkDate::from(nd), pd);
}
```

- [ ] **Step 2: Run, verify fail** — `cargo nextest run -p common permalink_date`
  Expected: FAIL — `PermalinkDate` not defined.

- [ ] **Step 3: Implement** in `common/src/time.rs` (add `use chrono::NaiveDate;` to the imports):

```rust
/// A validated permalink calendar date (`YYYY-MM-DD`), wrapping a `chrono::NaiveDate` so an
/// impossible date is unrepresentable — the permalink-route sibling of [`UtcInstant`]
/// (ADR-0072/0063/0065). Serde-transparent (newtype struct) → wire form `"2026-01-02"`,
/// decode validated by `NaiveDate`. Assembled from the URL's three `/YYYY/MM/DD/` segments
/// via [`from_ymd`](PermalinkDate::from_ymd); the wire carries the single ISO string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PermalinkDate(NaiveDate);

impl PermalinkDate {
    /// The one fallible door: an impossible date (bad month/day) yields `None`. Used to
    /// assemble the type from the URL's three parsed segments.
    #[must_use]
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Option<Self> {
        NaiveDate::from_ymd_opt(year, month, day).map(Self)
    }

    /// The inner `NaiveDate` (for in-Rust date comparison).
    #[must_use]
    pub fn value(self) -> NaiveDate {
        self.0
    }
}

impl From<NaiveDate> for PermalinkDate {
    fn from(date: NaiveDate) -> Self {
        Self(date)
    }
}

impl fmt::Display for PermalinkDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `NaiveDate`'s Display is ISO `YYYY-MM-DD` (4-digit zero-padded year) — the exact
        // string the storage query binds, so it replaces the old `format!("{y:04}-…")`.
        write!(f, "{}", self.0)
    }
}
```

(If `use std::fmt;` isn't already in `time.rs`, add it.)

- [ ] **Step 4: Run, verify pass** — `cargo nextest run -p common permalink_date` → PASS (4).

- [ ] **Step 5: Commit**

```bash
git add common/src/time.rs
git commit -m "feat(common): PermalinkDate newtype wrapping a validated NaiveDate (#583)"
```

Run `cargo xtask check` first (clean).

---

## Task 2: Thread `PermalinkDate` end-to-end (atomic — common test-support + storage + web + server + tests)

One commit: the signature changes couple all crates. Grouped by area below.

**Files:**

- Modify: `common/src/test_support.rs` (add `permalink_date`)
- Modify: `storage/src/posts.rs` (struct→re-export, fn sigs, drop guard, in-file tests)
- Modify: `web/src/posts/api.rs` (`get_post`), `web/src/posts/parse.rs`
  (`parse_permalink_params` + its tests), `web/src/posts/component.rs` (`PermalinkFetchKey`,
  `PostPage` resource)
- Modify: `server/src/projector/mod.rs` (`PermalinkPath`, date assembly)
- Modify (fixtures): `server/tests/storage/mod.rs`, `server/tests/atompub/atompub_posts.rs`
- Modify (test): `server/tests/projector/mod.rs` (new impossible-date soft-404 test)

**Interfaces:**

- Consumes: `common::time::PermalinkDate` + `from_ymd`/`value`/`Display`/`From<NaiveDate>`.
- Produces: `common::test_support::permalink_date(i32, u32, u32) -> PermalinkDate`;
  `get_post(username: Username, date: PermalinkDate, slug: Slug)`;
  `parse_permalink_params(...) -> (Option<Username>, Option<PermalinkDate>, Option<Slug>)`;
  storage `fetch_post_record`/`find_draft_by_permalink_for_user`/`get_post_by_permalink`
  taking `PermalinkDate`.

- [ ] **Step 1: Update the failing tests** (drive the behavior):

  - `web/src/posts/parse.rs` `mod tests`: replace `unparseable_date_segments_default_to_zero`
    with a `None` assertion, and update the valid/missing tests to the new return shape:

```rust
#[test]
fn parses_valid_permalink_params() {
    let (u, date, slug) =
        parse_permalink_params(Some("~alice"), Some("2026"), Some("01"), Some("02"), Some("hello"));
    assert_eq!(u.as_deref(), Some("alice"));
    assert_eq!(date, PermalinkDate::from_ymd(2026, 1, 2));
    assert_eq!(slug.as_deref(), Some("hello"));
}

#[test]
fn unparseable_or_impossible_date_is_none() {
    // non-numeric segment
    let (_, d1, _) = parse_permalink_params(Some("~a"), Some("x"), Some("01"), Some("02"), Some("s"));
    assert_eq!(d1, None);
    // impossible date (month 13)
    let (_, d2, _) = parse_permalink_params(Some("~a"), Some("2026"), Some("13"), Some("02"), Some("s"));
    assert_eq!(d2, None);
    // missing segment
    let (_, d3, _) = parse_permalink_params(Some("~a"), None, Some("01"), Some("02"), Some("s"));
    assert_eq!(d3, None);
}
```
    (Keep `username_without_tilde_is_none` / `unparseable_slug_is_none`, adjusting the
    destructure to the 3-tuple.)

  - `server/tests/projector/mod.rs`: add a dual-backend router test beside
    `permalink_unknown_serves_spa_shell` (copy its `oneshot`/shell-assertion harness):

```rust
#[apply(backends)]
#[tokio::test]
async fn permalink_impossible_date_serves_shell(#[case] backend: Backend) {
    // An impossible date (month 13) must be a soft-404 (SPA shell), not a 400/500 — the
    // SoftPath date assembly resolves it to None (#583/#504).
    let TestEnv { state, base: _base } = backend.setup().await;
    let app = projector_app(&state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/~ghost/2026/13/40/missing")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains(TEST_SHELL));
}
```
    (Exact harness of the sibling `permalink_unknown_serves_spa_shell` at
    `server/tests/projector/mod.rs:149`: `TestEnv { state, base: _base }` +
    `projector_app(&state)` + the `TEST_SHELL` marker const at `:23` — **not** `make_app`.)

- [ ] **Step 2: Run, verify fail** — `cargo nextest run -p web parse_permalink` (parse
  tests fail to compile against the old return type). Expected: FAIL.

- [ ] **Step 3: Implement — `common` test helper.** `common/src/test_support.rs` (add
  `use crate::time::PermalinkDate;`; place near `parse_utc_instant`):

```rust
/// Build a valid [`PermalinkDate`] for tests from a `(year, month, day)` triple.
///
/// # Panics
///
/// Panics if the triple is not a real calendar date.
#[must_use]
pub fn permalink_date(year: i32, month: u32, day: u32) -> PermalinkDate {
    PermalinkDate::from_ymd(year, month, day).expect("valid test permalink date")
}
```

- [ ] **Step 4: Implement — storage** (`storage/src/posts.rs` + dialect files unchanged):

  - Delete `pub struct PermalinkDate { … }`; add `pub use common::time::PermalinkDate;`.
  - `fetch_post_record`: signature `(posts, viewer, username: &Username, date: PermalinkDate,
    slug: &Slug)`; **delete** the `NaiveDate::from_ymd_opt(...).ok_or_else(...)?` guard; body
    `posts.get_post_by_permalink(username, date, slug, viewer, Utc::now()).await.map_err(...)`.
  - `get_post_by_permalink` (trait method sig + generic impl): take `date: PermalinkDate`;
    replace the `{ year, month, day }` destructure + `format!("{year:04}-{month:02}-{day:02}")`
    with `let date_str = date.to_string();` (Display → identical ISO string). SQL/binds/dialect
    `PERMALINK_DATE_CLAUSE` **unchanged**.
  - `find_draft_by_permalink_for_user`: take `date: PermalinkDate`; the in-Rust match compares
    `record.created_at.date_naive() == date.value()`.
  - In-file `#[cfg(test)]` calls (loose ints at ~:2844, :2858, :3205, :3211, :3263): pass
    `PermalinkDate::from(record.created_at.date_naive())` (or `permalink_date(y, m, d)` where a
    literal triple is used, e.g. the mock at :3263 → `permalink_date(2020, 1, 1)`).

- [ ] **Step 5: Implement — web:**

  - `web/src/posts/api.rs` `get_post`: signature `(username: Username, date: PermalinkDate,
    slug: Slug)`; body passes `date` to `fetch_post_record` / `find_draft_by_permalink_for_user`
    (import `common::time::PermalinkDate`).
  - `web/src/posts/parse.rs` `parse_permalink_params`: return
    `(Option<Username>, Option<PermalinkDate>, Option<Slug>)`; assemble the date:

```rust
let date = year
    .and_then(|v| v.parse::<i32>().ok())
    .zip(month.and_then(|v| v.parse::<u32>().ok()))
    .zip(day.and_then(|v| v.parse::<u32>().ok()))
    .and_then(|((y, m), d)| PermalinkDate::from_ymd(y, m, d));
```
    Update the `:13-19` doc comment ("fall back to `0`") to the `None`-on-invalid semantics.
  - `web/src/posts/component.rs`: `PermalinkFetchKey` alias (:905) →
    `(Option<Username>, Option<PermalinkDate>, Option<Slug>, u32)`. In the `PostPage` resource
    body, destructure `(username, date, slug)` and add a `let Some(date) = date else { return
    Err(WebError::validation("Invalid permalink")); }` arm beside the existing username/slug
    `None` arms; then `get_post(username, date, slug).await`.

- [ ] **Step 6: Implement — server projector** (`server/src/projector/mod.rs`):

  - `PermalinkPath` (:144) → `(SoftPath<Username>, SoftPath<i32>, SoftPath<u32>, SoftPath<u32>,
    SoftPath<Slug>)`.
  - The handler's guard (:152) becomes:

```rust
let (Some(username), Some(date), Some(slug)) = (
    username.into(),
    year.into()
        .zip(month.into())
        .zip(day.into())
        .and_then(|((y, m), d): ((i32, u32), u32)| PermalinkDate::from_ymd(y, m, d)),
    slug.into(),
) else {
    return shell_response(&shell);
};
```
    then `fetch_post_record(..., date, ...)` (drop the loose y/m/d).

- [ ] **Step 7: Implement — remaining fixtures:**

  - `server/tests/storage/mod.rs`: `PermalinkDate` import at :21 keeps resolving (re-export);
    literals at :1896, :1912, :1934, :5349, :5372 → `permalink_date(y, m, d)` (import
    `common::test_support::permalink_date`).
  - `server/tests/atompub/atompub_posts.rs:1814`: `storage::PermalinkDate { … }` →
    `permalink_date(y, m, d)`.

- [ ] **Step 8: Run, verify pass.** Pure/host bits: `cargo nextest run -p web parse_permalink`
  and `cargo nextest run -p common permalink_date` → PASS. Then the full gate for the
  DB/wasm surface:

Run: `cargo xtask check`
Expected: PASS — both backends, wasm-clippy clean, incl. the new `permalink_impossible_date_serves_shell`
and all existing permalink/soft-404/storage tests.

- [ ] **Step 9: Commit**

```bash
git add common/src/test_support.rs storage/src/posts.rs \
        web/src/posts/api.rs web/src/posts/parse.rs web/src/posts/component.rs \
        server/src/projector/mod.rs server/tests/storage/mod.rs \
        server/tests/atompub/atompub_posts.rs server/tests/projector/mod.rs
git commit -m "refactor(web,storage,server): thread PermalinkDate through the permalink boundary (#583)"
```

`cargo xtask check` already run in Step 8.

---

## Final verification (after Task 2)

- [ ] **Grep-check (AC2)** — no raw `PermalinkDate` struct-literal survives. Expected: **no matches**.

```bash
rg -n 'PermalinkDate \{' web/src storage/src server/src server/tests
```
(The reliable residual signal is a `PermalinkDate { … }` literal. The loose-int call sites are
caught by the type system — `cargo xtask check` in Step 8 is the real net, since a stray loose
`fetch_post_record(…, y, m, d, …)` no longer compiles.)

- [ ] **AC6 gate** — `cargo xtask validate --no-e2e`. Expected: PASS (static + clippy +
  wasm-clippy + coverage + guards). The permalink e2e tests run at ship (`jaunder-ship` full
  `validate`).

## Self-review

- **Spec coverage:** AC1 → Task 1. AC2 → Task 2 (steps 4–7) + grep. AC3 → Task 2 step 5
  (parse + resource) & step 1 (parse test). AC4 → Task 2 step 1 (router soft-404 test) +
  step 6 (assembly). AC5 → Task 2 steps 4/7 (storage fns + fixtures via `permalink_date`).
  AC6 → the gates.
- **Placeholders:** the projector/storage-test bodies reference "same setup as the sibling
  test" — the implementer copies the exact adjacent harness (`permalink_unknown_serves_spa_shell`
  / the `mk_row` fixtures); the signature + assembly code is given in full.
- **Type consistency:** `PermalinkDate`, `from_ymd`, `value`, `permalink_date`,
  `get_post(…, date, …)` used identically across tasks.

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-24-issue-583-permalink-date.md`.
Execution via **jaunder-iterate** (task-by-task, `cargo xtask check` per commit, checkboxes
in real time), after the plan-approval HALT.
