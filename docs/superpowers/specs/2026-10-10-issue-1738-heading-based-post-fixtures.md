# Heading-based Post fixtures (#1738)

## Outcome

Heading-based seeded Posts model ordinary publishing's Post Title and Rendered
Title semantics. Home visual evidence presents those Posts as titled, without an
untitled `#` permalink fallback.

## Load-bearing decisions

- Shared heading-based Markdown seeds use the existing publishing title
  derivation and title-rendering authorities; do not duplicate their rules.
- Preserve deterministic fixture slugs, authored bodies, counts, ordering
  assumptions, audiences, publication states, and batched-write behavior.
- Audit shared seed consumers and preserve deliberately untitled fixtures and
  their explicit permalink-fallback coverage. A body heading is not evidence
  that a Post Title or Rendered Title was persisted.
- Production rendering, title derivation, and slug behavior remain unchanged. No
  new architectural decision or implementation outline is required.

## Acceptance

- Reproduce the heading-only fixture discrepancy and compare with ordinary web
  creation; inspect persisted title projections through a supported owner read
  surface, not solely visible body text.
- Regression coverage proves the shared seed persists the derived Post Title and
  inline Rendered Title, retains authored body and deterministic slug, and
  preserves absent titles when no title source exists. Storage-backed proof
  covers both SQLite and PostgreSQL.
- Browser proof checks heading-based Home fixtures have a linked rendered title
  without the `Permalink` fallback; genuinely untitled published Posts retain
  the fallback and its existing accessible navigation proof.
- Comparable Home Before/After screenshots use the existing deterministic visual
  fixture, Studio styling, and the canonical 1280×720 Linux viewport. Review all
  changed canonical PNGs, update only through the supported snapshot updater,
  then run the normal non-update browser suite. Logs and review evidence stay in
  ignored run/session storage; actively compared baselines are committed.

## Boundaries

Fixture naming, affected regression coverage, and intentional baseline updates
only. No production-data remediation, schema/API change, body-heading removal,
presentation redesign, snapshot tolerances, masks, or disabled checks.
