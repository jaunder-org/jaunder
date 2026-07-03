# Issue #162 — Emacs publish flow + commands + write-back [C4 of #74] — Design

- Status: proposed
- Deciders: mdorman, Claude
- Milestone: **Emacs blogging front-end** (#4). Fourth and final sub-issue of
  the Unit-C parent #74; integrates C1 (#159 transport), C2 (#160 org→atom), and
  C3 (#161 media).
- Builds on: `jaunder--http-request` (C1), `jaunder--org->atom`/`jaunder-entry`/
  `jaunder--atom-entry->xml` (C2), `jaunder--localize-media`/
  `jaunder--atom-entry-fields` (C3), and the live-server ERT harness (#137,
  `jaunder-test--with-live-server`).
- Governing design: the epic spec's **Unit C** section
  (`docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md`,
  "Lifecycle" / "Two kinds of draft" / "Publish flow") and the #74 unit spec
  (`docs/superpowers/specs/2026-06-29-issue-74-emacs-authoring-publish.md`).
  This record captures the C4-specific decisions resolved at this cycle's
  interview — chiefly the **multi-blog configuration model** (a decision taken
  here, not in the epic spec) and the exact **server contract** the write-back
  consumes.

## Goal

Wire the three C1–C3 seams into the end-to-end authoring lifecycle: the
`jaunder-new-post` / `jaunder-publish` / `jaunder-save-draft` commands, publish
validation, safe-to-resume send ordering (media then entry), create-vs-update
keyed on `JAUNDER_ID`, `JAUNDER_ID`-first write-back of server-assigned values,
and the temp-file → `<slug>.org` rename. Publishing is always an explicit
per-buffer action.

## Server contract (verified against `server/src/atompub/posts.rs`)

The write-back and update path consume these exactly:

- **Create** — `POST /atompub/{user}/posts` with the entry XML → `201 CREATED`,
  headers `Location: {base}/atompub/{user}/posts/{post_id}` (**numeric**
  `post_id`, an `i64`) and `ETag`, body = the server's canonical entry XML
  (carries `<j:slug>` and `<published>` for a live post).
- **Update** — `PUT /atompub/{user}/posts/{post_id}` with the entry XML.
  `If-Match` is honored: a value other than `*` that does not equal the current
  `ETag` yields **`412` Precondition Failed**; absent `If-Match` skips the
  check. Success → `200 OK` + `ETag` + entry body.
- **`j:slug`** is emitted on every entry (draft or live) in namespace
  `xmlns:j="https://jaunder.org/ns/atompub"` as `<j:slug>…</j:slug>`.
- **`<published>`** appears only for a live (non-draft) post, offset-qualified.

So `JAUNDER_ID` is the numeric `post_id` = the last path segment of `Location`;
its presence in the buffer ⇒ **update** (PUT to `…/posts/{JAUNDER_ID}` with
`If-Match: {JAUNDER_SYNCED}`), its absence ⇒ **create** (POST to the
collection).

## Multi-blog configuration model [decision taken here]

The epic spec sketched a directory→blog config; this cycle resolves it
concretely and implements it now (rather than reusing the single-blog globals +
deferring).

- **`jaunder-blogs`** — a `defcustom` alist of `(DIRECTORY . PLIST)`, where
  `DIRECTORY` is a local absolute directory and `PLIST` carries `:base-url`,
  `:username`, and an optional `:format` (accepted for forward-compatibility but
  **not consumed in v1** — see Threading). Multiple blogs, one entry each.
- **Resolution** — `jaunder--resolve-blog (file-or-dir)` returns the active-blog
  plist by **longest-prefix** match of `DIRECTORY` against the file's expanded
  directory (so nested blog roots resolve to the most specific). Resolution
  order:
  1. a matching `jaunder-blogs` entry (longest prefix wins);
  2. else, if the single-blog globals `jaunder-base-url`/`jaunder-username` are
     set, synthesize a plist from them (backward-compatible single-blog path);
  3. else signal a clear error naming the unconfigured directory.
- **Threading** — the commands `let`-bind the existing special variables
  `jaunder-base-url` and `jaunder-username` from the resolved plist around the
  flow. **C1/C2/C3 code is unchanged** — the HTTP/auth/media helpers keep
  reading those specials; they simply see the active blog's values. This is the
  least-invasive way to add multi-blog without refactoring the transport layer.
  The blog's `:format` is **not** threaded in v1: the only content converter is
  org (`jaunder--org->atom` hardcodes `text/org`), so a `jaunder-default-format`
  special would be inert dead config — deferred until markdown/HTML converters
  exist (out of scope; #25 and later).
- **Backward compatibility** — a user who never sets `jaunder-blogs` and only
  sets the two globals keeps working (path 2). The globals remain `defcustom`s.

`jaunder-blogs` also tells `jaunder-new-post` **where** to write: the chosen
blog's `DIRECTORY`.

## Commands

### `jaunder-new-post` — minimal template + save (interactive prompts deferred)

1. Choose the target blog: default to the blog whose `DIRECTORY` contains
   `default-directory`; if none matches, prompt to pick from `jaunder-blogs`
   (`completing-read`).
2. Create a **timestamp-based temp file** `draft-<YYYYMMDDTHHMMSS>.org` in that
   blog's directory (the slug is server-assigned and unknown until publish).
3. Insert a fixed org template — `#+TITLE:` (empty), `#+DATE:` (now, as an org
   inactive/active timestamp), `#+KEYWORDS:`, `#+DESCRIPTION:`,
   `#+PROPERTY: JAUNDER_STATUS draft` — then position point in the body and
   `save-buffer`.

No `title/tags/status` prompting variant in this cycle — filed as a follow-on
(see "Separable concerns"). A plain `C-x C-s` on such a buffer is a **local-only
draft**: nothing is sent, the `draft-<timestamp>.org` name is kept.

### `jaunder-publish` — the publish flow

Runs against the current buffer, using the blog resolved from the buffer's file:

1. **Map + read bookkeeping.** Map the buffer to a `jaunder-entry` via
   `jaunder--org->atom`, and read the `JAUNDER_*` bookkeeping (`JAUNDER_ID`,
   `JAUNDER_SYNCED`, `JAUNDER_STATUS`, `JAUNDER_DATE_TZ`). On a first publish
   with `JAUNDER_DATE_TZ` unset, `#+DATE:` is interpreted in the machine's local
   zone — which equals the zone name captured in step 2b, so the two agree.
2. **Validate** (`jaunder--validate-publish`): the body (post-strip) is
   non-empty; a `scheduled` `JAUNDER_STATUS` requires a **future** `#+DATE:`
   (compared against now). Title/tags/summary optional. Errors abort **before**
   any network call **and before any buffer write** (step 2b), so a rejected
   publish leaves the on-disk file pristine. 2b. **Zone capture**
   (`jaunder--ensure-date-tz`, after validation passes): if `JAUNDER_DATE_TZ` is
   unset, record the machine's current IANA zone name into the `#+PROPERTY:`
   block, so `#+DATE:` is interpreted in a **recorded** zone rather than one
   silently re-inferred on a later machine (epic spec "Publish time").
   Idempotent — an existing value is preserved.
3. **Media** (C3 `jaunder--localize-media`): produce the sent body with local
   image links rewritten to server URLs. The on-disk buffer is never modified.
   Idempotent, so safe to re-run on retry.
4. **Send** (safe-to-resume ordering — all network mutation before any local
   destructive change): `JAUNDER_ID` present ⇒ `PUT …/posts/{JAUNDER_ID}` with
   `If-Match: {JAUNDER_SYNCED}` (when a synced ETag exists); else `POST` to the
   collection. Serialize the entry (with the media-substituted body) via
   `jaunder--atom-entry->xml`. A non-2xx status (incl. `412`) is surfaced as an
   error and leaves the on-disk file **pristine**.
5. **Write back, `JAUNDER_ID` first** (`jaunder--write-back`): on a confirmed
   2xx, persist to the buffer's `#+PROPERTY:` block in this order — `JAUNDER_ID`
   (numeric tail of `Location`; on create only — on update it is unchanged),
   then `JAUNDER_SLUG` (`<j:slug>` from the response body), `JAUNDER_SYNCED`
   (the `ETag` header, stored **verbatim** so the next publish's `If-Match` is
   byte-exact), `JAUNDER_SYNCED_AT` (now, RFC-3339 UTC), and the resolved
   publish time (below). Persisting the ID first turns a later failure (e.g.
   rename) into a self-healing PUT next time. The buffer is then saved.

   **Publish-time write-back.** `JAUNDER_DATE_UTC` is set to the resolved
   canonical UTC: the response `<published>` (converted offset→UTC) when the
   entry is **live** and the response carries one, else what the client sent.
   For the **"publish now"** path (status published, no `#+DATE:` — the client
   sends no `<published>` and the server stamps it), also **render `#+DATE:`**
   from that returned UTC interpreted in `JAUNDER_DATE_TZ`
   (`jaunder--utc->org-date`, the inverse of C2's `jaunder--org-date->utc`), so
   the buffer shows the actual publish time. When the author supplied an
   explicit `#+DATE:` (scheduled/backdated), it is left as written and
   `JAUNDER_DATE_UTC` records what was sent. A **draft** has no resolved publish
   time — nothing in this paragraph applies.

6. **Rename** (`jaunder--rename-to-slug`): rename the file **and** buffer from
   the temp name to `<JAUNDER_SLUG>.org` in the blog directory. A no-op when
   already so named; a pre-existing `<slug>.org` collision is handled (a numeric
   suffix is appended, never clobbered).

### `jaunder-save-draft`

`jaunder-publish` with the entry forced to `app:draft` regardless of buffer
`JAUNDER_STATUS` — i.e. a **server-side** draft (pushed, has a `JAUNDER_ID`,
carries `app:draft`). Forcing draft **also nils the entry's `published` slot**
before serialization: `jaunder--atom-entry->xml` emits `<published>` whenever
the slot is set, independent of the draft flag, so without clearing it a
force-draft of a dated buffer would emit a `<published>`-carrying draft —
harmless (the server ignores `<published>` on a draft) but contradicting the "no
`<published>` on a draft" invariant. Same send/write-back/rename path; no
publish-time write-back (a draft has no resolved publish time).

## Shared primitive extension

`jaunder--atom-entry-fields` (introduced in C3, currently returns
`content-src`/`content-type`) gains **`slug`** (from `<j:slug>`) and
**`published`** (from `<published>`), which the write-back consumes. Unit D's
`jaunder--atom->org` later extends the same primitive further. It stays pure and
ERT-tested. (Note: `libxml-parse-xml-region` namespace-prefix handling for
`j:slug` is pinned by a pure ERT test on real server-shaped XML.)

## Buffer write helpers

`JAUNDER_*` are `#+PROPERTY: KEY value` lines. A pure `jaunder--set-property`
sets-or-inserts a property line (idempotent replace of an existing `KEY`, else
insert into/after the header block), used by the write-back and the zone
capture. `#+DATE:` is a first-class keyword, set by a sibling
`jaunder--set-keyword`. Both operate on the header block only, never the body.
`jaunder--utc->org-date` renders an org `#+DATE:` timestamp from a canonical UTC
string interpreted in a zone (the inverse of C2's `jaunder--org-date->utc`),
used by the "publish now" write-back.

## Tests

Per the unit spec's test mapping:

- **Pure ERT** (`jaunder-test.el`, serverless): `jaunder--resolve-blog`
  (longest-prefix, globals fallback, unconfigured error);
  `jaunder--set-property` / `jaunder--set-keyword` (replace-existing,
  insert-new, body untouched); `jaunder--atom-entry-fields` slug/published
  harvest on server-shaped XML; `jaunder--validate-publish` (empty body
  rejected; scheduled-needs-future); `JAUNDER_ID` extraction from a `Location`
  URL; create-vs-update decision; `jaunder--utc->org-date` render + the
  unset-`JAUNDER_DATE_TZ` machine-zone capture; force-draft nils the `published`
  slot.
- **Live ERT** (`jaunder-publish-integration.el`, via
  `jaunder-test--with-live-server`): publish → post created with the right
  fields and `JAUNDER_ID`/`JAUNDER_SLUG`/`JAUNDER_SYNCED` written back;
  re-publish (has `JAUNDER_ID`) **updates**, not duplicates, and its `If-Match`
  matches the stored `JAUNDER_SYNCED` byte-for-byte; a stale `If-Match` → `412`
  surfaced and the file left pristine; an **untitled** post (slug derived
  server-side, round-tripped via `<j:slug>`) — including an all-symbol/emoji
  body — publishes; a `scheduled` future post; a pre-response failure leaves the
  on-disk file pristine; first-publish rename + `<slug>.org` collision handling.

No new gate wiring: pure tests run under the existing `ert` step; live tests
under the `e2e-elisp-integration` nixosTest (ADR-0035), which globs
`*-integration.el`.

## Separable concerns (filed as the plan's first task)

- **Interactive `jaunder-new-post` variant** — prompt for
  title/tags/format/status and pre-fill the template (Unit C UX enhancement).
- **Vanilla-Jaunder `format-media-type` warning** — warn when publishing to a
  server whose service document does not advertise the `format-media-type`
  feature (the per-entry `text/org` content type may be ignored, falling back to
  the account default). Epic-mandated (epic spec "Edge cases") but deferred: it
  needs a service-document fetch/cache the publish flow does not otherwise do.
- **Multi-machine timezone-mismatch warning** — warn at publish when the
  machine's current zone differs from a recorded `JAUNDER_DATE_TZ` (epic spec
  "Publish time", multi-machine guard). A nicety on top of the zone-capture
  already in this cycle's write-back.
- The epic spec's already-filed C/D follow-ons (#76 app-password
  self-provisioning, #80 pull-media, #81 WWW-Authenticate) are **not** in scope
  and remain as-is.

## Out of scope

- `jaunder--atom->org` full synthesis + pull/reconcile — Unit D (#75).
- Non-image media — #25. Markdown/HTML authoring buffers — future converters.
- Client self-provisioning of the app password — #76.
- The vanilla-Jaunder `format-media-type` warning and the multi-machine
  timezone-mismatch warning — both filed as follow-ons (see Separable concerns).

## Known v1 limitation

**Create-retry duplicate.** If a create `POST` commits server-side but its
response is lost, the client never records `JAUNDER_ID` and a retry creates a
duplicate (posts have no idempotency key; media dedups by sha256, posts do not).
Rare for a single-user client, visible and deletable; a server idempotency key
is follow-on **#79**.
