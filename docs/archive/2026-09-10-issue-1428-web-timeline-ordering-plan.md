# Web Post Timeline Ordering Implementation Outline

> Execute with `jaunder-iterate`, delegating slices through `jaunder-dispatch`.
> This outline exists because the work changes a both-backend keyset cursor, the
> private CSR wire interface, and cacheable public-projector URL variants.

## Scope

In:

- Publication-time Newest/Oldest ordering for the site, authenticated home,
  User, site-tag, and User-tag web Post timelines.
- Direction-bound timeline cursors, typed server-function requests, public
  projector query handling, shared rendering, and URL-driven CSR interaction.
- The regression, backend-parity, integration, projector, and browser proof
  required by the approved spec.
- The draft ADR and its architecture/product projections.

Out:

- Syndication Feeds, AtomPub Collections, drafts, scheduled management lists,
  and every non-Post listing.
- Persisted preferences, Author ordering, and new pagination models.

## Task outline

- [x] Task 1: Fix publication ordering across storage, endpoints, and projector
  - Contract: add `TimelineOrder` as a `#[text_enum]` closed wire enum; make
    `Page<Row, Cursor = PageCursor>` cursor-generic without changing existing
    `Page<Row>` callers; add `TimelineCursor { published_at, post_id, order }`
    and a cohesive `TimelinePageRequest { order, cursor, limit }`; use
    `Page<RenderedPost, TimelineCursor>` for web timelines only.
  - Contract: the storage `PostCursor` carries publication time, Post ID, and
    order. Conversion rejects a cursor whose embedded order differs from the
    request before any storage query. The five affected web surfaces share the
    four published-list storage methods, each taking order plus the optional
    order-bound cursor.
  - Contract: Newest uses `<` and descending keys; Oldest uses `>` and ascending
    keys. SQL direction and predicates come from closed static fragments, never
    bound or caller-provided SQL. Missing publication time on a published query
    row is an internal invariant error.
  - Contract: migrate the storage interface, all five timeline server functions,
    shared host fetch helpers, `PageSeed` variants, and four public projector
    operations as one clean cutover. One URL-order parser resolves absent or
    unknown values to Newest and `order=oldest` to Oldest; non-timeline seeds
    and endpoints remain unchanged.
  - Verification: reproduce the screenshot defect with creation and publication
    times deliberately opposed; prove dual-backend first/continuation
    boundaries, equal timestamps, both directions, and direct opposite-order
    cursor rejection. Run the existing viewer/visibility matrix in both orders
    on SQLite and PostgreSQL, including anonymous projection, identical admitted
    rows, authenticated `/app`, and unauthenticated `/app` rejection.
  - Verification: projector integration covers direct Newest, Oldest, and
    unknown-order loads for `/`, User, site-tag, and User-tag routes, including
    matching seed order and distinct complete-URL cache representations. Draft,
    scheduled, AtomPub Collection, Syndication Feed, endpoint-registration, and
    public-flow inventory checks remain unchanged or are updated only for the
    new timeline request contract.

- [x] Task 2: Deliver one URL-driven Order control across every web Post
      timeline
  - Depends on: Task 1's request, seed, route-order, storage, and projector
    contracts.
  - Contract: one pure renderer emits the accessible sort-direction icon button
    immediately above the Post list for both projector and CSR markup. Its
    accessible name and tooltip expose the active order and toggle action.
    Reactive code attaches behavior to that shared markup rather than
    maintaining a second markup twin.
  - Contract: one URL builder emits a bare timeline URL for Newest and
    `?order=oldest` for Oldest. Same-origin navigation pushes history; a query
    change advances the timeline generation before fetch, replaces from page
    one, and prevents stale prior-order work from appending. Bare URLs never
    consult account, site, cookie, or browser preference state.
  - Verification: host state/render tests prove icon-button semantics, canonical
    URLs, generation reset, and stale-result rejection. Browser proof exercises
    the order toggle, both directions, Load more, direct public Oldest paint
    with no CSR reorder, unknown fallback, back/forward restoration, bare-URL
    non-persistence while authenticated, and all five affected surfaces.
  - Documentation: keep the spec, draft ADR, `docs/ARCHITECTURE.md`,
    `docs/DESIGN.md`, public-reading flow, endpoint census, and e2e coverage
    matrix consistent with the delivered routes and tests.

## Risk checks

- `PageCursor` and default `Page<Row>` semantics remain intact for excluded
  draft and scheduled paths; only timeline pages use `TimelineCursor`.
- The private server-function input migration updates every generated type,
  registrar entry, mock, fixture, and caller in one clean cutover—no aliases or
  compatibility shims.
- Every exported-symbol change begins with language-server reference discovery;
  cfg-gated or generated callsites not returned by the server are reconciled
  with structural/text search.
- Both SQL directions preserve identical visibility, live-publication, deletion,
  tag, and page-size predicates on SQLite and PostgreSQL.
- Publication-time mutation may reposition a Post during an active walk; tests
  promise no omissions/duplicates only for an unchanged ordered data set.
- Projector and CSR use the same parsed order and pure control markup. No direct
  Oldest load may paint Newest first or reorder after mount.
- Query changes invalidate in-flight replacement and continuation requests
  before they can commit rows from the previous order.
- Feed-discovery URLs and protocol ordering are untouched despite nearby “feed”
  names in code and UI.
- Each task runs the agent feedback gate `devtool run -- cargo xtask check`
  before its commit; final shipping uses the repository's full jaunder
  commit/ship gates.
