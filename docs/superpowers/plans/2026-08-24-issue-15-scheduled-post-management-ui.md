# Issue #15 Scheduled Post Management UI — Implementation Outline

> Execute with `jaunder-iterate` and `jaunder-dispatch`. This outline exists
> because the work crosses storage/server/web/e2e boundaries and will be split
> across agents with stable contracts.

## Scope

In:

- A dedicated authenticated web route for Scheduled Post management.
- A scheduled-only server listing contract derived from `published_at > now`.
- Navigation from authenticated UI to the scheduled surface.
- Browser and server coverage for list filtering, ordering, editor handoff,
  reschedule, and pullback.

Out:

- AtomPub, Emacs, feed-worker, scheduler, or public visibility policy changes.
- Inline/bulk schedule mutation controls on the scheduled list.
- Redesigning `/drafts` beyond preserving its existing mixed unpublished
  behavior.

## Task outline

- [x] Task 1: Add scheduled-only listing contract
  - Contract: expose a web/server listing equivalent to the existing unpublished
    list shape, but filtered to Posts owned by the authenticated user with
    `published_at > now`, excluding drafts, live Posts, and Deleted Posts; order
    by `published_at ASC, post_id ASC`.
  - Contract: keep lifecycle truth in `published_at`; do not add schema,
    persisted status, or an alternate schedule flag.
  - Verification: focused server/web test proving scheduled-only filtering,
    owner isolation, Deleted Post exclusion, and ordering tie-breaker behavior.

- [x] Task 2: Add scheduled management route and navigation
  - Contract: route name/path is a dedicated scheduled management destination
    distinct from `/drafts`; authenticated navigation exposes it using the
    existing sidebar/navigation conventions.
  - Contract: direct unauthenticated access uses the same denial/sign-in
    behavior as comparable authenticated web pages before list data is rendered.
  - Contract: rows display title or fallback label, scheduled go-live time, and
    an edit link/path into the existing editor; no inline reschedule/pullback
    mutation controls.
  - Verification: focused web/browser proof that the route denies
    unauthenticated direct access before rows can render, renders scheduled rows
    only for an authenticated author, shows the empty state, and opens the
    existing editor from a row.

- [x] Task 3: Wire end-to-end management flows
  - Contract: reuse existing editor schedule controls for both mutations:
    reschedule updates the scheduled list time; clear schedule or save draft
    removes the Post from the scheduled list and leaves it reachable from
    Drafts.
  - Contract: preserve ADR-0027 public visibility behavior; scheduled Posts
    remain public-hidden until due and author-visible.
  - Verification: focused `devtool run -- cargo xtask e2e-local posts.spec.ts`
    or a narrower positional posts spec line covering scheduled list → editor →
    reschedule/pullback → scheduled list update.

## Risk checks

- The scheduled-list query must use an explicit `now`; no clock reads inside
  storage queries.
- The scheduled management route must not become a new public read surface.
- The existing `/drafts` mixed unpublished behavior and scheduled badge tests
  must keep passing.
- Ordering assertions must create two Scheduled Posts with the same scheduled
  time to pin the `post_id` tie-breaker.
- Any new server function URL must follow ADR-0082's `/api/<vertical>/<op>`
  namespace convention.
- Any new component file must follow ADR-0070's host/wasm file split and
  `CONTRIBUTING.md`'s `mod.rs` surface rule.
