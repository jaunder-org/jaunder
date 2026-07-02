# Inbound Data Handling & the User-Centered Hub — Design Notes

> **Status: living / durable / high-level.** This is a long-lived design
> reference — a high-level companion to `ARCHITECTURE.md` / `DESIGN.md` /
> `ROADMAP.md`, **not** a short-lived per-issue spec headed for `archive/`. We
> expect to work on it for a while; edit it freely as the thinking matures. It
> is not yet a set of accepted decisions. The short-lived
> `spec.md`/`plan.md`→`archive/` pattern applies to the discrete implementation
> **slices spawned from** this doc — those become dated specs and lock decisions
> as ADRs; this doc itself persists as the reference and is updated, not
> archived. Tracked by issue #172.
>
> Started 2026-06-29; web-surface architecture resolved-direction added
> 2026-06-30 (§4); the web leg's core decision locked 2026-07-01 as **ADR-0040**
> (leptos-CSR, spike-confirmed — narrows ADR-0002), with the projector/PageSeed
> implementation in flight (#178/#179, ADR-0041 pending merge). Reconcile with:
> ADR-0002 (Frontend Framework), ADR-0005 (Unified Content Model), ADR-0010
> (Multi-Protocol Integration), ADR-0015/0023 (AtomPub serialization / wire
> extensions), ADR-0020 (Content Visibility and Subscription Model), and
> `CONTEXT.md` (glossary).

## 1. Product framing — what Jaunder actually is

> **"Jaunder is your social media hub for the open web."**

That slogan is the whole design in one line: a **social media hub** (a
user-centered place to both produce and consume) **for the open web** (federated
over open protocols — ActivityPub, RSS/Atom, AT — not a walled garden).
Everything below unpacks it.

Jaunder is **not** a publishing platform that the public logs into. It is a
**personal communication hub that centers the user**, symmetric across two
halves:

- **Broadcast (outbound):** what the user _produces_, published out to the
  world. **This is the only half built so far.**
- **Receive (inbound):** what the user _consumes_ — others' broadcasts pulled in
  via ActivityPub, RSS/Atom, AT/Bluesky, and "whatever comes next." **Not yet
  modeled at all.**

The load-bearing consequence: **the audience for the rich front-end is the
account owner, not their readers.** Readers consume a user's output through
_their own_ tools (their feed reader, their Mastodon, their own Jaunder). The
user is best understood as a **node / actor** — a centered identity that both
emits and ingests. A multi-user instance is several co-resident nodes sharing a
box (the Mastodon posture), **not** a magazine with columnists.

There is deliberately no blog / site / publication entity: **the user is the
publication** (the schema is flat — users own posts). The apparent sub-feed
granularities (per-author, audience-scoped) are just syndication feeds; identity
in the full model is per-entity (§5–§7).

## 2. Two surfaces, two mandates (the SEO/discoverability split)

The reframe in §1 does **not** abandon text/SEO — it relocates it:

- **Public surface (server-rendered, thin, text-first):** profile + permalink
  pages, RSS/Atom feeds, ActivityPub objects, AT records. This is the
  **discoverability on-ramp** — how humans and search engines _find_ a Jaunder
  user at all. It must be crawlable semantic HTML _on purpose_, because
  ActivityPub is poor for discovery (federation fragments the index) and
  RSS/Atom presupposes you already found a web page. Mostly server-side Rust;
  decoupled from the app framework.
- **Owner's cockpit (rich client app):** the authenticated produce + consume
  workspace. **SEO is irrelevant here.** Offline + live updates matter
  enormously here (it contains a feed reader).

This split is what makes the front-end framework choice tractable: the cockpit
can be canvas-rendered / app-like without an SEO penalty, because the crawlable
surface is a separate server concern.

## 3. Client & front-end architecture — how the option space collapsed

> **Historical preamble.** This section surveyed the client-stack option space
> (2026-06-29): pure-Rust UI everywhere, one non-Rust app framework everywhere
> (Flutter / Compose MP), or native UIs over a shared Rust core. It is kept as a
> compressed record of the search; the **conclusions live in §4**, and the
> local-first offline/live architecture graduated into the sync engine (§6).

Targets: web front-end, iOS/iPadOS, Android, and (low priority) cross-platform
desktop.

What the survey established, in brief:

- **No single stack gives "UI written in Rust" _and_ truly native mobile feel**
  — pure-Rust UI (Dioxus, Tauri+Leptos) is a webview on mobile; Flutter /
  Compose MP render their own canvas, not platform widgets; the best native feel
  is a native UI over a shared Rust core via FFI. §4 resolves this: **leptos-CSR
  on the web (ADR-0040), native-over-FFI on mobile, all over one shared core.**
- **One Rust core, shared by the server and every client** — in the strict §8
  sense of **client**: software running the `jaunder-client` runtime. The core
  owns the protocol adapters, the normalized item model, the sync engine, the
  local store; every rich UI is a thin reactive view over it. **Protocol
  clients** (feed readers, AtomPub editors like MarsEdit or today's thin elisp)
  and **peers** (other AP/AT servers) consume the server's interoperable
  endpoints instead and share none of it. This survived intact and became the
  trinity (§4) and the `jaunder-core` / `jaunder-client` split (§7 Q8).
- **Offline + live are one "local-first" system** — the UI observes a local
  store that a background sync engine keeps fresh; offline works because the UI
  always reads local, live works because new events land in the store and the
  reactive UI re-renders. Now part of the sync-engine design (§6).

## 4. The web-surface architecture — resolved direction

> **Status: the web-leg core is locked as ADR-0040** (2026-07-01: leptos-CSR,
> spike-confirmed, narrows ADR-0002). The forcing function — the #173
> concurrent-SSR reactive-disposal instability — plus the root-cause analysis,
> the spike evidence, and the rejected alternatives live in that ADR, not here.
> The projector/PageSeed slice is in flight (#178/#179, ADR-0041 pending merge).
> The mobile/bindings direction below remains direction-not-ADR until those
> slices are built.

### The shape: "SSR the data, not the components"

**The crawlable page-set and the app-rich page-set are disjoint** — the false
assumption behind isomorphic SSR was that they had to be one set. The public
reading surface (profile, permalink, feed) is anonymous, crawlable, read-only,
and must degrade; the cockpit is authenticated, owner-only, and may require JS.
So the server emits **one response per public URL** — semantic HTML + an
embedded public-data blob + a boot script, _identical bytes for every anonymous
visitor_, fully CDN-cacheable, **no reactive runtime in the request path** — and
the **client's capability** decides what it becomes: a finished semantic
document (bots / no-JS), the booted SPA (anonymous humans — deliberately the
modern experience, not a bot fallback), or the SPA with cockpit affordances
unlocked (the owner). Degradation is an **emergent property** of one cacheable
response, not a code path.

### The trinity, not the tower

UI rendering leaves the server **entirely**. The decomposition:

- **`jaunder-core`** — the normalized, protocol-agnostic item model; the
  inbound/outbound protocol adapters (§5; cf. ADR-0005, ADR-0010); **and a pure,
  non-reactive `render_content(data) → HTML/AST`.**
- **The server** — APIs + interoperable protocol endpoints
  (HTTP-discoverability, WebFinger, ActivityPub, AT, AtomPub) + storage. **No UI
  runtime.**
- **`jaunder-client`** — the rich, stateful, authenticated **cockpit runtime**
  (sync engine, local store, offline, live), shared by the rich UIs.
- **The public projector** — a thin, stateless server-side renderer that calls
  `core::render_content` to emit the public HTML document + data blob.

These are **siblings over `jaunder-core`, not a tower**: the projector has the
**strictest** non-functional requirements (latency, cacheability, crawlability,
minimal attack surface) and so must depend on the **least**, not sit atop the
fattest, most stateful abstraction. **`jaunder-client` drives all _interactive /
authenticated_ human interaction; the public surface is a thin sibling sharing
the core _model and render_, not the client _runtime_.**

### Flash-free ⇒ render coincidence ⇒ leptos-CSR

The chain, each link forcing the next: the public first paint must be
**flash-free** (LCP = real content; no reflow/swap when the SPA boots) ⇒
flash-free means **render coincidence** (the server's painted content equals the
SPA's render of the same data) ⇒ both must call the **same pure, non-reactive
render fn** ⇒ that fn lives in `jaunder-core` as pure Rust ⇒ **the web SPA is
Rust**, sharing the fn by construction ⇒ within Rust, **leptos-CSR** — it reuses
the existing Leptos components, and CSR-only removes the entire #173 class.
ADR-0040 records the spike evidence and the rejected alternatives (Flutter/TS
markup-parity maintenance, a JS SSR sidecar, Dioxus). **Trap door:** sharing a
_reactive_ component and rendering it to string on the server is isomorphic SSR
again — share the pure fn, never the component.

### Flash-free for the authenticated owner

The one hard boundary is the **authenticated** visitor on a cacheable,
anonymously-rendered page — the server didn't know they were logged in (that is
_why_ it's cacheable), so the SPA must change something post-boot. Two
disciplines keep it flash-free:

- **Enhance, don't replace** — the SPA **decorates the server-painted DOM** with
  affordances (edit, drafts, read-state); content stays put, only chrome fades
  in. Never discard and re-render.
- **Pre-paint auth detection** — a tiny blocking script reads a JS-readable
  local token and marks `<html>` _before_ first paint, so CSS pre-adjusts and
  the SPA boots already knowing. Cacheability is untouched (identical bytes for
  all; the adjustment is client-side). Never discover auth via an _async_ call —
  that guarantees paint-then-swap.

**The cockpit is a distinct route**, never an enhancement of a public page —
that keeps every public page a pure enhance-case. At the root `/`, an authed
owner **stays on the enhanced front page by default**; a user preference
(synced, with a locally-cached copy the pre-paint script can read synchronously)
may redirect to the cockpit instead. With no cached copy yet (first load on a
new device): stay — **never redirect on a guess**.

### Mobile, and the client's three bindings (resolved 2026-06-30)

**Mobile = native UI over the shared core via FFI.** The endgame is true-native
per platform — **SwiftUI for iOS** (first-class, and for many users the
_primary_ surface) **now**, **Jetpack Compose for Android** later — each calling
`jaunder-core` / `jaunder-client` through **uniffi**-generated bindings.
Rejected: **Flutter** (canvas, not native widgets; weaker OS integration on the
primary surface) and **Compose Multiplatform** (its iOS renderer is _also_
Skia-canvas, not Apple-native, and the youngest of the three on iOS — so it does
not deliver the native feel that is the entire reason to go native). The
macOS-on-Nix box makes the toolchain workable: hermetic Nix for the Rust
cross-compile; the irreducibly-Apple parts (Xcode, signing, provisioning,
submission) stay imperative, plus a macOS CI leg the Linux CI cannot cover. The
**PWA/webview** shortcut is demoted to a possible throwaway prototype — not a
shipping path for a primary surface.

**The client is one runtime with three bindings.** `jaunder-client` (the
stateful runtime: sync engine, local store, offline, live) is consumed as:

1. **a library via FFI** (uniffi) → the SwiftUI / Compose native apps;
2. **wasm** → the web SPA;
3. **a process with a local IPC protocol** (a long-running daemon over stdio
   JSON-RPC) → **Emacs as a _full_ client** (the eglot pattern), plus any CLI /
   script / TUI.

The three bindings are cheaper than they look because they want the _same_ API
shape: uniffi's "owned, serializable values + callback-interfaces for streams"
is essentially JSON-RPC's "everything serializes + streams are notifications."
So the **FFI-first** discipline — design the `jaunder-core` / `jaunder-client`
public API to be uniffi-expressible from day one — buys the daemon and wasm
bindings almost for free. Emacs is the sharpest proof that the runtime/UI split
is real: the most un-graphical UI imaginable (buffers) drives the _same_ runtime
as SwiftUI — "not as beautiful, but fully featureful." Keep today's
AtomPub-over-HTTP elisp client **thin** until the daemon exists, so re-basing
Emacs onto it stays cheap.

**Consequence — read-state conflict (§7 Q7) is now day-one, not an edge case.**
A single user will routinely run _two or three full client instances at once_
(phone, Emacs, and web), each with its own local store and read/unread state,
all syncing to the server as peers. "Last-write-wins" across three live clients
can produce visible churn (an item read on the phone re-surfacing as unread in
Emacs), so the read-state/draft conflict model needs real thought up front —
resolved by §6's per-field fold.

## 5. The inbound frontier (the active "grind toward spec")

The current schema is **entirely outbound**. Note the trap: the existing
`subscriptions` table is `author_user_id` + `subscriber_ref` — _people who
subscribe to me_ (outbound distribution), **not** _me following external
sources_. The entire inbound domain is unmodeled. New concepts needed:

- **Sources I follow** — an external source (RSS/Atom URL, ActivityPub actor, AT
  repo, …), with its protocol kind and fetch/auth metadata.
- **Received items** — normalized copies of fetched/received content.
- **My read-state** — read/unread, saved, etc., over those items.

Design axis tying both halves together: **protocol-plurality** (ActivityPub +
RSS + AT + future). This demands an **adapter layer** normalizing many wire
protocols into one **protocol-agnostic internal item model** — outbound adapters
render internal posts _to_ each protocol; inbound adapters ingest each protocol
_into_ a common item shape. The existing `posts`/feed model is the seed of that
internal model (cf. ADR-0005 Unified Content Model, ADR-0010 Multi-Protocol
Integration). The adapters + normalization belong in the shared Rust core (§3),
reused by both server and clients.

### Inbound storage — resolved direction (2026-06-30)

**Separate per-protocol raw storage; one derived normalized view.** Received
content is stored **per protocol** — `received_feeds` (RSS/Atom), `received_ap`,
`received_at` — each holding the **high-fidelity, raw-ish, native** payload,
never a lossy common reduction. The unified timeline runs over a **derived**
normalized `Item` (the `jaunder-core` type) plus a thin **feed-index**;
normalization is **non-destructive and re-runnable** (improve an adapter →
re-derive Items from the retained raw; no re-fetch, no loss). The protocol
multiplication is bounded _below_ the index: feed, render, clients, and UI are
protocol-count-invariant. (Same seam as §4: unify the model, separate the
mechanism.)

**The archive is immutable + append-only** — ARCHITECTURE's "immutable history
of edits; preserve even if deleted from source," made concrete:

- Each observed state of `(source, upstream-id)` is a **new immutable version**
  (version, observed-at, upstream-state, raw payload, content-hash). Current
  view = latest; edit history = the chain.
- **Content-hash change detection** keeps it from ballooning (AT gives a CID
  free; hash AP/RSS) — re-polls that don't change just bump last-seen.
- **Disappearance ≠ deletion.** An entry silently dropping from a feed →
  **retain** (you were given a copy and never told to delete it); recorded as
  state, not a row removal.

**Deletion is operator-sovereign.** Only an **explicit delete signal** — an AP
`Delete`, an AT record delete, or a manual legal/erasure demand — triggers the
delete-request workflow. (RSS/Atom carry no delete signal, so they only ever
disappear → retain, unless an operator acts manually.)

1. The item enters **`delete-requested`** with **verified request attribution**
   (an AP `Delete` must pass a signature check — otherwise a forged delete is a
   censorship vector against the archive) and becomes **invisible to everyone
   but the operator**; content is retained.
2. The operator is **surfaced the pending request** and chooses: **honor**
   (purge — the _sole_ sanctioned destructive operation, the one exception to
   immutability), **keep hidden** (retain, invisible), or **restore** (reject
   the request, make visible again). Nothing is ever auto-purged.

**Archive vs. replica.** The **server** holds this high-fidelity immutable
archive; a **client local store** is an **evictable replica** (a window, for
offline) — so revocation/eviction acts on the _device replica_, not the archive.

**Replica eviction is hygiene, not security (settled 2026-07-01).** Once bytes
are delivered, no server mechanism can claw them back (Q4's "arrival = leak" is
the enforcement boundary); eviction is _cooperation by well-behaved clients_,
and the design must not pretend otherwise. Mechanics: revocation stops sync, and
the revoked credential's next contact gets a **distinguishable "credential
revoked" response** (not a bare 401 — the client must tell revocation from
transient auth failure), on which a well-behaved client purges its replica: the
"remote sign-out" pattern, implemented once in `jaunder-client` and inherited by
every binding. **No content TTL** — auto-expiry would sabotage offline (the
store's whole purpose) and only punishes honest clients; a device that never
reconnects keeps its window forever, the accepted price of irrevocable delivery.
The purge **scopes to received content and synced state**: locally-authored,
never-synced material (offline drafts) is exported as **orphaned drafts**, not
destroyed — the user's own words carry no entitlement. Stolen-device protection
is the OS's job (device encryption, remote wipe), not the sync protocol's.
Pairing with §6's skew strategy: **must-understand redact events** serve clients
that remain authorized; **purge-on-contact** serves clients that no longer are.
Two mechanisms, no overlap.

Settled (2026-07-01): on a **multi-user instance** the **node-owner** is the
deletion arbiter — the received copy sits in _their_ archive, so routine
delete-requests are theirs to honor / keep hidden / restore; the admin has no
business adjudicating another user's archive. The **instance operator** holds an
**override for legal/erasure demands**, which attach to whoever operates the
box, not to the account holder. (§7 Q2)

### Sources, delivery, and actors — resolved direction (2026-06-30)

**Sources are per-protocol too** — `source_feeds` / `source_ap` / `source_at` —
but here the driver is differing _connection mechanics_, not fidelity. Each
protocol's "thing I follow" carries different metadata and a different follow
lifecycle:

- **RSS/Atom** — a URL + (poll interval _or_ WebSub hub state). No handshake.
- **ActivityPub** — a stateful, authenticated handshake: send `Follow`, get
  `Accept`/`Reject`, `Undo` to unfollow (pending/accepted/rejected state).
- **AT** — repo DID + subscription state.

**Delivery is a separate axis from content-format.** How bytes _arrive_ is
independent of what they _mean_, so the **delivery** layer and the
**parser/normalizer** are separate (the Atom parser does not care whether bytes
came by poll or by push). Four delivery modes:

- **poll** (any feed; the fallback) ·
- **WebSub hub-push** (RSS/Atom: a `rel=hub` callback subscription with a
  lease + secret; hubless feeds fall back to poll) ·
- **AP inbox-push** (other servers POST signed activities to your inbox) ·
- **AT firehose / subscription**.

**Verify every push.** AP uses HTTP signatures; WebSub uses an HMAC over the
secret — the same "don't trust an unauthenticated push" rule (which also guards
the delete-request workflow above), two mechanisms. WebSub needs only a callback
_endpoint_ (a webhook) — far lighter than AP's full actor identity.

**Inbound ActivityPub forces federated-actor identity (§7 Q2).** You must _be_
an addressable actor (inbox/outbox/keys/signatures) both to **receive** AP
pushes and to **send** the `Follow` handshake. So "inbound AP" really means
"become a federated actor, then everything else" — Q2 is a **prerequisite**, not
a later trigger.

**Actors are first-class, at protocol-native identity.** A source is _what you
follow_; an item's **author can differ** (AP boost: you follow @alice, receive
@bob _via_ her). So model external **actors** as their own entity, keyed by the
protocol's canonical id (AP actor URI, AT DID, feed-author). At that granularity
— no identity-resolution needed — you get within-protocol attribution,
per-protocol author views, and an **address book** (every identity that has
flowed through your hub) _for free_.

**Identity-resolution is a deferred, optional `person` layer.** "Are AP-@alice
and alice.bsky the same _human_?" is the hard problem; isolate it. An optional
`person` groups actor-rows, populated by explicit user action or heuristics,
**never load-bearing**. (AP hands you one cheap signal: `Move`/`alsoKnownAs` on
account migration.) Cheap canonical mechanism (actors) as the foundation; hard
unification (persons) optional above it.

**`actors` is shared across inbound _and_ outbound.** An external AP actor is
the same entity whether they follow you or you follow them, so `actors` is
referenced by inbound (sources/authors) _and_ outbound (today's
`subscriber_ref`s) — and the address book spans both directions. Unlike
posts-vs-received-items, actors have **no disjoint invariants** across
direction, so this is the rare cross-direction unification that is correct
rather than a nullable-column trap.

## 6. The sync engine, read-state & the Item contract

> **Status: resolved direction from the 2026-06-30 session.** Read-state (the
> third inbound pillar, §5) turned out to be inseparable from the general **sync
> engine** and its **wire contract** — one mechanism — and it is cross-cutting
> (it carries inbound content _and_ outbound actions), so it lives in its own
> section.

**The standing principle, explicit at last (2026-07-01): content is immutable;
annotations are mutable.** Content on _both_ halves is versioned-immutable —
supersede, never mutate. That has been the stance since day one (ADR-0009,
2026-05-14: "High-Fidelity Retention via immutable revisions"), made concrete
inbound by the archive's observed versions (§5) and mirrored outbound by the
authored edit history. Around each immutable entity hangs a small, **closed**
constellation of per-user mutable facts — **annotations**: read, saved, muted,
liked, tags — mutated only by timestamped intents. This section is the machinery
of that constellation: how annotations move, fold, and converge.

### It is all one action stream (the read-state framing, corrected)

At the **command layer** there is no "read-state vs. reactions" split: marking
read, saving, tagging, and liking are _identical_ — a client emits an intent,
the server applies it, sometimes passing it along. The real seam is the **effect
layer**: does the action leave the building? read/save/mute/tag are **purely
internal** (private state); like/boost/reply have that same internal effect
**plus an outbound federated tail** (an AP `Like` to the OP's server). So:

- **"Liked" is part of synced state** (every client must show the star, exactly
  like read/saved) **and** additionally spawns an outbound activity. One unified
  action model; some actions carry a federated tail.
- **Clients never federate directly.** They emit intents; the **server owns all
  outbound** (it is always-on, it dedups, and you do not want three clients each
  firing the same `Like`). The federated tail is **best-effort with retry**: a
  failed delivery does _not_ un-set your local "liked" — local intent is
  authoritative for your state; the failure surfaces as "couldn't deliver."

Read-state proper covers read/unread, saved, muted, optionally tags. **Drafts
are _not_ read-state** — a draft is outbound authoring-in-progress — but they
**do sync**, as whole-draft LWW snapshots through the same outbox, so no single
device stays the only holder of a draft for long (which is what makes replica
purge on revocation a non-event for drafts, §5). What stays parked is concurrent
_editing_ of one draft — a genuine text-merge problem. Sync ≠ merge.

### The sync engine — event log, cursors, live fan-out

No database replication. State moves as **events**, not snapshots:

- The server holds the **authoritative action log** (a sequence) **and the
  materialized current state** (reads never fold the log — see "The log is
  bounded" below); each client holds a **replica + a cursor** (the last event
  seq it has).
- **Live:** the server fans new events out over the **SSE channel** (below) to
  connected clients, which apply them and advance their cursor.
- **Offline catch-up:** on reconnect a client asks for "events since my cursor"
  and replays the gap — **incremental, never a full pull.**
- **Cold start only:** a brand-new client takes one **snapshot**, then switches
  to the cursor+log path forever. (The same snapshot path doubles as the
  **log-compaction floor** — see "The log is bounded" below.)
- **Optimistic + outbox:** a client applies an action locally at once (the star
  fills instantly) and queues the intent in an **outbox** (works offline); the
  server's ack/seq reconciles.

**The log is also the conflict resolver.** Every client folds the _same_ ordered
log, so two offline clients that both acted converge deterministically. Conflict
policy is **per field**, on _deliberate actions only_ — clients sync deltas, not
whole-state snapshots, so a stale default can never clobber a real write (the
naive-LWW churn source):

- **read/unread** → "read" is effectively **monotonic** (read on any client ⇒
  read), with an _explicit_ mark-unread as a rarer timestamped override.
- **saved / muted** → LWW on explicit toggles.
- **tags** → an **OR-set** (per-element add/remove), not whole-set LWW.

No CRDT _library_ — CRDT-_flavored_, hand-rollable, the same append-only/fold
shape as the archive (§5). And because read-state can record **which archived
version** you read, an upstream edit after you read it surfaces as "edited since
read" rather than silently re-marking unread — the versioned archive and
read-state compose.

### What orders the log, and how the fold runs

**Two orders coexist, deliberately.** The log's order is **server acceptance
order** — a monotonic `seq` assigned as intents are accepted — and it is
authoritative for **replication**: every replica applies events in seq order,
which is what makes convergence trivial. **Intent** on LWW annotations is
decided by **`acted_at`** — a timestamp the _originating device_ writes into
each action delta at the moment the user acted (act-time, not send-time: an
offline outbox may hold hours of actions). Server-receipt time is never used for
intent — it is arrival order wearing a clock face, and "LWW by arrival" is
exactly the stale-write-resurrection bug the per-field fold exists to kill (save
on the phone at 09:00 offline, unsave on the laptop at 10:00 online: when the
phone syncs at 11:00, `acted_at` lets 10:00 win; arrival order would crown the
stale 09:00).

**The fold is O(1) and never reads history.** Every delta is **addressed** —
`(user, item, field, value, acted_at)` — so a conflict can only exist _within
one annotation_, and each annotation stores **its winning stamp beside its
value**. Applying a delta is one row lookup plus one timestamp comparison; the
annotation is a _sufficient summary_ of everything that ever happened to it. A
losing delta still enters the log: every replica replays it, runs the same
comparison against its own copy of the annotation, and converges on the same
answer. The log is **replayed, never queried**.

**Clock skew is accepted, not corrected.** These are one user's devices under
NTP; the worst case — a toggle resolving the wrong way — is fixed by
re-toggling. No skew estimation, no temporal reconstruction. The server may
clamp absurdly-future stamps at acceptance; `seq` breaks exact ties.

### The log is bounded; the state is materialized

Two facts a reader will come looking for, stated plainly:

- **Reads never fold.** The server's annotation tables **are the materialized
  fold** of the entire log prefix — rendering a timeline queries them (and the
  Item / archive stores), never the log. Clients likewise read their local store
  and replay only the cursor gap. Nothing, anywhere, reconstructs from genesis.
- **The log is not kept forever.** Its only consumer is a client catching up, so
  it is a **bounded replication buffer** (time/size horizon; operator policy,
  not architecture). A cursor that falls behind the horizon gets a clean "too
  stale → take a fresh snapshot" — the cold-start path doubling as the
  compaction floor. A snapshot _is_ the fold of the discarded prefix, so
  convergence is untouched.

**Cold start and the cursor.** A snapshot is cut at a **seq boundary** — it _is_
the fold of the log prefix up to seq N, taken as one consistent read — and the
new client's cursor becomes N. The cursor is a **log position, never a
wall-clock time** (clocks exist only inside the LWW fold, as `acted_at`). The
**client holds its cursor authoritatively**, presenting it on every reconnect,
which keeps catch-up stateless for the server; the server keeps a per-device
**advisory mirror** (last-acked seq) to inform compaction and to surface "this
device is three months stale" — advisory meaning its loss breaks nothing and it
never pins the log: a dead device's ancient cursor simply earns the too-stale →
re-snapshot path when it returns. After the snapshot there is **one uniform
stream**: seq-ordered events, some carrying Item upserts, some annotation deltas
— never a separate "object sync" that could fall out of step with the events.
And snapshot-then-stream is **what makes compaction safe at all**: the prefix
below N can be discarded precisely because a newcomer receives state-at-N
instead of replaying it.

**Do not confuse the two append-only structures.** The **received archive (§5)
is forever** — content retention is a product value, guarded by
operator-sovereign deletion. The **action log is transport**, with no archival
duty; anything historically meaningful (e.g. which archived version you read)
lives in annotations. **Nothing treats the log as a system of record** — every
event is also absorbed into a table that is.

### Annotations are a sidecar, not the data model

An **annotation** is a named, per-user, timestamped mutable fact attached to an
immutable entity — the user's **marginalia**: the printed page stays immutable;
the pencil marks are yours, mutable, and belong to your copy. Each annotation
kind declares its fold policy (read: monotonic; saved/muted: LWW; tags:
per-element) — classically each is an "LWW register" in CRDT terms, but that
names a unit of conflict resolution, not a storage architecture. The annotations
live as a thin sidecar keyed by `(user, item)`, annotating entities that live in
ordinary tables. Nothing dissolves into attribute soup: `posts` stays `posts`;
received content lives in the archive and its derived `Item`s; no entity is ever
"reconstructed" by querying annotations. And the set of annotation kinds is a
**closed, schema-defined vocabulary** — static types, grown only by deliberate
additive contract change, never an open key-space. The flavor, concretely:

```
read_state(user, item):    read  + read_acted_at    -- monotonic + override
                           saved + saved_acted_at   -- LWW
                           muted + muted_acted_at   -- LWW
item_tags(user, item, tag): present + acted_at      -- OR-set element
draft(user, draft_id):     content + acted_at       -- whole-draft LWW
```

An annotation is physically a **column pair `(value, acted_at)`** — not an
EAV/KV store. Entities with no multi-client write conflict (posts: single
author; Items: read-only derived copies) carry no stamps and no annotations at
all.

### The client side: local store, live channel, push

The local-first shape from §3, concretized against the engine above:

- **Local store** — the UI always reads the client's local replica: real SQLite
  on mobile/desktop; IndexedDB or SQLite-in-wasm-on-OPFS plus a service worker
  (PWA) on web. Offline browsing and live updates are **one system**: offline
  works because the UI reads local; live works because new events land in the
  store and the reactive UI re-renders.
- **Live channel** — **SSE**, one-way server→client, carrying the event fan-out.
  Cheap, early win. (Web Push / APNs / FCM is a _separate_ channel for when the
  app is closed — a notification tap-target, not a sync path.)
- **No heavy sync/CRDT dependencies** — hosted/heavyweight sync engines
  (ElectricSQL, PowerSync, Zero) and CRDT libraries (Automerge, cr-sqlite) stay
  parked: Jaunder is read-heavy and effectively **single-writer per item** (one
  author per post; inbound items are read-only copies), and the per-field fold
  above covers the real multi-client conflicts. Revisit only if collaborative
  multi-device _authoring_ becomes a goal.

### The Item wire contract — typed core + open facets

The **Item** is the linchpin: simultaneously the wire payload, the
`render_content` input, the FFI/wasm/IPC currency, and the feed row. It is
**typed core + open facets**:

- a fixed **core** every item has (id, author-ref, timestamps, a **semantic
  `kind`** — Note/Article/Reply/Repost, _not_ protocol — and rendered content);
- an open **`facets: Vec<Facet>`** (attachments, content-warning, poll,
  `inReplyTo`, quote, language-variants …), where `Facet` is `#[non_exhaustive]`
  with an **`Unknown { kind, raw }`** arm: an unrecognized facet from a newer
  server deserializes into `Unknown`, renders a placeholder or is skipped, and
  is **never dropped or fatal**;
- **protocol is provenance, not structure** — a tag pointing at the per-protocol
  raw archive (§5) for "view source," _not_ a top-level discriminant. A Mastodon
  Note, an RSS entry, and an AT post are the same animal for display; organizing
  the Item by protocol would re-couple every client to the protocol zoo. (A type
  parameter `Item<A>` was considered and rejected: Rust can't recover `A` at
  runtime for render decisions without `Any`/downcast, `Vec<Item<A>>` forces one
  `A`, and protocol-as-discriminant is the wrong axis. The render-time decision
  is a closed-world match on the facet enum with an `Unknown` tail — which _is_
  the runtime discriminant the type parameter was groping toward.)

So the wire speaks a **small closed vocabulary of normalized event types** (Item
upserts, action deltas, actor/source changes) — **not** the open-ended
per-protocol storage shapes. The N raw shapes stay server-side; the server's
normalization is the translation; clients never see storage.

### Version-skew strategy (lean)

The fleet is **structurally desynchronized** — the web SPA updates
near-instantly, but the iOS app (App Store lag + slow updaters) and the Emacs
client (user-pinned) trail badly. Skew is the permanent condition, the price of
the multi-binding reach. The stance is **lean**, justified by a structural fact:
on a **self-hosted personal hub the server operator _is_ the client user**, so
skew is mostly _intra-personal coordination_ — upgrade your server, upgrade your
own app. The strategy:

- **(i) One additive-only contract.** Never remove/rename/repurpose; only add
  optional facets/fields/event-types. Old-client/new-server is then free
  (ignore-unknown). Breaking changes are the avoided enemy.
- **(ii) Version handshake + a minimum-supported floor.** A client announces its
  contract version on connect; below the floor it gets a clean **"please
  update,"** never a crash or silent corruption. This earns its keep
  specifically on **multi-user instances**, where an admin's server upgrade can
  surprise a _different_ user's old client (the one place server-op ≠
  client-user).
- **(iii) Unknown-tolerant at every level — and never stall the cursor.**
  Unknown facet/field/event → preserve-or-skip but **always advance the
  cursor**, so an old client cannot get wedged on a new event type.
- **(iv) …except mark safety-critical events.** A v5 `redact`/delete event
  reaching a v3 client that "skips and advances" would keep showing content that
  must be hidden — a privacy failure. So events carry a **must-understand**
  flag; an unknown _critical_ event trips the floor (→ update-required) instead
  of being skipped.
- **(v) Centralize adaptation in the server.** The **canonical log is the source
  of truth; the wire shape is derived per-client(-version)** at fan-out, so the
  server can down-level for a genuine break and the log stays unchained from any
  wire version. Clients stay dumb — no version logic replicated across Swift /
  Kotlin / elisp / wasm.

**Lean in practice:** old-client/new-server = free; new-client/old-server =
absent-field-means-"not-present" + **feature-detect to gate UI affordances** to
what the server supports ("content with the features you're getting"); too-old =
graceful update-gate; genuine breaks = server down-levels or the floor advances,
built **only when a real break demands it** (no pre-built per-version projection
engine). The failure mode is always "please update," never silent corruption.

## 7. Design questions — settled and genuinely open

> Q-numbers are **stable labels** (issue #172 and this doc's cross-references
> use them); the list is split by status, not renumbered.

### Settled — one line + pointer

- **Q1 — Inbound schema shape** → §5. Per-protocol raw archive
  (append-only/immutable) → derived `Item` + feed-index; per-protocol `source_*`
  with a separate delivery axis; first-class protocol-native actors; `person`
  deferred.
- **Q2 — Federated-actor identity** → §5. A prerequisite for any inbound AP
  (receiving pushes _and_ the `Follow`/`Accept`/`Undo` handshake require being
  first-class addressable: inbox/outbox/keys/signatures). **Per-node actors** —
  each user is their own AP actor (`acct:user@host`; follows, follower
  collections, and keys are per-relationship, matching §1's user-is-the-node
  framing) — plus one low-profile **instance service actor** (`Application`
  type, the fediverse convention) for infrastructure signing (authorized fetch
  and other no-user-context server-to-server calls). Deletion arbiter: the
  **node-owner**, with an **instance-operator override** for legal demands.
- **Q3 — Sync unit** → §6. The **event log** + per-client cursor, not
  `feed_url`; identity is per-entity; the `feed_*` family is
  **syndication-only** (see **feed**, §8).
- **Q4 — Per-viewer entitlement filtering** → §6, ADR-0020. Server-side at
  sync-fan-out; the client never receives private content to filter locally
  (arrival = leak). Public = the cacheable projector path; entitled = a
  per-viewer stream, not cacheable across viewers.
- **Q5 (stance) — Revocation** → §5. Delivery is irrevocable: revoking a viewer
  stops _future_ delivery; already-delivered copies persist; the only removal
  path is the operator-mediated delete-request.
- **Q5 (mechanics) — Client-replica eviction** → §5. Hygiene, not security:
  revocation stops sync; a distinguishable "credential revoked" response
  triggers a cooperative client purge scoped to received content + synced state
  (never-synced drafts are exported, not destroyed); no content TTL.
  Must-understand redact events serve still-authorized clients; purge-on-contact
  serves revoked ones.
- **Q6 — Front-end framework** → §4, **ADR-0040** (narrows ADR-0002). Web =
  leptos-CSR (spike #177: 30/30 panic-free); mobile = native-over-FFI (uniffi;
  SwiftUI now, Jetpack Compose later — direction-not-ADR until a mobile slice is
  built).
- **Q7 — Read-state conflict model** → §6. Event-sourced action deltas,
  per-field fold (read-monotonic, OR-set tags, LWW saved/muted); drafts parked
  on the outbound side.
- **Q9 — Anonymous-SPA placement** → §4. The anonymous public SPA is thin (core
  render + direct API reads); the `jaunder-client` runtime activates only on
  auth, ideally code-split.

### Genuinely open

- **Q8 — The `jaunder-core` / `jaunder-client` API surface.** §4 fixes the
  _shape_ (core = item model + protocol adapters + the pure render fn; client =
  the stateful runtime) and the bindings (FFI/uniffi, wasm, JSON-RPC daemon).
  Still to pin: the exact API surface — designed **FFI-first
  (uniffi-expressible)** from day one so the daemon and wasm bindings come
  almost free. **Deliberately not designed in this doc (2026-07-01):** it
  graduates to its own design exercise — a multi-candidate interface interview
  plus a cheap **uniffi spike** (uniffi-expressibility is the kind of constraint
  that looks fine on paper and bites in practice) — **triggered by the first
  real client slice**: the CSR cockpit consuming a sync engine, or the Emacs
  daemon becoming concrete. The constraints that design must honor are all
  recorded here: the shape and bindings (§4), Item as the currency and the
  closed event vocabulary (§6), and the skew strategy (§6).

## 8. Terms & vocabulary

> Working vocabulary this note coins. **Settled terms graduate to `CONTEXT.md`**
> (the durable project glossary) as their referents become real — the same
> "design → ADR/CONTEXT" flow used for decisions. Until then this is the
> reference for naming these things _consistently_. (Already promoted: **feed**
> — see `CONTEXT.md` § Syndication; **protocol client** and the no-blog-entity
> relationship, 2026-07-01.)

### Components

- **`jaunder-core`** — the shared Rust crate: the normalized item model, the
  protocol adapters, and the **pure render fn**. Linked by server, clients, and
  projector.
- **`jaunder-client`** — the stateful client _runtime_ over `core` (sync engine,
  local store, offline, live). Consumed via three **bindings**: FFI/uniffi
  (native), wasm (web), a JSON-RPC **daemon** (Emacs/CLI).
- **client** (unqualified) — software running the `jaunder-client` runtime, via
  any binding: the native apps, the cockpit SPA, Emacs over the daemon. _Avoid_:
  bare "client" for protocol clients or peers.
- **protocol client** — third-party software speaking an open protocol at the
  server's interoperable endpoints: feed readers (consumer-facing) and AtomPub
  editors (owner-facing: MarsEdit, today's thin elisp). Never sees the sync
  contract. Emacs is a protocol client today and becomes a **client** when the
  daemon exists.
- **peer** — another federated server (an AP or AT node); server-to-server and
  mutual. Never a client.
- **projector** — the thin, stateless, **non-reactive** server-side renderer
  that emits public HTML (semantic content + data blob + boot script) via
  `core`'s render fn. Not a UI runtime.
- **trinity** — the shape: `jaunder-core` with three _siblings_ over it — the
  **server**, **`jaunder-client`**, and the **projector**. Not a tower; none
  sits "on top of" another.
- **public surface** — the anonymous, crawlable, cacheable reading pages
  (profile, permalink, feed). The discoverability on-ramp.
- **cockpit** — the authenticated owner's app-like produce + consume workspace;
  runs the full `jaunder-client`, on its own route. SEO-irrelevant.

### Roles

- **node-owner** — the user whose node is in question: their archive, their
  sources, their read-state. The deletion arbiter for their received copies.
- **instance operator** — whoever runs the box, possibly hosting several nodes.
  Holds the legal-demand override on deletion; on a single-user instance the two
  roles are the same person.

### Content model

- **Item** — the normalized, protocol-agnostic content unit that crosses the
  wire and feeds the render fn: **typed core + open facets**.
- **facet** — an optional typed part of an Item (attachments, poll,
  content-warning, quote, …); `#[non_exhaustive]` with an
  `Unknown { kind, raw }` arm for forward-compat.
- **provenance** — an Item's origin tag (protocol, source, upstream id, archived
  version). Protocol is _provenance_, **not** structure.
- **feed** — **strictly a syndication feed: RSS, Atom, or JSON Feed — nothing
  else.** The `feed_*` family (`feed_url`, `feed_cache`, `feed_events`,
  `source_feeds`) is syndication-only; never a synonym for publication, source,
  timeline, or actor.
- **source** — an external thing you follow, per-protocol (`source_feeds` /
  `source_ap` / `source_at`). _What_ you follow — distinct from _who_ authored
  an item.
- **actor** — an external identity at **protocol-native** granularity (AP actor
  URI, AT DID, feed author); shared inbound/outbound; the unit of the address
  book.
- **person** — a _deferred, optional_ grouping of `actor`s judged the same human
  (identity resolution). Never load-bearing.
- **received archive** — the per-protocol raw stores (`received_feeds` /
  `received_ap` / `received_at`): high-fidelity, append-only, immutable. The
  Item is _derived_ from it.

### Sync

- **event log** — the server's authoritative ordered sequence of Item upserts +
  action deltas. The sync unit (not `feed_url`).
- **cursor** — a client's position in the event log.
- **fan-out** — pushing new events to connected clients (over SSE),
  **entitlement-filtered** per viewer.
- **annotation** — a named, per-user, timestamped mutable fact attached to an
  immutable entity (read, saved, muted, liked, a tag): the user's marginalia on
  content. Each kind declares its fold policy (monotonic / LWW / OR-set);
  physically a `(value, acted_at)` column pair (classically an "LWW register").
  The kind-vocabulary is closed and grows only additively.
- **read-state** — the bundle of private, non-federating **annotations** on an
  item (read/unread, saved, muted, tags); event-sourced, per-annotation fold.
- **outbox** — a client's queue of pending local actions (optimistic apply,
  offline-tolerant).
- **federated tail** — the outbound activity some actions spawn (a like → an AP
  `Like`); server-owned, best-effort. Clients never federate directly.
- **entitlement filtering** — server-side enforcement that a viewer's stream
  holds only what their audience membership permits; never client-side.
- **delivery mode** — _how_ content arrives (poll / WebSub / AP-inbox /
  AT-firehose), distinct from **content-format** (_what_ it is). One parser,
  many delivery modes.

### Rendering

- **render coincidence** — the projector's server-painted content equals the
  client SPA's render of the same data (both call `core`'s pure render fn) → no
  swap on boot.
- **flash-free** — no reflow/swap when the SPA takes over first paint (LCP =
  real content).
- **enhance, don't replace** — the authed client _decorates_ the server-painted
  DOM rather than re-rendering a new one (the anti-flash rule for the owner).
- **"SSR the data, not the components"** — the server emits data + non-reactive
  HTML, never a reactive UI runtime; clients render from the data. The escape
  from concurrent-SSR (#173).

## 9. How this note is tracked

GitHub issue #172 ("Grinding toward a spec for inbound data handling") anchors
this in the **Federation** project and points here; discrete open questions and
async discussion live on the issue, the evolving substance lives in this file.
**This file is durable** — it stays and gets updated. When a discrete **slice**
is ready to build, _that slice_ graduates to a dated `archive/` spec+plan pair
and its locked decisions become ADRs; this reference persists.
