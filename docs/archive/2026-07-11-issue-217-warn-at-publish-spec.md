# Spec — emacs: three "warn at publish" authoring-hygiene warnings

**Issues:** #217, #206, #216 (milestone #4, _Emacs blogging front-end_).
**Combined cycle** — one worktree/branch/PR; **one commit per issue**, landed in
the order **#217 → #206 → #216**. The PR closes all three.

Derives from the epic spec's "Publish time and timezone" guard and its
capability / media-hygiene discussion
(`docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md`, esp.
L457–L492, L604–L610). Prerequisites #161 (media upload) and #162 (C4 publish
flow) are both landed.

## Shared design — the client warning idiom (established here)

The emacs client has **no** existing `display-warning`/`lwarn` call site and
**no** suppression `defcustom`. These three issues introduce the idiom; all
future publish-time warnings follow it.

- **Level/type:** `(display-warning 'jaunder MESSAGE :warning)` — warning type
  symbol `jaunder` (groups the entries under a single header in the `*Warnings*`
  buffer), level `:warning` (soft; never `:error`, never blocks).
- **Message convention:** every message is prefixed `jaunder: ` to match the
  existing `(error "jaunder: …")` convention across the client.
- **Never blocks.** A warning is emitted and the publish proceeds unchanged. The
  create/update request, buffer write-back, and return value are identical
  whether or not a warning fired.
- **Suppression:** each warning has its own boolean `defcustom` in
  `jaunder-config.el` (`:group 'jaunder`, `:type 'boolean`), **default `t`**
  (warning enabled). Setting it to `nil` suppresses that one warning only.
- **Best-effort:** a warning check that cannot run (no git, no repo, unreachable
  service doc, unset property) **skips silently** — it never errors and never
  blocks the publish.

### AC-shared

- **AC-S1:** A publish's request bytes, buffer write-back, and return value are
  identical whether a given warning fires or is suppressed — warnings are
  additive and side-effect-free on the publish path (assertable with a stubbed
  transport capturing the request body across the fire vs. suppressed cases).
- **AC-S2:** Each `defcustom` set to `nil` suppresses exactly its own warning
  and leaves the other two firing.
- **AC-S3:** All three messages are emitted via `display-warning` with type
  `jaunder`, level `:warning`, and a `jaunder: `-prefixed message.

---

## #217 — warn when machine zone differs from recorded `JAUNDER_DATE_TZ`

**Where:** in `jaunder-publish` (`elisp/jaunder-publish.el`), at the single
create/update choke point (after `jaunder--ensure-date-tz`, before the POST/PUT
~L173). The **pre-existing** recorded value is the `JAUNDER_DATE_TZ` buffer
property as read at the top of `jaunder-publish` (nil when unset), captured
**before** `jaunder--ensure-date-tz` may write it.

**Behavior:** let `recorded` be the pre-existing `JAUNDER_DATE_TZ` and `current`
be `(jaunder--current-zone-name)`. Emit the warning iff `recorded` is non-nil
**and** `(not (string= recorded current))` — **except** when both `recorded` and
`current` are **numeric offsets** (matching `^[+-][0-9]`, the
`format-time-string "%z"` fallback of `jaunder--current-zone-name` when no IANA
name resolves), in which case skip the warning. Two numeric offsets can differ
purely because of DST on the _same_ machine (`-0500` ↔ `-0400`), so comparing
them would false-positive; the meaningful guard is a recorded **IANA name** vs.
the current zone.

`defcustom`: `jaunder-warn-zone-mismatch` (default `t`).

Message (indicative):
`jaunder: recorded timezone %s differs from this machine's zone %s; #+DATE: will be interpreted in the recorded zone %s`
— naming `recorded` and `current`.

### AC-217

- **AC-217a:** Buffer with `JAUNDER_DATE_TZ: America/New_York`, machine zone
  `Europe/London` → exactly one `jaunder`-type warning naming both zones;
  publish proceeds.
- **AC-217b:** Buffer with `JAUNDER_DATE_TZ` **unset** → **no** warning (the
  zone is being captured this publish; there is nothing recorded to differ
  from), regardless of machine zone.
- **AC-217c:** Buffer with `JAUNDER_DATE_TZ` equal to the machine zone
  (string-equal IANA name) → **no** warning. Likewise, on an offset-only
  machine, a recorded **numeric offset** vs. a current numeric offset → **no**
  warning (even when the two offsets differ across DST), since both are
  offset-form.
- **AC-217d:** `jaunder-warn-zone-mismatch` = `nil` → **no** warning even in the
  AC-217a case.

---

## #206 — warn when referenced local media isn't git-tracked in the document's repo

**Where:** the `records` list is a local of `jaunder--localize-media`
(`elisp/jaunder-media.el`) and does **not** escape it (the function returns only
the rewritten body string), so the check lives **inside
`jaunder--localize-media`**, immediately after `jaunder--media-preflight` has
confirmed every `:path` is a readable file, iterating that same `records` list
(each plist carries a resolved absolute `:path`). Distinct from #161's fail-fast
on **missing** files: missing = error (abort); present-but-unversioned = warning
(proceed).

**Repo anchor:** "the document's git repo" = the repository containing the org
**buffer's file**. Determine it once (e.g.
`git -C <buffer-dir> rev-parse --show-toplevel`, or `vc-git-root`). If the
buffer is not visiting a file, is not inside a git work tree, or `git` is
unavailable → **skip the entire check** (best-effort).

**Per-file test:** a media file is _tracked_ iff
`git ls-files --error-unmatch -- <path>` (run with `default-directory`/`-C` =
the document repo) exits 0. Everything else — untracked-inside,
gitignored-inside, or outside the repo tree — is _not tracked_ and warns. Use
`call-process` (no shell). **One warning per distinct untracked file** — dedup
by resolved `:path`, so a file referenced twice in the body warns once.

`defcustom`: `jaunder-warn-untracked-media` (default `t`).

Message (indicative, per file):
`jaunder: referenced media %s is not tracked by git in this document's repository; a fresh clone will lack local preview`.

### AC-206

- **AC-206a:** Document in a git repo referencing two local images, one
  committed and one untracked → exactly **one** warning, naming the untracked
  file only.
- **AC-206b:** A referenced image that is inside the repo but **gitignored** →
  warns (treated as not tracked).
- **AC-206c:** A referenced image **outside** the document's repo tree → warns.
- **AC-206d:** All referenced media tracked → **no** warning.
- **AC-206e:** Document not in a git repo (or `git` absent) → **no** warning,
  and publish is unaffected (media still uploads per #161).
- **AC-206f:** `jaunder-warn-untracked-media` = `nil` → **no** warning even with
  untracked media.
- **AC-206g:** The same untracked image referenced twice in the body → exactly
  **one** warning (dedup by resolved `:path`).

---

## #216 — warn when the service doc omits the `format-media-type` feature

**Where:** in `jaunder-publish`, at the create/update choke point (~L173),
before the POST/PUT.

**Fetch + cache:** on first publish to a given base-url this session, `GET`
`(jaunder--build-url BASE "atompub" "service")` via `jaunder--http-request` and
parse the result. Cache the parsed capability in a session-scoped `defvar` alist
keyed by base-url — **at most one extra request per session per blog**.
`jaunder--http-request` returns 4xx/5xx in `:status` but **re-signals**
transport-level failures (DNS, connection-refused); wrap the fetch+parse in
`condition-case`/`ignore-errors` so **any** signal, non-2xx status, or
unparseable body is treated as "unknown": skip the warning, never abort the
publish, and do **not** cache a definitive answer (a later publish may retry).
Note the route is a global `GET /atompub/service` (no username segment), served
as `application/atomsvc+xml`.

**Detection (proper XML parse):** parse the service document with
`libxml-parse-xml-region` (as `jaunder--harvest-response-fields` already does),
which folds namespace prefixes — so match the extension element by its **local
name** `extension` (exactly as the client already matches `j:slug` by local name
`slug`), not a prefix-qualified `j:extension`. Read its `features` attribute
(`dom-attr … 'features`), split on whitespace, and test set-membership of
`format-media-type`. Absence of the element or the token → feature **missing**.
The element is emitted as an empty
`<j:extension version="1" features="format-media-type slug"/>` nested in
`app:workspace`.

**Cadence — once per session per blog:** the warning is emitted only when the
capability is first **computed as missing** for a base-url (the cache-miss
path). Subsequent publishes in the same session read the cache and stay silent.
(Global suppression via the `defcustom` still applies.)

`defcustom`: `jaunder-warn-missing-format-media-type` (default `t`).

Message (indicative):
`jaunder: server at %s does not advertise the format-media-type feature; it may store this post's org source verbatim instead of rendering it`.

### AC-216

- **AC-216a:** Service doc **without** a
  `j:extension features="… format-media-type …"` → exactly one warning on the
  first publish of the session to that blog.
- **AC-216b:** Second publish to the same blog in the same session → **no**
  repeat warning and **no** second service-doc fetch (cache hit).
- **AC-216c:** Service doc **with** `format-media-type` advertised → **no**
  warning.
- **AC-216d:** Service-doc fetch fails (non-2xx / network error) → **no**
  warning, publish proceeds, and the negative result is not cached (a later
  attempt may re-fetch).
- **AC-216e:** `format-media-type` present only as an incidental substring
  **outside** the `j:extension` `features` attribute → **no** warning
  (proper-parse requirement, not a raw substring match).
- **AC-216f:** `jaunder-warn-missing-format-media-type` = `nil` → **no** warning
  and **no** service-doc fetch is performed for warning purposes.

---

## Non-goals

- No new **blocking** behavior; #161's missing-file abort is unchanged.
- No persistence of the service-doc cache across emacs sessions.
- No zone _normalization_ (e.g. treating two IANA aliases as equal) — a plain
  string compare of the recorded IANA name vs. `jaunder--current-zone-name` is
  the guard.
- Non-image media (#25) and Markdown/HTML authoring buffers are out of scope;
  #206 covers whatever media #161's resolver surfaces.

## Testing

- **Unit (ERT, `elisp/test/jaunder-test.el`):** stub `display-warning` (capture
  type/message/level), `jaunder--current-zone-name`, `call-process`/git, and
  `jaunder--http-request`; drive each AC with `with-temp-buffer` org fixtures.
  This is the primary coverage surface, incl. the negative/suppression cases.
- **Integration (`elisp/test/jaunder-publish-integration.el`):** the live
  harness server **advertises** `format-media-type`, so AC-216c (no warning) is
  a natural live assertion; AC-217b/c and AC-206e (no-warning paths) can be
  exercised live via `jaunder-pub-test--in-buffer`.

The emacs client is not yet in the coverage gate (#82), so no coverage-marker
accounting is required this cycle; tests are still written to cover every AC
above.
