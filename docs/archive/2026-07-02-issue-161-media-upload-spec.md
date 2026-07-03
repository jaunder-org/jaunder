# Issue #161 — emacs: media upload + content-addressed link substitution (C3 of #74)

- **Issue:** jaunder-org/jaunder#161 (sub-issue of #74, Unit C —
  authoring/publish)
- **Depends on:** #159 (C1 transport, `jaunder--http-request`) ✓, #160 (C2
  `jaunder--org->atom`) ✓
- **Epic spec:**
  `docs/superpowers/specs/2026-06-29-issue-74-emacs-authoring-publish.md` (C3
  row)
- **Date:** 2026-07-02
- **Status:** design record (interview complete; captures resolved decisions,
  not implementation detail)

## Purpose

When an org post is published, any **local image** referenced by the body is
uploaded to the server and its link rewritten — **only in the sent body**, never
in the author's on-disk buffer — to the server's content-addressed URL.
`http(s)://` links are left alone; a missing/moved local file aborts the publish
with a clear error.

This is C3 of the Unit-C authoring pipeline: C2 (`jaunder--org->atom`) produces
a `jaunder-entry` whose `body` is the header-stripped org content still carrying
local links. C3 turns those local links into uploaded, absolute media URLs. C4
(publish flow) orders "media (C3) then entry send".

## Server contract (verified against `server/src/atompub/media.rs` + `common/src/media.rs`)

`POST /atompub/{username}/media`

- **Body:** raw image bytes.
- **Headers:** `Content-Type` = image MIME (stored and re-emitted verbatim by
  the server); `Slug` = filename (server sanitizes it); Basic auth — the path
  `jaunder--http-request` already uses.
- **Idempotency:** the server dedups by sha256 — `201 Created` for new content,
  `200 OK` when identical bytes already existed. Either way the response body is
  the media-link `<entry>`.
- **Response URL:** the `Location` header is the **edit** URL
  (`{base}/atompub/{user}/media/{sha}/{filename}`) — _not_ what goes in the
  post. The network-resolvable **binary** URL is the entry's
  `<content src="…">`:

  ```
  {base}/media/upload/{sha[0..2]}/{sha[2..4]}/{sha}/{filename}
  ```

  **The epic spec's `/media/{sha}/{filename}` shape is inexact** (it matches
  neither URL); this spec supersedes it.

## Resolved design decisions

| #                           | Decision                                                                                                                                                                                                                                                                                                                                                                                                                           |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **a. URL source**           | **Harvest `<content src>`** from the response entry XML. Server-authoritative, robust to future URL-layout changes, and rehearses the C4 harvest pattern. Never reconstruct the path client-side; never use `Location`.                                                                                                                                                                                                            |
| **b. Link detection**       | **`org-element` parse** (not regex). Qualify links whose `:type` is **`file`** with a **case-insensitive image extension** in `{png, jpg, jpeg, gif, webp, svg}`, **plus `attachment:` links** (org-attach). Bare fuzzy `[[x.png]]` and `http(s)` links are excluded by construction.                                                                                                                                              |
| **c. Path resolution base** | Resolve against the **live authoring buffer** — its real `default-directory` (relative `file:` links) and per-heading org-attach `DIR`/`ID` + org-id database (`attachment:` links). A detached body-string buffer cannot resolve org-attach per-heading dirs, so **the media step takes the live buffer as input.** Requires `org-attach`.                                                                                        |
| **d. Content-Type**         | Hardcoded **extension→MIME alist** over the qualifying set (`png→image/png`, `jpg`/`jpeg→image/jpeg`, `gif→image/gif`, `webp→image/webp`, `svg→image/svg+xml`). No mailcap.                                                                                                                                                                                                                                                        |
| **e. Dedup**                | Upload each distinct resolved file once (keyed by resolved absolute path; the server also dedups by sha256). Substitution is **position-based (collision-safe)**: two links with the _same_ raw target (e.g. `attachment:photo.png` under two headings) may resolve to _different_ files → different URLs, so each occurrence is rewritten independently, never by global string-replace.                                          |
| **f. Failure semantics**    | **Fail-fast, no partial publish.** (1) Pre-flight: resolve + existence-check every qualifying link first; if _any_ are unreadable, `error` with a single message listing **all** missing files and upload nothing. (2) Per upload: assert the response status is 200/201; any other status `error`s naming the file and status (`jaunder--http-request` returns 4xx/5xx in `:status`, not a signal, so this is an explicit check). |
| **g. Pipeline placement**   | A **separate step**, not folded into `jaunder--org->atom`. It resolves/uploads on the live buffer, then rewrites the harvested URLs into the **sent body produced by `jaunder--org->atom`**. C4 orders: `org->atom` → media step → send.                                                                                                                                                                                           |

### Structural moves

- **Pull `jaunder--atom-entry-fields` forward from C4 into C3.** Introduce a
  shared "AtomPub `<entry>` XML → alist" primitive now (built on
  `dom`/`libxml-parse-xml-region` — the first _parser_; current code only
  `dom-print`s). C3 consumes the **`content-src`** (and `content-type`) subset;
  C4 and Unit-D extend the same primitive (slug, `published`, ETag…). **The epic
  spec assigns this primitive to C4 in its scope-boundary prose (≈ lines 52–58)
  — annotate that paragraph to record the move.**
- **ADR (always-0000 workflow)** — write
  `docs/adr/0045-emacs-media-content-src.md` in the canonical format
  (`# ADR-0045: …` heading, `- Status: accepted`/`- Date:`/`- Issue:` tokens)
  and let **`cargo xtask adr renumber`** assign the number and regenerate the
  `docs/README.md` ADR table. Records (a) the client reads the binary URL from
  the response entry's `<content src>` (not `Location`, not reconstructed) and
  (b) the `jaunder--atom-entry-fields` primitive pulled forward into C3. **NB
  (changed since the interview): #196 shipped 2026-07-02 (PR#204) — the README
  table is now a generated projection and `adr-format`/ `adr-readme-parity`
  gates are enforced, so the interview's earlier "manual 0044 + hand-added
  README row" plan is obsolete; use the always-0000 flow (via the `jaunder-adr`
  skill).** The branch is now based on post-#204 main.

## Shape (helpers — final signatures land in the plan)

- `jaunder--atom-entry-fields (xml)` → alist of harvested entry fields
  (`content-src`, `content-type`, …); the shared primitive.
- `jaunder--media-content-type (filename)` → MIME string via the extension
  alist.
- **Detection/resolution** — walk the live buffer's **body region only** (the
  same accessible region C2 sends, i.e. after the `#+KEY:` header block) with
  `org-element-map` over `'link`, keeping qualifying links **in document order**
  with their resolved absolute paths and content types. `file:` →
  `expand-file-name` against `default-directory`; `attachment:` →
  `org-attach-expand`. Restricting to the body region is load-bearing: it makes
  this list and the sent body's links the **same set in the same order**, so the
  positional rewrite below is sound (a qualifying link inside a `#+KEY:` line
  would otherwise misalign the zip).
- **Upload** — for each distinct resolved path, read bytes, `POST` via
  `jaunder--http-request` with `Slug` + `Content-Type`, assert 200/201, then
  `jaunder--atom-entry-fields` the response to harvest `content-src`. No
  client-side filename sanitization: the harvested `content-src` already carries
  the server-sanitized filename.
- **Substitution** — rewrite each qualifying link **in the C2 sent body**
  positionally (zip the ordered resolved list against the body's links). Replace
  the **entire bracket inner-link** (the whole `:raw-link`,
  `file:`/`attachment:` prefix included) with the harvested absolute URL,
  preserving brackets and any `[…][description]` — output stays `[[URL]]` /
  `[[URL][desc]]`. A body with no qualifying links returns unchanged with zero
  uploads.

## Test plan

- **Pure ERT** (serverless):
  - Detection — `file:`/relative/absolute + `attachment:` qualify;
    case-insensitive extensions incl. `.jpeg`, `.PNG`; `http(s)`, bare fuzzy
    links, links in `src`/example blocks, and a qualifying link in a `#+KEY:`
    header line all excluded.
  - Content-type mapping over the extension set.
  - Sent-body substitution — single link; `[[t][desc]]` preservation;
    whole-`:raw-link` replacement (`file:`/`attachment:` prefix dropped); **two
    same-target links resolving to different files rewritten independently**
    (collision); **one file referenced by two links uploads once, both rewrite
    to the same URL** (dedup); empty/no-image body → unchanged, zero uploads;
    the source buffer left unmodified.
  - Absolute (`http(s)`) passthrough — no upload, no rewrite.
  - Missing-file pre-flight — one `error` listing all missing files, no upload
    attempted.
  - `jaunder--atom-entry-fields` — harvest `content-src`/`content-type` from a
    sample entry.
- **Live ERT** (`jaunder-test--with-live-server`, #137):
  - Real upload → `<content src>` harvested and substituted into the sent body.
  - Idempotent re-upload — unchanged image re-published creates no duplicate
    (server `200`), same URL harvested.
  - `attachment:` resolution via `org-attach-expand` under a per-heading `DIR` →
    correct upload.
  - Upload rejected (non-2xx) → `error` naming the file and status, publish
    aborted.

## Non-goals

- **Non-image media** (audio/video/docs) — #25.
- **Warn on unversioned local media** (soft publish-time git-tracking hygiene
  warning) — #206.
- **Download/localize media on pull** (offline preview of pulled posts) — #80.
- Rewriting the author's on-disk buffer — explicitly never done.
- Data-URI / inline-base64 images — no local file to upload.
