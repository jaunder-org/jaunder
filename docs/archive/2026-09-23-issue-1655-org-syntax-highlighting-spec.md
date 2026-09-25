# Issue #1655 — Semantic highlighting for published Org and Markdown code blocks

Implementation and independent review completed on the issue branch; PR CI and
explicit human merge approval remain pending.

## Outcome

Ship broad language highlighting, including Emacs Lisp and Haskell, in both Org
source blocks and explicitly labeled Markdown fenced code blocks. If the
evidence clears the safety, quality, and resource gates below, ship production
highlighting for both formats and languages in this issue. The result must
appear in existing and new published Posts on the public permalink, Local, and
Home; public Syndication Feeds continue to carry safe rendered HTML. Additional
languages must be addable through a bounded registry, without changing the trust
policy for each new grammar.

## Load-bearing decisions

- Emacs Lisp and Haskell are required regression languages because both occur on
  the observed production consumer, **not the extent of language support**. Ship
  a broad, pinned built-in catalog of programming and common markup/data
  languages with a reviewable alias inventory. Normalize labels by ASCII
  case-folding, including `emacs-lisp`/`elisp` and `haskell`/`hs`. Org
  `#+begin_src` labels and Markdown **fenced** code-block info labels use the
  same bounded registry of aliases, grammars and queries. A Markdown fence is
  eligible only when its explicit first info-string word is a supported label;
  indented or unlabeled blocks, inline code, and author-supplied raw HTML are
  never highlighter input. Unknown labels stay escaped, uncolored code; catalog
  additions require registry entries and tests, never new HTML privileges. Use
  **tree-sitter-highlight only**, with pinned grammars and compatible highlight
  queries for each included language; no syntect, TextMate lexer, runtime
  downloads, or author-provided plugins. Ship a reviewable inventory of each
  included grammar, compatible query and supported label/alias.
- Tree-sitter query captures, not raw parse-tree node names or arbitrary
  authored classes, map to the closed cross-language set `comment`, `keyword`,
  `string`, `number`, `function`, `type`, `variable`, `constant`, `operator`,
  and `punctuation`. Match a named capture to one category; unknown captures
  render without a token hook. Highlighted fragments are
  `<span class="j-syn-<category>">` inside `<pre><code>`; each category has a
  built-in `--j-syn-<category>` CSS default. These stable hooks are an
  **additive Style Contract v1 extension**: old Theme Packages remain valid,
  while public themes may override variables/selectors without replacing markup.
  Home continues using Jaunder's own styling; it never loads a custom public
  Theme Package.
- Highlight on the host at the existing Post render projection boundary, so
  web-created and Emacs-synchronized Posts use the same path and browser WASM
  gains no grammar dependency. For either authoring format, the comparison
  oracle is the decoded `<code>` text emitted by its current, unhighlighted
  exporter for the _same_ source; highlighted and fallback output must have
  exactly that Unicode text, including spaces, tabs, blank lines, and line
  endings. Keep both authored bodies and AtomPub native-source round trips
  byte-identical (including Org's canonical metadata-free body). Code is never
  executed. Keep both formats' existing Post Shortcode handling intact: a valid
  shortcode outside code still renders, while shortcode-looking text inside a
  code block remains literal.
- Keep `RenderedHtml`'s common-owned sanitization guarantee. Permit only the
  exact `j-syn-<category>` class tokens on `<span>` and existing `language-*`
  tokens on `<pre>`/`<code>`, not `style`, arbitrary class names, or additional
  active markup. The sanitizer's tag/attribute filter cannot inspect ancestors,
  so an author could forge a fixed token class elsewhere; CSS must scope token
  colors to Post-body `pre code` only. Include adversarial authored HTML tests.
  This explicitly amends ADR-0079's narrower class rule; no raw `RenderedHtml`
  constructor or sanitizing on storage read.
- Refresh **current projections of all active Org and Markdown Posts** after
  production adoption (published, scheduled, draft, public or private; exclude
  Deleted and HTML-format Posts) so future publication and old public permalinks
  agree. Use a dedicated presentation-only maintenance transition, not an
  ordinary meaningful Post edit: preserve body/format, authored metadata,
  `updated_at`, Post identity, AtomPub Member content ETag, and immutable Post
  Revisions. This is an explicit exception to ADR-0079's no-backfill decision
  and ADR-0136's revision-on-meaningful-change rule, documented in a proposed
  ADR draft and `docs/ARCHITECTURE.md`.
- Migration `0047` enqueues the existing offline `rebuild_rendered_posts`
  operation to update changed current derivatives of every Post (including
  retained Deleted and HTML-format Posts) on databases that already ran `0045`.
  It drains on database open before the separate bounded refresh, rolls back
  atomically on failure, and does not rewrite authored source or revisions.
  The bounded refresh processes at most 100 ascending active Org/Markdown Post
  IDs per transaction with a durable checkpoint; restart resumes without skipping an uncommitted row, and
  completion is observable. Guard each rendered replacement in the write
  transaction by matching read source, format, and prior rendered value **and by
  proving `deleted_at IS NULL` at write time**. A concurrent edit wins (retry a
  stale candidate once, then leave it to the author write's current renderer or
  fail visibly); a concurrently Deleted Post is skipped without retry or
  mutation. Derive affected public-feed membership from the locked current
  audience/publication state, not from an earlier read. Update derived Media
  references and enqueue the exact affected public-feed events atomically with
  any changed projection. Regenerate cached Syndication Feeds through the
  existing worker and preserve WebSub's outbox behavior; changed public page
  ETags and feed validators must follow changed bytes. Previously cached public
  HTML may remain stale for the existing five-minute `max-age` window; no
  unbounded in-place cache invalidation or history rewrite.
- **Resource gate:** count the UTF-8 bytes of each eligible Org or Markdown
  export's decoded block `<code>` text (not escaped/tokenized HTML). Visit
  blocks in document order; apply the same limits per Post across all eligible
  blocks. Only recognized-language blocks with payload at most 64 KiB may
  attempt parsing. At most 16 attempts and 128 KiB of attempted payload are
  allowed per Post. Each attempt consumes both its byte count and one slot even
  when Tree-sitter recovers malformed syntax, so such blocks cannot cause
  unbounded repeated work. Unknown languages, individually oversized blocks, and
  blocks that would exceed a cumulative limit remain entirely plain escaped code
  without consuming further budget; later smaller eligible blocks may still fit.
  Never partially token-wrap a rejected block. Tree-sitter normally recovers
  malformed syntax into a highlight stream; its output must preserve safe
  decoded exporter text, even when some tokens are styled. If a future
  Tree-sitter API exposes a distinct recoverable authored-input failure, a
  plain-code fallback is permitted and that attempt remains charged. Change the
  host Post renderer to return a typed error for unexpected grammar/query
  initialization, ABI, or infrastructure failures; propagate it through every
  web/AtomPub write, preview and refresh caller rather than panicking or
  returning a success-shaped plain-code fallback. A failed refresh must not
  advance its checkpoint. The owner considers the paired Tree-sitter
  measurements sufficient and does not require another latency threshold or
  language-by-language performance decision. Smoke-test every bundled grammar
  and query in both formats; keep the resource limits above for safety. Report
  stripped release server size and dependency closure for visibility, **not as a
  size gate**: the previously mentioned 5 MiB binary-growth limit was an agent
  mistake, never a user requirement. Go to production only if broad coverage and
  safety/fidelity proofs pass; do not ship partial format or catalog support.

## Acceptance

- A reproducible comparison records real Emacs Lisp/Haskell samples under **Org
  and Markdown**, malformed and markup-looking code, unknown and missing labels,
  exact per-block and cumulative UTF-8-byte/count boundaries (including mixed
  eligible, malformed-but-recovered, unknown and oversized blocks in order), and
  the built-in Tree-sitter grammar/query/alias inventory. Report
  binary/dependency size for visibility; do not reduce format or language
  support to meet a size or latency target.
- HTML `<code>` decoded text equals the same unhighlighted **Org or Markdown**
  export, scalar for scalar, for supported and fallback paths, including
  leading/trailing blank lines, tabs, `<`, `&`, quotes, invalid syntax, and
  script-like text. Markdown indented/unlabeled fences, inline code and raw
  authored HTML are never tokenized. Escaped markup never executes; a fallback
  emits no `j-syn-*` spans. Tests reject `style`, unrelated classes, forged
  token markup outside allowed values, and all new active HTML capabilities.
- Both backends prove web creation and AtomPub create/update for **Org and
  Markdown** preserve native source while current rendered HTML highlights both
  supported languages in each format. Test exact aliases, unknown labels,
  Markdown indented/unlabeled/raw-HTML exclusions, safe malformed recovery and
  over-budget fallback, and capture-category mapping. Markdown tests prove
  mixed-case first-word aliases and trailing info words work, but an alias only
  in a later word or embedded in a larger token does not trigger highlighting.
  For each format, a mixed document proves that valid Post Shortcodes outside
  code keep their behavior and shortcode-looking code text stays literal. All
  active Org and Markdown Posts (including scheduled/private/drafts) undergo
  the bounded refresh. The offline rebuild may update changed current rendered
  derivatives of retained Deleted and HTML-format Posts; it leaves their source
  and every historical Post Revision untouched.
- Dual-backend interruption/resume, repeated refresh, concurrent author edit,
  and concurrent soft-delete tests prove checkpoint/CAS behavior, no lost source
  or unrecorded author change, no mutation/retry of a Deleted Post, unchanged
  `updated_at` and AtomPub Member content ETag, and atomic rendered
  HTML/Media-reference/feed-event state. Public Atom, RSS, and JSON Syndication
  Feeds regenerate with current safe HTML and changed validators where bytes
  change; byte-identical rendered projections create no event. Public permalink
  ETags change when bytes change, with the documented cache-age bound.
- Browser assertions cover all four format/language pairs (Org × Emacs Lisp, Org
  × Haskell, Markdown × Emacs Lisp, Markdown × Haskell) on anonymous public
  permalink, Local, and authenticated Home, plus a narrow viewport with a long
  source line. Capture comparable before/after screenshots for Emacs Lisp and
  Haskell and at least one Markdown/Org format pair on a public permalink and
  Home under a built-in theme. On a **public** route only, verify a Theme
  Package can override one semantic token hook and old packages get readable
  defaults; Home must remain Jaunder-styled regardless of public theme
  selection. Document the additive v1 Style Contract extension.
- An injected invalid grammar/query initialization fails visibly and with its
  typed cause in the Post write/preview path; no partial Post is committed.
  Failure during a refresh preserves the last committed checkpoint and row.
  Expected malformed authored code remains safely preformatted with its decoded
  text unchanged, even if Tree-sitter error recovery still styles some tokens.
- Appropriate focused tests, independent security/architecture review, and the
  repository verification ladder pass before PR review. Production adoption
  includes a proposed ADR draft amending ADR-0079 and ADR-0136 and a matching
  architecture projection; `CONTEXT.md` is reviewed for vocabulary impact.

## Boundaries

- No code execution, unbounded grammar auto-loading, global CSS class admission,
  client-side highlighter, or retroactive rewriting of Post history. The Post
  source and AtomPub Member remain native Org or Markdown; Feeds remain rendered
  HTML, not an editing transport.
- A broad pinned language catalog ships in this issue. No highlighter can
  literally parse every language; record the exact supported labels and aliases
  and preserve escaped plain-code fallback for every unsupported label.
- The experiment alone is not a license to bypass the spec, security, or merge
  approval gates. If it fails, retain only reproducible decision evidence;
  production changes and their ADR are conditional on success.
