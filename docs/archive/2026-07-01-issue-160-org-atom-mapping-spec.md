# C2 — Emacs org→atom mapping (`jaunder--org->atom`) — issue #160

- Status: proposed
- Deciders: mdorman, Claude
- Milestone: **Emacs blogging front-end** (#4). Sub-issue of #74 (Unit C —
  authoring/publish); the second of four (C1 #159 done → **C2 #160** → C3 #161 →
  C4 #162).
- Builds on: C1's transport (`jaunder--http-request`, #159/ADR-0038), the elisp
  skeleton + seams (#73), and the server AtomPub entry surface
  (`common::atompub::entry_from_xml`, `server/src/atompub/mapping.rs`).
- Detailed design: **the epic spec's "Unit C" section**
  (`docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md`, and
  the #74 decomposition in `2026-06-29-issue-74-emacs-authoring-publish.md`).
  This per-issue spec does **not** restate that design; it records the **shape
  decisions, the timezone mechanism (with its gotchas), and the C2 test
  mapping** resolved in the #160 design interview.

## Goal

Fill the forward mapping seam: an authored org buffer → the AtomPub `<entry>`
wire document the server parses on create/update. Two pure, serverless-testable
functions land here:

1. `jaunder--org->atom` — org buffer → a structured `jaunder-entry` value (the
   metadata header block mapped to fields; the body carried separately with the
   header block stripped).
2. `jaunder--atom-entry->xml` — `jaunder-entry` → the wire `<entry>` XML string.

`jaunder--atom->org` (the reverse) stays entirely in Unit D (#75) and is
untouched here.

## Shape decisions (design interview, 2026-07-01)

These decisions are recorded as **ADR-0042**
(`docs/adr/0042-emacs-org-atom-mapping-struct-seam.md`); this section is the
fuller rationale.

### D1 — Two-layer seam: struct fields, then a separate serializer

`jaunder--org->atom` returns an **abstract** `jaunder-entry`, not wire XML; a
separate `jaunder--atom-entry->xml` serializes. Rationale: keeps the forward
mapping pure-data (trivial ERT: `(should (equal (jaunder-entry-title e) …))`);
lets C3 (#161) substitute media in the body field with a single slot mutation
before serialization; and localizes all **wire knowledge** (namespaces,
media-type strings, `app:control/app:draft` nesting, element order) in one
serializer function tested once. Symmetric with C4's reverse primitive
`jaunder--atom-entry-fields` (XML→fields).

### D2 — `cl-defstruct jaunder-entry` for the representation (not a plist)

The intermediate is a `cl-defstruct`, not a plist/alist. Rationale (verified on
the pinned Emacs, 30.2): a **misnamed field is caught early** instead of
silently producing wrong XML —

- constructor with an unknown keyword (`:titel`) → **error at byte-compile
  time** (the struct's compiler macro rejects it) and at runtime;
- a misnamed accessor → loud `void-function` at first call, and an undefined-
  function warning under file byte-compilation (→ build error once #108's
  warnings-as-errors elisp gate lands).

A plist gives none of this — `(plist-get f :titel)` / `(plist-put f :titel …)`
are silently `nil` / junk, so a typo drops (e.g.) the title from every post with
no signal. `cl-defstruct` does **not** give value-level type safety: `:type`
slot declarations are **not** runtime-enforced (a number stored in a `string`
slot is accepted). So this raises the floor on field-name mistakes, not on
value-type mistakes. Cost is negligible: `cl-lib` is built in, and
`plz-response` (already consumed by C1) is itself a `cl-defstruct`.

Fields: `title categories summary draft content-type body published` (see the
field mapping below for each).

### D3 — Emit XML via built-in `dom.el` / `dom-print` (not hand-rolled strings)

The serializer builds a **dom node** internally from the struct and calls
`dom-print`. Rationale: `dom-print` is built into Emacs (since 28 — stable
through 29/30, no NEWS changes), so it is **not** a new dependency (the
hand-rolled-vs-library trade had no dependency asymmetry). Verified on 30.2:
`dom-print` correctly escapes both text children and attribute values
(`&`→`&amp;`, `<`→`&lt;`, `>`→`&gt;`, `"`→`&quot;`), emits prefixed elements
(`app:control`, `app:draft`) and root `xmlns:*` attributes, and self-closes
empty elements (`<category term="rust" />`). The earlier byte-determinism
argument for hand-rolling was **withdrawn** — it applies to Unit D's org-buffer
synthesis, not to the entry XML we send (the server re-parses and re-serializes;
we store the response ETag). The dom node stays a private detail of the
serializer; the seam is the struct (D1).

### D4 — Emacs floor raised 27.1 → 29.1

`Package-Requires` in `elisp/jaunder.el` bumps from `(emacs "27.1")` to
`(emacs "29.1")`. Rationale: Emacs 29.1 (2023) is the floor on current Ubuntu
LTS (ships 29.3); it comfortably covers `dom-print` (since 28) and the
`encode-time` zone handling below. No user is expected below 29.1.

## Field mapping (org → `jaunder-entry`)

Standard org keywords carry standard data; `JAUNDER_*` file properties carry
jaunder-specific data. All read with `org-collect-keywords` /
`org-collect-keywords`-derived property access. Per the epic spec's "Org
vocabulary":

| org source                       | `jaunder-entry` slot           | notes                                                                                                                                                                                                                                           |
| -------------------------------- | ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `#+TITLE:`                       | `title`                        | **optional** (nil ⇒ untitled/microblog — no `<title>` emitted)                                                                                                                                                                                  |
| `#+KEYWORDS:`                    | `categories`                   | comma-split, multi-line, flattened → one `<category term>` each                                                                                                                                                                                 |
| `#+DESCRIPTION:`                 | `summary`                      | repeated lines joined with newline (ox-html semantics); nil ⇒ no `<summary>`                                                                                                                                                                    |
| `#+DATE:` + `JAUNDER_DATE_TZ`    | `published`                    | RFC-3339 UTC per the timezone rules below                                                                                                                                                                                                       |
| `JAUNDER_STATUS`                 | `draft` (+ `published` gating) | see status table                                                                                                                                                                                                                                |
| (the converter itself)           | `content-type`                 | always `text/org` — `org->atom` converts org, so the media type is knowable from the converter, **not** read from `JAUNDER_FORMAT` (which would only mislabel org body); non-org authoring buffers are separate future converters, out of scope |
| org body (header block stripped) | `body`                         | body-only `atom:content`; media substitution is C3                                                                                                                                                                                              |

**Header stripping.** The metadata header block
(`#+TITLE:`/`#+DATE:`/`#+KEYWORDS:`/`#+DESCRIPTION:`/`#+PROPERTY: JAUNDER_*`) is
removed from `body`; the server stores a clean, metadata-free canonical body and
never sees `JAUNDER_*` markup.

**Untitled / all-symbol / emoji.** An untitled post maps `title` → nil (no
`<title>`); the body carries the content. Slug generation is the server's job
(Unicode-robust, never-fail — Slug unit), so C2 does nothing special beyond
omitting the title. Tested.

### `JAUNDER_STATUS` → `draft` / `published`

Mirrors the server model (`is_draft` ⟺ no `published_at`):

| `JAUNDER_STATUS`                          | `draft` slot                                                  | `published` slot                                      |
| ----------------------------------------- | ------------------------------------------------------------- | ----------------------------------------------------- |
| `draft`                                   | `t` → `<app:control><app:draft>yes</app:draft></app:control>` | omitted                                               |
| `scheduled`                               | `nil`                                                         | the future UTC (validation that it is future is C4's) |
| `published`, with `#+DATE:`               | `nil`                                                         | the computed UTC                                      |
| `published`, no `#+DATE:` ("publish now") | `nil`                                                         | omitted — server stamps, C4 writes back               |

## Timezone → UTC computation (verified mechanism + gotchas)

`published` is canonical RFC-3339 UTC. Computation (verified on 30.2):

1. `org-parse-time-string` on the raw `#+DATE:` → a decoded-time list
   `(SEC MIN HOUR DAY MON YEAR DOW DST ZONE)`.
2. Resolve the zone from `JAUNDER_DATE_TZ` and set the decoded-time zone slot.
3. `encode-time` that list, then
   `(format-time-string "%Y-%m-%dT%H:%M:%SZ" TIME t)` for UTC.

**Verified:** an IANA zone name (`America/New_York`, `Europe/London`) is
**DST-correct** — NY 09:00 wall-clock → 13:00Z in July (EDT), 14:00Z in January
(EST); London 09:00 → 08:00Z in July (BST).

**Gotcha G1 — numeric offset must be integer seconds, not a string.** A numeric
fallback zone stored as a string (`"-0500"`) is **silently treated as UTC**
(wrong) by `encode-time`. Only an **integer seconds** offset (`(* -5 3600)`)
works. So when `JAUNDER_DATE_TZ` is a numeric offset, C2 must **parse it to
integer seconds** (`±HHMM`/`±HH:MM` → seconds) before encoding. This has a
dedicated regression test (a raw offset string would otherwise pass silently
wrong). The numeric fallback carries the epic spec's caveat: it does not
DST-adjust future scheduled times.

**Gotcha G2 — a bogus IANA name silently falls back to UTC** (no error). C2
cannot distinguish a typo'd zone from a deliberate UTC-year-round zone from
inside a pure conversion, and detecting/warning is only actionable in the
interactive publish flow — so **mitigation is deferred to C4**: a
zone-recognition predicate (validating a name by the existence of its zone file
`<zoneinfo>/NAME`, which precisely matches what the resolver reads, so genuinely
UTC-year-round zones like `Atlantic/Reykjavik`/`Africa/Abidjan` are not
false-flagged) plus a warn-on-unrecognized at capture/override time. It lands
with its consumer, not ahead of it. **Open cross-platform question (also C4):**
whether `encode-time` resolves IANA names on native Windows at all is unverified
(Windows lacks a `zoneinfo` tree); if it does not, DST conversion is degraded
there — a separate concern to track.

**Test-env note.** The timezone→UTC tests need a system zone database for
`encode-time` to resolve IANA names. A bare Nix `runCommand` sandbox has none,
so the hermetic `ert-check` sets `TZDIR = ${pkgs.tzdata}/share/zoneinfo`
(flake.nix). Without it, named zones silently resolve to UTC and these tests
fail in CI while passing on the host `ert` step — verified by building
`ert-check` before/after the fix.

**Scope note.** C2 computes UTC **given** a resolved `JAUNDER_DATE_TZ` (and
provides the offset-string→seconds parser). Capturing the machine's current IANA
zone at first publish, zone-recognition + warn-on-unrecognized, and writing the
three date fields back are **publish-flow behavior (C4)**.

## Test approach (pure ERT, serverless — `elisp/test/jaunder-test.el`)

Per the umbrella spec: C2 is **pure ERT** end to end, run under the existing
`ert` step (no gate wiring). Cases:

- **Field mapping**: title/keywords(comma+multiline+flatten)/description(join),
  content-type always `text/org` (a stray `JAUNDER_FORMAT` is ignored), each
  independently.
- **Header stripping**: `body` excludes the header block; `JAUNDER_*` never
  appears in `body`, including when an unmapped keyword interleaves the block.
- **Status mapping**: draft/scheduled/published(+date)/published(no-date) →
  `draft` + `published` per the table.
- **Timezone/UTC**: IANA DST-correct (summer vs winter); numeric-offset-string
  parsed to correct UTC (G1 regression); missing `#+DATE:` → nil.
- **Untitled + all-symbol/emoji** body: no `<title>`, body preserved.
- **Serializer (`jaunder--atom-entry->xml`)**: well-formed `<entry>` with
  correct namespaces; text/attribute escaping; `<category>` per tag; `app:draft`
  only when draft; media-type on `<content>`; `<published>` present/absent per
  slot; parseable by `common::atompub::entry_from_xml` (checked structurally in
  elisp; a round-trip against the server is C4's live-ERT territory).

## Out of scope (unchanged from the umbrella spec)

- `jaunder--atom->org` synthesis + pull/reconcile — Unit D (#75).
- Media scan/upload/substitution — C3 (#161); C2 leaves `body` with local links.
- The publish flow, validation of scheduled-future, write-back, rename — C4
  (#162).
- Capturing the machine's current IANA zone + date write-back — C4.
- Non-image media (#25); markdown/HTML authoring buffers; app-password
  self-provisioning (#76); `WWW-Authenticate` on 401 (#81).
