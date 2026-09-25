# Org Footnote Export — Implementation Outline

> Execute with `jaunder-iterate`; use `jaunder-dispatch` only for bounded,
> independently contracted work. Planning trigger: the fork/Jaunder interface,
> first-write Post ID, and transactional offline storage rebuild cross
> architecture and storage boundaries. The approved spec is
> `docs/superpowers/specs/2026-09-24-issue-1670-org-footnotes.md`.

## Scope

In: Org footnotes in the maintained exporter, Post-scoped rendering for new and
existing Posts, offline derivative rebuild, public browser and visual proof.

Out: a second parser, Emacs/client behavior, Markdown/HTML footnotes, historical
Post Revision rewriting, and duplicating PR #1671's fork/queue bootstrap.

**Baseline:** PR #1671 merged as `ce88c2c84c534a3e4777fe4db6a6d63d73628ec9`;
this checkout now includes its fork pin and offline queue contract. Work on the
orgize fork belongs to its own repository and must be reviewed and pinned back
into Jaunder; this outline does not authorize working in another checkout from
this session.

## Task outline

- [ ] Capture the baseline, then deliver tested footnote HTML export in
      `jaunder-org/orgize`.
  - Contract: exporter accepts a caller-supplied stable namespace for footnote
    fragment IDs; references/definitions and their back-links share it, while
    labels and displayed first-reference numbers are Post-local. Missing
    references stay literal; unreferenced definitions do not render. Keep the
    parser authoritative and non-footnote export unchanged. Coordinate fork
    implementation in its own authorized checkout; advance Jaunder's fork pins
    and Nix/vendor inputs only after reviewing its diff from #1671's revision.
  - Verification: fork tests for named/inline/repeated/forward/multiline/literal
    cases, semantic notes-section markup, preserved Org links/emphasis in notes,
    and the production source shape; comparable baseline screenshot of a public
    Org Post before presentation mutation.
- [ ] Supply a stable Post-scoped rendering identity on create and update.
  - Contract: the Post ID is available before first persisted render and enters
    Org body export on create and update. The offline rebuild takes the same ID
    through this rendering boundary in Task 3. Rendering retains the inseparable
    sanitized HTML/Media-reference aggregate; two identical Post bodies never
    share HTML anchors on a page.
  - Verification: focused host/storage tests for new and updated Posts,
    identical bodies with distinct IDs, sanitizer-preserved forward/back links,
    unchanged non-Org rendering and unchanged native AtomPub source.
- [ ] Rebuild all current Posts and dependent public projections offline.
  - Contract: add the next SQLx migration on each backend to enqueue the
    existing `rebuild_rendered_posts` operation, as `0045` did. Reuse its closed
    dispatcher and full current-Post pass (including retained Deleted Posts)
    with each Post's stable ID; do not create a second operation. Replace exact
    sanitized HTML-derived Media references, invalidate affected Syndication
    Feed cache/validators and enqueue `feed_events` atomically with completion
    of the pending operation. Preserve source, timestamps and historical Post
    Revisions. Respect the offline-only ADR-0092 exception, never a request-time
    unbounded write transaction; allocate the next migration numbers after
    #1671's `0045`.
  - Verification: `#[apply(backends)]` tests for SQLite/PostgreSQL old rows,
    including retained Deleted Posts and non-Org Posts, Post-ID-scoped links
    matching newly created and updated Posts, media inside referenced notes,
    feed cache/notification change and unaffected Posts. Injected failure rolls
    all effects back; retry produces the final HTML and exactly one committed
    notification per affected feed, with no pending work and no repeat effects
    on the next open.
- [ ] Demonstrate published footnotes in the browser and finish visual proof.
  - Contract: the public Post fixture includes forward/repeated references and a
    wrapped, linked definition; UI anchors navigate within their Post even on a
    multi-Post page.
  - Verification: focused `cargo xtask e2e-local <spec-or-file:line>`, relevant
    hermetic backend/browser lane when parity depends on it, and a before/after
    screenshot pair with identical viewport, theme, authentication, and fixture
    data state. Follow the repository's one-boot-per-page e2e discipline.

## Risk checks

- Keep Cargo, tools, flake input/lock, deny source policy and Nix vendor inputs
  on the same fork revision.
- Do not start the presentation mutation before the before screenshot is
  captured. Do not assume an external fork checkout is this session's working
  tree.
- Verify Post ID allocation/order against both dialects and every create path;
  preserve rendering/sanitization and ADR-0090's inseparable Media-reference
  contract.
- Do not overwrite #1671's pending migrations or rebuild old derivative data via
  a semantic Post edit. Keep reviewer-visible source and historical revisions
  unchanged.
- For each completed slice, use focused proof and `jaunder-commit` (stage the
  checked tree, then commit through precommit); broad gate only when the
  boundary warrants it. No unapproved lint suppressions or `Co-Authored-By`
  trailers.
