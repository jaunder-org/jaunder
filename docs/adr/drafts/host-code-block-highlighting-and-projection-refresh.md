# ADR-DRAFT: Host code-block highlighting and resumable projection refresh

- Status: proposed
- Date: 2026-09-23
- Issue: [#1655](https://github.com/jaunder-org/jaunder/issues/1655)

## Context

Org source blocks and explicitly labeled Markdown fences contain readable native
source, but their public rendered HTML currently carries no syntax cues. A
client-side renderer would enlarge the browser trust and dependency closure, and
rewriting authored bodies or historical Post Revisions would violate
[ADR-0015](../0015-atompub-serialization-surfaces.md) and
[ADR-0136](../0136-local-post-lifecycle.md). Existing stored projections need
refreshing if old and new Posts are to render consistently.
[ADR-0079](../0079-rendered-html-sanitization.md) narrowly permits code-block
language classes and deliberately avoids a backfill;
[ADR-0184](../0184-css-package-public-themes.md) makes the public Style Contract
available to Theme Packages.

## Decision

Highlight only explicitly labeled Org source blocks and Markdown fenced blocks
at the host Post-render boundary. Use only `tree-sitter-highlight` with pinned
language grammars and compatible highlight queries; Emacs Lisp (`elisp`,
`emacs-lisp`) and Haskell (`hs`, `haskell`) are regression languages among the
broad built-in catalog. The statically linked grammar/query inventory and
aliases are reviewable and tested. Labels are ASCII-case-folded. Parse at most
64 KiB of decoded exporter code per block. Allow at most 16 attempts and 128 KiB
attempted bytes per Post, visiting blocks in document order. Unrecognized or
over-budget code stays escaped, uncolored source. Keep the decoded code text
identical to that of the same format's unhighlighted exporter. An expected
malformed source can fall back to plain code; unexpected grammar/query/engine
errors are typed failures, not successful writes.

Widen `common::render::sanitize` only to the ten closed `j-syn-*` span classes:
`comment`, `keyword`, `string`, `number`, `function`, `type`, `variable`,
`constant`, `operator`, and `punctuation`. Keep existing `language-*` pre/code
classes. Map Tree-sitter query captures into that vocabulary rather than
granting arbitrary grammar or author styling. Only Post-body `pre code` token
selectors acquire color, because the sanitizer cannot inspect element ancestry
and an author can forge a permitted class elsewhere. Provide built-in CSS
variable defaults and allow public Theme Packages to override these additive
Style Contract v1 hooks; Home remains Jaunder-styled. This amends ADR-0079's
narrower class allowlist without weakening its `RenderedHtml` safety guarantee.

Once the renderer passes integrated safety, fidelity, and latency checks,
refresh _current active Org and Markdown Post projections_ with a bounded,
durable checkpoint before accepting traffic or starting Syndication Feed
workers. One transaction locks/rechecks progress and at most 100 ascending Post
IDs, CAS-checks current source, format, prior rendered bytes and active state,
and atomically replaces changed rendered HTML, Media references, affected
public-feed events and its checkpoint. A stale author edit is rerendered once or
fails visibly; a Deleted Post is skipped. Resumption and concurrent startup must
not skip or double-apply a row. Old-version writers are drained before a rolling
deployment refresh. The transition changes presentation only: preserve authored
source, Post identity, timestamps, AtomPub Member content ETag, and immutable
Post Revisions. This narrow exception to ADR-0079's no-backfill choice and
ADR-0136's revision-on-meaningful-change rule does not authorize arbitrary
history rewriting.

## Consequences

The broad pinned Tree-sitter language catalog belongs in the host binary, not
browser WASM. Every new language needs alias, quality, safety, and resource
tests, not a new HTML privilege. Theme overrides operate through scoped CSS
rather than broadening sanitization. Startup can fail if a refresh cannot safely
complete, and a rolling deployment must fence old writers first. The refresh
updates affected public Syndication Feeds and validators through the existing
event/outbox mechanism while the existing five-minute public cache bound
remains; it does not promise instantaneous CDN invalidation. The owner accepted
the representative Tree-sitter latency evidence as sufficient after rejecting
the original +25 ms overhead criterion; no additional performance threshold
gates this catalog. No binary-growth cap was a user requirement; report stripped
size for visibility only. Safety and fidelity remain gates.
