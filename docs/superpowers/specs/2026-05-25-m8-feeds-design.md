# M8 — Published Feeds: Design

Status: approved 2026-05-25
Beads: jaunder-69y (M8.1), jaunder-gds (M8.2), jaunder-hir (M8.3), jaunder-ksv (M8.4), jaunder-3y4 (M8.5), jaunder-9d1 (M8.6), jaunder-ox8 (M8.7), jaunder-j4f (M8.8)

## Goal

Publish each user's posts as RSS, Atom, and JSON Feed at canonical URLs. Updates to posts propagate to feeds incrementally via a cache + event queue, with optional WebSub pings to a configured hub. No outbound federation; this milestone is consumed by feed readers and WebSub hubs only.

## Scope and surfaces

Four feed surfaces, three formats each:

```
GET /feed.{rss,atom,json}                          — site feed
GET /tags/:tag/feed.{rss,atom,json}                — site-tag feed
GET /~:username/feed.{rss,atom,json}               — user feed
GET /~:username/tags/:tag/feed.{rss,atom,json}     — user-tag feed
```

Routes mount at the bare prefix (no `/api`), public, no auth required.

## Visibility rules

Feeds include posts that are all of:

- Published (i.e. `published_at IS NOT NULL`).
- Not future-dated (`published_at <= now()`).

Drafts and unpublished posts are excluded. Deletion is out of scope for M8 — see "Out of scope" below — so no Atom `<at:deleted-entry>` tombstones in this milestone.

## Hybrid window

Each feed renders posts selected by the union of two thresholds, driven by `site_config` keys:

- `feeds.min_items` (default 20) — most recent N posts.
- `feeds.min_days` (default 30) — all posts published within the last D days.

The rendered set is the **union** of these two: quiet blogs still show their N most recent posts; busy blogs show the last D days. Items are ordered by `published_at DESC`.

The window is encapsulated as a value type in `common::feed::window`:

```rust
pub struct HybridWindow {
    pub min_items: u32,  // default 20
    pub min_days: u32,   // default 30
}

impl HybridWindow {
    pub fn from_site_config(cfg: &SiteConfig) -> Self;

    /// Posts published at or after this instant are inside the day-threshold.
    pub fn cutoff_date(&self, now: DateTime<Utc>) -> DateTime<Utc>;

    /// Apply the union rule to a slice of posts ordered by `published_at DESC`:
    /// include while `i < min_items` OR `published_at >= cutoff_date`; stop
    /// when both predicates fail.
    pub fn select<'a, P: HasPublishedAt>(&self, posts: &'a [P], now: DateTime<Utc>) -> &'a [P];
}
```

Storage trait method `list_published_in_window(surface, window: &HybridWindow)` uses `window.cutoff_date()` and `window.min_items` to bound its SQL; `select()` is unit-testable without a database and is the single home of the union semantics.

## Architecture

Three concentric layers plus a worker:

```
              ┌───────────────────────────────────────┐
              │  common::feed (pure)                  │
              │   • render_rss / render_atom / render_json
              │   • FeedMetadata, FeedItem            │
              │   • feed_etag()                       │
              │   • feed_path::{parse, canonicalize}  │
              └───────────────────────────────────────┘
                              ▲
              ┌───────────────┴───────────────────────┐
              │  storage::feed_cache (trait)          │
              │  storage::feed_events (trait)         │
              │   impls in storage::{sqlite,postgres} │
              └───────────────────────────────────────┘
                              ▲
              ┌───────────────┴───────────────────────┐
              │  server::feed                         │
              │   • handlers (axum)                   │
              │   • events (enqueue helper)           │
              │   • worker (tokio_cron_scheduler job) │
              │   • regenerate_feed (shared helper)   │
              └───────────────────────────────────────┘
                              ▲
              ┌───────────────┴───────────────────────┐
              │  common::websub                       │
              │   • WebSubClient trait                │
              │   • HttpWebSubClient (reqwest)        │
              │   • NoopWebSubClient                  │
              │   • CapturingWebSubClient (test-utils)│
              └───────────────────────────────────────┘
```

`regenerate_feed(state, feed_url)` is invoked from two places: the worker (eager regen on event tick) and the handler (lazy regen on cache miss). One helper, two callers — no duplicated logic.

## Canonical `feed_url`

All cache keys, event rows, and observability events use the **decoded path form** of the feed URL — e.g. `/~alice/feed.rss`, `/tags/c++/feed.atom`, `/tags/日本語/feed.json`. Hostnames and percent-encoding are added only when composing absolute URLs at HTTP send time (WebSub `hub.url`, feed `self` link, page `<link rel="alternate">` `href`).

A single canonicalizer in `common::feed::feed_path`:

```rust
enum FeedFormat { Rss, Atom, Json }
enum FeedSurface {
    Site,
    SiteTag { tag: String },
    User { username: String },
    UserTag { username: String, tag: String },
}

fn canonicalize(surface: &FeedSurface, format: FeedFormat) -> String;
fn parse(path: &str) -> Option<(FeedSurface, FeedFormat)>;
```

Round-trip: `parse(canonicalize(s, f)) == Some((s, f))` for every valid input.

`site.base_url` is the only source of host. Absolute composition is a one-liner: `format!("{}{}", state.site.base_url.trim_end_matches('/'), feed_url)` with percent-encoding applied to path segments.

## Storage

### Migrations

`storage/migrations/sqlite/0014_create_feed_cache.sql`
`storage/migrations/postgres/0014_create_feed_cache.sql`
`storage/migrations/sqlite/0015_create_feed_events.sql`
`storage/migrations/postgres/0015_create_feed_events.sql`

### `feed_cache`

| column         | notes |
|----------------|-------|
| `feed_url`     | PK, TEXT, decoded form |
| `body`         | TEXT — serialized feed (RSS/Atom/JSON) |
| `etag`         | TEXT — strong validator from `feed_etag()` |
| `content_type` | TEXT |
| `updated_at`   | TIMESTAMP — max(item.updated_at) or `generated_at` if empty |
| `generated_at` | TIMESTAMP — when this row was last regenerated |

### `feed_events`

| column            | notes |
|-------------------|-------|
| `id`              | PK |
| `feed_url`        | TEXT, indexed |
| `status`          | TEXT — `pending` \| `claimed` \| `done` \| `failed` |
| `attempts`        | INTEGER, default 0 |
| `last_error`      | TEXT, nullable |
| `next_attempt_at` | TIMESTAMP, default now |
| `claimed_at`      | TIMESTAMP, nullable — set when a tick transitions `pending → claimed` |
| `created_at`      | TIMESTAMP |
| `regenerated_at`  | TIMESTAMP, nullable |
| `pinged_at`       | TIMESTAMP, nullable |

Index `(status, next_attempt_at)` for the claim query; index `(feed_url, status)` for grouping at claim time.

### Storage traits

In `storage/src/feed_cache.rs`:

```rust
pub struct FeedCacheRow { /* columns above */ }

#[async_trait]
pub trait FeedCacheStorage: Send + Sync {
    async fn get(&self, feed_url: &str) -> Result<Option<FeedCacheRow>, FeedCacheError>;
    async fn upsert(&self, row: FeedCacheRow) -> Result<(), FeedCacheError>;
    async fn delete(&self, feed_url: &str) -> Result<(), FeedCacheError>;
}
```

In `storage/src/feed_events.rs`:

```rust
pub struct FeedEventRecord { /* columns above */ }

#[async_trait]
pub trait FeedEventStorage: Send + Sync {
    async fn enqueue(&self, feed_url: &str) -> Result<i64, FeedEventError>;
    /// Claim up to `limit` rows that are either:
    ///   • `status='pending' AND next_attempt_at <= now`, OR
    ///   • `status='claimed' AND claimed_at < now - lease_timeout` (stuck-claim recovery)
    /// Transitions them to `status='claimed'`, sets `claimed_at = now`, in one transaction.
    async fn claim_pending_batch(&self, limit: usize, lease_timeout: Duration)
        -> Result<Vec<FeedEventRecord>, FeedEventError>;
    async fn mark_regenerated(&self, ids: &[i64]) -> Result<(), FeedEventError>;
    async fn mark_pinged(&self, ids: &[i64]) -> Result<(), FeedEventError>;
    async fn mark_failed(&self, ids: &[i64], error: &str, next_attempt_at: DateTime<Utc>) -> Result<(), FeedEventError>;
    async fn mark_exhausted(&self, ids: &[i64], error: &str) -> Result<(), FeedEventError>;
}
```

Concrete impls live under `storage/src/sqlite/feed_cache.rs`, `storage/src/sqlite/feed_events.rs`, `storage/src/postgres/feed_cache.rs`, `storage/src/postgres/feed_events.rs`. `AppState` gains `feed_cache: Arc<dyn FeedCacheStorage>` and `feed_events: Arc<dyn FeedEventStorage>`.

### Claim-time deduplication

Multiple events may pile up on the same `feed_url` (e.g. five edits to one post). The worker groups claimed rows by `feed_url`, runs regen once per group, and marks all rows in the group `done` together. This preserves the audit trail while avoiding duplicate work.

### Stuck-claim recovery (claim lease)

If a worker process dies between `claim_pending_batch` and `mark_done`/`mark_failed`, the claimed rows would otherwise stay `claimed` forever. Each claim records `claimed_at = now`, and the next tick's claim query re-eligibles any row whose claim is older than `lease_timeout` (hard-coded **5 minutes** for M8 — comfortably longer than any sane regen + ping + retry).

Concretely the claim predicate is:

```
(status = 'pending' AND next_attempt_at <= now)
  OR (status = 'claimed' AND claimed_at < now - lease_timeout)
```

A row re-claimed under the lease will redo regen if it had completed but not yet been marked `regenerated_at`. This is safe: `regenerate_feed` ends with an UPSERT on `feed_cache` (PK = `feed_url`), so re-running it is idempotent — at worst wasted work, never corruption. To keep that "wasted work" window small, `regenerate_feed` UPSERTs the cache row and `mark_regenerated` for its event ids inside the *same* transaction.

## `site_config` keys

In `storage/src/site_config.rs`, add typed helpers (defaults in parens):

- `feeds.min_items` → `i64` (20)
- `feeds.min_days` → `i64` (30)
- `feeds.websub_hub_url` → `Option<String>` (None)

## Serializers (`common::feed`)

Pure functions, no I/O. Built on `rss` and `atom_syndication` crates; JSON Feed 1.1 hand-rolled (small, no good crate).

```rust
pub struct FeedMetadata {
    pub title: String,
    pub description: Option<String>,
    pub canonical_url: String,   // absolute
    pub self_url: String,        // absolute
    pub hub_url: Option<String>, // absolute, from feeds.websub_hub_url
    pub updated_at: DateTime<Utc>,
}

pub struct FeedItem {
    pub title: Option<String>,
    pub permalink: String,
    pub summary: Option<String>,
    pub content_html: String,
    pub published_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

pub fn render_rss(meta: &FeedMetadata, items: &[FeedItem]) -> String;
pub fn render_atom(meta: &FeedMetadata, items: &[FeedItem]) -> String;
pub fn render_json(meta: &FeedMetadata, items: &[FeedItem]) -> String;

pub fn feed_etag(items: &[FeedItem], generated_at: DateTime<Utc>) -> String;
```

### Empty-feed semantics

When `items.is_empty()`:

- Feed `updated_at` = the cache row's `generated_at`.
- `feed_etag` = strong validator hash of `(generated_at, 0, "")`.

This produces a stable validator so 304s work, and changes naturally when the first post is published.

## `regenerate_feed(state, feed_url)`

The shared helper used by both worker and handler-miss path:

1. `parse(feed_url)` → `(surface, format)`. Bail with internal error if unparseable (should not happen for cache/event-stored URLs).
2. Run `list_published_in_window(surface, min_items, min_days)` — new query method on `PostStorage` that returns the hybrid-window post set ordered by `published_at DESC`. Single query batched with a tag join.
3. Build `FeedMetadata` with absolute URLs and the configured `hub_url`.
4. `render_rss` / `render_atom` / `render_json` based on `format`.
5. Compute `feed_etag` and `Content-Type`.
6. `UPSERT` the `feed_cache` row.
7. Return the row to the caller.

## Handlers (`server::feed::handlers`)

Routes wired in `server::lib`:

```
GET /feed.{rss,atom,json}                      → feed_site
GET /tags/:tag/feed.{rss,atom,json}            → feed_site_tag
GET /~:username/feed.{rss,atom,json}           → feed_user
GET /~:username/tags/:tag/feed.{rss,atom,json} → feed_user_tag
```

Each handler:

1. Build `feed_url = canonicalize(surface, format)`.
2. `feed_cache.get(&feed_url)`:
   - On miss → `regenerate_feed(state, &feed_url)` → row.
   - On hit → use row.
3. Check `If-None-Match` against `row.etag` → 304 with empty body if match.
4. Check `If-Modified-Since` against `row.updated_at` → 304 if not modified.
5. Otherwise 200 with body and headers:
   - `Content-Type`: `application/rss+xml; charset=utf-8` | `application/atom+xml; charset=utf-8` | `application/feed+json`
   - `ETag: <strong validator>`
   - `Last-Modified: <updated_at>`
   - `Cache-Control: public, max-age=300`

Emits `feed.served { feed_url, format, cache_hit, status }`.

## Mutation hooks (`server::feed::events::enqueue_feed_events`)

```rust
async fn enqueue_feed_events(
    state: &AppState,
    post: &Post,
    old_tags: &[String], // pre-mutation tags; same as post.tags for create
) -> Result<(), FeedEventError>;
```

Computes affected surfaces:

- Site feed.
- User feed (`post.author`).
- For each tag in `union(post.tags, old_tags)`: site-tag feed + user-tag feed.

For each surface × `{rss, atom, json}`: one `feed_events` row, `status=pending`, `next_attempt_at=now()`. Called from `create_post`, `update_post`, `publish_post`, `unpublish_post` inside the same transaction as the post mutation.

A tag-edit example: post tags change `{rust, web}` → `{rust, leptos}`. Affected tag feeds: `rust`, `web`, `leptos` (union). Rows enqueued: site (×3) + user (×3) + site-tag × {rust, web, leptos} (×3 each) + user-tag × {rust, web, leptos} (×3 each) = 3 + 3 + 9 + 9 = 24 rows.

## Worker (`server::feed::worker`)

Job added to the existing `tokio_cron_scheduler` instance (M6). Hard-coded 10s interval for M8; configurability deferred (bead filed).

Per tick:

1. `claim_pending_batch(limit=200, lease_timeout=5min)` — picks up to 200 eligible rows (either fresh `pending` or `claimed` past their lease — see "Stuck-claim recovery"), transitioning them to `claimed` and stamping `claimed_at` atomically. The 200-row cap is a starting heuristic, revisit with production data.
2. Group claimed rows by `feed_url`.
3. For each group:
   - `regenerate_feed(state, feed_url)` → cache UPSERTed. Mark group `regenerated_at = now`.
   - If `feeds.websub_hub_url` is set:
     - `WebSubClient::send_publish(hub_url, absolute(feed_url))`, 5s timeout.
     - 2xx → mark group `done`, `pinged_at = now`.
     - Non-2xx or transport error → increment `attempts`, set `last_error`, `next_attempt_at = now + backoff(attempts)` with schedule `[1m, 5m, 30m, 2h, 2h, 2h]`. After attempt 6, `mark_exhausted` → `status='failed'`, emit `feed.websub.ping.exhausted`.
   - If `feeds.websub_hub_url` is unset: mark group `done`, `pinged_at = regenerated_at`.
4. Regen failure (DB error) → cache row untouched; events go to retry with same backoff schedule.

**No tick-overlap lock**: distinct `feed_url`s don't conflict; `claim_pending_batch`'s atomic transition prevents two ticks from grabbing the same row. The claim lease handles the dead-worker case.

**Ordering invariant**: the cache UPSERT commits *before* the hub ping is sent. When the hub re-fetches the feed in response to the ping, the handler reads the fresh `feed_cache` row. A failed ping leaves the cache row already updated (correct — direct fetchers get fresh content; only WebSub fan-out retries).

## WebSub (`common::websub`)

`common::websub` is a module, with each implementation in its own file:

```
common/src/websub/
    mod.rs        — WebSubClient trait, WebSubError, re-exports
    http.rs       — HttpWebSubClient (reqwest)
    noop.rs       — NoopWebSubClient
    capturing.rs  — CapturingWebSubClient (cfg(any(test, feature = "test-utils")))
```

```rust
// mod.rs
#[async_trait]
pub trait WebSubClient: Send + Sync {
    async fn send_publish(&self, hub_url: &str, feed_url: &str) -> Result<(), WebSubError>;
}

// http.rs
pub struct HttpWebSubClient { /* reqwest client, 5s timeout */ }

// noop.rs
pub struct NoopWebSubClient;

// capturing.rs
#[cfg(any(test, feature = "test-utils"))]
pub struct CapturingWebSubClient { /* Mutex<Vec<(hub_url, feed_url, sent_at)>> */ }
```

`send_publish` POSTs `hub.mode=publish&hub.url=<feed_url>` with `Content-Type: application/x-www-form-urlencoded`, 5s timeout. 2xx → `Ok`; everything else → `Err`.

Shape mirrors `common::mailer`. `AppState.websub: Arc<dyn WebSubClient>`, defaulting to `NoopWebSubClient` when `feeds.websub_hub_url` is unset.

This is ~20 lines of reqwest, not library-worthy, so it lives inside `common::` rather than being extracted as a crate. But because it is "our implementation," it gets a real network-level test (see Testing).

## Auto-discovery

New file `web/src/components/feed_discovery.rs` exporting `<FeedDiscovery base_path=... />`. Single component emits three `<link rel="alternate" type=... title=... href=... />` via `leptos_meta::Link`, one per format, composing canonical paths via `common::feed::feed_path`.

Invoked from the four page components in `web/src/pages/`: home (site), user profile, tag, user-tag.

## Observability

Per ADR 0011. Events emitted:

- `feed.regen.started { feed_url }`
- `feed.regen.completed { feed_url, item_count, bytes, duration_ms }`
- `feed.regen.failed { feed_url, error }`
- `feed.websub.ping.attempted { feed_url, hub_url, attempt }`
- `feed.websub.ping.succeeded { feed_url, hub_url, attempt, duration_ms }`
- `feed.websub.ping.failed { feed_url, hub_url, attempt, error, next_attempt_at }`
- `feed.websub.ping.exhausted { feed_url, hub_url }`
- `feed.served { feed_url, format, cache_hit, status }`

## Testing

Per CLAUDE.md: TDD. Tests are written *with* the code they cover, not at the end. The grouping below is for spec clarity, not phase ordering.

### Unit (in-file)

- `common::feed` serializers: with/without titles, with/without tags, multi-item, empty items, hub-set vs. hub-absent metadata.
- `feed_etag` stable for identical input; distinct for changed input; well-defined for empty.
- `common::feed::feed_path` round-trip: every surface × format; tag with `+`, `日本語`; leading/trailing whitespace rejection.
- `HttpWebSubClient` **against an in-process axum hub on a random port**: wire-level assertions on form body, Content-Type, timeout-error mapping, 2xx vs. non-2xx. This is the real-network test promised by "our implementation deserves network tests."

### Integration (both SQLite + Postgres)

- `feed_cache` / `feed_events` CRUD; event lifecycle; claim-time grouping; retry/backoff scheduling.
- `site_config` typed helpers: defaults + overrides.
- Handlers per format × per surface: cache-hit, cache-miss (lazy regen), 304 on `If-None-Match` and `If-Modified-Since`, correct `Content-Type`.
- Mutation hooks: expected event rows after each of create/update/publish/unpublish, including the tag-edit case (old + new union enqueued).
- Worker with `CapturingWebSubClient`: events drain, cache rows update, ping recorded, retry/backoff path, `hub_url`-unset short-circuit (`pinged_at = regenerated_at`).

### E2E (Playwright, Nix VM)

- Two users publish; per-user feeds in all three formats; both posts at correct URLs and order.
- Mock hub via `JAUNDER_WEBSUB_CAPTURE_FILE` (mirrors `JAUNDER_MAIL_CAPTURE_FILE`): publish → ping ≤ ~30s; edit → second ping.
- 304 short-circuit: fetch, capture ETag, refetch with `If-None-Match`, assert 304 empty.
- Empty-feed: per-user feed for a user with no published posts → 200 + valid empty body for each format.
- Auto-discovery: scrape `<head>` on each surface, follow every `<link rel="alternate">`, assert 200 + expected content-type.

End2end helpers added to `end2end/tests/websub.ts` mirroring `end2end/tests/mail.ts`: `readWebSubPings()`, `waitForNewPing(previousCount, timeout?)`. `scripts/e2e-local.sh` exports `JAUNDER_WEBSUB_CAPTURE_FILE` alongside the existing capture vars.

## Out of scope (filed as separate beads)

- **Post deletion + Atom `<at:deleted-entry>` tombstones.** M8 handles unpublish (which already enqueues feed events via the same path); hard delete is a separate effort.
- **Configurable worker interval** (`feeds.worker_interval_secs`) — P4, so we don't forget.
- **Site-config-change feed invalidation** (e.g. site title rename → regen everything) — defer; revisit if it bites.

## Bead fixups

After this spec is committed:

- Each M8.x bead's description has "See docs/milestones/M8.md Step N" — update to point at this spec file.
- M8.2 mentions `common::storage::feed_cache` / `common::storage::feed_events`; these now live under `storage::` per the post-M7 reorg. Update descriptions to match.

## Implementation order

Dependency DAG from the beads stands:

```
M8.1 (serializers) ────┐
M8.2 (storage)     ────┼──→ M8.4 (handlers) ──┐
M8.3 (site_config) ────┘                      ├──→ M8.5 (auto-discovery) ──┐
                                              │                            │
M8.2 ──→ M8.6 (mutation hooks) ───────────────┤                            ├──→ M8.8 (e2e)
                                              │                            │
M8.1, M8.2, M8.3 ──→ M8.7 (worker + WebSub) ──┘                            │
                                                                           │
        all upstream beads ────────────────────────────────────────────────┘
```
