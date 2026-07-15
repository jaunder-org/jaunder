# `FeedPath` newtype Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Type the feed cache / feed-event identity key as a validated,
canonical `FeedPath` newtype, replacing the bare `&str`/`String` that crosses
`feed_cache`, `feed_events`, and `feed_urls_needing_catchup`.

**Architecture:** A standard `#[derive(StrNewtype)]` (ADR-0063) `str`-backed
newtype in `common::feed`, whose `FromStr` funnels through the existing
`parse()` + `canonicalize()` grammar (the one normalizing chokepoint), plus an
infallible `canonical` constructor for producers that already hold a
`(FeedSurface, FeedFormat)`. Threaded bottom-up (common → storage → server →
web) in compile-clean, behavior-preserving steps.

**Tech Stack:** Rust, `macros::StrNewtype`, `thiserror`, `sqlx`, `async_trait`,
`mockall`, `rstest`/`rstest_reuse` (dual-backend), `cargo nextest`,
`cargo xtask`.

Spec:
[`docs/superpowers/specs/2026-07-14-issue-399-feed-path-newtype.md`](../specs/2026-07-14-issue-399-feed-path-newtype.md).
The plan is "how"; the spec is "what/why" — read it for the Problem framing, the
path-not-URL correction, and the Decision. This plan does not restate them.

## Global Constraints

- **ADR-0063 trailer:** `FeedPath` uses `#[derive(StrNewtype)]` (non-secret
  variant); only `canonical` and `FromStr` are hand-written. No `as_str()`
  inherent method (the `str` traits replace it).
- **No `url` crate** in `common`/`storage`/`server`/`web` — `FeedPath` is
  `String`-backed. (`url` stays an `xtask`-only dep.)
- **Backend parity (ADR-0019):** the generic `FeedCacheStore`/`FeedEventStore`
  logic stays single-impl; per-backend row mappers live in the
  `sqlite`/`postgres` dialect files. Storage tests are dual-backend
  (`#[apply(backends)]`) — a bare `#[tokio::test]` on a storage behavior fails
  the `test-backend-pattern` guard.
- **Coverage (ADR-0050):** provenance-guaranteed defensive arms carry
  `// cov:ignore` with a rationale; `expect_used`/`unwrap_used` denied in
  production (tests may `unwrap`/`expect`).
- **Gate:** each task's commit runs the full `cargo xtask check` via the
  pre-commit hook — run `cargo xtask check` first so it is clean
  (**jaunder-commit**). **No `Co-Authored-By` trailer.**
- **`canonicalize()` keeps its `String` return** — it is also the HTML-`href`
  producer; do not retype it.

---

## Review header

**Scope — in:** `FeedPath` newtype in `common::feed`; typing the identity key
across `feed_cache` (`FeedCacheRow`, `get`, `delete`), `feed_events`
(`FeedEventRecord`, `enqueue`), and `PostStorage::feed_urls_needing_catchup`;
`affected_feed_urls`; the forced consumers in
`server/src/feed/{handlers, regenerate,worker}.rs` and `web/src/feed_events.rs`.

**Scope — out (Task 1 files it):** the absolute feed URLs (`self_url`,
`canonical_url`, `hub_url`/`websub_hub_url`, the WebSub ping target) and
`permalink`/`preview_url`. Different type, different justification.

**Tasks:**

1. File the absolute-URL-newtype follow-up issue (jaunder-issues).
2. Introduce `FeedPath` (+ `canonical`, `FromStr`, `InvalidFeedPath`) in
   `common::feed`; retype `affected_feed_urls -> Vec<FeedPath>`.
3. `PostStorage::feed_urls_needing_catchup -> Vec<FeedPath>` (storage + test).
4. `feed_events` speaks `FeedPath` (record + `enqueue(&FeedPath)` + backend row
   mappers) and its worker consumers (grouping map, `process_feed_group`,
   tracing, the exhaustion-test rewrite).
5. `feed_cache` speaks `FeedPath` (record + `get`/`delete(&FeedPath)` + row
   mapper) and its consumers (`handlers` via `canonical`,
   `regenerate_feed (&FeedPath)`).

**Key risks / decisions:**

- **Ordering is compile-forced.** feed_events (Task 4) must precede
  `regenerate_feed(&FeedPath)` (Task 5): the worker's `process_feed_group`
  feed_url must already be `FeedPath` for the exact-type call to compile. Tasks
  2→3→4→5 is the right order — but each task compiles **only if it edits every
  caller of the signature it changes, tests included.** A signature change
  breaks the whole workspace, so each task carries a **Test call-site sweep**
  naming all `server/tests/**` and in-file test callers, not just the production
  files.
- **`.bind()` needs `.as_ref()`.** sqlx `.bind` is generic over `Encode`; a
  `&FeedPath` does not deref-coerce there. Every internal bind uses
  `feed_url.as_ref()`. (Callers still pass `&feed_path` with no conversion —
  argument-position deref coercion handles `&str`-param helpers like
  `percent_encode_path`, `ping_websub`.)
- **Two defensive arms become unreachable:** the `Decode` read-back mapping (the
  `feed_url` column is written only via a validated `FeedPath`) and
  `regenerate_feed`'s `BadUrl` `None` arm. Both `// cov:ignore`, not tested.
- **Two regen-failure tests lose their trigger.** Both the mock exhaustion test
  (`worker.rs`, `tick_marks_exhausted_when_regen_fails_past_backoff_table`) and
  the **integration** `worker_applies_backoff_on_regen_failure`
  (`server/tests/feed/feed_worker.rs:222`) enqueue an _invalid_ feed_url to
  force `RegenerateError::BadUrl` — un-enqueueable under `FeedPath` (that is the
  guarantee). A valid `FeedPath` leaves only a `Storage` failure, which a
  real-backend integration test can't cleanly inject, so both are rehomed as
  **mock-based worker unit tests** forcing a `Storage` error (Task 4). The
  real-backend `mark_failed`/`mark_exhausted` scheduling SQL is already covered
  by the dual-backend `feed_events` storage tests.
- **Line numbers below are approximate anchors** (~1-line drift vs the live
  tree); trust the symbol names — the compiler and the file are the source of
  truth.

---

## Task 1: File the absolute-URL-newtype follow-up issue

**Files:** none (tracker only).

**Interfaces:**

- Produces: an open issue number to reference in the spec's Non-goals (already
  linked generically) and in commit bodies if useful.

- [ ] **Step 1: File the issue** via **jaunder-issues** (GitHub MCP
      `issue_write`), milestone "Domain-value type safety (newtypes)", label
      `type-safety`, title
      `types: AbsoluteUrl newtype for composed feed/site URLs`. Body (verbatim
      intent):

  > Split from #399 (which typed only the feed **identity path**, `FeedPath`).
  > The **absolute** feed URLs are a distinct type: `FeedMetadata.self_url`/
  > `canonical_url`/`hub_url` (`common/src/feed/metadata.rs`),
  > `FeedMeta.self_url` and paging hrefs (`common/src/atompub/entry.rs`),
  > `FeedsConfig.websub_hub_url` (`common/src/feed/mod.rs`), and the WebSub ping
  > target composed in `server/src/feed/worker.rs`. These are composed
  > server-side from `base_url` + a percent-encoded path
  > (`server/src/feed/regenerate.rs`) and have real scheme/host/percent
  > normalization concerns — a `url`-crate-backed `AbsoluteUrl`/`SiteUrl` (per
  > ADR-0063 invariant axis). Separately, consider typing
  > `permalink`/`preview_url` (`web/src/posts/*`), which are paths that cross
  > serde to the wasm client. Do not fold onto `FeedPath` — different grammar.
  > Depends on / follows #399.

- [ ] **Step 2: Verify** the issue is open with the right milestone/label
      (`issue_read`), and note its number in the final PR body at ship.

_No commit — tracker-only task._

---

## Task 2: Introduce `FeedPath` in `common::feed`

**Files:**

- Modify: `common/src/feed/feed_path.rs` (add the type; retype `canonicalize`
  callers-internal producer `affected_feed_urls`).
- Modify: `common/src/feed/mod.rs:2` (re-export `FeedPath`, `InvalidFeedPath`).
- Test: `common/src/feed/feed_path.rs` `#[cfg(test)]` (in-file, the module's
  existing convention).

**Interfaces:**

- Consumes: existing `parse`, `canonicalize`, `FeedSurface`, `FeedFormat`.
- Produces:
  - `pub struct FeedPath(String)` deriving
    `Clone, Debug, PartialEq, Eq, Hash, StrNewtype` — full non-secret trailer
    (`Display`, `AsRef`/`Borrow`/ `Deref<str>`, `TryFrom<String>`,
    `From<Self> for String`, `PartialEq<str>`/ `<&str>`, serde).
  - `pub struct InvalidFeedPath` (`thiserror::Error`).
  - `impl FeedPath { pub fn canonical(surface: &FeedSurface, format: FeedFormat) -> Self }`
    (infallible).
  - `impl FromStr for FeedPath { type Err = InvalidFeedPath; }`.
  - `pub fn affected_feed_urls<'a, I>(username: &Username, tags: I) -> Vec<FeedPath>`
    (retyped from `Vec<String>`).

- [ ] **Step 1: Write the failing tests** (add to the in-file `mod tests`):

```rust
#[test]
fn from_str_accepts_and_roundtrips_all_canonical_surfaces() {
    for format in [FeedFormat::Rss, FeedFormat::Atom, FeedFormat::Json] {
        for surface in [
            FeedSurface::Site,
            FeedSurface::SiteTag { tag: tag("rust") },
            FeedSurface::User { username: user("alice") },
            FeedSurface::UserTag { username: user("alice"), tag: tag("rust") },
        ] {
            let canon = canonicalize(&surface, format);
            let fp: FeedPath = canon.parse().expect("canonical path parses");
            assert_eq!(fp.as_ref(), canon);
            // canonical agrees with canonicalize
            assert_eq!(FeedPath::canonical(&surface, format).as_ref(), canon);
        }
    }
}

#[test]
fn from_str_normalizes_case_in_segments() {
    // The one residual normalization: username/tag segments lowercase.
    assert_eq!("/~Alice/feed.rss".parse::<FeedPath>().unwrap(), *"/~alice/feed.rss");
    assert_eq!("/tags/Rust/feed.atom".parse::<FeedPath>().unwrap(), *"/tags/rust/feed.atom");
}

#[test]
fn from_str_is_idempotent() {
    let once = FeedPath::canonical(
        &FeedSurface::UserTag { username: user("alice"), tag: tag("rust") },
        FeedFormat::Json,
    );
    assert_eq!(once.as_ref().parse::<FeedPath>().unwrap(), once);
}

#[test]
fn from_str_rejects_non_canonical_and_invalid() {
    for bad in ["/feed.xml", "/tags/c++/feed.rss", "/~/feed.rss",
                "/something/feed.rss", "//feed.rss", "not-a-feed-url"] {
        assert!(bad.parse::<FeedPath>().is_err(), "should reject {bad}");
    }
}

#[test]
fn derefs_and_displays_as_str() {
    let fp = FeedPath::canonical(&FeedSurface::Site, FeedFormat::Rss);
    fn takes_str(s: &str) -> usize { s.len() }
    let _ = takes_str(&fp);                 // Deref<str> coercion
    assert_eq!(fp.to_string(), "/feed.rss"); // Display
    assert_eq!(fp, *"/feed.rss");            // PartialEq<str>
}

#[test]
fn affected_feed_urls_yields_feed_paths() {
    let urls = affected_feed_urls(&user("alice"), [&tag("rust")]);
    assert!(urls.iter().any(|u| u == &*"/feed.rss"));
    assert!(urls.iter().all(|u| u.as_ref().starts_with('/')));
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p common feed_path` Expected: FAIL — `FeedPath` /
`canonical` / `InvalidFeedPath` not defined; `affected_feed_urls` returns
`Vec<String>` (type mismatch on `u == &*"..."`).

- [ ] **Step 3: Implement**

Add to `feed_path.rs` (import
`use macros::StrNewtype; use std::str::FromStr; use thiserror::Error;`):

```rust
/// A validated, canonical feed identity path (e.g. `/feed.rss`,
/// `/~alice/tags/rust/feed.atom`): the dedup/SQL key for `feed_cache` and
/// `feed_events`. Its *identity* — constructed via [`FromStr`] (untrusted
/// strings) or [`canonical`](FeedPath::canonical) (a decomposed surface).
/// The trailer is generated by `#[derive(StrNewtype)]` (ADR-0063); only
/// `canonical` and the normalizing `FromStr` are hand-written. `Hash` — it is
/// a `HashMap` key in the feed worker; no `Ord` (feed paths are never sorted).
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct FeedPath(String);

/// Error returned when a string is not a valid canonical feed path.
#[derive(Debug, Error)]
#[error("not a valid feed path (expected a canonical /…/feed.{{rss,atom,json}} path)")]
pub struct InvalidFeedPath;

impl FeedPath {
    /// Infallible, provenance-guaranteed constructor: a `(surface, format)` is
    /// always a valid canonical path. Delegates the format logic to
    /// [`canonicalize`] (its single source of truth).
    #[must_use]
    pub fn canonical(surface: &FeedSurface, format: FeedFormat) -> Self {
        Self(canonicalize(surface, format))
    }
}

impl FromStr for FeedPath {
    type Err = InvalidFeedPath;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (surface, format) = parse(s).ok_or(InvalidFeedPath)?;
        Ok(Self::canonical(&surface, format))
    }
}
```

Retype `affected_feed_urls`: change the signature return to `Vec<FeedPath>` and
the final loop to `urls.push(FeedPath::canonical(surface, format));` (drop the
`canonicalize` call there). Add `FeedPath, InvalidFeedPath` to the
`common/src/feed/mod.rs` re-export line.

_Body is pinned by Step 1's tests (accept/roundtrip, case-normalize,
idempotence, reject, deref/display, `affected_feed_urls` element type) — no
additional narration needed._

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p common feed_path` Expected: PASS. Then
`cargo xtask check --no-test` (fmt + clippy clean; the consumers `worker.rs:84`
/ `web/src/feed_events.rs:32` still compile because `enqueue(&url)`
deref-coerces `&FeedPath` → `&str`).

- [ ] **Step 5: Commit**

```bash
git add common/src/feed/feed_path.rs common/src/feed/mod.rs
git commit -m "feat(feed): FeedPath newtype for the feed identity key (#399)"
```

Run `cargo xtask check` first (jaunder-commit).

---

## Task 3: `feed_urls_needing_catchup -> Vec<FeedPath>`

**Files:**

- Modify: `storage/src/posts.rs:727` (trait method), `:1649` (generic impl
  body).
- Test: `server/tests/storage/mod.rs:3372`
  (`feed_urls_needing_catchup_returns _stale_feeds`).

**Interfaces:**

- Consumes: `common::feed::FeedPath`, `canonical`, `parse`.
- Produces:
  `async fn feed_urls_needing_catchup(&self, now: DateTime<Utc>) -> sqlx::Result<Vec<FeedPath>>`
  (trait + impl). `MockPostStorage` regenerates.

- [ ] **Step 1: Update the storage test to expect `FeedPath`**

In `feed_urls_needing_catchup_returns_stale_feeds`
(`server/tests/storage/mod.rs`), the returned `stale` is now `Vec<FeedPath>`.
Update assertions to compare via `PartialEq<str>` / a `FeedPath` (e.g.
`assert!(stale.iter().any(|u| u == &*"/feed.rss"))`), matching the existing
expected values. Keep the dual-backend `#[apply(backends)]` harness.

- [ ] **Step 2: Run it, verify it fails to compile**

Run: `cargo nextest run -p server feed_urls_needing_catchup` Expected: FAIL
(compile) — impl still returns `Vec<String>`; assertion type mismatch.

- [ ] **Step 3: Implement**

In `storage/src/posts.rs`, change both the trait signature (`:727`) and the impl
(`:1649`) return type to `sqlx::Result<Vec<FeedPath>>`. In the impl body, keep
the parsed `format` and push the typed path:

```rust
let Some((surface, format)) = common::feed::parse(&feed_url) else {
    continue; // corrupt/legacy column value — skip (unchanged behavior)
};
if let Some(max) = max_published_at_for_surface::<DB>(&self.pool, &surface, now).await? {
    if max > generated_at {
        needing.push(common::feed::FeedPath::canonical(&surface, format));
    }
}
```

(`needing: Vec<FeedPath>`.) No new fallible/`Decode` path — the existing
`continue` already covers an unparseable column.

- [ ] **Step 4: Run it, verify it passes**

Run: `cargo nextest run -p server feed_urls_needing_catchup` Expected: PASS.
`worker.rs:78` (`enqueue(&url)`, `url: FeedPath`) still compiles via deref
coercion (`enqueue` is still `&str`). `cargo xtask check --no-test` clean.

- [ ] **Step 5: Commit**

```bash
git add storage/src/posts.rs server/tests/storage/mod.rs
git commit -m "refactor(storage): feed_urls_needing_catchup returns FeedPath (#399)"
```

Run `cargo xtask check` first (jaunder-commit).

---

## Task 4: `feed_events` speaks `FeedPath` (+ worker plumbing)

**Files:**

- Modify: `storage/src/feed_events.rs` (`FeedEventRecord.feed_url`, `enqueue`
  signature, generic `enqueue` bind).
- Modify: `storage/src/sqlite/feed_events.rs:52`,
  `storage/src/postgres/feed_events.rs:46` (row mappers → `FeedPath`, `Decode`).
- Modify: `server/src/feed/worker.rs` (grouping map `:123`, `process_feed_group`
  `:146-186`, `tracing::info!` `:172`, the `event()` test helper `:316`, the
  exhaustion test `:468`, and the integration-test conversion below).
- Test (call-site sweep — **all** must be edited this task or the workspace
  won't compile):
  - in-file storage tests in `storage/src/feed_events.rs` and
    `storage/src/sqlite/feed_events.rs` (the `:164`-area
    `.enqueue(&format!(…))`).
  - in-file worker tests in `server/src/feed/worker.rs`.
  - `server/tests/feed/feed_worker.rs` — `.enqueue(feed_url)` (81, 144, 202,
    231, 305, 502), `state.feed_cache.get(feed_url)`-adjacent bindings, the
    `pending.iter().map(|r| &r.feed_url)` collect at `:452` (`Vec<&String>` →
    `Vec<&FeedPath>`) and its `u.as_str() == "/feed.atom"` at `:458`
    (`u.as_ref() == "/feed.atom"` — `FeedPath` has no `as_str`), and the
    integration test at `:222`.
  - `server/tests/storage/mod.rs` — `fe.enqueue("/feed-marks.rss")` at `:970`.
  - `server/tests/misc/backup_fixture.rs` —
    `.enqueue("https://example.com/feed.xml")` at `:164` (a **semantic** change
    — see Step 1.4).

**Interfaces:**

- Consumes: `common::feed::FeedPath`.
- Produces:
  - `FeedEventRecord.feed_url: FeedPath`.
  - `FeedEventStorage::enqueue(&self, feed_url: &FeedPath) -> Result<i64, FeedEventError>`
    (trait + generic impl; `MockFeedEventStorage` regenerates).
  - `worker::process_feed_group(&self, feed_url: FeedPath, …)`; grouping map
    `HashMap<FeedPath, Vec<FeedEventRecord>>`.

- [ ] **Step 1: Update the failing tests**
  1. `feed_events.rs` `#[cfg(test)] mod tests`: add a
     `fn fp(s: &str) -> FeedPath { s.parse().unwrap() }` helper; change every
     `.enqueue("/feed.rss")` to `.enqueue(&fp("/feed.rss"))`. (Behavior
     unchanged; dual-backend harness intact.)
  2. `worker.rs` `event()` helper (`:316`):
     `feed_url: feed_url.parse().expect ("valid feed path in test")` (field is
     now `FeedPath`; `feed_url: &str` param stays). Its callers pass valid paths
     (`"/feed.rss"`) — no change — **except** the exhaustion test.
  3. **Rewrite the exhaustion test**
     (`tick_marks_exhausted_when_regen_fails _past_backoff_table`, `:468`): the
     event carries a valid path and the failure is forced via storage, not a bad
     URL:

```rust
let mut site_config = storage::MockSiteConfigStorage::new();
// Force RegenerateError::Storage at the first read inside regenerate_feed.
site_config
    .expect_get_feeds_config()
    .times(0..)
    .returning(|| Err(sqlx::Error::PoolClosed));
site_config.expect_get_feeds_websub_hub_url().times(0..).returning(|| Ok(None));
site_config.expect_get_identity().times(0..).returning(|| Ok(SiteIdentity {
    title: "Jaunder".to_owned(), base_url: None,
}));
let mut posts = storage::MockPostStorage::new();
posts.expect_feed_urls_needing_catchup().times(0..).returning(|_| Ok(vec![]));
let mut events = storage::MockFeedEventStorage::new();
// Valid FeedPath; high attempt count pushes the next attempt past the backoff
// table, so the storage failure marks the batch exhausted (terminal).
events.expect_claim_pending_batch().times(1)
    .returning(|_, _| Ok(vec![event(1, "/feed.rss", 10)]));
events.expect_mark_exhausted().times(1).returning(|_, _| Ok(()));
let w = worker(site_config, posts, storage::MockFeedCacheStorage::new(), events);
w.tick().await;
```

4. **`backup_fixture.rs:164` (semantic):** the fixture enqueues
   `"https://example.com/feed.xml"` — an _absolute URL_ that is **not** a valid
   `FeedPath` (`from_str` rejects it). The fixture only needs _a_ feed event to
   round-trip through backup/restore, so change it to a representable path:
   `.enqueue(&"/feed.rss".parse::<FeedPath>().expect("valid feed path"))`. Check
   `assert_backup_fixture_restored` for a matching `feed_url` assertion and
   update it to `/feed.rss` too if present.
5. **Convert the integration regen-failure test.**
   `worker_applies_backoff_on_regen_failure` (`feed_worker.rs:222`, real-backend
   `#[apply(backends)]`) forces `BadUrl` by enqueuing `/this-is-not-a-feed-url`
   — impossible under `FeedPath`, and a real backend can't inject a `Storage`
   failure. **Delete it** from `feed_worker.rs` and add a mock-based sibling in
   `worker.rs` (next to the exhaustion test), forcing the failure via storage
   and asserting the _reschedule_ (not exhaust) branch:

```rust
// worker.rs #[cfg(test)] — mock reschedule-on-regen-failure (replaces the
// deleted integration test whose bad-URL trigger a FeedPath makes impossible).
#[tokio::test]
async fn tick_reschedules_on_regen_failure_within_backoff() {
    let mut site_config = storage::MockSiteConfigStorage::new();
    site_config.expect_get_feeds_config().times(0..)
        .returning(|| Err(sqlx::Error::PoolClosed));  // regenerate_feed → Storage
    site_config.expect_get_feeds_websub_hub_url().times(0..).returning(|| Ok(None));
    site_config.expect_get_identity().times(0..).returning(|| Ok(SiteIdentity {
        title: "Jaunder".to_owned(), base_url: None,
    }));
    let mut posts = storage::MockPostStorage::new();
    posts.expect_feed_urls_needing_catchup().times(0..).returning(|_| Ok(vec![]));
    let mut events = storage::MockFeedEventStorage::new();
    // attempts = 0 → next attempt is still inside the backoff table, so the
    // batch is rescheduled (mark_failed), not exhausted; and no ping is sent.
    events.expect_claim_pending_batch().times(1)
        .returning(|_, _| Ok(vec![event(1, "/feed.rss", 0)]));
    events.expect_mark_failed().times(1).returning(|_, _, _| Ok(()));
    let mut cache = storage::MockFeedCacheStorage::new();
    cache.expect_upsert().times(0);  // no cache row written on regen failure
    let w = worker(site_config, posts, cache, events);
    w.tick().await;
}
```

     (Asserts the same three facts as the deleted test — no cache row, no ping,
     event rescheduled — via mock expectations. The real-backend `mark_failed`
     scheduling SQL stays covered by `feed_events`'s dual-backend
     `mark_failed_increments_attempts_and_reschedules`.)

- [ ] **Step 2: Run them, verify they fail to compile**

Run: `cargo nextest run -p storage feed_events` and
`cargo nextest run -p server feed::worker` Expected: FAIL (compile) — record
field / `enqueue` signature / grouping map still `String`.

- [ ] **Step 3: Implement**
  1. `feed_events.rs`: `FeedEventRecord.feed_url: FeedPath` (`:35`);
     `FeedEventStorage::enqueue(&self, feed_url: &FeedPath)` (`:56`); the
     generic `enqueue` binds `.bind(feed_url.as_ref())` (`:172`).
  2. Backend row mappers — in `sqlite/feed_events.rs:52` and
     `postgres/feed_events.rs:46`:

```rust
feed_url: common::feed::FeedPath::try_from(r.get::<String, _>("feed_url"))
    // cov:ignore — the feed_url column is written only via a validated
    // FeedPath; a parse failure means DB corruption, surfaced as Decode.
    .map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
```

     (`InvalidFeedPath` must be `std::error::Error + Send + Sync + 'static` for
     `Decode` — `thiserror` gives that.) Both mappers currently build the record
     in an **infallible** `.map(|r| FeedEventRecord { … }).collect()`; the `?`
     above makes the closure return `Result`, so the closure body wraps in `Ok(…)`
     and the terminal becomes `.collect::<Result<Vec<_>, sqlx::Error>>()`.

3. `worker.rs`: grouping map `HashMap<FeedPath, Vec<FeedEventRecord>>` (`:123`);
   `process_feed_group(&self, feed_url: FeedPath, …)` (`:146`); the completion
   log becomes `tracing::info!(feed_url = %feed_url, …)` (`:172`).
   `regenerate _feed(…, &feed_url)`, `ping_websub(&feed_url, …)`,
   `on_regen_failure (&feed_url, …)` keep their `&str` params (argument-position
   deref coercion).

_Bodies are pinned by the updated tests plus the single-line `Decode` mapping
contract above._

- [ ] **Step 4: Run, verify pass**

Run: `cargo nextest run -p storage feed_events` and
`cargo nextest run -p server feed::worker` Expected: PASS.
`cargo xtask check --no-test` clean (`go_live_pass`'s two `enqueue(&url)` loops
now pass `&FeedPath`, matching the new signature).

- [ ] **Step 5: Commit**

```bash
git add storage/src/feed_events.rs storage/src/sqlite/feed_events.rs \
        storage/src/postgres/feed_events.rs server/src/feed/worker.rs \
        server/tests/feed/feed_worker.rs server/tests/storage/mod.rs \
        server/tests/misc/backup_fixture.rs
git commit -m "refactor(feed): feed_events speaks FeedPath (#399)"
```

Run `cargo xtask check` first (jaunder-commit).

---

## Task 5: `feed_cache` speaks `FeedPath` (+ handlers, regenerate)

**Files:**

- Modify: `storage/src/feed_cache.rs` (`FeedCacheRow.feed_url`, `get`/`delete`
  signatures, `row_from_tuple`, binds).
- Modify: `server/src/feed/handlers.rs:32-47` (`canonical`, `get(&feed_url)`).
- Modify: `server/src/feed/regenerate.rs:28-108` (`feed_url: &FeedPath`, row
  build, `BadUrl` `None`-arm `cov:ignore`).
- Test (call-site sweep — the `get`/`delete`/`regenerate_feed`/`FeedCacheRow`
  changes break each of these; all edited this task):
  - in-file storage tests in `storage/src/feed_cache.rs`.
  - `server/tests/feed/feed_regenerate.rs` — `regenerate_feed(…, "/…/feed.rss")`
    ×6 (`:62, 107, 147, 171, 234, 307`, `&str` literals → `&fp(…)`) and the
    `.get("/…/feed.rss")` assertions (`:78, 123, 162, 186`).
  - `server/tests/feed/feed_handlers.rs` — `.get("/~alice/feed.rss")` (`:98`)
    and the `FeedCacheRow { feed_url: … }` literals (`:172, 210, 246`).
  - `server/tests/feed/feed_worker.rs` — `.get(feed_url)` (`:238`) and the
    `FeedCacheRow { … }` at `:358` (the `feed_url` bindings there were retyped
    to `FeedPath` in Task 4, so these use `&feed_url` / `feed_url.clone()`).
  - in-file `sample_row`-style `FeedCacheRow` builders in
    `server/src/feed/regenerate.rs` (`:215`) and `server/src/feed/handlers.rs`
    (`:194`).
  - `server/tests/storage/mod.rs` — the `mk_row` `FeedCacheRow` builder
    (`:3394`).

**Interfaces:**

- Consumes: `common::feed::FeedPath`, `canonical`.
- Produces:
  - `FeedCacheRow.feed_url: FeedPath`.
  - `FeedCacheStorage::get(&self, feed_url: &FeedPath) -> Result<Option <FeedCacheRow>, FeedCacheError>`;
    `delete(&self, feed_url: &FeedPath)`. `MockFeedCacheStorage` regenerates.
  - `regenerate_feed(…, feed_url: &FeedPath) -> Result<FeedCacheRow, RegenerateError>`.

- [ ] **Step 1: Update the failing tests**

`feed_cache.rs` `#[cfg(test)] mod tests`: add
`fn fp(s: &str) -> FeedPath { s.parse().unwrap() }`; the `sample(url: &str)`
helper sets `feed_url: fp(url)`; change
`.get("/feed.rss")`/`.delete("/feed.rss")`/`.get("/missing")` to
`.get(&fp("/feed.rss"))` etc. (`"/missing"` is not a valid FeedPath — use a
valid-but-absent `fp("/tags/none/feed.rss")` for the miss test); assertions
`got.feed_url == *"/feed.rss"` via `PartialEq<str>`. Apply the same sweep to the
concrete files listed above (`feed_regenerate.rs`, `feed_handlers.rs`,
`feed_worker.rs`, the in-file `sample_row` builders, and `storage/mod.rs`
`mk_row`): `regenerate_feed(&fp(…))`, `.get(&fp(…))`,
`FeedCacheRow { feed_url: fp(…), … }`. Keep dual-backend harnesses.

- [ ] **Step 2: Run, verify fail to compile**

Run: `cargo nextest run -p storage feed_cache` and
`cargo nextest run -p server feed::` Expected: FAIL (compile) —
record/signatures still `String`/`&str`.

- [ ] **Step 3: Implement**
  1. `feed_cache.rs`: `FeedCacheRow.feed_url: FeedPath` (`:15`);
     `get(&self, feed_url: &FeedPath)` (`:32`) and
     `delete(&self, feed_url: &FeedPath)` (`:34`); binds `feed_url.as_ref()`
     (`:80`, `:123`) and `row.feed_url.as_ref()` (`:107`); `row_from_tuple`
     (`:39`) returns `Result<FeedCacheRow, sqlx::Error>` building
     `feed_url: FeedPath::try_from(t.0).map_err(|e| sqlx::Error::Decode(Box::new(e)))?`
     with the same `// cov:ignore` rationale as Task 4; `get` maps
     `row.map(row_from_tuple).transpose()`.
  2. `handlers.rs`:
     `let feed_url = common::feed::FeedPath::canonical(&surface, format);`
     (`:32`); `feed_cache.get(&feed_url)` and `regenerate_feed(…, &feed_url)`
     pass `&FeedPath`.
  3. `regenerate.rs`: `regenerate_feed(…, feed_url: &FeedPath)` (`:32`); the row
     build `feed_url: feed_url.clone()` (`:98`); the parse line keeps its shape
     but is now unreachable —

```rust
let (surface, format) =
    // cov:ignore — feed_url is a validated FeedPath, so parse always succeeds;
    // BadUrl is retained as a mapped (never-hit) error, not an expect()/panic.
    parse(feed_url).ok_or_else(|| RegenerateError::BadUrl(feed_url.to_string()))?;
```

     (`percent_encode_path(feed_url)` unchanged — `&FeedPath` derefs to `&str`.)

- [ ] **Step 4: Run, verify pass**

Run: `cargo nextest run -p storage feed_cache` and
`cargo nextest run -p server feed::` Expected: PASS. Then the **full gate**:
`cargo xtask validate --no-e2e` Expected: PASS (fmt, clippy, coverage incl. the
two `cov:ignore` arms, all tests). This is the spec's acceptance gate.

- [ ] **Step 5: Commit**

```bash
git add storage/src/feed_cache.rs server/src/feed/handlers.rs \
        server/src/feed/regenerate.rs server/tests/feed server/tests/storage/mod.rs
git commit -m "refactor(feed): feed_cache speaks FeedPath (#399)"
```

Run `cargo xtask check` first (jaunder-commit).

---

## Self-Review

- **Spec coverage:** `FeedPath` + trailer + `FromStr` chokepoint (Task 2);
  `feed_cache` records/`get`/`delete` (Task 5); `feed_events` record/`enqueue`
  (Task 4); `feed_urls_needing_catchup` (Task 3); `affected_feed_urls` (Task 2);
  `canonical` (Task 2); handlers/regenerate/worker propagation (Tasks 4–5);
  normalization-in-one-tested-place (Task 2 tests); `Decode`/`BadUrl` cov:ignore
  (Tasks 4–5); exhaustion-test rewrite (Task 4); absolute-URL follow-up (Task
  1); `validate --no-e2e` clean (Task 5 Step 4). All spec Acceptance bullets
  map.
- **Placeholder scan:** none — every implement step carries either full tests
  (Task 2) or exact signatures + the single-line contract the tests can't pin
  (the `Decode`/`cov:ignore` arms).
- **Type consistency:** `FeedPath`, `canonical(&FeedSurface, FeedFormat)`,
  `InvalidFeedPath`, `fp(&str)`/`sample(&str)` helpers, `enqueue(&FeedPath)`,
  `get`/`delete(&FeedPath)`, `regenerate_feed(&FeedPath)`,
  `feed_urls_needing_catchup -> Vec<FeedPath>` are used identically across
  tasks.
