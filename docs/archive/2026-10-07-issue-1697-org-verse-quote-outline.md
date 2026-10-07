# Org verse and quote rendering implementation outline (#1697)

> Execute with jaunder-iterate; use jaunder-dispatch for bounded delegation if
> useful. Trigger: paired upgrade migrations and stored-projection correctness.
> Contract: [approved spec](../specs/2026-10-07-issue-1697-org-verse-quote.md).

## Scope

In: Emacs-compatible verse/quote body rendering, repeatable offline rebuild
request on both backends, regression and browser/visual proof.

Out: other Org block semantics, parser replacement, historical rewrites, live
production operations, and Theme Package contract changes.

## Task outline

- [x] Task 1: Correct sanitized Org verse/quote output.
  - Before presentation mutations, capture the deterministic published fixture
    on its signed-out public permalink in the built-in theme at 390 px and 1280
    px widths; retain reproduction data for Task 3.
  - Contract: `host::render` remains the canonical renderer. Preserve paragraph
    and explicit-break verse semantics, relative indentation, inline content,
    multi-paragraph quotes, and the existing sanitizer/shortcode boundary.
    Prefer a bounded exporter correction over changing the parser; any fork
    revision change must preserve the documented audited pin/lock contract.
  - Verification: focused host regressions prove the collapsed-verse symptom
    before fixing it, then semantic paragraph/`<br>` output (not `<pre>`), blank
    lines, two indentation depths, emphasis/links, delimiter case, quote
    paragraphs, safe escaping, inert shortcodes, and unchanged prose/literals.
    Use `devtool run -- cargo xtask test-local -- -p host org_` for focused
    proof.
- [x] Task 2: Upgrade existing current Post projections on both backends.
  - Depends on Task 1's corrected renderer. Contract: matching next available
    SQLite/PostgreSQL migrations enqueue `rebuild_rendered_posts`; keep its
    existing atomic queue drain, directory locks, and retry behavior unchanged.
    Recheck migration numbering against current main before final integration.
  - Verification: `#[apply(backends)]` upgrade tests prove stale current Org
    rendering is corrected even after earlier rebuild requests completed,
    including Deleted Posts. Preserve source, edit times, revisions, and AtomPub
    Member content ETags; verify Media references and affected public
    Syndication Feed events, no duplicate event for unchanged rows, and no
    repeat on reopen. Reuse existing focused rollback/locking proof when the
    handler is unchanged. Run the focused storage migration suite through
    `devtool run -- cargo xtask test-local -- -p storage migrations::tests`.
    Update the architecture's migration/rebuild description with this slice.
- [x] Task 3: Demonstrate the published browser result and complete conformance.
  - Depends on Tasks 1–2. Contract: a deterministic published Org fixture uses
    the same canonical output on Local and its public permalink, with no
    test-only renderer or new application endpoint.
  - Verification: focused Playwright flow proves verse layout and inline styling
    plus semantic quote paragraphs on both routes at both spec viewports.
    Capture final permalink screenshots with Task 1's identical fixture/theme/
    authentication conditions; surface paired Before/After images together.
    Re-run the focused browser proof after presentation-affecting fixes; use
    existing project review, commit, push, and CI authorities for final
    delivery.

## Risk checks

- Keep renderer output sanitized; verse is prose, not a raw-HTML bypass, and
  nested shortcode-looking text must stay inert.
- A newly numbered enqueue migration, not editing an applied SQL migration,
  supplies the rebuild to upgraded databases; both dialects must match.
- Rebuild only derived current projections under the existing offline boundary;
  preserve historical rendered bytes and mutation identity. Changed current HTML
  must not leave stale Media or public Syndication Feed projections.
- Capture the baseline before visual mutations; keep transient screenshots out
  of committed visual baselines. No live production rebuild is part of
  execution.
