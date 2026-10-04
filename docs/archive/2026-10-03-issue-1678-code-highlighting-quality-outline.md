# Issue #1678 — code highlighting quality implementation outline

> Execute with `jaunder-iterate`; use `jaunder-dispatch` only for a bounded
> task. The approved
> [spec](2026-10-03-issue-1678-code-highlighting-quality-spec.md) is
> authoritative. This outline exists because semantic-token changes cross the
> sanitizer/Style Contract security boundary and refreshing existing Post
> projections crosses migrations, both storage backends, concurrent startup and
> public Syndication Feed events.

## Scope

In: quality throughout the existing pinned language catalog, class-only
themeable markup, preservation of source and rendering budgets, durable refresh
of current projections, and proof on real Post surfaces.

Out: narrowing the catalog, browser-side highlighting, author plugins, arbitrary
authored classes or inline styles, rewriting source/revisions, changing support
for unlabeled/HTML/unknown code, new visual-snapshot variants.

## Task outline

- [x] **1. Establish the quality oracle and choose a rendering path.** Build a
      checked-in corpus manifest with at least one real, role-annotated sample
      per supported grammar; every canonical label and alias must work through
      both Org and Markdown. Include the two linked current production excerpts
      with their actual exporter text and preserve the originally reported
      malformed Haskell bytes separately alongside adversarial source and exact
      decoded-text expectations. Measure current versus candidate capture/HTML
      quality using Tree-sitter CLI `highlight` and, where useful, an
      independent toolkit as diagnostic comparators, not as unreviewed
      production replacements or exact-output oracles. Investigate material
      capture/appearance differences on the same corpus; shared queries limit
      how independent the CLI can be. Record unsupported-language behavior
      alongside comparative capture quality and the choice before changing
      production rendering; capture comparable unchanged presentation baselines
      for the spec's representative routes, states and viewports before any
      visual mutation.
  - Contract: an executable, reviewable corpus identifies expected token
    ranges/roles per grammar, so later renderer and sanitizer work has one
    stable quality oracle. Preserve catalog/aliases and existing budgets. If no
    candidate can meet the corpus safely, stop for a design decision rather than
    shrinking coverage or relaxing a gate.
  - Verification: the historical malformed Haskell source reproduces the
    whole-line cascade in reference queries, while corrected live Haskell gains
    distinct token roles; Emacs Lisp still tests missing call heads. Failures
    name the grammar and the expected-versus-actual role/range. Exact text,
    catalog/alias census and unknown-label plain-code evidence are inspectable.
    The comparison records material differences and whether they reveal missing
    or misleading Jaunder markup, not merely that two tools serialize different
    HTML. Baseline screenshots are transient review artifacts, not committed
    `@visual` snapshots. The pinned CLI was absent from the devShell; the
    comparison instead exercised the pinned highlighting API directly and
    records this limitation rather than claiming independent-toolkit
    confirmation.
- [x] **2. Establish the closed semantic-token contract.** Map reliable query
      captures into the smallest reviewed additive set of semantic hooks
      required by the corpus. Preserve class-only host-rendered spans and
      compatible public Theme Package overrides; Home stays built-in. If
      comparative evidence requires a different engine than the #1655 draft's
      `tree-sitter-highlight`-only decision, write a new numberless ADR
      documenting the explicit contradiction and successor; otherwise document
      the revised closed hooks and security decision in a numberless ADR.
      Project the resulting decision into `docs/ARCHITECTURE.md` in the same
      change and consider `CONTEXT.md`.
  - Contract: `common::render::sanitize` strips unsupported hooks, event/active
    attributes and styles. Permitted author-forged hooks can survive but remain
    visually inert outside Post-body `pre code`; all token colors have safe
    built-in defaults and can be overridden by public themes without breaking
    existing Style Contract v1 packages. No raw capture name is a sanitizer
    privilege.
  - Verification: adversarial sanitizer/security tests, built-in color and
    custom-theme override checks; no unsupported class colors outside code.
    Check docs links/ADR projection in the normal gate.
- [x] **3. Deliver catalog-wide quality at the shared host projection.** Make
      each corpus entry pass in both Org and Markdown through the integrated
      host renderer, correcting misleading query captures and maintaining
      meaningful role distinctions rather than painting whole blocks one color.
      Keep native Post source, exact decoded exporter text, eligibility rules,
      typed unexpected failures, budget/fallback rules and AtomPub Member
      content unchanged. Tests and integration checks must cover every supported
      alias and representative captured roles, not just initialization smoke
      tests.
  - Contract: new Posts and preview/render callers share one reviewed bounded
    capture-to-hook mapping. Catalog additions must bring their own quality
    fixture and role assertions. Do not advance to existing-Post refresh until
    safety, fidelity and catalog quality pass.
  - Verification: focused host red/green tests
    (`devtool run -- cargo xtask test-local -- -p host code_highlight`), common
    sanitizer tests, and dual-backend web/AtomPub create/update checks where
    rendering behavior crosses storage; broader gate only at integration
    boundaries.
- [x] **4. Re-admit completed installations and refresh stored projections
      safely.** Add one SQLx migration per backend that only enqueues the
      existing offline `rebuild_rendered_posts` operation, even after earlier
      #1655/#1670 requests drained. Do not dispatch a version-specific Rust
      operation, render inside SQL, or reset the separate version-1 bounded
      refresh checkpoint. The queue drain runs before traffic and updates only
      changed current derivatives of active, retained Deleted and HTML-format
      Posts without altering source, timestamps, revisions or AtomPub Member
      ETags. Its transaction commits changed projection, Media references,
      affected public Syndication Feed outbox events and queue deletion together;
      the worker regenerates feed bytes and validators asynchronously.
  - Contract: old writers drained; a failure leaves the queue row for retry,
    repeated startup cannot enqueue twice, and a byte-identical projection
    produces no new feed event. The existing bounded refresh remains available
    for installations with unfinished version-1 progress but is not rearmed by
    this ordinary re-render request.
  - Verification: `#[apply(backends)]` migration-from-already-drained,
    idempotent open, rollback/retry and no-op checks, including active Org and
    Markdown, retained Deleted and HTML-format Posts. Assert unchanged bounded
    progress, source/revision/timestamp/ETag, and correct outbox events; consume
    affected events and prove regenerated public feed bytes and validators
    change when representation bytes change. Keep the existing bounded-refresh
    tests as the proof of that independent version-1 path.
- [x] **5. Prove public results and prepare the reviewed change.** Verify new
      and refreshed code on public permalink, Local and authenticated Home,
      including unknown-language fallback and public Syndication Feeds; inspect
      representative built-in/custom theme rendering and narrow width. Capture
      matched after images against Task 1's transient before images and surface
      pairs for human judgment. Run focused e2e first, then the appropriate
      broad diagnostic and normal commit/PR gates under `jaunder-iterate`.
  - Contract: only existing standard `@visual` baselines may change if their
    owning rendering truly changes; do not add mobile/theme variants.
    Presentation changes invalidate the after image until recaptured. New ADRs
    and `docs/ARCHITECTURE.md` land with the feature, not as post-merge
    documentation.
  - Verification: focused `devtool run -- cargo xtask e2e-local posts.spec.ts`
    (or the narrower changed file/line), browser markup assertions and transient
    comparable screenshots. On a stable Local/Home behavior test with
    highlighted code mounted, run `expectAccessible(page)` under
    `@accessibility` without exclusions; keep manual legibility review for
    permalinks and narrow widths. Include the repo's dual-backend/broad gate
    evidence before the merge checkpoint.

## Risk checks

- The #1655 proposed ADR requires `tree-sitter-highlight` only. Replacing it
  contradicts that choice: cite and supersede explicitly with a new draft, never
  silently edit the older Decision or weaken the `RenderedHtml` trust boundary.
- The prior offline rebuild and bounded Post refresh may already be complete on
  production databases; migrating only new installs or resetting a cursor
  without re-enqueueing would leave old Posts stale. Test this state before
  declaring success.
- Existing custom Theme Packages, source/AtomPub fidelity, sanitizer allowlists
  and public cache/feed validators are load-bearing. No fresh palette may color
  an author-forged span outside Post-body code.
- Do not broaden the existing `@visual` snapshot population or suppress tests,
  coverage or lint without the owner's explicit approval. Commit only the staged
  tree that was gated; no `Co-Authored-By` trailer.
