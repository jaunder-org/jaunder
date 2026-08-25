# Issue #1182 — Pull AtomPub Posts into deterministic Org files

## Outcome

A server-only Post pulls into one exact, non-clobbering `<slug>.org` with native
source and sync metadata. Empty titles omit `#+TITLE`; destinations and failures
leave the root unchanged. Governing: D2 of the approved
[issue-75 contract](2026-08-25-issue-75-emacs-reconcile.md) and epic Unit D.

## Load-bearing decisions

- Correct `post_to_entry`: `PostRecord.title = None` emits the required Atom
  title element with empty text, never the slug. Any present title, including
  one equal to the slug, remains unchanged.
- D2 alone replaces `jaunder--atom->org`. Its pure mapping consumes a complete
  Member Entry, strong ETag, sync wall clock, and current zone; returns exact
  Org bytes without I/O; and builds on shipped Atom, Org, and date helpers.
- Extend the Atom harvester once with direct-child parsing for exactly one title
  (empty allowed), ordered categories, optional summary, exactly one content,
  draft marker, published value, exactly one edit URI, and exactly one `j:slug`.
  Existing media/publish fields and callers remain compatible.
- Required input has one title, decimal-terminal edit URI, non-empty slug,
  recognized content, and strong quoted non-weak ETag. Missing, duplicate,
  malformed, or ambiguous required data errors before local mutation.
- Map `text/org`→`org`, `text/markdown`→`markdown`, and
  `html`/`text/html`→`html`; preserve decoded text content byte-for-byte. For
  `xhtml`, require the Atom XHTML `div`, omit that wrapper, and concatenate its
  child nodes using canonical XML serialization with text XML-escaped. Never
  render, trim, download, or rewrite body/media URLs.
- Empty title omits title metadata. A multiline title emits one `#+TITLE` per
  XML-normalized LF-delimited logical line, including empty lines; extend the
  forward Org mapper to join repeated title values with LF, so single-line
  behavior is unchanged and pull→publish is reversible.
- Header order: repeated optional `#+TITLE`; optional `#+DATE`; optional
  `#+KEYWORDS` with categories joined comma-space in wire order; repeated
  optional `#+DESCRIPTION` preserving summary lines; `JAUNDER_STATUS`; optional
  `JAUNDER_DATE_TZ`, `JAUNDER_DATE_UTC`; `JAUNDER_FORMAT`, `JAUNDER_SLUG`,
  `JAUNDER_ID`, `JAUNDER_SYNCED`, `JAUNDER_SYNCED_AT`; blank line; body.
- Empty summary/categories are omitted. Metadata is verbatim except explicit
  title-line, format, status, and date projections. Every header line ends LF.
- `app:draft=yes` maps to `draft` with no dates. Otherwise require
  `atom:published`: preserve its offset text as `JAUNDER_DATE_UTC`, record the
  captured zone, render `#+DATE` there, and classify future as `scheduled`, else
  `published`.
- `JAUNDER_SYNCED` is the exact ETag. `JAUNDER_SYNCED_AT` is the pull's captured
  UTC RFC-3339 second. Capture clock/zone once so derived fields agree and tests
  can fix both.
- D2 exposes one D3-facing pull operation: accept configured root plus shipped
  D1 server-only Member; GET its edit URI inside `jaunder--with-blog`; return
  pulled/blocked with exact path. Preflight the D1 slug path before GET, require
  response ID and slug to match that Member, then use that same direct-child
  path.
- Install via a same-directory temporary file plus atomic no-replace operation.
  Always remove the temporary artifact. Existing/preflight or race-won target
  returns blocked without network/write damage; transport, HTTP, mapping, temp
  write, or install failures signal with root and destination bytes unchanged.
- Public Atom and no-clobber semantics require an outline after approval. No new
  ADR is needed; optional titles and client boundaries are already settled.

## Acceptance

- Rust tests cover empty versus real/equal-to-slug titles; shared-backend Member
  GET asserts serialized empty versus genuine title on SQLite and PostgreSQL.
- Pure ERT fixes clock/zone and asserts exact bytes for
  draft/scheduled/published, every format including canonical XHTML, repeated
  title/summary lines and round-trip, category order, dates, native body/media
  URL, and sync markers.
- Pure ERT invokes the D3-facing operation with a D1 Member and asserts GET URI,
  identity recheck, exact pulled/blocked result/path, malformed inputs, weak
  ETag, unsafe/stale slug, non-2xx/transport/mapping failure, and no local
  mutation.
- Fault-injected ERT covers temp-write/install failures and destination races;
  destination/root bytes remain unchanged and no temporary artifact survives.
- Live ERT pulls an untitled server-only Org Post through real transport to
  exact `<slug>.org`, verifies ordered bytes and no title, then proves an
  occupied destination remains byte-identical and blocked.

## Boundaries

- Only reversible title parsing touches publish; all D1/D3/D4 behavior is out.
