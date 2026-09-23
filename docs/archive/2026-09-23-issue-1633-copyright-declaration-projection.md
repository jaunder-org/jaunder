# Copyright Declaration projection (#1633)

## Outcome

Public Post HTML and Atom/RSS Syndication Feeds obtain their Copyright
Declaration from one typed, projection-only owner, with no change to rendered
markup, feed bytes, or persisted data. JSON Feed retains its approved structured
`_jaunder` representation.

## Load-bearing decisions

- A Copyright Declaration is derived from the Post's immutable UTC creation
  year, the author's **current** Display Name (or canonical Username fallback),
  and the author's **current** Content License, per ADR-0206. It is not a Post
  or a stored snapshot.
- The shared projection owns the resolved year, author identity text, Content
  License, and the canonical `© YEAR NAME · LABEL` text. Public HTML, Atom, and
  RSS consume this owner instead of independently composing the text.
- HTML continues to escape the author's name and produce its existing optional
  license link using the markup renderer, not by emitting preformatted text as
  raw HTML. Atom and RSS use the plain canonical text.
- The projection also owns the copyright-only `© YEAR NAME` text for JSON Feed.
  JSON Feed's `_jaunder` item keeps its existing `copyright`, `rights`, and
  nullable `license` fields; it does not substitute the full canonical text for
  its structured fields.
- The projection lives on the shared host/browser-compatible domain side of the
  web/feed boundary; storage records, Post Revisions, and AtomPub DTOs do not
  acquire declaration data.

## Acceptance

- One implementation owns both declaration text forms. Fixed-fixture
  before/after byte comparisons pin the public HTML copyright footer and
  complete Atom, RSS, and JSON Feed representations; named author, Username
  fallback, license-link, and no-link assertions remain green. No markup,
  namespace, or feed-byte drift is accepted.
- Projection tests pin the UTC creation year, Display Name fallback, license
  label/text, and JSON copyright-only form; author names with HTML-special
  characters remain escaped on the web surface.
- Syndication Feed semantic fingerprint preimage and digest for a fixed fixture
  remain byte-identical to the pre-refactor baseline (year, resolved author
  name, license token in their existing order), as well as changing when any of
  those inputs changes. Unchanged inputs keep their existing identity and
  representation time.
- No schema migration, new wire field, changed markup, or user-visible behavior
  is introduced.

## Boundaries

- No change to Content License options, rights policy, author identity rules,
  WebSub event production, feed cache policy, or AtomPub representations.
- No new ADR: ADR-0206 already establishes current, projection-only rights and
  their public surfaces.
