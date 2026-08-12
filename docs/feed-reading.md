# Web-Feed Reading — machinery & experience

- **Status:** living design reference (a peer of `hub-architecture.md`, same
  discipline: durable substance lives here; discrete slices spawn issues with
  short-lived spec/plan pairs; settled choices lock as ADRs).
- **Born:** 2026-07-06, from the issue #282 design interview.
- **Re-verified against the tree:** 2026-08-11 (ADR corpus 65 → 112; the
  citations and house conventions below were refreshed then — see §2.8).
- **Scope:** everything between "the user pastes a URL" and "the user has read,
  kept, or reacted to an article" — for **syndication feeds** (RSS, Atom, JSON
  Feed). The hub machinery (Item derivation, sync engine, annotations) is
  `hub-architecture.md` §5–§6 territory; this doc designs the feed side of that
  boundary and names the interface points.

## 1. Purpose and boundary

The web-feed machinery is designed **on its own terms** — the way a good
standalone feed-fetching service would be — not distorted to fit the hub's
convenience. Its obligation to the rest of jaunder is a single clean contract:

> An **append-only archive of observed entry versions** (plus source/health
> metadata), queryable, from which the hub derives normalized `Item`s
> non-destructively and re-runnably.

Everything above that contract (Items, timelines, read-state, sync) is hub
machinery with its own design track. Everything below it (sources, polling,
fetching, parsing, archiving, discovery) is this doc. The boundary decision is
recorded in `docs/adr/drafts/feed-machinery-hub-boundary.md`.

Consequence for sequencing: issue #282 (RSS/Atom ingestion v1) **ends at the
archive**; the archive → Item derivation is its own hub-side issue (#919).

## 2. The feed machinery (resolved direction; v1 = #282)

### 2.1 Source model — shared fetch units + per-user follows

The inbound-syndication identifier family is **`ajr_*`** (**A**tom / **J**SON
Feed / **R**SS): unfamiliar but unambiguous, and deliberately distinct from the
outbound `feed_*` family so an identifier's direction is always legible
(`CONTEXT.md`). Two entities, honoring ADR-0006's shared-ingestion tier:

- **`ajr_feeds`** — the node-global **fetch unit**, unique per canonical feed
  URL (v1 canonicalization: lowercase scheme/host, strip default port and
  fragment; otherwise exact-string — trailing-slash variants are distinct).
  Everything about _fetching_ lives here: poll state (interval override,
  next-due, backoff), HTTP cache validators (ETag/Last-Modified), recorded poll
  hints, health counters, discovered redirect targets.
- **`ajr_follows`** — user → fetch unit. Everything about _one user's
  relationship to the feed_ lives here (later: folders, per-source toggles like
  full-text scraping). v1 populates it trivially for the single user, but the
  seam exists from day one: a second follower must not cause a second fetch
  loop, and later per-user settings must not force a schema split.
- **Unfollow exists from v1** — a fetch unit with zero followers is never
  scheduled (its row and its archive are retained; retention doctrine is about
  received content, not about polling forever at a typo). Removing a fetch unit
  outright stays out of scope until lifecycle & health (§2.7).

Authenticated feeds (§8) refine fetch-unit identity to (canonical URL,
credential). Recorded in `docs/adr/drafts/feed-source-model-and-archive.md`.

### 2.2 Delivery — polling now, push later, one parse path always

**Delivery is a separate axis from content-format** (hub-architecture §5): the
parser never knows whether bytes arrived by poll or by push.

v1 polling policy (#282):

- Node-default interval **1 hour**; per-source override column; **15-minute
  floor** enforced everywhere (adopting the lower bound of ADR-0010's adaptive
  range as v1's fixed floor).
- **Error backoff**: consecutive failures double the effective interval, capped
  at 24 h; success resets. HTTP 429 / `Retry-After` honored. Failures never
  remove a source.
- **Jitter** spreads due times so a node's feeds don't thundering-herd.
- Feed-supplied cadence hints (`<ttl>`, `syn:updatePeriod`, `Cache-Control`) are
  **recorded, not obeyed** — inputs for adaptive polling.
- Mechanically: the proven claim-lease queue pattern (frequent scheduler tick
  claims due sources; ADR-0021 transaction discipline).

Follow-ons:

- **Adaptive polling** _(#920)_ — choose each source's interval from observed
  posting cadence + recorded hints, clamped 15 m–24 h (realizes ADR-0010).
- **WebSub — subscriber side** _(#921)_ — `rel=hub` discovery, subscribe/renew
  lease lifecycle, HMAC-verified callback (never trust an unauthenticated push —
  hub-architecture §5), unsubscribe, automatic poll fallback for hubless feeds.
  Pure delivery: lands bytes in the same parse → archive pipeline. Note the
  **publisher** side already exists (`server/src/websub/`, pinging hubs about
  our own feeds); the two share the spec and the `http.rs` client seam, nothing
  else.

### 2.3 The fetcher — polite and paranoid

One HTTP fetch component, reused by every future fetch path (auto-discovery,
interaction probing, full-text scraping, image proxying), so its properties are
inherited system-wide:

- **Conditional GET**: send `If-None-Match`/`If-Modified-Since` from stored
  validators; a 304 costs almost nothing and bumps last-seen.
- **SSRF guard** (v1, in the fetcher's bones): http(s) schemes only; resolve and
  reject private/loopback/link-local destinations, **re-checked on every
  redirect hop**; operator-config allowlist for genuinely internal feeds.
- **Caps**: response size, timeout, decompression limit, redirect limit.
- **Identification**: honest `User-Agent` naming jaunder and the node.
- **Recording**: every fetch outcome (status, timing, validator result, error)
  is recorded per source — the raw material for health surfacing and adaptive
  polling. Operational data, prunable; _not_ part of the sacred archive.
- Permanent redirects (301/308) are recorded in v1; the URL-rewrite _policy_
  belongs to lifecycle & health (§2.7).

### 2.4 Parsing — formats behind a seam

RSS 2.x and Atom in v1 (the read side of registry `rss` / `atom_syndication`, on
quick-xml ≥ 0.41 — ADR-0089; the ADR-0043 forks this doc first cited are retired
and 0043 is superseded); **JSON Feed** as a follow-on _(doc-only until real)_.
Parsing produces a format-neutral internal view (entries with identity, display
fields, authors, hints) used for extraction — while the archive stores each
entry's **native** payload untouched (§2.5). Adding a format touches the parser
seam and nothing downstream.

### 2.5 The received archive — entry identity + immutable versions

Three tables realize hub-architecture §5's archive rules for feeds (§5's
conceptual `received_feeds` made concrete):

- **`ajr_entries`** — one row per **known entry**: which fetch unit, its
  `upstream_id`, which identity rule produced that id, first-seen and last-seen.
  The one mutable archive row (last-seen advances), and the stable identity that
  query surfaces address.
- **`ajr_entry_versions`** — one row per **observed state** of an entry: version
  number, observed-at, content-hash, the native fragment, extraction copies.
  **Perfectly immutable** — never updated for any reason; the archive's core
  invariant lands on a table boundary instead of a convention.
- **`ajr_channel_versions`** — observed states of the feed's _own_ metadata
  (title/description/icon/self-links — RSS's "channel", Atom's feed header),
  versioned on its own cadence so it never churns entry versions. Its identity
  is the `ajr_feeds` row itself.

Semantics:

- **Payload**: the entry's **native fragment** — the RSS `<item>`/Atom `<entry>`
  XML (later: the JSON Feed item object) — character-exact post-decode (UTF-8;
  original charset recorded).
- **Context re-injection** (XML): an extracted fragment may depend on context
  declared on sliced-away ancestors — namespace prefixes, `xml:base`
  (relative-URL resolution), `xml:lang`. In-scope declarations are re-injected
  onto the fragment root: only the namespace prefixes the fragment actually uses
  in element/attribute names (injecting _all_ in-scope prefixes would mint a
  spurious version of every entry when a feed generator adds one root
  namespace), in deterministic sorted order, plus effective `xml:base`/
  `xml:lang`. Invariant (tested, including an `xml:base` fixture): a stored
  fragment re-parses standalone and yields the same model it did in situ. JSON
  Feed items need none of this — they carry no inherited context — and
  character-exact slicing is directly available (`serde_json` `RawValue`), which
  is why the format unification is safe (next bullet).
- **One version table for all feed formats** works precisely because the payload
  is format-native + format-tagged; only identity, versioning machinery, and
  extraction copies are shared columns — nothing shared ever carries format
  semantics. (The per-protocol split of hub-architecture §5 is feeds vs AP vs AT
  — RSS/Atom/JSON Feed are all "syndication feed" per `CONTEXT.md`.)
- **Change detection**: SHA-256 over the fragment bytes. Same hash → bump the
  entry's last-seen, no new row. Different → new immutable version.
- **Entry identity**: `atom:id` → RSS `guid` → entry link → content-derived
  hash; the applied rule is recorded, so a source's identity quality is
  observable.
- **Disappearance ≠ deletion**: entries that scroll off the feed are retained;
  recorded as state, never a row removal. (RSS/Atom carry no delete signal;
  deletion is operator-sovereign per hub-architecture §5.)
- **Channel envelope canonicalization**: "the feed document minus its entries"
  is a non-contiguous splice — done naively, its bytes (and hash) change
  whenever entry count/positions shift, minting a channel version per post and
  defeating the table. The splice is therefore canonicalized deterministically
  (each removed entry range collapses with its trailing whitespace); pinned in
  the archive ADR, not improvised at implementation.
- **Thin index columns** (title, link, published/updated, extraction copies of
  authors) live on version rows for query surfaces; the fragment remains the
  source of truth and every extraction is re-runnable from it.
- **Hash churn** (feeds embedding volatile bytes — comment counts, tracking
  tokens) mints spurious versions; v1 hashes raw bytes fidelity-pure and
  _observes_ churn as a health signal; mitigation policy (e.g. per-source
  semantic-hash fallback) is lifecycle & health work (§2.7).

### 2.6 Author capture — data now, identities later

RSS/Atom authorship is messy: feed-level `author`/`managingEditor`, per-entry
`atom:author` (name/email/uri), free-text `dc:creator`, and the newer
`fediverse:creator` extension. The feed machinery's job is **faithful capture**:
structured observed-attribution fields on each entry version and on channel
versions, recording which element each came from. Comment-feed links
(`wfw:commentRss`, RFC 4685 `link rel="replies"`) are captured the same way —
they're in-feed data and cost nothing.

Folding observations into first-class **actor identities** (shared with
outbound, protocol-native keys, the address book) is hub-side work, homed with
Item derivation — an Item's author-ref points at an actor; the archive's capture
is its evidence.

### 2.7 Lifecycle & health _(#924)_

The user's window into "is my feed working": per-source health derived from the
fetch log — consecutive failures, last success, 410-gone detection, dead-feed
heuristics, permanent-redirect rewrite policy (with history), hash-churn
flagging and mitigation, and the surfacing UX ("this feed has been failing for a
week; the site moved here; fix or unfollow?"). v1 records everything this needs;
this slice adds the policy + presentation.

### 2.8 House conventions this machinery inherits

The feed machinery is designed on its own terms (§1), but it is built in this
repo and obeys its standing decisions. The ones that shape the schema and the
surfaces, current as of 2026-08-11:

- **URLs are role-tagged newtypes** — `TaggedUrl<Role>` with a distinct
  zero-sized role per meaning (ADR-0112); `AbsoluteUrl` is deleted and there is
  no neutral tag. Inbound source URLs therefore need **their own role**, not the
  outbound `FeedUrl` (`TaggedUrl<Feed>` = a feed jaunder publishes).
- **Timestamps** cross boundaries as `UtcInstant` (ADR-0072); **ids** are
  newtypes with the ADR-0063/0101 trailer, bridged to sqlx per ADR-0071.
- **Closed string sets** (identity rule, fetch outcome, payload format) are
  `#[text_enum(sqlx, …)]` (ADR-0091), not free-text columns.
- **There is no hand-rolled JSON API.** The server mounts exactly one API route,
  `/api/{*fn_name}`; every wire op is a `#[server]` fn at `/api/<vertical>/<op>`
  with a verb-led ident (ADR-0082, ADR-0065). An inspection surface for the
  archive is a web vertical, not a REST router.
- **Storage** is trait + dialect + generic store (ADR-0019), contract-tested on
  both backends (ADR-0053, ADR-0103), with SQLite write-lock occupancy bounded —
  no per-row write loops when archiving a poll's entries (ADR-0092).

## 3. Discovery

### 3.1 Feed auto-discovery _(#922)_

Naming caution: `web/src/feed_discovery/` already exists and means the
**outbound** thing — the `<link rel="alternate">` / RSD tags advertising
jaunder's own feeds. Inbound discovery needs a different module name (the
`ajr_*` reasoning applies to modules, not just tables).

Paste any URL, get its feeds: fetch the page (guarded fetcher), probe
`<link rel="alternate" type="application/{rss,atom}+xml;application/feed+json">`,
fall back to common paths (`/feed`, `/rss.xml`, `/atom.xml`, `/index.xml`),
surface multi-feed disambiguation (comments feed vs posts feed) at add time.

### 3.2 Interaction-surface discovery _(#923; cost-aware by design)_

Given an article, learn where reactions can land. Comprehensive **in what it
records**, economical **in when it probes**:

- **Source-level probe** (once per source + slow re-probe cadence): webmention
  endpoint, AP actor presence, site-level social hints. Site properties don't
  need per-entry fetches.
- **Entry-level probe** (lazy — on article open or first interaction attempt):
  the specific AP object URI for _this_ article
  (`rel=alternate type=application/activity+json`), AT hints.
- **Free tier**: comment-feed links arrive in the feed itself — captured by the
  archive with zero extra fetches.
- Everything found is recorded, including affordances jaunder can't yet act on.
  Probe results are stored data with a re-probe policy, not improvised
  per-button checks.

The naive alternative — eagerly fetching every article page at ingest — roughly
doubles fetch volume with pages 10–50× feed size, against servers that already
told us everything in the feed. Declined.

## 4. Content enrichment _(doc-only until real)_

- **Full-text scraping** — for summary-only feeds: fetch the article page
  (guarded fetcher), readability-style extraction, stored as **derived** content
  alongside — never overwriting — the archive; per-source toggle on the follow.
- **LLM summarization** — for summary-less articles: a derived annotation on the
  hub-side Item (never touches the archive); its own design must pin provider,
  cost, and privacy stance. Hub-side (needs Items).
- **Image/tracker proxying** — route article images through the node: strips
  trackers, hides reader IPs from publishers; a caching proxy on the guarded
  fetcher; cache/size policy of its own.
- **Enclosures & media** — podcasts (`enclosure`, iTunes tags), YouTube feeds
  (`media:group`): the fragments retain them today; extraction and presentation
  (a play button, an episode list) are experience-side work.

## 5. The reading experience (interface points into the hub)

The cockpit reader consumes hub Items; these capabilities are _homed elsewhere_
and mapped here so this doc's vocabulary lines up:

- **Read/unread** — the first sync-engine annotation (#283 carries both the
  Item-upsert events and the read-state deltas) and the minimal reader (#285);
  monotonic fold with explicit mark-unread override.
- **Read-later** — the `saved` annotation (hub-architecture §6's LWW saved/muted
  pair); no new machinery needed, just reader UI.
- **Summary-as-you-scroll → full text on open** — Item carries summary/content;
  where the feed lacks a real summary, enrichment (§4) supplies one as
  derived/annotation data.
- **Folders/tags, mute/filter rules, unread counts, feed icons** — cockpit track
  (#203) plus annotation vocabulary; feed icons come from channel metadata +
  favicon fetch.
- **Search over items** — hub-side FTS across derived Items (FTS5 / tsvector per
  backend); the Item storage design must leave room for it (constraint stated in
  the derivation issue).
- **Dedup across routes** (same article via RSS and AP) — deferred with the
  optional `person` identity layer (hub-architecture §5); never load-bearing.

## 6. Social interactions

**The model**: a reaction is **your content** — authored in jaunder, owned by
you, retained in your archive, _published by the existing outbound half_ — and
then **delivered** in whatever way the target site supports (discovered per
§3.2). Reactions are never ephemeral UI actions fired at a remote server and
forgotten. There is no separate "reactions subsystem"; there is discovery +
outbound publishing + per-protocol delivery.

Priority: **Webmention first** (needs no actor identity — deliverable while the
AP track is in flight), AP second, comment feeds as read-only context, AT
documented as open.

- **Webmention** _(#925 — the first interaction slice)_: like/reply = a post
  with microformats (`u-like-of` / `u-in-reply-to`) targeting the article,
  published outbound, plus a webmention POST to the discovered endpoint;
  delivery outcome surfaced to the user.
- **ActivityPub** _(doc-only until #286/#287 land)_: like/reply as genuine AP
  activities from your actor to the discovered AP object — lands in the author's
  notifications like any fediverse interaction. Blocked on federated actor
  identity (#286) and pairs with AP inbound (#287) so replies flow back.
- **Comment-feed display** _(doc-only)_: ingest a discovered comment feed —
  recursively, as a source through the same machinery — and render the
  conversation read-only under the article.
- **AT/Bluesky** _(open)_: no article↔record discovery standard yet; watch
  embeds/WhiteWind practice.
- **Trackback/Pingback** — **declined**: effectively dead, spam-dominated;
  Webmention is the successor.

## 7. Subscription portability — OPML _(doc-only until real)_

Import: bulk source/follow creation through the same add pipeline (validation
included, §2.2's prompt-async check per feed). Export: from follows. Folder
structure round-trips once folders exist (§5).

## 8. Authenticated feeds _(doc-only until real)_

Private feeds (basic-auth, capability-token URLs — Patreon, paid newsletters):
credentials attach to the **fetch unit**, and fetch-unit identity becomes
(canonical URL, credential) — two users with different tokens are two fetch
units; same token, one. Secret storage, and redaction in fetch logs/health
surfaces, are part of the slice.

## 9. Decomposition & sequencing

Filed 2026-08-11, with native blocked-by links; #282's title and body were
amended to match (it ends at the archive, and its non-goals now point at the
issues that own them):

| Issue                                                                                                                              | Blocked by                                          |
| ---------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------- |
| #282 RSS/Atom ingestion v1 _(ends at the archive, proven by server fns + a minimal add/unfollow UI)_                               | —                                                   |
| #919 archive → Item derivation _(hub-side: Item shape, adapter home, actors entity, sanitization-at-derivation, re-run machinery)_ | #282; blocks #283's Item-upserts, summaries, search |
| #920 adaptive polling (§2.2)                                                                                                       | #282                                                |
| #921 WebSub subscriber (§2.2)                                                                                                      | #282                                                |
| #922 feed auto-discovery (§3.1)                                                                                                    | #282                                                |
| #923 interaction-surface discovery (§3.2)                                                                                          | #282                                                |
| #924 lifecycle & health (§2.7)                                                                                                     | #282                                                |
| #925 webmention interactions (§6)                                                                                                  | #923                                                |

Milestones: #282 and #919 sit in **Inbound v1** — they are the producer→hub arc
that epic exists for. The other five are feed machinery **past** that arc, so
they have an epic of their own, **Web-feed reading (post-v1)**, created
2026-08-11 for exactly this section's contents.

Doc-only until real (file from the section when picked up): JSON Feed (§2.4),
full-text scraping / LLM summarization / image proxy / enclosures (§4), search
(§5), AP interactions / comment-feed display (§6), OPML (§7), authenticated
feeds (§8).

```
#282 ──┬── #919 derivation ─┬── #283 Item-upserts ── #285 reader
       │                    ├── LLM summaries
       │                    └── search
       ├── #920 adaptive polling      ├── JSON Feed
       ├── #921 WebSub                ├── full-text scraping
       ├── #922 auto-discovery        ├── OPML
       ├── #924 lifecycle & health    ├── authenticated feeds
       ├── #923 interaction disc. ─┬── #925 webmention
       │                           ├── AP interactions (+#286/#287)
       │                           └── comment-feed display
       └── image proxy                └── enclosures/media
```

## 10. Open questions

- **AT article interaction** — no discovery standard; revisit when one emerges
  (§6).
- **Churn mitigation policy** — semantic-hash fallback shape, per-source vs
  global (§2.5 → §2.7).
- **Multi-follower UX** — when a second real follower appears: per-follow
  interval _requests_ folding into the fetch unit's effective interval? (Schema
  is ready; policy undecided.)
- **Retention pressure** — the archive is forever by design; fragment-level
  storage keeps growth proportional to real change, and the fetch log is
  prunable, but a disk-pressure story (operator-visible archive size, per
  source) will eventually be wanted.

## 11. Vocabulary (feed-side)

- **AJR** — the inbound-syndication family (**A**tom / **J**SON Feed / **R**SS);
  the `ajr_*` identifier prefix. Unfamiliar but unambiguous; chosen over
  `feed_*` (outbound-reserved) precisely so direction is legible.
- **Fetch unit** — one (canonical URL [, credential]) polled/pushed stream; an
  `ajr_feeds` row.
- **Follow** — one user's relationship to a fetch unit; an `ajr_follows` row.
  Zero follows → the fetch unit is unscheduled.
- **Entry** — one known `(fetch unit, upstream-id)` identity; an `ajr_entries`
  row. (The syndication wire object — `atom:entry` / RSS `<item>` — consistent
  with `CONTEXT.md`'s AtomPub "Entry" reservation, which should widen to cover
  syndication wire entries as terms graduate.)
- **Entry version** — one observed immutable state of an entry; an
  `ajr_entry_versions` row holding a native fragment.
- **Channel version** — one observed state of a feed's own metadata; an
  `ajr_channel_versions` row.
- **Fragment** — the entry's native payload (XML element / JSON object),
  context-complete (namespaces, `xml:base`, `xml:lang`), character-exact.
- **Affordance** — a discovered place a reaction can land (webmention endpoint,
  AP object, comment feed).

Terms graduate to `CONTEXT.md` as they become code.
