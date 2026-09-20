# Issue #1590 — Inline-rendered Post titles

## Outcome

Post article headings render safe inline markup authored in Markdown, Org, or
HTML instead of exposing secondary-markup delimiters. Atom Syndication Feeds
carry the same rendered title, while RSS and JSON Syndication Feeds expose its
clean visible text without source markup or HTML tags.

## Load-bearing decisions

- `PostTitle` remains canonical authored source. Its `PostFormat` determines how
  it is interpreted.
- Host rendering derives a dedicated **Rendered Title** together with rendered
  body output whenever a titled Post is created or its content or format
  changes.
- A Rendered Title is sanitized, canonical, inline-only HTML. It has a stronger
  contract than body HTML and a distinct trusted type and decode boundary.
- A dedicated, narrowly configured `ammonia` builder owns title sanitization.
  Its complete retained-element policy is `b`, `strong`, `i`, `em`, `u`, `s`,
  `del`, `code`, `sub`, `sup`, `mark`, `small`, and `br`, with no attributes.
  Markdown and Org support every textual inline construct their parsers can
  project into that policy, rather than only the issue's examples.
- HTML titles are interpreted as untrusted fragments under the same closed
  policy, not displayed as literal markup.
- Links lose their wrapper and destination but retain their rendered label.
  Images and removed block wrappers do not synthesize replacement text or
  spacing. Comments disappear. `script`, `style`, `template`, `iframe`, `object`,
  `embed`, `svg`, `math`, and audio/video/source/track elements disappear with
  their descendants. No event handler, URL, style, class, or other attribute
  survives.
- Canonical serialization is deterministic. The plain-text feed projection uses
  `ammonia` to strip all title markup, `html-escape` to decode entities once,
  converts `br` to one space, collapses every whitespace run to one ASCII space,
  and trims.
- A source title with no surviving visible text persists an empty Rendered
  Title. Web presentation omits its heading; RSS and JSON Feed omit their
  optional title; Atom emits its required empty HTML title construct. Authored
  source is never substituted back into a presentation protocol.
- Current Posts and full Post Revisions persist the Rendered Title beside the
  authored title, format, and rendered body. Those values form one derived write
  aggregate and are stored atomically.
- The persisted representation pins parser-version behavior just as rendered
  body HTML does. Reads do not reparse titles or maintain a permanent
  render-when-missing compatibility path.
- Migration 0039 adds nullable Rendered Title columns. Because no production
  instances exist, it does not repair legacy rows; new titled writes persist a
  derivative while titleless records keep none.
- Both SQLite and PostgreSQL implement the same schema, migration, mutation,
  semantic-no-op, revision, backup, restore-validation, and decode invariants.
- Host ammonia validates persisted bytes by requiring exact policy-preserving
  reconstruction. Invalid persisted bytes fail typed reads and can never reach
  an unescaped sink; server-authored DTO bytes are trusted by CSR exactly like
  rendered body HTML.
- Backup restore retains ADR-0174's restore-and-report policy: invalid Rendered
  Title payloads are restored as source bytes and reported as typed-domain
  diagnostics, while subsequent typed reads reject them. Structural absence is
  valid only for a titleless record; every authored title has a persisted
  derivative, including the empty canonical fragment.
- Shared web Post article headings consume the persisted Rendered Title. The
  public projector and CSR client paint the same trusted bytes.
- Atom Syndication Feed entry titles use the Rendered Title as an HTML text
  construct.
- RSS and JSON Syndication Feed item titles use readable text produced by
  stripping the persisted Rendered Title with `ammonia`. This projection decodes
  canonical entities and is not regex tag stripping.
- Feed semantic fingerprints, serializer revisions, and durable cache
  invalidation account for every title representation that changes feed bytes.
- Slug derivation, document metadata, AtomPub, editor fields, and
  source-oriented administration continue to consume the authored title.

## Acceptance

- An Org title containing emphasis and underline syntax renders the
  corresponding safe inline presentation in timeline cards and on its permalink,
  without showing the delimiters.
- Representative Markdown emphasis, strong emphasis, deletion, inline code,
  entities, and labelled-link titles render correctly; link destinations do not
  create nested anchors.
- Equivalent safe HTML title markup renders as inline presentation after
  sanitization.
- Malicious or structurally invalid Markdown, Org, and HTML title inputs cannot
  introduce active markup, block markup, media, nested interactive content, or
  DOM structure outside the title fragment.
- Labelled-link text remains visible while image alternative text is not
  synthesized; content-free or active-only source produces the specified empty presentation
  without leaking authored markup.
- Titleless Posts retain their existing rendering and storage behavior.
- Create and meaningful update operations persist matching authored and rendered
  titles on SQLite and PostgreSQL. A semantic no-op creates neither a Post
  Revision nor a timestamp change.
- A title or format change captures the complete prior Rendered Title in the
  same Post Revision as the prior source and rendered body.
- Migration tests prove both backend schemas add nullable Rendered Title
  columns; new titled writes persist derivatives and titleless records remain
  null.
- Backup and restore tests preserve Rendered Title bytes exactly, diagnose
  invalid payloads without blessing them as trusted HTML, and reject invalid
  title/Rendered Title presence combinations on both backends. Typed reads
  reject bytes that ammonia would change, including active, attributed,
  interactive, block, malformed, and content-free fragments.
- Browser Post headings use the persisted fragment without requiring title
  parsing or sanitization in CSR code.
- Atom emits a standards-conforming HTML title construct. RSS and JSON Feed emit
  marker-free, tag-free, entity-decoded visible text for the same Post.
- AtomPub round-trips the authored title unchanged, and slug and document-title
  behavior remains based on authored source.
- Every feed-output-affecting title change alters the corresponding semantic
  fingerprint, and pre-feature durable feed caches cannot survive as current
  representations.
- Focused tests pin exact persisted HTML and RSS/JSON text for each retained
  element and for representative Markdown, Org, and HTML source, including
  links, removed images and block wrappers, `br`, entities, adjacent nodes,
  whitespace, comments, malformed input, active elements, and empty output.
- Focused tests also cover storage parity, revisions, backup/restore, wire
  decode, web rendering, feed rendering, and feed identity.
- End-to-end assertions exercise Markdown, Org, and HTML titles on the public
  timeline and permalink. The existing public-timeline Chromium and Firefox
  visual baselines are updated through the repository's supported snapshot
  workflow if their pixels intentionally change; no new visual state or mobile
  baseline is introduced.
- `cargo xtask check` passes.

## Boundaries

- This change does not alter title authoring controls, title extraction from
  Post bodies, slug algorithms, metadata syntax, or AtomPub's native-source
  contract.
- It does not make embedded title links independently clickable; their labels
  are presentation text within the Post permalink.
- It does not admit images, audio, video, iframes, widgets, or other embedded
  content into Post headings.
- It does not format titles in revision lists, history administration, editor
  inputs, browser/document metadata, or other source-oriented surfaces.
- It does not re-render stored titles merely because a parser dependency is
  upgraded; any future bulk rewrite requires an explicit migration decision.
- It does not generalize Rendered Title into an arbitrary trusted-HTML escape
  hatch or reuse body HTML's broader structural contract.
