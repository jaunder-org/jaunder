# Rendered-body fallback summaries implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for a bounded task
> when useful. This outline exists because the approved behavior changes public
> Syndication Feed representations and requires a dual-backend
> cache-invalidating migration.

## Scope

In:

- One host-owned rendered-HTML-to-summary projection.
- Draft/scheduled labels, permalink description metadata, and Atom/JSON Feed
  summary metadata.
- Feed-cache invalidation on SQLite and PostgreSQL and the architecture
  documentation that describes the projection.

Out:

- Persisted derived summaries, public-site summary paragraphs, AtomPub
  summaries, permalink title changes, Site Tagline changes, and Syndication Feed
  excerpting.

## Task outline

- [x] Task 1: Establish and document the host-owned derived-summary boundary.
  - Contract: expose one host API that accepts `RenderedHtml` and returns
    `Option<PostSummary>`; use `ammonia::Builder::empty()`, entity decoding, the
    approved Unicode-whitespace normalization, and the exact lexical
    sentence/word/scalar rules. Keep `ammonia` and HTML handling out of wasm and
    storage; share only target-independent truncation logic where needed for the
    100-scalar label projection. Update both current-state summary passages in
    `docs/ARCHITECTURE.md`; preserve ADR-0063 as historical Decision text under
    ADR-0127.
  - Verification: host/common unit tests pin element stripping, entities,
    Unicode whitespace, adjacent-element behavior, textless rendering, sentence
    terminators and closing punctuation, abbreviations/decimals, word fallback,
    and 100/500-scalar limits; documentation formatting and links pass with the
    architecture view describing the new boundary.

- [x] Task 2: Project effective summaries onto web-owned presentation metadata.
  - Contract: authored summary wins; otherwise call Task 1's host API. Resolve
    the titleless unpublished-row fallback server-side to a compact summary or
    slug without representing a slug as `PostSummary`. Carry permalink
    description separately from the authored summary and without widening every
    timeline row; public Post rendering and AtomPub continue to observe authored
    summary absence.
  - Verification: focused web/server tests prove title, authored-summary,
    derived-summary, and textless-slug draft labels; permalink description/Open
    Graph metadata; unchanged permalink title, public summary paragraph,
    AtomPub, Post/revision persistence, and Site Tagline behavior.

- [ ] Task 3: Project effective summaries into Syndication Feeds and expire old
      cached bytes.
  - Contract: Atom `<summary>` and JSON Feed `summary` use the effective
    summary; RSS description and every format's complete rendered body remain
    unchanged. Add matching SQLite/PostgreSQL migration `0038` that invalidates
    `feed_cache` without changing its schema; regenerated fingerprints and ETags
    include the newly projected summaries through the existing `FeedItem`
    contract.
  - Verification: focused feed tests prove authored precedence, derived
    Atom/JSON metadata, unchanged complete content and RSS behavior, plus a
    dual-backend `#[apply(backends)]` migration test proving pre-migration cache
    rows are removed.

## Risk checks

- The extraction boundary consumes only already-sanitized `RenderedHtml`; no raw
  source-format branch or second parser is introduced.
- `RenderedPost.summary` retains authored-summary semantics, and metadata-only
  additions do not inflate all timeline rows contrary to ADR-0097.
- AtomPub native-source round trips remain unchanged under ADR-0015.
- The cache migration is present and ordered identically for both dialects; it
  deletes disposable cache rows only.
- Feed serialization keeps full article bodies and cache semantic identity
  changes with effective summary content.
- `docs/ARCHITECTURE.md` no longer describes raw-body-line fallback derivation;
  accepted ADR Decision text remains immutable under ADR-0127.
- Each checked task is committed through `jaunder-commit`; no lint suppression
  is introduced without explicit approval and no commit gains a `Co-Authored-By`
  trailer.
