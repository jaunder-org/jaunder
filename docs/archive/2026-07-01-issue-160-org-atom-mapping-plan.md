# Plan — C2 org→atom mapping (issue #160)

Spec: `docs/superpowers/specs/2026-07-01-issue-160-org-atom-mapping.md`.
Approach: **test-first** (pure ERT, serverless) in `elisp/test/jaunder-test.el`;
implementation in `elisp/jaunder.el`. Each task is one red→green→commit, gated
with `cargo xtask check` (runs the elisp `ert` step among the rest); during
iteration run the elisp suite directly for speed, and byte-compile `jaunder.el`
to exercise the struct-accessor safety (D2) even though #108's
warnings-as-errors gate is not yet landed.

No separable-concerns issue-filing task: C3 (#161) and C4 (#162) already exist;
the deferred date write-back / current-zone capture is explicitly C4; media is
explicitly C3.

## Task 1 — struct + core field mapping + header stripping

**Red:** ERT for `jaunder--org->atom` over `with-temp-buffer` org buffers: title
(present/absent → nil), `#+KEYWORDS:` (comma-split, multi-line, flattened),
`#+DESCRIPTION:` (repeated lines joined with newline), `content-type` always
`text/org` (org→atom converts org — knowable from the converter, **not** read
from `JAUNDER_FORMAT`), `JAUNDER_STATUS` → `draft` (draft⇒t,
scheduled/published⇒nil), body-only `body` with the header block stripped and no
`JAUNDER_*` leaking (including when an unmapped keyword interleaves the block),
and an untitled all-symbol/emoji body.

**Green:**

- Bump `Package-Requires` → `(emacs "29.1")` (D4); `(require 'cl-lib)`,
  `(require 'org)`, `(require 'dom)`, `(require 'url-util)`.
- `(cl-defstruct (jaunder-entry (:constructor jaunder--make-entry)) title categories summary draft content-type body published)`.
- Replace the `jaunder--org->atom` stub: read fields via `org-collect-keywords`
  (TITLE/KEYWORDS/DESCRIPTION and PROPERTY lines parsed for `JAUNDER_*`); set
  `content-type` to the `text/org` constant; `draft` from `JAUNDER_STATUS`.
  Leave `published` nil (Task 2). Non-mutating: read the buffer, strip the
  header block from a copy of `(buffer-string)`.
- Header stripping: remove the leading contiguous run of **any** org
  file-keyword lines (`#+KEY:`, so an interleaved unmapped keyword can't halt
  stripping) and blank lines; the remainder is `body` (trailing-trimmed).

## Task 2 — timezone → UTC (`published`)

**Red:** ERT for the date computation: IANA DST-correct (NY summer→13:00Z, NY
winter→14:00Z), numeric-offset **string** `"-0500"`/`"-05:00"` parsed to the
correct UTC (the G1 regression — a raw string would be silently wrong), missing
`#+DATE:` → `published` nil, and the status table's `published`-no-date
("publish now") → nil while `published`+date → the UTC, `scheduled`+date → the
UTC.

**Green:**

- `jaunder--offset->seconds` — parse `±HHMM`/`±HH:MM` → integer seconds (G1).
- `jaunder--org-date->utc` — `org-parse-time-string` → set the decoded-time zone
  slot to the resolved zone (IANA string as-is; a numeric offset parsed to
  integer seconds) → `encode-time` → `format-time-string "%…Z" t`.
- Wire into `jaunder--org->atom`: set `published` per the spec's status table
  (omit for draft and for publish-now).

## Task 3 — serializer `jaunder--atom-entry->xml`

**Red:** ERT: a full `jaunder-entry` serializes to a well-formed `<entry>` with
the Atom + `app` (+ `j` if used) namespaces; text and attribute escaping
(`&`/`<`/`>`/`"`); one `<category term>` per tag; `<summary>`/`<title>` present
only when set; `<content type=…>` carries the media type and the body;
`<published>` present iff the slot is set; `<app:control><app:draft>yes…`
present iff `draft`. A structural assertion that the output contains the
expected elements (a live round-trip through the server is C4).

**Green:** build a `dom` node from the struct (attributes as alists, prefixed
`app:control`/`app:draft`, root `xmlns:*`), then `(dom-print node nil t)` into a
string. All wire knowledge lives here (D1/D3).

## Task 4 — docs touch-up

Update `elisp/README.md` if it lists seam status (mark `jaunder--org->atom`
implemented). The spec + this plan are already written; the numbered ADR for
D1–D4 is **deferred to ship** (issue #178 claims ADR-0041; mint the next free
number then). Confirm `cargo xtask validate --no-e2e` green before ship.

## Verification

- Per task: elisp ERT green + `jaunder.el` byte-compiles clean +
  `cargo xtask check` green.
- Final: `cargo xtask validate --no-e2e` (the pre-push-style gate). The live
  publish path that exercises this mapping end-to-end against a real server is
  C4's `*-integration.el`; C2 ships pure ERT only.
