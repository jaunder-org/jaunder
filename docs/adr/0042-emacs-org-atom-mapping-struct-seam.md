# ADR-0042: The Emacs org→atom Mapping — Struct Seam, `dom-print` Serialization, Emacs 29.1 Floor

- Status: accepted
- Deciders: mdorman, Claude
- Date: 2026-07-02

## Context and Problem Statement

C2 (#160) fills the `jaunder--org->atom` seam: turning an authored org buffer
into the AtomPub `<entry>` the server parses on create/update. It is pure and
serverless-testable. Four cross-cutting shape decisions govern how the mapping
is structured and how it composes with the later units (C3 media, C4 publish
flow, D reverse mapping) that build on it. This ADR records them; the mechanical
field mapping and timezone details live in the issue spec.

## Decision Drivers

- The forward mapping should be trivially unit-testable as pure data, and should
  compose cleanly with C3 (media substitution in the body) and C4 (send).
- Mistakes in a hand-built wire document (a dropped field, a mislabeled type)
  should fail loud/early, not silently produce wrong XML.
- Keep the client dependency-light (the transport already added only `plz`).
- Pin a realistic minimum Emacs so the above can rely on built-ins.

## Decision Outcome

### D1 — Two-layer seam: abstract struct, then a separate serializer

`jaunder--org->atom` returns an **abstract `jaunder-entry`**, not wire XML; a
separate `jaunder--atom-entry->xml` renders the wire `<entry>`. This keeps the
forward mapping pure-data (`(should (equal (jaunder-entry-title e) …))`), lets
C3 substitute media in the body slot with one mutation before serialization, and
localizes all **wire knowledge** (namespaces, media-type strings,
`app:control/app:draft` nesting, element order) in one serializer tested once.
It is symmetric with the reverse primitive C4/D will add (`atom-entry-fields`,
XML→fields).

### D2 — `cl-defstruct jaunder-entry` for the representation (not a plist)

The intermediate is a `cl-defstruct`, not a plist. A misnamed field is then
caught early: the constructor rejects an unknown keyword **at byte-compile
time** (the struct's compiler macro) and at runtime, and a misnamed accessor is
a loud `void-function` plus an undefined-function warning under file
byte-compilation (→ a build error once #108's warnings-as-errors elisp gate
lands). A plist gives none of this — `(plist-get f :titel)` is a silent `nil`,
so a typo drops a field from every post with no signal. `cl-defstruct` does
**not** add value-type safety (`:type` is not runtime-enforced); it raises the
floor on field-name mistakes only. Cost is negligible — `cl-lib` is built in and
`plz-response` (already consumed by C1) is itself a `cl-defstruct`.

### D3 — Emit XML via built-in `dom.el` / `dom-print`, not hand-rolled strings

The serializer builds a `dom` node and calls `dom-print`. `dom-print` is built
into Emacs (since 28), so this is **not** a new dependency — the
hand-rolled-vs-library choice had no dependency asymmetry, and the library
removes a hand-written escaper. Verified on the pinned Emacs (30.2): `dom-print`
escapes both text and attribute values (`&`,`<`,`>`,`"`), emits prefixed
elements (`app:control`/`app:draft`) and root `xmlns:*` attributes, and
self-closes empty elements. The byte-determinism argument that might favor
hand-rolling was **withdrawn**: it applies to Unit D's org-buffer round-trip,
not to the entry we send (the server re-parses and re-serializes; the client
stores the response ETag). The `dom` node is a private detail of the serializer;
the seam remains the struct (D1).

### D4 — Emacs floor raised 27.1 → 29.1

`Package-Requires` becomes `(emacs "29.1")`. 29.1 (2023) is the floor on current
Ubuntu LTS (ships 29.3); it comfortably covers `dom-print` (since 28) and the
`encode-time` zone handling the mapping relies on. No user is expected below it.

Rejected alternatives:

- **Return finished XML from `jaunder--org->atom`** (collapse D1). C3 would then
  have to reach into an XML string to substitute media links, and the tests
  would conflate mapping with serialization.
- **plist instead of `cl-defstruct`** (D2). Lighter and consistent with the HTTP
  layer's `(:status …)` plist, but a field typo is a silent wrong post — the
  correctness argument outranks the consistency one for a fixed-shape record.
- **Hand-rolled XML string** (D3). No dependency advantage over `dom.el` (both
  built-in), and it reintroduces a bespoke escaper.

## Consequences

- Good: the forward mapping is pure data — trivial, fast ERT; C3/C4 compose on
  the struct without touching wire concerns.
- Good: field-name mistakes fail at byte-compile / first call, not silently.
- Good: no new dependency for serialization; the escaper is Emacs', not ours.
- Neutral: `cl-lib`/`dom`/`url-util` are now required (all built-in).
- Bad/So-what: the 29.1 floor drops pre-2023 Emacs, an acceptable line for a new
  client.

## Verification

- Pure ERT suite green on host **and** in the hermetic `ert-check` (the latter
  needs a zone database — `TZDIR` provided to the derivation for `encode-time`).
- `jaunder.el` byte-compiles clean with `byte-compile-error-on-warn` (exercising
  D2's accessor/constructor checks).
- Serializer output verified well-formed via `libxml-parse-xml-region` and
  structurally against the elements `common::atompub::entry_from_xml` parses; a
  live round-trip against the server is C4's territory.
