# ADR-0023: AtomPub Jaunder Wire Extensions (format media types, `j:slug`, capability discovery)

- Status: accepted
- Deciders: mdorman, Claude
- Date: 2026-06-26
- Supersedes: the content-type _token scheme_ of
  [ADR-0015](0015-atompub-serialization-surfaces.md) (its separate-serializers
  principle stands)

## Context and Problem Statement

The Emacs front-end needs a single blog to mix Org, Markdown, and HTML posts,
and to recover the server-assigned slug for every post (including
drafts/scheduled). ADR-0015 serialized Org and Markdown identically as
`atom:content type="text"`, so format is **not expressible per entry**, and the
slug is only recoverable from a published permalink. We need a wire
representation that (a) distinguishes formats per entry, (b) exposes the slug
everywhere, and (c) degrades gracefully for non-aware clients (e.g. MarsEdit)
and against an older/vanilla Jaunder.

## Decision Drivers

- Reuse standard Atom/AtomPub before inventing markup (RFC 4287 §6 requires
  processors to ignore unknown foreign-namespace markup).
- Per-entry format fidelity without user-agent sniffing.
- Backward/forward compatibility with non-extension-aware clients and servers.
- A single, fully-tested, trivially reversible coupling point for the wire
  mapping.

## Decision Outcome

**1. Format travels in the standard `atom:content` `type` as a media type.**

| `PostFormat` | wire `type`                                                |
| ------------ | ---------------------------------------------------------- |
| `Org`        | `text/org` (de-facto; org-aware clients only)              |
| `Markdown`   | `text/markdown` (IANA RFC 7763)                            |
| `Html`       | `html` token (NOT `text/html`, which means _escaped_ text) |

Bare `text`, an unrecognized `text/*`, or an absent type → the account
`default_post_format` (unchanged fallback). RFC 4287 §4.1.3 permits a full media
type in `type`, and any `text/*` is carried as inline escaped text exactly like
`type="text"`.

**2. The mapping is encapsulated in a `format_wire` seam** — two pure functions
in `server/src/atompub/mapping.rs` (the seam has since moved to
`common/src/atompub`):

```rust
fn wire_to_format(content_type: Option<&str>, default: PostFormat) -> PostFormat;
fn format_to_wire(format: PostFormat) -> &'static str;
```

`wire_to_format` is **lenient** (recognizes `text/org`, `text/markdown`, `html`,
and defensively `xhtml`/`text/html`; tolerates a media-type parameter; falls
back to `default` otherwise) so reading never breaks. `format_to_wire` is the
**only** MarsEdit-risk surface — reverting `text/markdown`→`text` is a one-line
change.

**3. `j:slug`** — the one Jaunder foreign-markup element (namespace
`https://jaunder.org/ns/atompub`), read-only, emitted on **every** entry (drafts
and scheduled included), `xmlns:j` declared only when emitted (mirroring
`xmlns:app`). Incoming `j:slug` is ignored.

**4. Capability discovery** — the service document advertises
`<j:extension version="1" features="format-media-type slug"/>` so a client
detects support once and degrades gracefully against a vanilla/older Jaunder.

## Consequences

- Good: a blog mixes Org/Markdown/HTML per entry; the slug is always
  recoverable; clients can feature-detect.
- Good: the entire wire policy is two unit-tested pure functions — reversible in
  one line if a client misbehaves.
- Bad: `text/org` has no IANA registration (acceptable — it is consumed only by
  org-aware clients fully under our control).
- Open (carried from ADR-0015): a non-format-aware client (MarsEdit) editing an
  Org/Markdown post and re-sending bare `type="text"` triggers a format
  downgrade to the account default. Affects only deliberate two-client setups;
  same class as ADR-0015's title-only-edit open question.
