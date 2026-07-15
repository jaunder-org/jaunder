# Spec — #399: `FeedPath` newtype for the feed identity key

- Issue: [#399](https://github.com/jaunder-org/jaunder/issues/399)
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  (this issue is one of its enumerated newtypes; no amendment)
- Related: [#14](https://github.com/jaunder-org/jaunder/issues/14),
  [#17](https://github.com/jaunder-org/jaunder/issues/17) (same type-safety
  family); a follow-up issue (filed as the plan's first task) covers the
  **absolute** feed URLs — see Non-goals.
- Date: 2026-07-14

## Problem

The feed cache and the feed-event queue are keyed by a feed's identity, yet that
key crosses as a bare `&str`/`String`:

- `FeedCacheStorage::{get,delete}(feed_url: &str)` and
  `FeedCacheRow.feed_url: String` (`storage/src/feed_cache.rs`).
- `FeedEventStorage::enqueue(feed_url: &str)` and
  `FeedEventRecord.feed_url: String` (`storage/src/feed_events.rs`).
- `PostStorage::feed_urls_needing_catchup(now) -> Vec<String>`
  (`storage/src/posts.rs`), whose output the worker feeds straight into
  `enqueue`.

Any string can be passed where the identity key is expected, and at a call site
the key is indistinguishable from any other string — a **transposition hazard**.

**Correction to the issue framing (resolved in the design interview).** The
issue describes this key as a URL with _host-case / http-vs-https /
trailing-slash_ normalization hazards. The code shows otherwise: this key is a
**site-relative canonical path** (`/feed.rss`, `/~alice/tags/rust/feed.atom`),
produced **only** by `common::feed::canonicalize()` (via `affected_feed_urls()`
and the feed handlers), on both the write side (`enqueue`, `regenerate_feed`'s
upsert) and the read side (`server/src/feed/handlers.rs` builds the surface from
parsed route args and canonicalizes — it never probes the cache with a raw
request path). It has no host and no scheme, so those hazards cannot arise, and
the "two spellings → two cache entries" divergence is already structurally
prevented. The `feed_cache` module doc names it exactly: _"the canonical
(decoded) path form."_

What is _not_ yet a type-level fact: (1) the key is a validated canonical feed
path rather than an arbitrary string (the transposition guarantee), and (2) the
one residual normalization the grammar performs — `common::feed::parse()`
lowercases the username/tag segments (`/~Alice/feed.rss` → `/~alice/feed.rss`) —
is not funnelled through a single named chokepoint.

## Why it earns a type (ADR-0063)

- **Invariant axis** — a `FeedPath` is exactly the set of strings
  `canonicalize()` can emit: a valid `(FeedSurface, FeedFormat)` in canonical
  (case-normalized) spelling. Its `FromStr` is the one place that invariant is
  enforced and normalization happens.
- **Transposition axis** — it is the dedup/SQL key for `feed_cache` and
  `feed_events`; typing it stops an arbitrary string being passed as the key.

ADR-0063 §Consequences already enumerates `FeedUrl` as an anticipated newtype;
this issue realizes it under the more accurate name **`FeedPath`** (it is a
path, matching the existing `common/src/feed/feed_path.rs` module and the module
doc). No ADR amendment is required — this is a standard `str`-backed newtype
using the existing `#[derive(StrNewtype)]` trailer.

## Decision

### `FeedPath` — the validated canonical feed path, in `common::feed`

```rust
// common/src/feed/feed_path.rs
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct FeedPath(String);

/// Error returned when a string is not a valid canonical feed path.
#[derive(Debug, thiserror::Error)]
#[error("not a valid feed path (expected a canonical /…/feed.{{rss,atom,json}} path)")]
pub struct InvalidFeedPath;

impl FeedPath {
    /// The infallible, provenance-guaranteed constructor: a `(surface, format)`
    /// pair is *always* a valid canonical path, so this cannot fail. Used by the
    /// identity-key producers (`affected_feed_urls`, the feed handlers) that
    /// already hold a decomposed surface. Delegates the format logic to
    /// `canonicalize` (its single source of truth).
    #[must_use]
    pub fn canonical(surface: &FeedSurface, format: FeedFormat) -> Self {
        Self(canonicalize(surface, format))
    }
}

impl FromStr for FeedPath {
    type Err = InvalidFeedPath;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // parse() is the grammar; re-canonicalize so the stored form is THE
        // canonical spelling (idempotent; normalizes case in user/tag segments).
        let (surface, format) = parse(s).ok_or(InvalidFeedPath)?;
        Ok(Self::canonical(&surface, format))
    }
}
```

- Lives in **`common::feed`** (String-backed, so wasm-safe; **no `url` crate** —
  it is not a dependency of `common`/`storage`/`server`/`web`, and these values
  are relative paths that `Url::parse` cannot represent without a base).
- **Standard `StrNewtype` trailer** (the same as `Tag`): `Display`,
  `AsRef`/`Borrow`/`Deref<str>`, `TryFrom<String>`, `From<Self> for String`,
  `PartialEq<str>`/`<&str>`, and the validating serde bridge. Derive `Hash` (it
  is a `HashMap` key in the worker), no `Ord` (nothing sorts feed paths).
- Two constructors, deliberately: `canonical` (infallible, for producers that
  already hold a `(surface, format)`) and `FromStr`/`TryFrom<String>` (fallible,
  the boundary for untrusted strings and DB read-back). `FromStr` is the sole
  validating/normalizing chokepoint (ADR-0063). The serde bridge comes free with
  the standard trailer; the storage records don't serialize today, but the
  standard surface is kept for consistency and costs nothing.

### `canonicalize` is unchanged; `affected_feed_urls` returns `FeedPath`

`canonicalize(surface, format) -> String` **keeps its current signature.** It is
also the feed **href** producer for HTML output (`web/src/feed_discovery.rs`,
`web/src/render/mod.rs`), where the value is a presentation string, not an
identity key, and where a newtype would force `.to_string()` churn (Leptos
`href=` needs `IntoAttributeValue`, which a `common` type cannot implement
without depending on leptos). Typing the identity key is done via
`FeedPath::canonical`, which wraps `canonicalize`, so there is one source of the
format logic and the presentation sites are untouched.

`affected_feed_urls` — whose only consumers are the two `enqueue` fan-outs (the
worker's steady-state go-live pass and `web::feed_events`) — returns the typed
form so those consumers hold a `FeedPath`:

```rust
pub fn affected_feed_urls<'a, I>(username: &Username, tags: I) -> Vec<FeedPath>
// builds each entry via FeedPath::canonical(&surface, format)
```

`parse(path: &str) -> Option<(FeedSurface, FeedFormat)>` is unchanged (the
grammar `FromStr`/`canonical`-callers delegate to); code needing the decomposed
`(surface, format)` from a `FeedPath` calls `parse(&feed_path)` via `Deref`.

## Propagation

### `storage` — `feed_cache`, `feed_events`, and the catch-up producer

- **Record fields become `FeedPath`:** `FeedCacheRow.feed_url: FeedPath`,
  `FeedEventRecord.feed_url: FeedPath`.
- **Trait signatures speak `FeedPath`** (threading the type through, per the
  issue): `FeedCacheStorage::get(&self, feed_url: &FeedPath)` and
  `delete(&self, feed_url: &FeedPath)`;
  `FeedEventStorage::enqueue(&self, feed_url: &FeedPath)`. Callers holding a
  `FeedPath` pass `&feed_path` with **no conversion**. The one place a `&str` is
  needed is the internal sqlx `.bind(...)`: it becomes
  `.bind(feed_url.as_ref())` (sqlx's `.bind` is generic over `Encode`, so
  `Deref` coercion does not apply to it — `.as_ref()` is required, exactly the
  ADR-0063 boundary shape where the newtype flows to callers but the encoder
  sees `&str`).
- **`PostStorage::feed_urls_needing_catchup(now) -> sqlx::Result<Vec<FeedPath>>`**
  (trait + generic impl in `posts.rs`). Its body already
  `SELECT feed_url FROM feed_cache`, `parse()`s each, and **skips unparseable
  rows** (`continue`); it now keeps the parsed `format` and pushes
  `FeedPath::canonical(&surface, format)` — infallible, no new Decode path, and
  the existing skip covers a corrupt column.
- **DB read-back (record construction) is fallible** where the column is mapped
  into a record field, mapping the impossible failure to `sqlx::Error::Decode`
  (the #400 `build_invite_record` pattern):
  - `feed_cache.rs::row_from_tuple` builds `FeedPath::try_from(t.0)` → on `Err`,
    `sqlx::Error::Decode`; threaded through `get`. `upsert` binds
    `row.feed_url.as_ref()`.
  - `sqlite/feed_events.rs` and `postgres/feed_events.rs` row mappers build
    `FeedPath::try_from(r.get::<String,_>("feed_url"))` → `Decode`.
  - These `map_err(Decode)` arms are **unreachable** (the column is written only
    via a validated `FeedPath`); they carry a `// cov:ignore` with that
    rationale rather than a synthetic corrupt-row test (see Tests).
- `mockall::automock` on both traits regenerates against `&FeedPath`
  automatically; mock-using tests pass a `FeedPath` (see Tests).

### `server` — feed handlers, regenerate, worker

- `handlers.rs`: `let feed_url = FeedPath::canonical(&surface, format);` (was
  `canonicalize(...)`); `feed_cache.get(&feed_url)` passes `&FeedPath`.
- `regenerate.rs`: `regenerate_feed(…, feed_url: &FeedPath)`. Internals reach
  `&str` via `Deref`: `percent_encode_path(feed_url)` unchanged;
  `FeedCacheRow.feed_url: feed_url.clone()` (was `.to_string()`). The
  `parse(feed_url).ok_or(RegenerateError::BadUrl(…))` line is now **defensively
  unreachable** (a `FeedPath` always parses); its `None` arm carries a
  `// cov:ignore` and the `BadUrl` variant is retained as the mapped (never-hit)
  error rather than an `.expect()` — no `expect_used`, no panic.
- `worker.rs`:
  - `go_live_pass`: `for url in feed_urls_needing_catchup(now).await?` →
    `url: FeedPath`, `enqueue(&url)`; `for url in affected_feed_urls(...)` →
    `url: FeedPath`, `enqueue(&url)`.
  - `tick`: the grouping map becomes `HashMap<FeedPath, Vec<FeedEventRecord>>`
    (`FeedPath: Hash + Eq`); `groups.entry(rec.feed_url.clone())` unchanged in
    shape.
  - `process_feed_group(feed_url: FeedPath, …)`;
    `regenerate_feed(…, &feed_url)`. The `tracing::info!(feed_url, …)`
    bare-identifier capture becomes `tracing::info!(feed_url = %feed_url, …)`
    (`FeedPath` has `Display`, not `tracing::Value`).
    `ping_websub(&feed_url, …)` and `on_regen_failure(&feed_url, …)` keep their
    `&str` params — `&FeedPath` deref-coerces at the (argument-position) call.

### `web` — event fan-out only

- `web/src/feed_events.rs::enqueue_feed_events` iterates
  `affected_feed_urls(...)` (now `Vec<FeedPath>`) and calls
  `events.enqueue(&url)` (`&FeedPath`). No signature change to the public server
  fn.
- `web/src/feed_discovery.rs` and `web/src/render/mod.rs` are **unchanged** —
  they call `canonicalize(...) -> String` for `href=` output (see Decision).

## Tests

- `common::feed::feed_path`:
  - `FeedPath::from_str` accepts every canonical surface×format (round-trips via
    the existing `canonicalize`/`parse` fixtures) and **rejects** the same
    inputs `parse` rejects (bad ext, non-canonical prefix, invalid
    tag/username).
  - **Normalization in one place** (the acceptance):
    `"/~Alice/feed.rss" .parse::<FeedPath>()` == `"/~alice/feed.rss"`;
    `"/tags/Rust/feed.atom"` normalizes to lowercase.
  - Idempotence: `FeedPath::canonical(&s, f).as_ref().parse::<FeedPath>()`
    equals the `canonical` result (the stored form is a fixed point);
    `canonical` agrees with `canonicalize`
    (`canonical(...).as_ref() == canonicalize(...)`).
  - `Deref`/`AsRef` bind as `&str`; `Display` round-trips.
- `storage::feed_cache` / `feed_events`: existing backend tests updated to
  construct the key as `FeedPath` (a `fp("/feed.rss")` helper =
  `"/feed.rss".parse().unwrap()`); behavior unchanged. The `map_err(Decode)`
  read-back arms are **not** unit-tested — they are `// cov:ignore` defensive
  (the column is unreachable-invalid by construction). This is an explicit,
  reasoned coverage exception, not an untested assertion.
- `server::feed::worker`:
  - The exhaustion test
    (`tick_marks_exhausted_when_regen_fails_past_backoff_ table`) loses its
    trigger — a `FeedEventRecord.feed_url: FeedPath` cannot hold
    `"not-a-feed-url"`, and `regenerate_feed` no longer fails on parse.
    **Rewrite it** to force `RegenerateError::Storage` instead: the event
    carries a valid `fp("/feed.rss")`, and a mocked storage read inside
    `regenerate_feed` (e.g. `posts.list_published_in_window` or a `site_config`
    getter) returns `Err`, so the batch still reaches the backoff-past-table →
    `mark_exhausted` path. The `event(id, feed_url, attempts)` helper
    takes/parses a valid `FeedPath`.
  - Other worker mock tests that
    `expect_feed_urls_needing_catchup().returning(|| Ok(vec![]))` are unaffected
    (empty `Vec<FeedPath>` infers); any returning a non-empty list use
    `fp(...)`.
- `server/tests/storage/mod.rs`
  (`feed_urls_needing_catchup_returns_stale_feeds`, the `canonicalize`-built
  URLs at :3406): updated to expect/compare `FeedPath` (via `PartialEq<str>` or
  `fp(...)`), behavior unchanged.

## Acceptance

- `feed_cache` (`FeedCacheRow`, `get`, `delete`), `feed_events`
  (`FeedEventRecord`, `enqueue`), and `feed_urls_needing_catchup` speak
  `FeedPath`; `affected_feed_urls` and `FeedPath::canonical` produce it.
- Feed identity is normalized in exactly one tested place (`FeedPath::from_str`
  → `parse` + `canonicalize`); a non-canonical spelling is not representable as
  a stored key.
- An arbitrary `String` can no longer be passed as the cache/event key nor held
  in a record field (the transposition guarantee).
- `FeedPath` uses the standard `#[derive(StrNewtype)]` trailer; only its
  `canonical` constructor and normalizing `FromStr` are hand-written.
- `cargo xtask validate --no-e2e` clean.

## Non-goals

- **The absolute feed URLs** — `self_url`, `canonical_url`, `hub_url` /
  `websub_hub_url` (`FeedMetadata`, `FeedMeta`, `FeedsConfig`), and the WebSub
  ping target composed in `worker.rs`. These are genuine absolute URLs composed
  server-side from `base_url` + a percent-encoded path, render/ping-path-only,
  and warrant a **different** type (a `url`-crate-backed `AbsoluteUrl`/`SiteUrl`
  with real scheme/host normalization). Filed as a follow-up issue (the plan's
  first task). Folding them onto `FeedPath` is incoherent — they do not share
  its path grammar.
- **`permalink` / `preview_url`** — paths that cross serde to the wasm client as
  `href=` values (`web/src/posts/*`). A separate typing concern (noted in the
  follow-up); not this issue.
- **`canonicalize`'s return type** — it stays `String` (the HTML-href producer);
  only the identity-key surface is typed. No storage schema change: the
  `feed_url` TEXT columns are unchanged.

## Amendments (post-review, as shipped)

Two changes to what shipped versus the plan above, both made during the
whole-branch review and captured here so the archived record matches the code:

- **`feed_events` corrupt-row handling: purge-and-skip, not `Decode`.** The
  cold-blind review found that mapping a corrupt `feed_events.feed_url` to
  `sqlx::Error::Decode` inside `claim_pending_batch` would fail the whole batch
  and **wedge the feed worker** on the bad row forever (it is leased then
  re-claimed each cycle). Since such a row can only arise from DB tampering (all
  `enqueue` paths write a validated `FeedPath`) and names no identifiable feed,
  the claim now **skips it, purges it (`DELETE` + `warn!`), and drains the rest**
  — matching `posts.rs`'s existing `continue`-skip. This is tested dual-backend
  (`claim_purges_rows_with_unparseable_feed_url`), so the arm is no longer
  `cov:ignore`. `feed_cache.get` keeps the `Decode` + `cov:ignore` approach: its
  arm is *provably unreachable* (it binds a valid `FeedPath` in `WHERE feed_url =
  $1`, so any matched row's key re-parses), and it is a single-row read the
  caller recovers from — so the asymmetry is deliberate, not an oversight.
- **`feed_url` → `feed_path` for `FeedPath`-typed bindings.** The `FeedCacheRow`
  / `FeedEventRecord` **fields**, the `get`/`delete`/`enqueue`/`regenerate_feed`
  parameters, and the local variables/test bindings that hold a `FeedPath` are
  named `feed_path` (the name matches the type). Deliberately **kept** `feed_url`:
  the DB **column** (in all SQL / `.get("feed_url")` — the fields mirror it), the
  `affected_feed_urls` / `feed_urls_needing_catchup` function names, the
  `&str` params that receive a `FeedPath` via `Deref` (`ping_websub`,
  `on_regen_failure`, `send_publish`) and their telemetry, and the JSON-Feed
  `"feed_url"` output key.
