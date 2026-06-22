# Emacs blogging front-end for Jaunder (AtomPub) — Epic Design

* Status: draft (awaiting review)
* Date: 2026-06-16
* Deciders: mdorman, Claude
* Tracking beads: `jaunder-4yjr` (full scheduled-post management UI),
  `jaunder-hww6` (broaden Emacs media upload beyond images)

## Goal

Build an Emacs front-end for authoring and managing a Jaunder blog over Jaunder's
AtomPub (RFC 5023) interface, and make the small server-side extensions that
front-end needs. Two author workflows:

1. **Authoring** — create an org-mode buffer from a template (saved as a local
   draft with a temporary name), edit until satisfied, then explicitly publish:
   push content + metadata (and referenced media) to the server, write
   server-assigned values back into the file, and rename it to a slug-derived
   filename.
2. **Blog management / reconcile** — enumerate posts on both sides; pull posts that
   exist on the server but not locally, and report (but do not auto-resolve)
   divergence, orphans, and local-only drafts. Reconcile never pushes; publishing
   is always explicit.

## Decomposition and build order

The work is four units. The two server units gate the two Emacs units, so the
build order is **A → B → (C, D)**. All four are specced here together because they
share vocabulary (the `j:` namespace and the org keyword/property mapping); each
section is self-contained enough to drive its own implementation plan.

| Unit | Title | Surface |
|------|-------|---------|
| A | Scheduled publishing | storage + web UI + (small) AtomPub |
| B | AtomPub format media types + Jaunder extension | common + server |
| C | Emacs authoring / publish workflow | elisp |
| D | Emacs blog management / reconcile | elisp |

## Cross-cutting decisions

* **Extension mechanism is foreign markup.** Atom (RFC 4287 §6) and AtomPub
  require processors to ignore unknown foreign-namespace markup, so a Jaunder
  namespace is added alongside the existing `app:` control namespace. Reuse
  standard Atom elements wherever they exist; add `j:` markup only where none
  does. Namespace URI: `https://jaunder.org/ns/atompub`.
* **Standard elements carry standard data.** Title → `atom:title`, tags →
  `atom:category`, publish time → `atom:published`, draft → `app:draft`, and
  per-entry **format** → the standard `atom:content` `type` (a media type). Only
  the server-assigned **slug**, which has no standard home, needs a `j:` element.
* **Metadata is mapped client-side.** The Emacs client reads org
  keywords/properties and builds AtomPub elements; the server keeps receiving
  clean AtomPub. No server-side org-header parsing is added. (`orgize` remains the
  server's Org→HTML renderer only.)
* **Record, do not infer.** Post state (draft/scheduled/published) and deletions
  are explicit actions/fields, never inferred from the absence of a value or file.
* **No secondary index file.** Remote identity lives in the org file itself.

---

## Unit A — Scheduled publishing

### Goal

Let an author set a future publish time so a post becomes publicly visible only
once that time arrives. Available from the main web UI and honored over AtomPub.
This is also a prerequisite for the Emacs `#+DATE:` scheduled-publish flow.

### Current state

* Visibility is **inconsistent** today. Most public reads gate on
  `published_at IS NOT NULL` only (e.g. `storage/src/posts.rs:514, 591, 613, 645,
  665, 869, 893, 945, 971`), while the feed "window" functions already gate on
  `published_at <= $1` (`storage/src/posts.rs:1094-1185`). A future-dated post
  would leak through the first set immediately.
* "Drafts" are defined as `published_at IS NULL` (`storage/src/posts.rs:697-721`).
* The slug is frozen the moment `published_at` becomes non-NULL
  (`storage/src/sqlite/posts.rs:59`, `storage/src/postgres/posts.rs:60`:
  `slug = CASE WHEN published_at IS NULL THEN $2 ELSE slug END`).
* A feed worker already exists: `tokio_cron_scheduler` ticking every 10s
  (`server/src/feed/worker.rs:152`) drains a `feed_events` queue and regenerates
  cached feeds via `regenerate_feed`, which already calls
  `list_published_in_window(.., now)` (`server/src/feed/regenerate.rs:63`).
* AtomPub create **ignores** the entry's `<published>` and stamps `Utc::now()`
  for non-drafts (`server/src/atompub/posts.rs:290-294`); update passes a publish
  boolean (`!fields.is_draft`) to `perform_post_update`
  (`server/src/atompub/posts.rs:378`).

### Design

Three post states, derived purely from `published_at`:

* **draft** — `published_at IS NULL`
* **scheduled** — `published_at IS NOT NULL AND published_at > now`
* **live** — `published_at IS NOT NULL AND published_at <= now`

Changes:

1. **Unify public visibility.** Every public read must gate on
   `published_at IS NOT NULL AND published_at <= now`, not `IS NOT NULL` alone.
   Audit `storage/src/posts.rs` and fold the `<= now` condition into the queries
   that lack it; the window functions already have it.
2. **Author surfaces.** The author still sees drafts and additionally a
   **scheduled** surface (the `IS NULL` drafts query will not show scheduled
   posts). Minimal for v1: scheduled posts appear in the author's post list with a
   "scheduled for <time>" marker. (Full management UI — a scheduled list,
   in-place reschedule, pull-back-to-draft — is deferred to `jaunder-4yjr`.)
3. **Go-live.** Pure query-time visibility makes on-demand HTML pages flip
   instantly. Cached feeds need a nudge: extend the existing scheduler tick to
   enqueue a feed-regeneration `feed_event` when a scheduled post crosses `now`
   (≈10s latency). Track activation by querying posts whose `published_at` falls
   in `(last_tick, now]`.
4. **Compose UI (minimal).** Add a datetime control to the publish form to set a
   future publish time; store UTC, display/accept the author's local time. A past
   value is allowed and means "immediately live with that timestamp" (backdating;
   same code path).
5. **Slug freeze stays at schedule time.** Keep today's
   `published_at IS NOT NULL` freeze rule unchanged — once scheduled, the slug is
   final and retrievable (this is what makes the Emacs filename stable; see
   unit C).
6. **AtomPub honors `<published>`.** Thread an explicit
   `Option<DateTime<Utc>>` publish time through creation and update instead of
   forcing `now()`. `perform_post_creation` already takes `published_at`;
   `perform_post_update` (`storage`) must change its publish flag from a bool to
   an explicit optional timestamp (or gain a timestamp parameter).

### Defaults

* Timezone: store UTC, UI in local time.
* Backdating: allowed (immediately live).
* Unschedule (clear `published_at` back to draft): rides the existing draft toggle
  if cheap; otherwise deferred to `jaunder-4yjr`.

### Edge cases / tests

* A scheduled post is absent from every public surface (post page, profile,
  feeds, tag pages) until `now >= published_at`; present immediately after.
* The author's own views show draft + scheduled + live distinctly.
* Backdated create is live immediately with the supplied timestamp.
* Feed reflects a go-live within one worker tick.
* Backend parity (SQLite + Postgres) for every changed query.

### Out of scope

Full scheduled-post management UI (`jaunder-4yjr`).

---

## Unit B — AtomPub format media types + Jaunder extension

### Goal

Allow a per-entry format choice (so a single blog can mix Org, Markdown, and HTML
posts) using the **standard `atom:content` `type` attribute** carrying a media
type, and expose the server-assigned slug via a small `j:` foreign-markup element
(it has no standard home). Both are ignored or degrade gracefully for non-aware
clients (e.g. MarsEdit).

### Current state

* `entry_to_post_fields` maps content type → format: `html`/`xhtml` → `Html`;
  `text`/absent → the user's account-level `default_post_format`
  (`server/src/atompub/mapping.rs:35-68`). Org vs Markdown is therefore **not**
  expressible per-entry today.
* `post_to_entry` emits `type="text"` for Org/Markdown and `type="html"` for HTML
  (`server/src/atompub/mapping.rs:79-128`); the slug is only recoverable from the
  public permalink in the `alternate` link, and only for published posts.
* Foreign markup is already supported generically: an `Extension` type plus an
  `entry.extensions` map (namespace prefix → local name → values), used for
  `app:control/app:draft`. Helpers `is_draft`/`set_draft`
  (`common/src/atompub/entry.rs:37-77`) read/write it; `write_entry`
  (`common/src/atompub/entry.rs:412-472`) special-cases draft emission and
  declares `xmlns:app` only when a draft is present.
* The service document is built from a `ServiceDocument` struct rendered by
  `render_service_document` (`server/src/atompub/service.rs:17-63`,
  `common::atompub::service`).

### Design

**Format via the standard content `type`.** The `atom:content` `type` attribute
is not limited to the `text`/`html`/`xhtml` tokens — Atom (RFC 4287 §4.1.3) allows
a full media type, and any `text/*` type is carried as inline escaped text exactly
like `type="text"`. So the format maps to the wire as:

| `PostFormat` | wire `type` | notes |
|--------------|-------------|-------|
| `Org` | `text/org` | de-facto type (no IANA registration); consumed only by org-aware clients |
| `Markdown` | `text/markdown` | IANA-registered (RFC 7763) |
| `Html` | `html` (token) | unchanged. **Not** `text/html`, which would mean *escaped* text, not HTML markup |

Bare `text`, `text/*` not listed above, or an absent type → the user's
account-level `default_post_format` (unchanged fallback).

**Encapsulate the mapping in a `format_wire` seam.** `mapping.rs` is already the
single coupling point (`entry_to_post_fields`, `post_to_entry`,
`server/src/atompub/mapping.rs`). Extract the format policy into two pure
functions so the wire representation is a one-place, fully-tested, trivially
reversible decision:

```rust
fn wire_to_format(content_type: Option<&str>, default: PostFormat) -> PostFormat;
fn format_to_wire(format: PostFormat) -> &'static str;
```

* `wire_to_format` is **lenient**: it recognizes `text/org`, `text/markdown`, the
  `html` token, and (defensively) `xhtml`/`text/html`, accepts an optional media
  type parameter (e.g. `text/markdown; variant=…`), and falls back to `default`
  for `text` or anything unrecognized. Reading never breaks regardless of how the
  outgoing scheme later changes or what a client sends back.
* `format_to_wire` is the **only** MarsEdit-risk surface. If `text/markdown`
  proves troublesome, reverting it to `text` is a one-line, unit-tested change.

No `j:format` element — the standard slot carries the format.

**`j:slug`** (the one remaining `j:` element) — read-only; declare
`xmlns:j="https://jaunder.org/ns/atompub"` on an entry only when emitted (mirroring
the conditional `xmlns:app`). Emitted on **every** entry including
drafts/scheduled (fixing today's gap where the slug is only recoverable from a
published permalink). If a client sends `j:slug`, the server ignores it. Add
`j_slug`/`set_j_slug` helpers in `common::atompub` alongside `is_draft`/`set_draft`
backed by the existing `extensions` map; extend `write_entry`
(`common/src/atompub/entry.rs`) to emit it and conditionally declare `xmlns:j`.

**Capability discovery.** Advertise support in the service document as versioned
foreign markup so a client can detect it once and degrade gracefully against a
vanilla/older Jaunder:

```xml
<service xmlns="http://www.w3.org/2007/app"
         xmlns:j="https://jaunder.org/ns/atompub">
  <j:extension version="1" features="format-media-type slug"/>
  <workspace>…</workspace>
</service>
```

Add the marker to `ServiceDocument` + `render_service_document`
(`server/src/atompub/service.rs`, `common::atompub::service`).

**Supersede ADR-0015.** Its `type="text"`-for-Org/Markdown decision is replaced by
the media-type mapping above. Write a follow-on ADR (or amend 0015) documenting
the mapping, the `format_wire` seam, and the lenient parser.

### Compatibility and the documented limitation

* Format partitions by audience: `html` (everyone, unchanged), `text/markdown`
  (MarsEdit + smart clients; registered), `text/org` (org-aware clients only —
  fully under our control).
* The MarsEdit acceptance checklist is re-run for **Markdown + HTML only**;
  `text/org` needs no third-party verification.
* **Documented limitation (mixed-client format downgrade):** if a non-format-aware
  client (e.g. MarsEdit connected to a blog that also has Emacs-authored Org
  posts) edits such a post and re-sends bare `type="text"`, the lenient parser
  maps it to the account `default_post_format` — a format downgrade. This only
  affects someone deliberately running two clients against one blog, and is the
  same class of issue as ADR-0015's existing title-only-edit open question.

### Edge cases / tests

* `type="text/org"` stores `Org`; `type="text/markdown"` (with or without a
  parameter) stores `Markdown`; `type="html"` stores `Html`.
* Bare `type="text"`, absent type, or an unrecognized `text/*` → account default
  (regression-guard the MarsEdit path).
* `format_to_wire`/`wire_to_format` round-trip every `PostFormat` and are unit
  tested in isolation (the swap point).
* Every outgoing entry (draft, scheduled, live, all formats) carries the correct
  content `type` and `j:slug`.
* Incoming `j:slug` is ignored.
* Service document validates and contains `j:extension` with
  `features="format-media-type slug"`.

---

## Unit C — Emacs authoring / publish workflow

### Goal

A dependency-light Emacs package to author posts in org-mode and publish them over
AtomPub, including media upload and server-value write-back.

### Org vocabulary

Standard data uses standard org keywords — `#+TITLE:` (→ `atom:title`), `#+DATE:`
(publish time), `#+KEYWORDS:` (→ `atom:category` tags), `#+DESCRIPTION:` (→
`atom:summary`) — and only genuinely jaunder-specific data uses
`#+PROPERTY: JAUNDER_KEY value` lines (the standard org file-level property
mechanism). All are read with `org-collect-keywords`, which returns one value per
occurrence; the client joins repeated `#+DESCRIPTION:` lines with newlines (ox-html
semantics) and splits `#+KEYWORDS:` lines on commas, then flattens. `#+KEYWORDS:`
is preferred over `#+CATEGORY:` because `#+CATEGORY:` is single-valued and carries
Org agenda-category behavior, whereas `#+KEYWORDS:` is behavior-free, multi-line,
and semantically "terms describing the content."

Title is **optional** — a titled post is a blog entry, an untitled post is a
microblog/note (the server already supports untitled posts; title is `Option`,
slug derives from the body).

```org
#+TITLE: My Post                          ; optional → atom:title
#+DATE: [2026-07-01 Wed 09:00]            ; org timestamp, local wall-clock; the human-editable publish time
#+KEYWORDS: rust, programming             ; comma-separated, multi-line → atom:category terms (tags)
#+DESCRIPTION: An excerpt (repeat the line for more)  ; optional, multi-line → atom:summary
#+PROPERTY: JAUNDER_STATUS draft          ; draft | scheduled | published (explicit intent)
#+PROPERTY: JAUNDER_DATE_TZ America/New_York          ; zone #+DATE is in; captured at publish, overridable
#+PROPERTY: JAUNDER_DATE_UTC 2026-07-01T13:00:00Z     ; canonical RFC 3339 UTC; what is sent and compared
#+PROPERTY: JAUNDER_FORMAT org            ; org (default) | markdown | html → atom:content type (media type)
#+PROPERTY: JAUNDER_SLUG my-post          ; server-written, read-only
#+PROPERTY: JAUNDER_ID 42                 ; server-written; presence ⇒ update vs create
#+PROPERTY: JAUNDER_SYNCED "<etag>"       ; server-written; last-synced ETag (see unit D)

Body in org…
```

`JAUNDER_STATUS` is the explicit intent and authoritative; `#+DATE:` is the
timestamp it refers to. No state is inferred from a missing value.

### Publish time and timezone

Org timestamps carry no timezone, so a bare `#+DATE:` is ambiguous once it leaves
the machine that wrote it. The **canonical** value is therefore `JAUNDER_DATE_UTC`
(RFC 3339 UTC) — it is what gets sent as `atom:published` and what reconcile
(unit D) compares. The `#+DATE:` org timestamp is a human-editing convenience, and
the zone used to interpret it is **recorded** (not re-inferred from the publishing
machine):

* `#+DATE:` — local wall-clock publish time, editable by the author.
* `JAUNDER_DATE_TZ` — the zone that wall-clock is in. Captured from the machine's
  local zone at first publish; the author may override it. Stored as an **IANA zone
  name** (e.g. `America/New_York`), not a fixed offset, so a future scheduled time
  converts correctly across a DST boundary. Implementation falls back to a numeric
  offset (with a caveat) only when the IANA name is unavailable.
* `JAUNDER_DATE_UTC` — canonical = `#+DATE:` interpreted in `JAUNDER_DATE_TZ`.

Rules:

* **On publish**: interpret `#+DATE:` in `JAUNDER_DATE_TZ` (capture the machine's
  current zone if `…_TZ` is unset), compute UTC, send it, and write all three
  fields back. If the author edited the wall-clock since the last publish
  (`#+DATE:`+`TZ`→UTC differs from the stored `…_UTC`), that is new intent and
  drives the new UTC; the zone is unchanged unless the author also edited `…_TZ`.
* **On pull**: the server's `atom:published` (always offset-qualified) is
  authoritative → write `JAUNDER_DATE_UTC` verbatim and render `#+DATE:` from it.
  If no `…_TZ` is recorded (fresh pull), capture the machine's current zone and
  record it.
* **"Publish now"** (status=published, no explicit time): omit `atom:published`,
  let the server stamp it, then write all three fields back from the response — so
  the canonical UTC always originates server-side.
* **Multi-machine guard**: if the machine's current zone differs from a recorded
  `JAUNDER_DATE_TZ` at publish, warn that the timestamp will be interpreted in the
  recorded zone, not the current one.

### Configuration

A `defcustom` alist maps a local directory → blog: base URL, username, default
format. Credentials are an **app password** looked up via Emacs `auth-source`
(`.authinfo.gpg`/`.netrc`) keyed by host + username — secrets never live in the
config. AtomPub auth is app-password-over-HTTP-Basic and requires HTTPS.

### HTTP

Built-in **`url.el`** (zero external dependencies). The media endpoint takes raw
bytes (not multipart), so a raw-body POST plus HTTP Basic is sufficient; the
package handles its own response parsing and error surfacing.

### Lifecycle

A post may live locally before it ever reaches the server, and publishing is
always an explicit per-buffer action (reconcile never pushes — see unit D).

1. **Create** — `jaunder-new-post` selects the target **blog** (look up the
   directory→blog alist; default to the blog whose directory contains the current
   `default-directory`, else prompt), creates a **timestamp-based temporary file**
   in that directory (e.g. `draft-20260616T143200.org` — the slug is server-
   assigned and unknown until publish, and no title exists yet), inserts the
   template, and saves. An interactive variant additionally prompts for
   title/tags/format/status.
2. **Local-only draft** — plain `C-x C-s` saves the file with **no server
   interaction**: `JAUNDER_STATUS: draft`, no `JAUNDER_ID`. The author can return
   and edit any number of times. (Distinct from a *server-side* draft, below.)
3. **Publish** — `jaunder-publish` (or `jaunder-save-draft` for an explicit
   server-side draft, pushed with `app:draft` for cross-machine/MarsEdit
   visibility) runs the publish flow below.

### Two kinds of draft

* **Local-only draft** — exists only on disk; never pushed. Plain save.
* **Server-side draft** — pushed with `app:draft`; has a `JAUNDER_ID`. Created by
  the explicit `jaunder-save-draft` command.

### Publish flow

1. Read keywords/properties → AtomPub elements (`atom:title`, `atom:category`,
   `atom:published` (from `#+DATE:`/`JAUNDER_DATE_UTC`), `atom:category` (from
   `#+KEYWORDS:`), `atom:summary` (from `#+DESCRIPTION:`), `app:draft` per
   `JAUNDER_STATUS`, and the `atom:content` `type` media type per
   `JAUNDER_FORMAT`).
2. Validate: a non-empty body is required; title/tags/summary optional;
   `scheduled` requires a future `#+DATE:`.
3. Media: scan the body for org links to **local image files**
   (png/jpg/gif/webp/svg), upload each raw to `/atompub/{user}/media`
   (idempotent by sha256), rewrite the link to the returned absolute URL.
   (Non-image attachments deferred to `jaunder-hww6`.)
4. Send: `JAUNDER_ID` present ⇒ `PUT` (with `If-Match` from the stored ETag),
   else `POST`. The org source is the entry content; the buffer (headers
   included) is stored verbatim server-side and rendered by `orgize`.
5. Write back `JAUNDER_ID`, `JAUNDER_SLUG`, `JAUNDER_SYNCED` (ETag), and the
   resolved publish time (`#+DATE:`, `JAUNDER_DATE_TZ`, `JAUNDER_DATE_UTC`; see
   "Publish time and timezone") into the buffer.
6. **Rename** the file and buffer from the temporary name to `<JAUNDER_SLUG>.org`
   in the blog's directory (handling a name collision; a no-op if already named).
   The slug is frozen at publish, so the name is stable thereafter.

### Edge cases / tests

* A new post saved with plain `C-x C-s` is a local-only draft: nothing is sent to
  the server and it keeps its `draft-<timestamp>.org` name.
* On first publish the temporary file is renamed to `<slug>.org`; a pre-existing
  `<slug>.org` collision is handled, not clobbered.
* Untitled post publishes; slug derives from the body server-side and round-trips
  back via `j:slug`.
* Re-publishing an existing post (has `JAUNDER_ID`) updates rather than
  duplicates; a stale ETag yields `412` and is surfaced.
* A `scheduled` status with a future timestamp produces a server-scheduled post.
* Media links are uploaded once (idempotent) and rewritten to absolute URLs;
  re-publish does not re-upload unchanged images.
* Against a vanilla Jaunder (no `j:extension` advertising `format-media-type`),
  the client warns that a per-entry `text/org`/`text/markdown` content type may
  not be honored (the server would fall back to the account default format).

### Out of scope

Non-image media (`jaunder-hww6`); Markdown/HTML authoring buffers (the
`JAUNDER_FORMAT` field is designed to allow them, but v1 targets Org).

---

## Unit D — Emacs blog management / reconcile

### Goal

Reconcile a blog directory against the server collection: pull posts that exist on
the server but not locally, and report (not auto-resolve) divergence. Per blog
(one directory ↔ one collection). Reconcile **never pushes** — local→server is
always the explicit `jaunder-publish` action (see unit C), so private local-only
drafts are never surfaced as remote posts.

### Design

Enumerate both sides — page the AtomPub collection feed; scan the directory's
`.org` files reading each `JAUNDER_ID`. `JAUNDER_ID` is the join key.

| Situation | v1 action |
|-----------|-----------|
| Local-only, never synced (no `JAUNDER_ID`) | **Report only** as a local draft not on the server — never auto-pushed |
| Server-only, no local file | **Pull**: reconstruct the `.org` (body + properties from the entry, content `type`, and `j:slug`), save as `<slug>.org` |
| Both, `JAUNDER_ID` matches | Compare `JAUNDER_SYNCED` vs the server's current ETag (and file state) → **report** divergence; no auto-resolve |
| Synced locally, server now 404s | **Report** as orphaned / remotely-deleted; no auto-action |

* **Pull + report only.** The single mutating action reconcile takes is pulling a
  server-only post into a new local file. Everything else is reported.
* **Sync marker.** Every publish/pull stores the server ETag as `JAUNDER_SYNCED`,
  giving the reconcile the data to detect (and report) divergence and to power the
  future sophisticated reconcile.
* **Preview and confirm by default.** Reconcile shows a plan ("N to pull, K
  diverged, J orphaned, M local drafts") and asks before applying the pulls,
  because it writes local files.
* **Deletion is explicit.** `jaunder-delete-post` (called from the post's buffer)
  issues the AtomPub `DELETE`, then deletes the local file/buffer on success. A
  bare file deletion is **not** a delete signal — it re-syncs (pulls) on the next
  reconcile by design. A post deleted elsewhere is reported as orphaned, never
  silently re-pushed or locally deleted.

### Edge cases / tests

* A new server post pulls down to a correctly named `<slug>.org` with faithful
  properties and body.
* A local-only draft (no `JAUNDER_ID`) is reported but **never pushed** by
  reconcile.
* A post edited on one side is reported diverged, not overwritten.
* `jaunder-delete-post` removes both sides; a hand-deleted file resurrects on
  reconcile.
* Reconcile preview accurately predicts the pulls it then performs.

### Out of scope (future "sophisticated reconcile")

Automatic divergence resolution (3-way merge / last-write-wins choices); deletion
propagation for hand-deleted files (would need a tombstone store, deliberately
avoided); bulk push of local drafts (a future explicit `jaunder-publish-directory`
action, not a reconcile behavior).

---

## Testing and conventions

Server/storage units (A, B) follow `CONTRIBUTING.md`: backend parity across SQLite
and Postgres, the coverage policy, and the verify ladder. Emacs units (C, D) are
new elisp; establish a small ERT suite and document how to run it. All public
visibility changes in unit A must be exercised against both backends.
