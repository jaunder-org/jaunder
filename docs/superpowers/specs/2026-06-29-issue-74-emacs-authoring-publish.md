# Unit C — Emacs authoring / publish workflow (issue #74) — Design

- Status: proposed
- Deciders: mdorman, Claude
- Milestone: **Emacs blogging front-end** (#4). Parent unit; fans out into four
  review-sized sub-issues (C1–C4 below).
- Builds on: the elisp skeleton + seams (#73), the live-server ERT harness
  (#137, `jaunder-test--with-live-server`), and the server AtomPub surface
  (#70/#71). Governing ADRs: 0023 (wire extensions), 0024 (server-side org
  canonicalization), 0035 (live integration harness).
- Detailed design: **the epic spec's "Unit C" section**
  (`docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md`). This
  per-issue spec does **not** restate that design; it records the #74-specific
  **decomposition, scope boundary, and test mapping**.

## Goal

Implement a dependency-light Emacs package to author org-mode posts and publish
them over AtomPub — body-only `atom:content`, app-password Basic auth, media
upload with buffer links left intact, and safe-to-resume publish ordering with
`JAUNDER_ID`-first write-back — exactly as the epic spec's Unit C describes. The
three `jaunder.el` seams it fills are `jaunder--http-request` and
`jaunder--org->atom` (plus a new shared `jaunder--atom-entry-fields` primitive);
`jaunder--atom->org` is **not** touched here (see Scope boundary).

## Decomposition — four sub-issues, four small PRs

Landed as four review-sized sub-issues under parent #74, each its own PR sharing
this spec. Order: **C1 → C2 → C3 → C4** (C3 uses C1's transport; C4 integrates
all). Each sub-issue lands with its own committed tests — no scaffolding-only
PRs.

| Sub-issue               | Deliverable                                                                                                                                                                                                                                                                                                              | Tests it lands with                                                                                                                                                                                                                                 |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **C1 — HTTP transport** | `jaunder--http-request`: `plz` (curl) request/response (ADR-0038, not `url.el`), Basic-auth header wiring (`jaunder--auth-secret`), status/`ETag`/`Location` header access via a `plz-response`→`(:status :headers :body)` plist converter, error surfacing (4xx/5xx returned in `:status`, not signalled).              | **Pure ERT**: `plz-response`→plist conversion + case-insensitive header lookup. **Live ERT** (harness): authed `GET /atompub/{user}/posts` _through_ `jaunder--http-request`, assert parsed status + body; a 4xx status is returned, not signalled. |
| **C2 — org→atom**       | `jaunder--org->atom`: `org-collect-keywords` → `atom:title`/`atom:category`/`atom:summary` + `app:draft` + content media-type; strip the `JAUNDER_*`/metadata header block; body-only content; `#+DATE`+`JAUNDER_DATE_TZ` → `JAUNDER_DATE_UTC` (IANA zone, DST-correct, numeric-offset fallback).                        | **Pure ERT** (serverless): each mapping, header stripping, the timezone/UTC computation, untitled + all-symbol/emoji cases.                                                                                                                         |
| **C3 — media**          | Scan the body for org links to local images (png/jpg/gif/webp/svg), raw-upload each to `/atompub/{user}/media` (sha256-idempotent), substitute the returned `/media/{sha}/{filename}` URL **only in the sent body** (on-disk buffer untouched); `http(s)://` links left alone; missing local file surfaced as an error.  | **Pure ERT**: link detection + sent-body substitution + already-absolute passthrough. **Live ERT** (harness): real upload + idempotent re-upload (no re-upload of unchanged images).                                                                |
| **C4 — publish flow**   | `jaunder-new-post` / `jaunder-publish` / `jaunder-save-draft`; validate (non-empty body; `scheduled` ⇒ future `#+DATE`); media (C3) then entry send ordered safe-to-resume; `POST` (create) vs `PUT`+`If-Match` (update); `JAUNDER_ID`-first write-back of the harvested server values; rename temp file → `<slug>.org`. | **Live ERT** (harness): publish→post created with right fields; re-publish updates not duplicates; stale ETag → 412 surfaced; `scheduled` future post; pre-response failure leaves the on-disk file pristine; rename + collision handling.          |

## Scope boundary — `atom->org` stays in Unit D

The `jaunder--atom->org` seam (full **Atom entry → org buffer** synthesis:
header block + body, for pull/reconcile) is implemented **entirely in Unit D
(#75)** and is out of scope here.

On publish, C4 does **not** synthesize org from the response — it already has
the authored body. It only **harvests the server-assigned values** for
write-back: `JAUNDER_ID` from the `Location` header, `JAUNDER_SYNCED` from the
`ETag` header, and `JAUNDER_SLUG` (`j:slug`) + the resolved `atom:published`
from the returned entry.

To read `j:slug`/`atom:published` from the response without duplicating XML
parsing between C and D, **C4 lands a small shared primitive
`jaunder--atom-entry-fields`** (entry XML string → alist of element values). C4
consumes the slug/published subset; Unit D's `jaunder--atom->org` later builds
the full org-buffer synthesis on top of the same primitive. (Option B of the
C/D-boundary decision.) `jaunder--atom-entry-fields` is pure and ERT-tested in
C4.

## Test approach

- **Pure ERT** (`*-test.el`, serverless) for every pure
  mapping/parse/substitution helper (C2 in full; the parsing parts of C1/C3/C4).
- **Live ERT** (`*-integration.el`, via `jaunder-test--with-live-server` from
  #137) for transport (C1), media upload (C3), and the end-to-end publish flow
  (C4).
- No new gate wiring: pure tests run under the existing `ert` step; live tests
  under the `e2e-elisp-integration` nixosTest (ADR-0035), which globs
  `*-integration.el`.

## Out of scope

- `jaunder--atom->org` full synthesis + pull/reconcile — Unit D (#75).
- Non-image media uploads — #25.
- Markdown/HTML authoring buffers (the `JAUNDER_FORMAT` field allows them; v1
  targets org).
- Client self-provisioning of the app password — #76 (v1 pastes it into
  `auth-source`).
- `WWW-Authenticate` on 401 — #81 (deferred pending client experiments).

## Edge cases

Per the epic spec's "Unit C — Edge cases / tests" (local-only draft keeps its
temp name; first-publish rename + collision; untitled/all-symbol post publishes;
re-publish updates; stale ETag → 412; media substituted in sent body only;
pristine on-disk file on pre-response failure; vanilla-Jaunder format-media-type
warning).
