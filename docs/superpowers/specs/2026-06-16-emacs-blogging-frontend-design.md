# Emacs blogging front-end for Jaunder (AtomPub) — Epic Design

- Status: reviewed (revised 2026-06-26; awaiting final spec review)
- Date: 2026-06-16 (revised 2026-06-26)
- Deciders: mdorman, Claude
- Milestone: **Emacs blogging front-end** (#4) — units #70–#75, follow-ons
  #76–#83; see "Issue decomposition & follow-ons". Planning tracked by #68.
- Deferred-tail issues (already filed): **#15** (full scheduled-post management
  UI), **#25** (broaden Emacs media upload beyond images)

> **2026-06-26 review note.** This document was revised after a gap review that
> verified the spec's assumptions against the live codebase. Key changes: auth
> uses the app-password facility that **already exists**; org metadata is
> normalized to a canonical body **server-side** (reversing the original
> "client-side only / no server org parsing" decision); the go-live nudge is
> made restart-durable; local divergence uses file mtime (best-effort); media
> links are **never** rewritten in the local buffer; slug generation is made
> Unicode-robust and never-fail; and the elisp test suite gets a real home in
> the verify gate. Each change is marked **[2026-06-26]** at its section. A
> consolidated issue decomposition and a list of follow-on issues are at the
> end.

## Goal

Build an Emacs front-end for authoring and managing a Jaunder blog over
Jaunder's AtomPub (RFC 5023) interface, and make the small server-side
extensions that front-end needs. Two author workflows:

1. **Authoring** — create an org-mode buffer from a template (saved as a local
   draft with a temporary name), edit until satisfied, then explicitly publish:
   push content + metadata (and referenced media) to the server, write
   server-assigned values back into the file, and rename it to a slug-derived
   filename.
2. **Blog management / reconcile** — enumerate posts on both sides; pull posts
   that exist on the server but not locally, and report (but do not
   auto-resolve) divergence, orphans, and local-only drafts. Reconcile never
   pushes; publishing is always explicit.

## Decomposition and build order

The work decomposes into the units below. The server units gate the Emacs units,
so the build order is **A → B → (elisp infra → C, D)**, with the slug-robustness
prerequisite landable independently. Each unit is self-contained enough to drive
its own implementation plan; **C and D are each larger than one code review and
fan out into several review-sized issues** (see "Issue decomposition").

| Unit  | Title                                                                                 | Surface                            |
| ----- | ------------------------------------------------------------------------------------- | ---------------------------------- |
| A     | Scheduled publishing                                                                  | storage + web UI + (small) AtomPub |
| B     | AtomPub format media types + Jaunder extension + **server-side org canonicalization** | common + server + storage          |
| Infra | **elisp package skeleton + ERT harness + flake/CI wiring**                            | elisp + flake + xtask              |
| C     | Emacs authoring / publish workflow                                                    | elisp                              |
| D     | Emacs blog management / reconcile                                                     | elisp                              |
| Slug  | **Unicode-robust, never-fail slug generation** (product-wide; surfaced by C)          | common + storage                   |

## Cross-cutting decisions

- **Extension mechanism is foreign markup.** Atom (RFC 4287 §6) and AtomPub
  require processors to ignore unknown foreign-namespace markup, so a Jaunder
  namespace is added alongside the existing `app:` control namespace. Reuse
  standard Atom elements wherever they exist; add `j:` markup only where none
  does. Namespace URI: `https://jaunder.org/ns/atompub`.
- **Standard elements carry standard data.** Title → `atom:title`, tags →
  `atom:category`, publish time → `atom:published`, draft → `app:draft`, and
  per-entry **format** → the standard `atom:content` `type` (a media type). Only
  the server-assigned **slug**, which has no standard home, needs a `j:`
  element.
- **Metadata lives in structured fields; the server normalizes org bodies to a
  canonical metadata-free form. [2026-06-26 — revised]** The original decision
  was "metadata is mapped client-side only; no server-side org-header parsing."
  That was already untrue: the server parses `#+TITLE:` today
  (`extract_org_title`, `common/src/render.rs:149-194`). The revised decision:
  **both** ingestion paths (web/mobile compose form and AtomPub entry) converge
  on one canonical stored body that is **free of the header lines the server
  stores elsewhere**. The server strips only the headers it recognizes and
  stores structurally (today `#+TITLE:`); **unrecognized header lines remain in
  the body, verbatim**, and round-trip faithfully. Clients **synthesize** their
  own header block on the way out from clean Atom; the server never emits
  client-specific (`JAUNDER_*`) markup. See Unit B "Server-side org
  canonicalization" and Unit C "Org vocabulary / atom:content".
- **Local representation differs from the served form; the client maps between
  them. [2026-06-26]** This applies to both the org header block (above) and to
  **media links** (Unit C): the on-disk org file always carries
  locally-previewable links and headers; the server always carries clean Atom +
  a canonical body with resolvable media URLs. Neither side is allowed to
  corrupt the other.
- **Record, do not infer.** Post state (draft/scheduled/published) and deletions
  are explicit actions/fields, never inferred from the absence of a value or
  file.
- **No secondary index file.** Remote identity (and the local sync marker) lives
  in the org file itself.
- **Auth uses the existing app-password facility. [2026-06-26]** App passwords
  already exist server-side — `create_app_password`
  (`web/src/sessions/mod.rs:56`), `list_sessions`/`revoke_session`, and a
  browser management page (`web/src/pages/sessions.rs`); they are labelled,
  non-expiring session tokens accepted over HTTP Basic. v1 consumes a
  **manually-minted, manually-pasted** app password (no new server work).
  Self-provisioning and a `WWW-Authenticate` challenge are follow-ons.

---

## Unit A — Scheduled publishing

### Goal

Let an author set a future publish time so a post becomes publicly visible only
once that time arrives. Available from the main web UI and honored over AtomPub.
This is also a prerequisite for the Emacs `#+DATE:` scheduled-publish flow.

### Current state

- Visibility is **inconsistent** today. Most public reads gate on
  `published_at IS NOT NULL` only (e.g.
  `storage/src/posts.rs:514, 591, 613, 645, 665, 869, 893, 945, 971`), while the
  feed "window" functions already gate on `published_at <= $1`
  (`storage/src/posts.rs:1094-1185`). A future-dated post would leak through the
  first set immediately.
- "Drafts" are defined as `published_at IS NULL`
  (`storage/src/posts.rs:697-721`).
- The slug is frozen the moment `published_at` becomes non-NULL
  (`storage/src/sqlite/posts.rs:59`, `storage/src/postgres/posts.rs:60`:
  `slug = CASE WHEN published_at IS NULL THEN $2 ELSE slug END`).
- A feed worker already exists: `tokio_cron_scheduler` ticking every 10s
  (`server/src/feed/worker.rs:152`) drains a `feed_events` queue and regenerates
  cached feeds via `regenerate_feed`, which already calls
  `list_published_in_window(.., now)` (`server/src/feed/regenerate.rs:63`).
- AtomPub create **ignores** the entry's `<published>` and stamps `Utc::now()`
  for non-drafts (`server/src/atompub/posts.rs:290-294`); update passes a
  publish boolean (`!fields.is_draft`) to `perform_post_update`
  (`server/src/atompub/posts.rs:378`).

### Design

Three post states, derived purely from `published_at`:

- **draft** — `published_at IS NULL`
- **scheduled** — `published_at IS NOT NULL AND published_at > now`
- **live** — `published_at IS NOT NULL AND published_at <= now`

Changes:

1. **Unify public visibility.** Every public read must gate on
   `published_at IS NOT NULL AND published_at <= now`, not `IS NOT NULL` alone.
   Audit `storage/src/posts.rs` and fold the `<= now` condition into the queries
   that lack it; the window functions already have it. **Ensure `published_at`
   is indexed** — the window/catch-up queries below depend on it.
2. **Author surfaces.** The author still sees drafts and additionally a
   **scheduled** surface (the `IS NULL` drafts query will not show scheduled
   posts). Minimal for v1: scheduled posts appear in the author's post list with
   a "scheduled for <time>" marker. (Full management UI — a scheduled list,
   in-place reschedule, pull-back-to-draft — is deferred to **#15**.)
3. **Go-live (restart-durable). [2026-06-26 — revised]** Pure query-time
   visibility makes on-demand HTML pages flip instantly; only **cached feeds**
   need a nudge. The mechanism:
   - **Steady state** — each scheduler tick enqueues a feed-regeneration event
     for posts whose `published_at` lands in the in-memory window
     `(last_tick, now]`, then advances `last_tick`. Cheap; bounded by an indexed
     range scan.
   - **Startup catch-up** — `last_tick` is in-memory, so a restart that
     straddles a scheduled go-live would otherwise drop that window (the post is
     live on its permalink but silently never enters the cached feeds). To heal
     this, on startup (`last_tick` unset) run **one feed-relative** pass:
     enqueue regeneration for any feed with a live post newer than the feed's
     own `generated_at`, then seed `last_tick = now` and switch to windowing.
     (Confirm cached feeds store a `generated_at` to compare against; add one if
     not.)
   - **Division of labor** — the tick handles _only_ future-dated → live
     transitions. **Immediate publishes, including backdated ones, enqueue their
     own feed regeneration on the write path**, so the tick never reasons about
     backdating.
4. **Compose UI (minimal).** Add a datetime control to the publish form to set a
   future publish time; store UTC, display/accept the author's local time. A
   past value is allowed and means "immediately live with that timestamp"
   (backdating; same code path).
5. **Slug freeze stays at schedule time.** Keep today's
   `published_at IS NOT NULL` freeze rule unchanged — once scheduled, the slug
   is final and retrievable (this is what makes the Emacs filename stable; see
   unit C).
6. **AtomPub honors `<published>`.** Thread an explicit `Option<DateTime<Utc>>`
   publish time through creation and update instead of forcing `now()`.
   `perform_post_creation` already takes `published_at`; `perform_post_update`
   (`storage`) must change its publish flag from a bool to an explicit optional
   timestamp (or gain a timestamp parameter).

### Defaults

- Timezone: store UTC, UI in local time.
- Backdating: allowed (immediately live).
- Unschedule (clear `published_at` back to draft): rides the existing draft
  toggle if cheap; otherwise deferred to **#15**.

### Edge cases / tests

- A scheduled post is absent from every public surface (post page, profile,
  feeds, tag pages) until `now >= published_at`; present immediately after.
- The author's own views show draft + scheduled + live distinctly.
- Backdated create is live immediately with the supplied timestamp.
- Feed reflects a go-live within one worker tick.
- **A go-live that occurs while the worker is down is reflected in cached feeds
  after the next startup catch-up** (simulate a restart straddling
  `published_at`).
- Backend parity (SQLite + Postgres) for every changed query.

### Out of scope

Full scheduled-post management UI (**#15**).

---

## Unit B — AtomPub format media types + Jaunder extension + server-side org canonicalization

### Goal

Allow a per-entry format choice (so a single blog can mix Org, Markdown, and
HTML posts) using the **standard `atom:content` `type` attribute** carrying a
media type; expose the server-assigned slug via a small `j:` foreign-markup
element; and normalize every ingested org body to one canonical, metadata-free
stored form.

### Current state

- `entry_to_post_fields` maps content type → format: `html`/`xhtml` → `Html`;
  `text`/absent → the user's account-level `default_post_format`
  (`server/src/atompub/mapping.rs:35-68`). Org vs Markdown is therefore **not**
  expressible per-entry today.
- `post_to_entry` emits `type="text"` for Org/Markdown and `type="html"` for
  HTML (`server/src/atompub/mapping.rs:79-128`); the slug is only recoverable
  from the public permalink in the `alternate` link, and only for published
  posts.
- Foreign markup is already supported generically: an `Extension` type plus an
  `entry.extensions` map (namespace prefix → local name → values), used for
  `app:control/app:draft`. Helpers `is_draft`/`set_draft`
  (`common/src/atompub/entry.rs:37-77`) read/write it; `write_entry`
  (`common/src/atompub/entry.rs:412-472`) special-cases draft emission and
  declares `xmlns:app` only when a draft is present.
- The service document is built from a `ServiceDocument` struct rendered by
  `render_service_document` (`server/src/atompub/service.rs:17-63`,
  `common::atompub::service`).
- **Org metadata today.** The web create/update path stores title/tags/summary
  as _separate_ fields; only the **title** is derived from the body, via a
  hand-rolled line scanner `extract_org_title` (`common/src/render.rs:149-194`)
  that returns `(title, remaining_body)` — but the caller **discards the
  stripped body** (`render.rs:96`), so the `#+TITLE:` line stays in storage and
  orgize renders it. No `#+KEYWORDS:`/`#+PROPERTY:` extraction exists.

### Design

**Format via the standard content `type`.** The `atom:content` `type` attribute
is not limited to the `text`/`html`/`xhtml` tokens — Atom (RFC 4287 §4.1.3)
allows a full media type, and any `text/*` type is carried as inline escaped
text exactly like `type="text"`. So the format maps to the wire as:

| `PostFormat` | wire `type`     | notes                                                                            |
| ------------ | --------------- | -------------------------------------------------------------------------------- |
| `Org`        | `text/org`      | de-facto type (no IANA registration); consumed only by org-aware clients         |
| `Markdown`   | `text/markdown` | IANA-registered (RFC 7763)                                                       |
| `Html`       | `html` (token)  | unchanged. **Not** `text/html`, which would mean _escaped_ text, not HTML markup |

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

- `wire_to_format` is **lenient**: it recognizes `text/org`, `text/markdown`,
  the `html` token, and (defensively) `xhtml`/`text/html`, accepts an optional
  media type parameter (e.g. `text/markdown; variant=…`), and falls back to
  `default` for `text` or anything unrecognized. Reading never breaks regardless
  of how the outgoing scheme later changes or what a client sends back.
- `format_to_wire` is the **only** MarsEdit-risk surface. If `text/markdown`
  proves troublesome, reverting it to `text` is a one-line, unit-tested change.

No `j:format` element — the standard slot carries the format.

**`j:slug`** (the one remaining `j:` element) — read-only; declare
`xmlns:j="https://jaunder.org/ns/atompub"` on an entry only when emitted
(mirroring the conditional `xmlns:app`). Emitted on **every** entry including
drafts/scheduled (fixing today's gap where the slug is only recoverable from a
published permalink). If a client sends `j:slug`, the server ignores it. Add
`j_slug`/`set_j_slug` helpers in `common::atompub` alongside
`is_draft`/`set_draft` backed by the existing `extensions` map; extend
`write_entry` (`common/src/atompub/entry.rs`) to emit it and conditionally
declare `xmlns:j`.

**Server-side org canonicalization. [2026-06-26 — new]** Both ingestion paths
converge on one canonical stored body that is free of the header lines the
server stores structurally. Concretely:

- Extend the existing `extract_org_title`/`derive_post_metadata` seam to **keep
  the stripped body it currently discards**, and store _that_ as the body.
  (Today only the title is kept; the stripped body is thrown away at
  `render.rs:96`.)
- **Only recognized headers are stripped** (today `#+TITLE:`; the value goes to
  the title column). **Unrecognized `#+FOO:` header lines remain in the body
  verbatim** and round-trip.
- Tags/summary/published/format/slug are already structured on both paths (web
  form fields; Atom elements from emacs) and were never in the body — no parsing
  of `#+KEYWORDS:`/`#+DESCRIPTION:` is added. (Teaching the server the _full_
  header block — so raw-org web authoring works — is follow-on **β**, out of
  scope here.)
- **Consequences to handle in this unit:**
  - Stripping the title line changes web rendering (the org-document title no
    longer renders inline; the page template's title column shows instead).
    Verify/remove any resulting double-title; add a regression test.
  - **Backfill** existing stored bodies that still contain a `#+TITLE:` line, or
    decide explicitly to leave legacy bodies untouched (strip-on-next-write
    only).
  - The canonical form must be **byte-deterministic** so the
    strip-and-resynthesize round-trip does not produce false divergence in Unit
    D.

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

**Supersede ADR-0015.** Its `type="text"`-for-Org/Markdown decision is replaced
by the media-type mapping above. Write a follow-on ADR (or amend 0015)
documenting the mapping, the `format_wire` seam, the lenient parser, and the
server-side org canonicalization.

### Compatibility and the documented limitation

- Format partitions by audience: `html` (everyone, unchanged), `text/markdown`
  (MarsEdit + smart clients; registered), `text/org` (org-aware clients only —
  fully under our control).
- The MarsEdit acceptance checklist is re-run for **Markdown + HTML only**;
  `text/org` needs no third-party verification.
- **Documented limitation (mixed-client format downgrade):** if a
  non-format-aware client (e.g. MarsEdit connected to a blog that also has
  Emacs-authored Org posts) edits such a post and re-sends bare `type="text"`,
  the lenient parser maps it to the account `default_post_format` — a format
  downgrade. This only affects someone deliberately running two clients against
  one blog, and is the same class of issue as ADR-0015's existing
  title-only-edit open question.

### Edge cases / tests

- `type="text/org"` stores `Org`; `type="text/markdown"` (with or without a
  parameter) stores `Markdown`; `type="html"` stores `Html`.
- Bare `type="text"`, absent type, or an unrecognized `text/*` → account default
  (regression-guard the MarsEdit path).
- `format_to_wire`/`wire_to_format` round-trip every `PostFormat` and are unit
  tested in isolation (the swap point).
- Every outgoing entry (draft, scheduled, live, all formats) carries the correct
  content `type` and `j:slug`.
- Incoming `j:slug` is ignored.
- Service document validates and contains `j:extension` with
  `features="format-media-type slug"`.
- **Canonicalization:** a body containing a recognized header (`#+TITLE:`)
  stores a body with that line removed and the title in its column; a body
  containing an **unrecognized** `#+FOO:` keeps it verbatim; the same post
  pulled and re-published is byte-identical (round-trip determinism).

---

## Infra unit — elisp package skeleton, ERT harness, and verify-gate wiring

### Goal [2026-06-26 — new]

Give the elisp units a real home in the verify ladder before C/D land.
CONTRIBUTING.md makes "green → you may move on" an invariant; today there is
**zero elisp** in the repo and **no emacs** in `flake.nix` or CI, so elisp would
otherwise be untested by policy. This unit is the foundation C and D build on,
and is **its own full issue**.

### Design

- **Package skeleton** — one `.el` package (the directory/file layout C and D
  extend), with the shared plumbing stubs (HTTP via `url.el`, auth, org↔atom
  mapping seams).
- **ERT harness** — an ERT suite plus a documented way to run it.
- **Toolchain** — add emacs to the flake (devshell + check input,
  cachix-pulled).
- **Gate wiring** — run the **pure** ERT unit tests as a host `StepSpec` in
  **both `check` and `validate`**, mirroring the existing
  `prettier --check end2end` precedent (`xtask/steps/static_checks.rs`). Revisit
  the tier only if it becomes a drag. **Live-server integration tests**
  (publish/pull round-trip against a running jaunder) are the e2e-VM-tier shape
  and are **a separate issue**, not v1-blocking.
- **Coverage** — elisp is **exempt from the Rust coverage gate** (cargo-llvm-cov
  instruments Rust only). State the elisp testing expectation directly instead:
  unit tests for every pure mapping/transform function. Bringing elisp under
  coverage is a tracked **p4** follow-on.

---

## Unit C — Emacs authoring / publish workflow

### Goal

A dependency-light Emacs package to author posts in org-mode and publish them
over AtomPub, including media upload and server-value write-back.

### Org vocabulary

Standard data uses standard org keywords — `#+TITLE:` (→ `atom:title`),
`#+DATE:` (publish time), `#+KEYWORDS:` (→ `atom:category` tags),
`#+DESCRIPTION:` (→ `atom:summary`) — and only genuinely jaunder-specific data
uses `#+PROPERTY: JAUNDER_KEY value` lines (the standard org file-level property
mechanism). All are read with `org-collect-keywords`, which returns one value
per occurrence; the client joins repeated `#+DESCRIPTION:` lines with newlines
(ox-html semantics) and splits `#+KEYWORDS:` lines on commas, then flattens.
`#+KEYWORDS:` is preferred over `#+CATEGORY:` because `#+CATEGORY:` is
single-valued and carries Org agenda-category behavior, whereas `#+KEYWORDS:` is
behavior-free, multi-line, and semantically "terms describing the content."

Title is **optional** — a titled post is a blog entry, an untitled post is a
microblog/note (the server already supports untitled posts; title is `Option`,
slug derives from the body). With the Unicode-robust never-fail slug change (see
the Slug unit), even an untitled note whose body is all-symbol/all-emoji
publishes (it gets a synthetic slug) rather than failing with `NoSlugFromPost`.

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
#+PROPERTY: JAUNDER_SYNCED_AT 2026-07-01T13:00:05Z    ; client-written wall-clock of last sync (local-divergence baseline; see unit D)

Body in org…
```

`JAUNDER_STATUS` is the explicit intent and authoritative; `#+DATE:` is the
timestamp it refers to. No state is inferred from a missing value.

The `JAUNDER_*` properties are **emacs-client-local bookkeeping** and are
excluded from what is sent as `atom:content` (see "atom:content" below) — the
server stores a clean, metadata-free canonical body and never sees `JAUNDER_*`
markup.

### Publish time and timezone

Org timestamps carry no timezone, so a bare `#+DATE:` is ambiguous once it
leaves the machine that wrote it. The **canonical** value is therefore
`JAUNDER_DATE_UTC` (RFC 3339 UTC) — it is what gets sent as `atom:published` and
what reconcile (unit D) compares. The `#+DATE:` org timestamp is a human-editing
convenience, and the zone used to interpret it is **recorded** (not re-inferred
from the publishing machine):

- `#+DATE:` — local wall-clock publish time, editable by the author.
- `JAUNDER_DATE_TZ` — the zone that wall-clock is in. Captured from the
  machine's local zone at first publish; the author may override it. Stored as
  an **IANA zone name** (e.g. `America/New_York`), not a fixed offset, so a
  future scheduled time converts correctly across a DST boundary. Implementation
  falls back to a numeric offset (with a caveat) only when the IANA name is
  unavailable.
- `JAUNDER_DATE_UTC` — canonical = `#+DATE:` interpreted in `JAUNDER_DATE_TZ`.

Rules:

- **On publish**: interpret `#+DATE:` in `JAUNDER_DATE_TZ` (capture the
  machine's current zone if `…_TZ` is unset), compute UTC, send it, and write
  all three fields back. If the author edited the wall-clock since the last
  publish (`#+DATE:`+`TZ`→UTC differs from the stored `…_UTC`), that is new
  intent and drives the new UTC; the zone is unchanged unless the author also
  edited `…_TZ`.
- **On pull**: the server's `atom:published` (always offset-qualified) is
  authoritative → write `JAUNDER_DATE_UTC` verbatim and render `#+DATE:` from
  it. If no `…_TZ` is recorded (fresh pull), capture the machine's current zone
  and record it.
- **"Publish now"** (status=published, no explicit time): omit `atom:published`,
  let the server stamp it, then write all three fields back from the response —
  so the canonical UTC always originates server-side.
- **Multi-machine guard**: if the machine's current zone differs from a recorded
  `JAUNDER_DATE_TZ` at publish, warn that the timestamp will be interpreted in
  the recorded zone, not the current one.

### atom:content — body only, headers synthesized client-side [2026-06-26 — revised]

`atom:content` is the **org body only**, not the whole buffer. The client maps
the metadata header block
(`#+TITLE:`/`#+KEYWORDS:`/`#+DESCRIPTION:`/`#+PROPERTY: JAUNDER_*`) to Atom
elements and **strips it from what it sends**; the server then applies its
canonical normalization (Unit B). On pull, the client **synthesizes** a fresh
header block from the Atom elements (plus locally-recaptured `JAUNDER_DATE_TZ`)
and prepends it to the returned body. (The original spec text — "the buffer,
headers included, is stored verbatim" — is replaced by this.) Round-trip
synthesis must be byte-deterministic (fixed header order/format) so reconcile
(Unit D) does not report false divergence.

### Configuration

A `defcustom` alist maps a local directory → blog: base URL, username, default
format. Credentials are an **app password** looked up via Emacs `auth-source`
(`.authinfo.gpg`/`.netrc`) keyed by host + username — secrets never live in the
config. The app password is **minted manually** in the web sessions page
(`create_app_password`) and pasted into `auth-source` for v1; self-provisioning
is a follow-on. AtomPub auth is app-password-over-HTTP-Basic and requires HTTPS.

### HTTP

Built-in **`url.el`** (zero external dependencies). The media endpoint takes raw
bytes (not multipart), so a raw-body POST plus HTTP Basic is sufficient; the
package handles its own response parsing and error surfacing.

### Lifecycle

A post may live locally before it ever reaches the server, and publishing is
always an explicit per-buffer action (reconcile never pushes — see unit D).

1. **Create** — `jaunder-new-post` selects the target **blog** (look up the
   directory→blog alist; default to the blog whose directory contains the
   current `default-directory`, else prompt), creates a **timestamp-based
   temporary file** in that directory (e.g. `draft-20260616T143200.org` — the
   slug is server- assigned and unknown until publish, and no title exists yet),
   inserts the template, and saves. An interactive variant additionally prompts
   for title/tags/format/status.
2. **Local-only draft** — plain `C-x C-s` saves the file with **no server
   interaction**: `JAUNDER_STATUS: draft`, no `JAUNDER_ID`. The author can
   return and edit any number of times. (Distinct from a _server-side_ draft,
   below.)
3. **Publish** — `jaunder-publish` (or `jaunder-save-draft` for an explicit
   server-side draft, pushed with `app:draft` for cross-machine/MarsEdit
   visibility) runs the publish flow below.

### Two kinds of draft

- **Local-only draft** — exists only on disk; never pushed. Plain save.
- **Server-side draft** — pushed with `app:draft`; has a `JAUNDER_ID`. Created
  by the explicit `jaunder-save-draft` command.

### Publish flow

1. Read keywords/properties → AtomPub elements (`atom:title`, `atom:category`,
   `atom:published` (from `#+DATE:`/`JAUNDER_DATE_UTC`), `atom:category` (from
   `#+KEYWORDS:`), `atom:summary` (from `#+DESCRIPTION:`), `app:draft` per
   `JAUNDER_STATUS`, and the `atom:content` `type` media type per
   `JAUNDER_FORMAT`). The `atom:content` body is the org body **with the
   metadata header block stripped** (see "atom:content").
2. Validate: a non-empty body is required; title/tags/summary optional;
   `scheduled` requires a future `#+DATE:`.
3. **Media (buffer is never rewritten). [2026-06-26 — revised]** Scan the body
   for org links to **local image files** (png/jpg/gif/webp/svg), upload each
   raw to `/atompub/{user}/media` (idempotent by sha256), and substitute the
   returned sha-derived URL (`/media/{sha}/{filename}`) **only in the body sent
   to the server** — the on-disk buffer keeps its local, previewable links. The
   mapping is content-addressed (sha256 of the file), so no stored map is needed
   and divergence detection normalizes local links to their sha-URL before
   comparing. Links already `http(s)://` are left untouched; a missing/moved
   local file is surfaced as an error. (Non-image attachments deferred to
   **#25**.)
4. **Send (ordered to be safe-to-resume). [2026-06-26 — revised]** `JAUNDER_ID`
   present ⇒ `PUT` (with `If-Match` from the stored ETag), else `POST`. Perform
   all network mutations (media, then the entry send) **before** any destructive
   local change; a pre-response failure (incl. `412` stale ETag) leaves the
   on-disk file pristine to retry. Media re-upload is safe because it is
   idempotent.
5. **Write back, `JAUNDER_ID` first.** On a confirmed response, persist
   `JAUNDER_ID` _before_ anything else that can fail, then `JAUNDER_SLUG`,
   `JAUNDER_SYNCED` (ETag), `JAUNDER_SYNCED_AT` (now), and the resolved publish
   time (`#+DATE:`, `JAUNDER_DATE_TZ`, `JAUNDER_DATE_UTC`). Persisting the ID
   first turns a later failure (e.g. rename) into a self-healing `PUT` next
   time.
6. **Rename** the file and buffer from the temporary name to
   `<JAUNDER_SLUG>.org` in the blog's directory (handling a name collision; a
   no-op if already named). The slug is frozen at publish, so the name is stable
   thereafter.

**Known v1 limitation — create-retry duplicate.** If a create `POST` commits on
the server but its response is lost (network drop), the client never records
`JAUNDER_ID` and a retry creates a duplicate (posts have no idempotency
mechanism; media dedups by sha256, posts do not). For a single-user client this
is rare and the duplicate is visible and deletable; a server-side idempotency
key is a follow-on (more important for mobile clients).

### Edge cases / tests

- A new post saved with plain `C-x C-s` is a local-only draft: nothing is sent
  to the server and it keeps its `draft-<timestamp>.org` name.
- On first publish the temporary file is renamed to `<slug>.org`; a pre-existing
  `<slug>.org` collision is handled, not clobbered.
- Untitled post publishes; slug derives from the body server-side and
  round-trips back via `j:slug`. An all-symbol/all-emoji untitled note still
  publishes (synthetic slug), not `NoSlugFromPost`.
- Re-publishing an existing post (has `JAUNDER_ID`) updates rather than
  duplicates; a stale ETag yields `412` and is surfaced.
- A `scheduled` status with a future timestamp produces a server-scheduled post.
- **Media links are uploaded once (idempotent) and substituted to absolute URLs
  in the sent body only; the on-disk buffer's local links are unchanged** so
  inline preview keeps working; re-publish does not re-upload unchanged images.
- A pre-response send failure leaves the on-disk file pristine; `JAUNDER_ID` is
  persisted before rename so a rename failure self-heals on the next publish.
- Against a vanilla Jaunder (no `j:extension` advertising `format-media-type`),
  the client warns that a per-entry `text/org`/`text/markdown` content type may
  not be honored (the server would fall back to the account default format).

### Out of scope

Non-image media (**#25**); Markdown/HTML authoring buffers (the `JAUNDER_FORMAT`
field is designed to allow them, but v1 targets Org); self-provisioning of app
passwords (follow-on).

---

## Unit D — Emacs blog management / reconcile

### Goal

Reconcile a blog directory against the server collection: pull posts that exist
on the server but not locally, and report (not auto-resolve) divergence. Per
blog (one directory ↔ one collection). Reconcile **never pushes** —
local→server is always the explicit `jaunder-publish` action (see unit C), so
private local-only drafts are never surfaced as remote posts.

### Design

Enumerate both sides — page the AtomPub collection feed; scan the directory's
`.org` files reading each `JAUNDER_ID`. `JAUNDER_ID` is the join key.

| Situation                                  | v1 action                                                                                                                             |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------- |
| Local-only, never synced (no `JAUNDER_ID`) | **Report only** as a local draft not on the server — never auto-pushed                                                                |
| Server-only, no local file                 | **Pull**: reconstruct the `.org` (synthesized header block + body from the entry, content `type`, and `j:slug`), save as `<slug>.org` |
| Both, `JAUNDER_ID` matches                 | Classify with the 2×2 below → **report** divergence; no auto-resolve                                                                  |
| Synced locally, server now 404s            | **Report** as orphaned / remotely-deleted; no auto-action                                                                             |

**Divergence detection (2×2). [2026-06-26 — revised]** Two independent
best-effort signals:

- **server-changed** = the server's current ETag ≠ stored `JAUNDER_SYNCED`.
- **local-changed** = the file's mtime is meaningfully newer than the recorded
  `JAUNDER_SYNCED_AT` wall-clock (with a small tolerance, since the final
  write-back lands a hair after the stamp). The baseline is a recorded
  wall-clock, **not** a stored mtime — storing the mtime in-file would re-bump
  it.

|                     | server ETag == `JAUNDER_SYNCED` | server ETag changed           |
| ------------------- | ------------------------------- | ----------------------------- |
| **local unchanged** | unchanged                       | **server-ahead** (offer pull) |
| **local changed**   | **local-ahead** (offer publish) | **conflict** (report only)    |

Best-effort caveats (tolerable because reconcile only _reports_): git checkout,
backup restore, or `touch` can produce a false "local-ahead"; an edit inside the
tolerance window can be missed; the server ETag is time-based today, so a
content-identical re-save reports as server-ahead (a content-based ETag is a
follow-on that removes this).

- **Pull keeps server URLs (v1). [2026-06-26]** A pulled server-only post is
  reconstructed with the server's media URLs in the body (they resolve over the
  network); media is **not** downloaded and links are **not** localized in v1,
  so a freshly-pulled post is not yet locally previewable. Download-and-localize
  on pull is a follow-on.
- **Pull + report only.** The single mutating action reconcile takes is pulling
  a server-only post into a new local file. Everything else is reported.
- **Sync marker.** Every publish/pull stores the server ETag as `JAUNDER_SYNCED`
  and the wall-clock as `JAUNDER_SYNCED_AT`, giving the reconcile the data to
  detect (and report) divergence and to power the future sophisticated
  reconcile.
- **Preview and confirm by default.** Reconcile shows a plan ("N to pull, K
  diverged, J orphaned, M local drafts") and asks before applying the pulls,
  because it writes local files.
- **Deletion is explicit.** `jaunder-delete-post` (called from the post's
  buffer) issues the AtomPub `DELETE`, then deletes the local file/buffer on
  success. A bare file deletion is **not** a delete signal — it re-syncs (pulls)
  on the next reconcile by design. A post deleted elsewhere is reported as
  orphaned, never silently re-pushed or locally deleted.

### Edge cases / tests

- A new server post pulls down to a correctly named `<slug>.org` with a
  faithfully synthesized header block and body.
- A local-only draft (no `JAUNDER_ID`) is reported but **never pushed** by
  reconcile.
- A post edited on one side is reported (server-ahead / local-ahead), not
  overwritten; edited on both is reported as a conflict.
- `jaunder-delete-post` removes both sides; a hand-deleted file resurrects on
  reconcile.
- Reconcile preview accurately predicts the pulls it then performs.

### Out of scope (future "sophisticated reconcile")

Automatic divergence resolution (3-way merge / last-write-wins choices);
deletion propagation for hand-deleted files (would need a tombstone store,
deliberately avoided); bulk push of local drafts (a future explicit
`jaunder-publish-directory` action, not a reconcile behavior);
download-and-localize media on pull (follow-on).

---

## Slug unit — Unicode-robust, never-fail slug generation [2026-06-26 — new]

### Goal

Slugs are the product-wide user-facing URLs. Today generation is ASCII-only and
can hard-fail; this unit makes it Unicode-faithful and guaranteed to succeed.
Surfaced by the Emacs untitled-note path but a **general improvement** (its own
near-term issue, independent of the epic).

### Current state

- `slugify_title` (`common/src/slug.rs:75-94`) keeps **only** `[a-z0-9]`;
  everything else becomes a hyphen. So `"café"` → `"caf"`, `"Héllo"` →
  `"h-llo"`, `"日本語"` → `None`. Returns `None` when nothing survives →
  `NoSlugFromPost` (`storage/src/post_service.rs:286`), surfaced as BadRequest.
- The `Slug` newtype `FromStr` (`common/src/slug.rs:25-40`) enforces
  `[a-z0-9][a-z0-9-]*`. This is the **single chokepoint**: both slug
  _generation_ and inbound _URL resolution_ funnel through it
  (`web/src/posts/mod.rs:282-283`).
- Collision handling exists and is reusable: per-author-per-day unique index +
  numeric-suffix retry (`candidate_slug`, `post_service.rs:219-227`).
- **No length cap** anywhere; DB column is `TEXT`.

### Design — (A) Unicode-preserving + never-fail

1. **Never hard-fail.** When derivation yields nothing usable, fall back to a
   synthetic slug (e.g. `post-<id>` or a short hash). Every post gets a slug.
2. **Preserve Unicode.** Relax the charset to Unicode letters/digits (Rust
   `char::is_alphanumeric()` — true for `日`/`é`/`я`/`٣`, false for symbols
   **and emoji**, which are Unicode _Symbols_, not letters). `日本語` →
   `日本語`, `café` → `café`. Symbol/emoji-only input keeps nothing → lands on
   the fallback (correct).
3. **Normalize in the chokepoint.** Centralize **NFC** normalization and
   Unicode-lowercasing in `Slug::from_str` so stored slugs and inbound-URL
   lookups compare consistently (the DB unique index / `WHERE slug = ?` compare
   bytes).
4. **Add a length cap** (chars or bytes — CJK inflates ~9 bytes/char when
   percent-encoded).
5. **Backward compatible:** existing `[a-z0-9-]` slugs remain valid (the new
   charset is a superset) → **no data migration**.

### Edge cases / tests

- `slugify_title("café")` → `café`; `"日本語"` → `日本語`; `"Москва"` →
  `москва`; `"🚀🎉"` and `"!!!"` → synthetic fallback (never `None`/error).
- On-wire form is percent-encoded UTF-8; **verify Leptos percent-decodes the
  slug path segment to UTF-8 before `Slug` parsing**
  (`web/src/pages/mod.rs:176-182`, `web/src/posts/mod.rs:274-291`).
- NFC: a slug stored in NFC is found by an NFD-encoded inbound request
  (normalize both).
- Length cap enforced; collision suffix still works on a Unicode base.
- Backend parity (SQLite + Postgres) for the unique index and lookups.

---

## Testing and conventions

Server/storage units (A, B, Slug) follow `CONTRIBUTING.md`: backend parity
across SQLite and Postgres, the coverage policy, and the verify ladder. All
public visibility changes in unit A must be exercised against both backends.
Emacs units (C, D) and the Infra unit are new elisp: a host-run ERT suite wired
into `check` and `validate` (see the Infra unit), with elisp interim-exempt from
the Rust coverage gate and unit tests required for every pure mapping/transform
function.

---

## Issue decomposition & follow-ons [2026-06-26 — new]

**Process (required at implementation kickoff).** The **first** implementation
step is to create the milestone and all issues below (with native dependency
links encoding the build order), **before** any code. Once created, **back-fill
their final identifiers into this spec and into the implementation plan**,
replacing the placeholder labels. Issue creation follows the `jaunder-issues`
conventions.

**Milestone:** _Emacs blogging front-end_ — groups every issue below; the
deferred-tail issues **#15** and **#25** are attached as the post-v1 tail. The
issue boundary is "**reviewable in a single code review**"; C and D are each too
large for one review and become **parent issues with review-sized sub-issues**.

**Build order:** A → B → (Slug ∥) → Infra → (C, D). Slug is independent and can
land in parallel; Infra gates C and D; A and B gate C and D.

| Issue            | Unit                                                                                                                                                                                                             | Depends on    |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------- |
| **#70**          | A — Scheduled publishing (storage + web + AtomPub; restart-durable go-live)                                                                                                                                      | —             |
| **#71**          | B — Format media types + `j:slug` + server-side org canonicalization                                                                                                                                             | —             |
| **#72**          | Slug — Unicode-robust, never-fail slug generation                                                                                                                                                                | —             |
| **#73**          | Infra — elisp package skeleton + ERT harness + flake/CI wiring                                                                                                                                                   | —             |
| **#74** (parent) | C — Emacs authoring / publish — sub-issues (created at C's cycle): ① HTTP/auth client core · ② org↔atom mapping + canonical-body strip · ③ publish flow + write-back/rename · ④ media upload + sha-link mapping | #70, #71, #73 |
| **#75** (parent) | D — Emacs reconcile — sub-issues (created at D's cycle): ① enumerate/join both sides · ② pull (reconstruct `.org`) · ③ divergence reporting (2×2) · ④ explicit delete                                            | #70, #71, #73 |

**Follow-on issues (file alongside, not in this milestone's v1 critical path):**

- **#76** — Self-provisioning of the app password (client logs in once →
  `create_app_password` → stores the token) — Unit C enhancement. _(blocked by
  #74)_
- **#77 (β)** — server parses the _full_ org header block so raw-org web/mobile
  authoring works (out of scope here; would duplicate the web form fields →
  needs precedence rules). _(blocked by #71)_
- **#78** — Content-based ETag (server; removes the time-based divergence
  false-positive; touches `If-Match`). _(independent)_
- **#79** — Create idempotency key (server; matters most for mobile clients).
  _(independent)_
- **#80** — Download-and-localize media on pull (Unit D enhancement; enables
  offline preview of pulled posts). _(blocked by #75)_
- **#81** — `WWW-Authenticate` challenge on 401 — **deferred until we have
  client code and can experiment**. _(blocked by #74)_
- **#82** (p4) — include the emacs client in coverage. _(blocked by #74, #75)_
- **#83** (p4) — include the e2e tests in coverage. _(independent)_
