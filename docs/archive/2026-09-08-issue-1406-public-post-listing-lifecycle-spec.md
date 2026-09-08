# Public Post Listing Lifecycle

Issue: #1406

## Outcome

The public User timeline, site-tag listing, and User-tag listing use one
concrete, route-aware Post listing lifecycle. The consolidation preserves their
current validation, seed adoption, presentation, mutation revalidation,
pagination, and rendering behavior while preventing asynchronous work from an
obsolete listing generation from changing the current listing.

## Load-bearing decisions

- One concrete lifecycle owns the behavior shared by the three public listing
  routes. This is not a reusable callback-driven list framework.
- The lifecycle is keyed by an exhaustive typed route with three variants: a
  User profile, a site-wide Tag, and a User-scoped Tag.
- Route parameters continue to cross the existing validated `Username` and `Tag`
  parsing chokepoints. Malformed parameters never reach a public listing
  endpoint.
- When both parts of a User-tag route are malformed, Username validation keeps
  its existing precedence over Tag validation.
- Route-specific endpoint selection remains typed: User listings fetch by User,
  site-tag listings fetch by Tag, and User-tag listings fetch by User and Tag.
  The consolidation does not introduce string dispatch or callback-injected
  fetch policy.
- Route-specific presentation is exhaustive typed data owned beside the route,
  not conditionals spread through lifecycle wiring. It includes the title,
  discovery surface, row `TagCtx`, empty text, and the presence of RSD and
  subscription controls.
- User profile presentation retains its User discovery, AtomPub RSD,
  subscription control, and User-scoped `TagCtx`.
- Site-tag presentation retains its site-tag discovery, site-wide `TagCtx`, and
  tag-specific empty text.
- User-tag presentation retains its User-tag discovery, User-scoped `TagCtx`,
  and tag-specific empty text.
- A `PageSeed` is adopted only when its variant and every typed route value
  match the current route. An absent, malformed-route, wrong-kind, or mismatched
  seed cannot initialize the listing.
- Post mutation settlement continues to revalidate the authoritative first page
  through the standard `Invalidator` idiom. Local row insertion or replacement
  does not substitute for revalidation.
- `MutationOutcome::Confirmed` and `MutationOutcome::CommitIndeterminate` both
  notify the invalidator and revalidate. A rollback-confirmed operation failure
  does not; an indeterminate result remains visibly error-like rather than
  claiming whether the write committed.
- A successfully fetched first page replaces the visible listing, including any
  rows previously appended by Load more, and establishes the replacement cursor
  and `has_more` state.
- Every route incarnation and mutation-driven revalidation begins a new listing
  generation before its asynchronous fetch starts.
- Load-more work captures the generation for which it was dispatched. Its
  completion may append rows or report a pagination failure only while that
  generation remains current.
- A stale Load-more completion is ignored after route replacement or mutation
  revalidation, whether it settles before or after the replacement first-page
  request.
- A Load-more completion that settles before a new generation begins may update
  the old listing normally; the later first-page result still supersedes that
  complete old state.
- Current-generation Load more keeps cursor pagination, appends successful rows
  in order, installs the returned cursor and `has_more` value, and prevents
  duplicate concurrent dispatch.
- Current-generation Load-more failure retains the already rendered first page,
  appended rows, and retry capability.
- Destination theme adoption remains coordinated with committing the fetched
  destination. Superseded theme results cannot update the current route.
- The shared public shell and its sibling route-specific chrome retain the Style
  Contract and current paint-transition behavior.
- Host-testable route, seed, and transition decisions remain outside wasm-only
  component wiring. Leptos `Resource`, `Effect`, spawning, and view construction
  remain inside the wasm-facing file boundary.
- The public Post listing continues to use its existing flat timeline state. The
  consolidation does not introduce a keyed reactive `Store` without mutable
  per-row state that requires one.
- No new ADR is required: the design applies ADR-0040, ADR-0041, ADR-0060,
  ADR-0061, ADR-0063, ADR-0070, ADR-0082, ADR-0083, ADR-0093, and ADR-0164
  without changing their decisions.

## Acceptance

- `UserTimelinePage`, `SiteTagPage`, and `UserTagPage` delegate route
  validation, seed adoption, initial loading, mutation revalidation, first-page
  replacement, and pagination transitions to the one concrete lifecycle.
- No route retains an independent copy of that lifecycle or introduces a second
  generic listing abstraction.
- Each valid route calls only its existing public endpoint with typed validated
  values; malformed route values make no endpoint request.
- Host tests prove Username-first User-tag validation and rejection of absent,
  wrong-kind, and mismatched-route seeds.
- Host tests prove that first-page application replaces prior pagination state
  and establishes the returned cursor and `has_more` value.
- Host tests prove that a stale Load-more success cannot append after either a
  mutation revalidation or route replacement begins.
- Host tests prove that a stale Load-more failure cannot overwrite the status of
  the replacement listing.
- Host tests prove that current-generation Load-more success appends rows and
  advances pagination, while failure retains rendered rows and permits retry.
- Mutation coverage proves that confirmed and commit-indeterminate outcomes
  invalidate the listing, rollback-confirmed operation failures do not, and
  successful revalidation supersedes the prior visible pagination state.
- Existing browser coverage continues to prove User, site-tag, and User-tag
  titles, discovery links, RSD and subscription controls, `TagCtx` links, empty
  states, Load more behavior, and mutation-driven listing refresh.
- Existing layout-shift and paint-transition coverage continues to pass for all
  three routes, demonstrating that the shared shell still satisfies the Style
  Contract.
- Host and wasm checks demonstrate the existing target boundary: decision and
  transition logic compiles and is tested on the host, while browser wiring
  remains wasm-only.

## Boundaries

- This work changes no public endpoint path, request or response payload, cursor
  format, PageSeed wire shape, projector behavior, or theme-resolution policy.
- It changes no Post mutation response to carry rendered listing rows and adds
  no optimistic local reconciliation policy.
- It does not alter malformed-route error presentation, first-page fetch-error
  presentation, empty-state wording, or route-specific chrome.
- It does not change Post ordering, visibility, Tag membership, pagination page
  size, or concurrent-write storage semantics.
- It does not move server functions, merge the host/wasm file split, render
  reactive components on the server, or add server-side UI dependencies.
- It adds no generic callback framework, compatibility adapter, deprecated
  lifecycle path, or parallel route implementation.
