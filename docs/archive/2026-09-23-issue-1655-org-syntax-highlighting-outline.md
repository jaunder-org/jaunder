# Issue #1655 — Org and Markdown syntax highlighting implementation outline

> Execute with `jaunder-iterate`; use `jaunder-dispatch` only for an
> individually bounded task. The approved
> [spec](2026-09-23-issue-1655-org-syntax-highlighting-spec.md) is
> authoritative. This outline exists because token markup changes the
> sanitizer/Style Contract and refreshing stored projections crosses
> dual-backend lifecycle, concurrency, and feed-outbox boundaries.

## Scope

In: a broad pinned built-in language catalog including Emacs Lisp and Haskell,
safe host rendering, additive public semantic token hooks, presentation-only
refresh of current active Org and Markdown Post projections, public Syndication
Feed coherence, browser proof, and the conditional proposed ADR/architecture
projection.

Out: runtime grammar downloads or author plugins, Markdown indented/unlabeled
fences and inline code, highlighting author-supplied raw HTML, client-side/WASM
highlighting, arbitrary authored CSS/style admission, edits to native source or
historical Post Revisions, an unbounded rewrite of legacy content.

## Task outline

- [x] **1. Compare candidate viability and capture an unchanged baseline.** Use
      the same real Emacs Lisp/Haskell and adversarial fixtures under both Org
      and Markdown, including malformed and 64 KiB blocks; record highlight
      quality, parser/query/alias compatibility and exact-text fidelity with a
      small isolated harness. Capture the unmodified server's stripped release
      binary and paired baseline timings through both current production render
      seams under the pinned devShell. Candidate cost estimates are **not** the
      final go/no verdict: only the integrated seam can measure that (Task 4).
      If the catalog cannot highlight useful source safely, stop and request a
      design revision before touching the sanitizer.
  - Contract for later tasks: candidate host-only highlighter, broad pinned
    catalog, bounded alias registry, capture-to-category mapping, fixed fixture
    corpus and preserved baseline artifacts under identical build/measurement
    settings.
  - Verification: rerunnable fixture/harness tests and the recorded baseline
    command/results; no production Post render change.
- [x] **2. Establish the sanitized semantic-token contract.** Add exactly the
      spec's ten `j-syn-*` span classes to the common-owned sanitizer policy and
      corresponding narrowly scoped built-in CSS defaults; make public custom
      theme overrides an additive Style Contract v1 surface. Keep Home
      Jaunder-styled. Document the deliberate ADR-0079 expansion in a proposed
      ADR draft and project it into `docs/ARCHITECTURE.md` with descriptive path
      citations.
  - Contract: only `<span class="j-syn-<category>">` and existing `language-*`
    code/pre classes survive; authors cannot inject other classes or styles. CSS
    token colors apply only under Post-body `pre code`; unsupported token names
    are unstyled.
  - Verification: common sanitizer tests with forged raw HTML, arbitrary
    classes/styles/event attributes and legal markup; theme override/default
    proof on the correct public/Home surfaces.
- [x] **3. Render eligible blocks through the shared host Post projection.**
      Insert the chosen highlighter at both the Org source-block and Markdown
      fenced-code exporter boundaries without altering either authored body or
      AtomPub Members. Use only tree-sitter-highlight with pinned grammars and
      compatible highlight queries. Ship a broad built-in catalog with a
      reviewable alias inventory; no runtime grammar downloads. Implement
      supported aliases, first-info-word matching for Markdown fences, and
      UTF-8-byte accounting over the exporter's decoded `<code>` text plus
      per-Post attempt accounting; keep Markdown indented/unlabeled code, inline
      code and raw authored HTML unhighlighted. Preserve valid Post Shortcodes
      outside code and shortcode-looking text inside it in both formats.
      Unknown and over-limit eligible blocks remain escaped plain code. Malformed
      source uses Tree-sitter recovery and preserves the exporter's text.
  - Contract: the decoded `<code>` text from every highlighted/fallback result
    equals the same unhighlighted exporter for that format, scalar-for-scalar;
    no supported capture escapes the ten-category vocabulary. Change
    `host::render::render` and `render_post` to return a typed `Result` for
    unexpected registry/query initialization, ABI or infrastructure failures;
    propagate that error through each storage post-service and web/AtomPub
    create/update, preview, and maintenance caller. Do not log an unexpected
    failure and return a success-shaped fallback.
  - Verification: focused host unit tests for all four regression
    language/format pairs, catalog smoke tests covering every approved grammar
    in both formats, Markdown first-info-word/mixed-case/trailing-word
    eligibility, with supported aliases in later words or embedded in larger
    tokens remaining unhighlighted, plus indented/unlabeled/inline/raw-HTML
    exclusions, mixed shortcode/code documents, aliases, order-sensitive
    cumulative limits, malformed/HTML-looking text, sanitizer round-trip, and
    injected unexpected query/registry error propagation; backend-parity
    web/AtomPub create/update and web preview tests for both formats'
    native-source fidelity and typed errors without a partial write.
- [x] **4. Verify integrated safety and broad coverage.** Smoke-test every
      bundled Tree-sitter grammar/query in both exporters, verify representative
      colored tokens, source fidelity, sanitizer constraints and the bounded
      resource behavior. The owner accepted the representative Tree-sitter
      latency evidence as sufficient; no new performance threshold gates this
      implementation. Record stripped release server and dependency closure
      sizes for visibility only. **If a safety, fidelity, or coverage gate
      fails, fix it rather than retaining a partial one-language or one-format
      feature.**
  - Contract: validated Tree-sitter-only renderer and broad pinned inventory
    authorize Tasks 5–6.
  - Verification: catalog smoke tests, source-fidelity fixtures, security tests
    and release size report; no new performance approval step.
- [x] **5. Refresh existing current projections without an author revision.**
      SQL migrations in both backends create/seed a versioned refresh-progress
      row only; they do not render HTML. After migrations and before the new
      server mounts its router or starts feed/WebSub workers, run the
      presentation-only refresh to completion (or fail startup visibly). The
      versioned row exposes current version, committed cursor, and completed
      status. Each batch selects at most 100 ascending active Org or Markdown
      Post IDs (never HTML-format Posts) after the committed cursor; within a
      write transaction, lock/recheck progress and current Post state, CAS
      source/format/rendered value plus `deleted_at IS NULL`, and commit changed
      HTML, Media references, exact affected-feed events and checkpoint
      atomically. Coordinate concurrent PostgreSQL startups through the locked
      progress row (SQLite serializes writers); a second starter re-reads
      committed progress rather than applying a stale batch. Rolling deployments
      must drain old-version Post writers first so they cannot restore an
      obsolete projection after the checkpoint. Preserve timestamps, source,
      AtomPub ETag and historical Post Revisions; document the narrow ADR-0136
      exception in the proposed ADR and project it into `docs/ARCHITECTURE.md`.
  - Contract: no-op/Deleted rows advance the checkpoint without events. A stale
    candidate is refetched and rerendered once; a second stale result or
    unexpected render/storage failure aborts visibly without advancing past that
    ID. Reuse/extract existing affected-feed calculation and derive public
    membership from the locked current Post: draft, private and future-scheduled
    rows produce no event; currently public rows reach the affected Site, User,
    Site Tag and User Tag Atom/RSS/JSON paths. Resume never skips an uncommitted
    row; completion is persisted and observable.
  - Verification: dual-backend tests for active Org and Markdown
    public/private/draft/scheduled/Deleted rows plus untouched HTML-format rows,
    version/cursor resume/idempotence, concurrent startup/edit/deletion,
    second-stale failure, atomic rollback on derived-state failure, unchanged
    timestamp/ETag/revision, Media-reference consistency and exact feed events.
- [x] **6. Demonstrate the end-to-end public result.** Verify newly created and
      refreshed Org and Markdown Posts in the public permalink, Local and Home,
      all four format/language combinations and a plain fallback at a narrow
      viewport; check public Atom/RSS/JSON Syndication Feeds and regenerated
      validators/WebSub outbox from the refreshed projection. Capture comparable
      before/after screenshots for both languages and at least one same-language
      Org/Markdown pair under built-in styling and a public Theme Package
      override, with Home unaffected by the package. Commit the report, tests,
      draft ADR and architecture projection with the implementation.
  - Contract: public HTML/ETags change with changed rendered bytes after
    refresh, while caches may honor existing five-minute max-age; AtomPub
    Member/native source stays unchanged.
  - Verification: focused `devtool run -- cargo xtask e2e-local <spec:line>` and
    dual-backend `test-local` proof, then applicable hook/gates and final
    independent Standards/Spec/security reviews. Stop before merge approval
    under `jaunder-ship`.

## Risk checks

- No generic `RenderedHtml` trust door or read-time sanitizing; the `common`
  sanitizer still rejects arbitrary authored styling and executable markup. The
  browser WASM closure does not gain the host highlighter or catalog.
- A refresh must not substitute for an author mutation: no new Revision,
  timestamp bump, or AtomPub content-ETag change; CAS includes
  `deleted_at IS NULL` at write time, with feed membership derived from locked
  current state.
- Keep lifecycle write, Media-reference replacement and affected-feed event
  creation atomic for both databases; no silent success if one derived operation
  fails. Ensure the refresh does not leave a feed cache with old HTML
  indefinitely or falsely claim immediate global CDN invalidation.
- ADR-0079 and ADR-0136 are explicitly amended by a numberless proposed draft
  and matching architecture view if production is adopted. Check `CONTEXT.md`
  for terminology; don't edit generated `docs/README.md` or promote the ADR on
  this branch.
- Each completed task has focused proof and is committed through
  `jaunder-commit`'s staged pre-commit gate. No lint suppression without
  explicit approval; no co-author trailer. `jaunder-ship` handles final review,
  branch currency, push, PR and CI monitoring, then halts before merge.
