# Issue #730 — authed-cls row measurement must not sample the shared timeline

## Outcome

`authed-cls.spec.ts` keeps proving that the owner-only action column is additive
across projector paint → wasm mount, but the row-position sample is taken on a
page where the measured row cannot be displaced by other tests' content drift.
The test no longer measures a post row's absolute position on the shared `/`
timeline.

## Load-bearing decisions

- Treat `/` as unsafe for absolute post-row y-position assertions: it is the
  shared site-wide timeline, other tests' Posts appear there, new Posts are
  prepended, and the projector response can differ from the live CSR refetch.
- Preserve the behavioral intent of `authed-cls`: the owner's `.j-post-acts`
  column may appear after mount, but `.j-post-head` and `.j-post-body` must not
  move as a result.
- Scope the probe to a route populated only by this test's unique Username, or
  otherwise make the row measurement relative to a stable anchor. A row sample
  on `/` is out of bounds for this fix.
- Do not change `expectNoShiftAcrossMount`; issue #730 is about the probe
  target, not the shared mount-shift helper.
- Keep `tolerancePx: 0` unless a new, documented validate-matrix result proves a
  browser-specific sub-pixel tolerance is required.

## Acceptance

- The delivered diff removes the absolute `.j-post-head` / `.j-post-body` row
  measurement on `/` from `end2end/tests/authed-cls.spec.ts`.
- The test still asserts that `.j-post-acts` becomes visible after mount,
  proving the reactive owner-only affordance mounted.
- The test still measures both `.j-post-head` and `.j-post-body` across mount on
  a page where this test's Post is the only matching row.
- A focused local e2e run covering `authed-cls.spec.ts` passes.

## Boundaries

- No changes to `end2end/tests/layout-shift.ts`.
- No broad rewrite of CLS coverage or `timeline-cls.spec.ts`.
- No retry, timeout, or tolerance loosening as a substitute for removing the
  content-drift source.
