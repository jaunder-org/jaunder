# Spec: RSS/Atom ingestion v1 — sources, polling, received archive (issue #282)

- Date: 2026-07-06
- Issue: [#282](https://github.com/jaunder-org/jaunder/issues/282) (scope
  amended by this cycle's design interview — see "Scope amendments")
- Parent design: `docs/feed-reading.md` (durable); `docs/hub-architecture.md`
  §5; ADR-0005, ADR-0006, ADR-0009, ADR-0010
- Decisions recorded: `docs/adr/drafts/feed-machinery-hub-boundary.md`,
  `docs/adr/drafts/feed-source-model-and-archive.md`

## Summary

The first inbound producer: follow syndication feeds by polling, and archive
every observed entry state into an append-only, per-entry-versioned raw archive.
**This slice ends at the archive** — it is the feed machinery of
`docs/feed-reading.md` §2, proven end-to-end by an inspection surface (JSON
endpoints + a minimal add/unfollow UI), not by Item derivation.

Naming: all inbound-syndication identifiers use the **`ajr_*`** family
(**A**tom/**J**SON Feed/**R**SS — `CONTEXT.md`), deliberately distinct from the
outbound `feed_*` family.

## Scope amendments vs. the filed issue

Settled in the design interview + reviews; the issue text is updated at plan
time:

- **Out (moved to a new hub-side "archive → Item derivation" issue):** Item
  derivation, the `jaunder-core` adapter question, the first-class actors
  entity, sanitization-at-derivation.
- **Author _capture_ stays** (structured observed-attribution data), the actors
  _entity_ goes.
- **In (added):** SSRF-guarded fetcher; a minimal add/unfollow UI page (the
  filed "no UI" non-goal is relaxed to exactly this); prompt-async add
  validation; **minimal unfollow** (an add-only surface plus never-remove
  retention would otherwise make a typo'd URL an eternal fetch loop).

## Non-goals

WebSub, JSON Feed, adaptive polling, auto-discovery, interaction-surface
probing, full-text scraping, lifecycle/rewrite policy (301 targets are
_recorded_ only), fetch-unit **removal** (unfollow stops scheduling; deleting an
`ajr_feeds` row and archive-facing lifecycle live with lifecycle & health), Item
derivation, actors, read-state, sync, any reader UI. All are homed in
`docs/feed-reading.md` §9's decomposition.

## Design

### Storage (all tables dual-backend per ADR-0019; tests per ADR-0053)

**`ajr_feeds`** — node-global fetch unit, unique per canonical feed URL.
Canonicalization (v1): lowercase scheme/host, strip default port and fragment;
otherwise exact-string (trailing-slash variants are distinct fetch units —
stated, not hidden behind "canonical").

- Identity/config: `ajr_feed_id`, `url` (unique, length-capped — feed-supplied
  strings participate in unique indexes, so `url` and `upstream_id` carry
  explicit caps), `interval_override_s` (nullable), `created_at`.
- Poll state: `next_poll_at`, `consecutive_failures`, `last_success_at`,
  `last_attempt_at`.
- HTTP cache validators: `etag`, `last_modified` (nullable).
- Recorded-not-obeyed hints: `hint_ttl_s`, `hint_update_period`,
  `hint_cache_control_max_age_s` (nullable).
- Recorded redirect: `permanent_redirect_url` (nullable; rewrite policy is a
  follow-on).

**`ajr_follows`** — user → fetch unit: `user_id`, `ajr_feed_id`, `followed_at`;
unique per pair. All "whose feed is this" queries go through it, and
**scheduling is gated on it**: a fetch unit with zero follows is never claimed
by the poller (row + archive retained).

**`ajr_entries`** — one row per known entry (the stable identity that query
surfaces address): `ajr_entry_id`, `ajr_feed_id`, `upstream_id` (length-capped),
`id_rule` (`atom-id` | `rss-guid` | `link` | `content-hash` — the fallback
chain, recorded), `first_seen_at`, `last_seen_at`. Unique on
`(ajr_feed_id, upstream_id)`. The one mutable archive table (`last_seen_at`
advances).

**`ajr_entry_versions`** — one row per observed state; **perfectly immutable**
(inserted, never updated, never deleted):

- `ajr_entry_id`, `version`, `observed_at`, `content_hash` (SHA-256 of fragment
  bytes; same hash → bump the entry's `last_seen_at` only).
- Payload: `fragment` — the native `<item>`/`<entry>` element,
  **context-re-injected** (see Parse → archive), character-exact UTF-8;
  `source_charset` recorded; `format` (`rss2` | `atom`).
- Thin index columns (extraction copies; fragment is truth): `title`, `link`,
  `published_at`, `updated_at`.
- Author capture: structured entry-level attribution — for each observed author:
  name/email/uri + originating element (`atom:author`, `dc:creator`,
  `fediverse:creator`, RSS `author`). Stored as a JSON column (archive-side
  data; the actors entity is hub-side).
- Comment-feed capture: `comments_feed_url` (from `wfw:commentRss` / RFC 4685
  `link rel="replies"`), `comments_page_url` (RSS `comments`), nullable.

**`ajr_channel_versions`** — observed states of the feed's own metadata
(identity is the `ajr_feeds` row): `ajr_feed_id`, `version`, `observed_at`,
`content_hash`, the **canonicalized channel envelope** (see Parse → archive),
extracted `title`, `description`, `site_url`, `icon_url`, channel-level author
capture. New version only on (canonicalized) hash change; immutable like entry
versions.

**`ajr_fetches`** — per-fetch outcome log (operational, prunable — explicitly
_not_ the archive): `ajr_feed_id`, `fetched_at`, `outcome` (`ok` |
`not-modified` | `http-error` | `network-error` | `parse-error` | `ssrf-refused`
| `too-large` | `timeout`), `http_status`, `bytes`, `duration_ms`,
`error_detail`, `new_entry_versions` (count). Pruned to the most recent N per
source (constant, ~50).

Storage traits follow ADR-0019 (trait + dialect where SQL diverges, generic
store, both-backend aliases), wired into `AppState` per ADR-0016 and exposed via
`provide_context` / `Extension` as today.

### Scheduler + poll pipeline (`server/src/ajr/`)

- A `tokio-cron-scheduler` repeated job (the `FeedWorker` precedent) ticks every
  ~10 s: claims due sources (`next_poll_at <= now` **and** follower count > 0),
  fetches, parses, archives, records the outcome, and computes the next due
  time. Fetches within a tick are **sequential** (v1 scale; no concurrency
  machinery).
- **Claim semantics**: claim-by-advancing-`next_poll_at` in a single statement
  (ADR-0021 discipline; `FOR UPDATE SKIP LOCKED` on Postgres). Accepted
  consequence, stated: a crash mid-fetch silently delays that source one
  effective interval — there is no lease column to reap.
- Effective interval: `interval_override_s` or the node default **3600 s**,
  clamped to ≥ **900 s** (the lower bound of ADR-0010's adaptive range, adopted
  as v1's fixed floor); on failure, doubled per consecutive failure, capped at
  **86 400 s**; `Retry-After` (429/503) taken as a lower bound when larger; ±
  bounded random **jitter** applied to every computed due time.
- Constructed at the composition root with its storage/client deps as
  constructor params; held alive alongside the existing workers (ADR-0016).

### Fetcher

One reusable component (the `websub/http.rs` seam pattern: trait + reqwest
impl + injectable fake for tests):

- Conditional GET from stored validators; 304 → `not-modified`, bump entry
  `last_seen_at` is **not** implied (nothing was observed) — validators
  refreshed if present, outcome recorded.
- **SSRF guard, with the TOCTOU hole closed**: http(s) only; resolve the
  hostname, reject private/loopback/link-local/unspecified ranges (v4 + v6), and
  **pin the vetted IP into the connection** (`ClientBuilder::resolve()` / custom
  connector) so a second DNS answer cannot swap the destination (DNS-rebinding
  defense). Auto-redirect **off**; hops followed manually (cap 5), re-running
  the full resolve-vet-pin per hop. Operator-config allowlist (host/CIDR) as the
  escape hatch.
- **The allowlist is load-bearing for tests, not just exotic deployments**: the
  Nix e2e sandbox (ADR-0034) and the integration fake-server tests can reach
  **only loopback** — exactly what the guard refuses — so allowlist
  configuration must be plumbable via test/e2e config from day one.
- Caps: response size **10 MiB measured post-decompression**, total timeout (30
  s).
- 301/308 chains record the final URL into `permanent_redirect_url`.
- `User-Agent: jaunder/<version> (+<node base URL>)`.

### Parse → archive (the hard kernel — designed, not discovered)

- Detect format (Content-Type, falling back to content sniffing), parse with the
  read side of `rss`/`atom_syndication` (ADR-0043 forks).
- **Fragment extraction is a second pass**: the parsing crates expose no byte
  spans, so a parallel `quick-xml` scan over the decoded document captures each
  top-level `<item>`/`<entry>` element's span (span capture must handle CDATA,
  comments, and same-named nested elements inside extensions). Fragments align
  with parsed entries **by position**; the alignment invariant is asserted
  (count mismatch → the whole fetch records `parse-error` and archives nothing —
  never archive misaligned data).
- **Context re-injection** onto each fragment root: the namespace prefixes the
  fragment actually uses in element/attribute names (not all in-scope — a feed
  generator adding one root namespace must not churn every entry), in
  deterministic sorted order, plus effective `xml:base` and `xml:lang` where
  inherited. Invariant: a stored fragment re-parses standalone and yields the
  same model it did in situ — verified via a synthetic-envelope test harness
  (wrap the fragment in a minimal `<feed>`/`<channel>` and re-parse; the crates
  don't parse bare elements publicly).
- **Channel envelope**: the document minus its entry ranges, canonicalized
  deterministically (each removed range collapses with its trailing whitespace)
  so the envelope's hash reflects channel _content_ change, not entry
  count/position shifts.
- For each entry: compute `upstream_id` by the fallback chain (recording the
  rule); find-or-create the `ajr_entries` identity row; hash the fragment;
  append a version or bump `last_seen_at`.
- Extract index columns, author capture, comment-feed links, and poll hints.
- A parse failure archives nothing and records `parse-error` (the feed may be
  temporarily serving an error page; the archive only ever holds things that
  parsed as feeds).

### Surfaces

- **JSON endpoints** (operator-authenticated via existing session auth; axum
  handlers with `Extension<Arc<dyn …Storage>>`):
  - `POST /api/ajr/feeds {url}` → 201 + source. The handler validates the URL
    **syntactically** (parses as a URL, scheme ∈ {http, https}; rejected
    otherwise) — cheap string checks are synchronous; everything requiring the
    network is not. The source is created with `next_poll_at = now`
    (**prompt-async validation**: reachability and feed-parseability are proven
    by the normal poll machinery within seconds; no synchronous fetch in the
    handler). Duplicate URL (under the documented canonicalization) → the
    existing source (idempotent add + follow).
  - `DELETE /api/ajr/feeds/:id/follow` → **unfollow**: removes the caller's
    follow; a fetch unit with zero follows stops being scheduled; the
    `ajr_feeds` row and the archive are retained. Re-adding the URL re-follows
    the existing unit.
  - `GET /api/ajr/feeds` → sources incl. poll state + latest health (derived
    from `ajr_fetches`).
  - `GET /api/ajr/feeds/:id/entries` → current entries (each with its latest
    version), paginated per ADR-0004 conventions.
  - `GET /api/ajr/entries/:id/versions` → the version chain incl. fragments
    (addressed by the stable `ajr_entry_id`).
  - (Exact path prefix may be adjusted at plan time to match router conventions;
    the shape is the contract.)
- **Minimal UI** (cockpit-side Leptos page, authenticated): an add-feed form + a
  sources list showing url/title, last fetch outcome, next poll, and an
  **unfollow** action — the list is what makes the prompt-async check's result
  visible. Server fns wrap the same service the JSON handlers use (one
  add/unfollow pipeline, two thin surfaces).

## Acceptance criteria (each observable)

Storage & archive semantics:

1. All new storage traits pass contract tests on **both** backends
   (`#[apply(backends)]`; ADR-0053).
2. Ingesting a fixture feed twice with identical bytes — served **without** HTTP
   validators, so hash-dedup itself is exercised rather than short-circuited by
   a 304 — yields **no** new entry versions on the second pass;
   `ajr_entries.last_seen_at` advances.
3. Changing one entry's content in the fixture yields exactly one new version
   for that entry (prior version rows bit-identical after the pass); other
   entries untouched.
4. Removing an entry from the fixture feed leaves its identity and versions
   readable (disappearance ≠ deletion).
5. A stored fragment re-parses standalone and yields the same field values it
   did in situ — fixtures include `dc:`/`media:`-namespaced entries **and an
   `xml:base`-on-root feed with relative entry links** (resolved URLs must
   match); the stored channel envelope re-parses standalone the same way.
6. Entry identity: fixtures exercising `atom:id`, `guid`, link-only, and
   none-of-the-above each archive under the documented rule, with `id_rule`
   recorded.
7. Author capture: fixtures with `atom:author`, `dc:creator`, and
   `fediverse:creator` populate structured attribution with correct provenance;
   channel-level author captured on the channel version.
8. Comment-feed capture: a fixture carrying `wfw:commentRss` (and RSS
   `comments`) populates `comments_feed_url` / `comments_page_url`; an Atom
   fixture with RFC 4685 `link rel="replies"` populates `comments_feed_url`.
9. Channel metadata change (e.g. retitled feed) versions the channel record
   without minting entry versions — **and** a feed update that only
   adds/removes/reorders entries mints **no** channel version (the
   canonicalized-splice test).

Polling & fetch:

10. A source due now is fetched by the worker within one tick interval;
    `next_poll_at` advances by the effective interval ± jitter; the 15-minute
    floor and 1-hour default are enforced (unit-testable interval computation).
11. After a 200 with validators, the next poll sends `If-None-Match` /
    `If-Modified-Since`; a 304 records `not-modified` and archives nothing
    (fake-server test).
12. Consecutive failures double the effective interval (capped at 24 h) and a
    success resets it; `Retry-After` is honored as a lower bound.
13. Every fetch appends an `ajr_fetches` row with the documented outcome
    taxonomy; the log is pruned to the per-source cap.
14. SSRF: adding `http://127.0.0.1/…`, `http://169.254.169.254/…`, or a feed
    whose redirect hops into a private range is refused with an `ssrf-refused`
    outcome (fake-server redirect test); an allowlisted host fetches
    successfully; non-http(s) schemes are rejected at add time (the handler's
    syntactic validation).
15. Oversize (measured **post-decompression**) and slow responses are cut off
    and recorded (`too-large`, `timeout`).
16. A redirect chain longer than the cap (5 hops) fails the fetch with a
    recorded outcome rather than following forever.
17. Fetch requests carry the documented `User-Agent` (observable at the fake
    server).
18. A well-formed feed served with a wrong `Content-Type` (e.g. `text/html`)
    still parses via sniffing; a non-feed body records `parse-error`.
19. Poll hints present in a fixture are recorded on the source and demonstrably
    do **not** affect the computed interval.
20. A permanent redirect records `permanent_redirect_url` while the source
    continues polling the original URL (rewrite is out of scope).

Surfaces:

21. `POST /api/ajr/feeds` responds 201 without blocking on any fetch, and the
    source's first fetch outcome is observable via `GET` shortly after
    (prompt-async validation, e2e-testable); posting a duplicate URL (under the
    documented canonicalization) returns the existing source and adds a follow
    rather than a second fetch unit.
22. Unfollow: after `DELETE …/follow`, the fetch unit accrues **no** further
    `ajr_fetches` rows across subsequent ticks; its row and archive remain
    readable; re-adding the URL re-follows the same unit (no duplicate).
23. All ajr endpoints reject unauthenticated and non-operator callers.
24. The UI page can add a feed, shows its fetch status on refresh/poll, and can
    unfollow it; covered by an e2e spec across the standard backend×browser
    matrix.
25. Entries/version endpoints return the archived data (spot-checkable against a
    fixture), paginated, addressed by stable `ajr_entry_id`.

Conventions:

26. `cargo xtask validate` green (static, clippy, coverage incl. new code, e2e).
27. New tables exist in **both** migration trees with parity.
28. Naming: all new identifiers use the `ajr_*` family per `CONTEXT.md` (updated
    in this cycle); no `feed_*` naming on inbound machinery; the follows table
    avoids "subscription" naming (the outbound `subscriptions` trap).

## Test strategy notes

- Fixture feeds (RSS 2.0 + Atom; namespaced, `xml:base`, author-variant, and
  validator-less variants) as repo test data; fake HTTP server (the
  injectable-fetcher seam) for conditional GET, redirects, SSRF, caps, and
  backoff tests — no live network in tests.
- **Loopback is SSRF-refused by default, and the hermetic sandboxes can reach
  nothing else** — the allowlist is therefore part of test/e2e wiring, not an
  afterthought; the e2e fixture feed is the node's **own** syndication feed (the
  only feed that exists inside the sandbox).
- Interval/backoff/jitter computation is a pure function, unit-tested
  exhaustively; the worker integration test only proves claim → fetch → archive
  → reschedule.
- e2e: one Playwright spec (add via UI → status visible → unfollow) — the matrix
  cost is per-spec, so exactly one.

## Follow-ups this spec commits to filing (plan task 1)

Per `docs/feed-reading.md` §9: archive → Item derivation (hub-side); adaptive
polling; WebSub; feed auto-discovery; interaction-surface discovery; lifecycle &
health; webmention interactions — each with native blocked-by links. Plus the
#282 issue-body amendment.
