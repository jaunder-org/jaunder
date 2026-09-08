# Centralized Public Projection Orchestration

## Outcome

Public projector routes use one deep `PublicProjector` module for anonymous data
fetching, effective-theme selection, public document construction, cache
negotiation, semantic shell misses, and observable boundary failures. The
refactor preserves every public byte, cache identity, route, and route-specific
failure behavior while removing the partial `username_page_response` seam and
duplicated orchestration from route handlers.

## Load-bearing decisions

- `PublicProjector` is constructed with exactly the Post, User, and Theme
  storage interfaces plus `Shell`. It does not receive `AppState` or another
  heterogeneous dependency holder, preserving ADR-0016.
- The module exposes one typed public-route operation. Its closed route variants
  represent permalink, site timeline, profile, site tag, and user tag
  projection.
- Route handlers retain route-specific soft decoding and select a typed
  operation only after decoding succeeds. Malformed projector routes continue to
  produce the SPA shell rather than extractor-generated `400` responses.
- Each operation variant owns its anonymous fetch, `PageSeed` construction,
  effective-theme owner and route, fetch-error disposition, and stable
  observability context. Handlers do not pass generic callbacks or loosely
  coupled policy flags.
- Every public fetch uses `ViewerIdentity::Anonymous`. Request authentication,
  cookies, and viewer state cannot influence projected bytes or cache identity,
  preserving ADR-0041.
- `PublicProjector` owns cacheable document construction, ETag and
  conditional-request handling, shell mapping, boundary observability, and
  sanitized non-cacheable `500` construction. Operations accept the request
  headers needed for cache negotiation.
- The module uses a typed internal outcome that distinguishes cacheable public
  presentation, semantic SPA-shell miss, and boundary failure. One
  response-mapping path consumes that outcome, emits each failure at most once,
  and constructs the final HTTP response.
- A semantic miss and a swallowed fetch failure produce the existing no-store
  SPA shell without becoming a boundary failure. A boundary failure remains
  observable and produces the existing sanitized, non-cacheable `500` response.
- Route-specific behavior remains distinct:
  - permalink malformed routes and absent Posts are shell misses; Post-fetch and
    theme-resolution failures are boundary failures; successful Posts use author
    theme ownership;
  - site timeline fetch and theme-resolution failures are boundary failures;
    successful listings use site theme ownership;
  - profile malformed usernames are shell misses; listing failures are swallowed
    to the shell; an unknown valid Username remains an empty cacheable profile
    using site fallback ownership; user lookup and theme-resolution failures are
    boundary failures;
  - site-tag malformed tags are shell misses; listing failures are swallowed to
    the shell; theme-resolution failures are boundary failures; successful
    listings use site theme ownership;
  - user-tag malformed paths, unknown Users, and listing-stage
    failures—including the listing fetch's User lookup—are swallowed to the
    shell; the later theme-owner User lookup and theme-resolution failures are
    boundary failures; successful listings use author ownership with the
    existing site fallback.
- Effective theme selection continues to preserve site ownership, author
  overrides, and fallback ownership. This refactor does not alter Theme storage
  or resolution policy.
- `PageSeed`, `PublicPresentation`, and the shared pure rendering functions
  remain the public projection contract. No second serializer or render path is
  introduced.
- `username_page_response` is deleted after profile and user-tag routes move to
  `PublicProjector`; no compatibility wrapper, callback seam, or deprecated
  alias remains.
- The implementation lives outside `projector/mod.rs`; that file remains
  assembly and explicit re-exports only, preserving ADR-0128.
- ADR-0041 already governs the architectural choice, so this refactor adds no
  ADR. The projector section of `docs/ARCHITECTURE.md` is updated to describe
  the centralized seam. `CONTEXT.md` remains unchanged because no domain
  vocabulary changes.

## Acceptance

- `permalink`, `site_timeline`, `profile`, `site_tag`, and `user_tag` handlers
  soft-decode their route inputs and delegate all remaining projection
  orchestration to one `PublicProjector` interface.
- `PublicProjector` has only the exact Post, User, Theme, and Shell constructor
  dependencies, and its operation type is closed and route-specific.
- No `username_page_response`, route-supplied fetch callback, parallel
  document-construction path, or duplicated fetch/theme/error orchestration
  remains.
- Existing projector integration coverage in
  `server/tests/projector/{listing,permalink,tags,caching}.rs` passes unchanged
  except for additions needed to lock the centralized interface or an uncovered
  route policy.
- Projected content, `PageSeed` variants, ETags, conditional `304` responses,
  cache headers, and repeat-request byte identity remain unchanged.
- Authenticated and anonymous requests for the same public URL produce identical
  projected bytes and cache identity.
- Malformed projector routes, absent resources, unknown Users, and swallowed
  listing failures retain their exact existing shell versus cacheable-empty
  behavior.
- Site themes, author theme overrides, and fallback ownership produce the same
  effective presentations as before.
- Failures classified by their route operation as boundary failures remain
  observable exactly once and return sanitized, non-cacheable `500` responses.
  Failures classified as swallowed—including profile and tag listing failures
  and their nested User lookups—remain observable as swallowed errors and return
  the no-store shell.
- `cargo xtask check` passes, and the relevant projector integration and
  browser-facing behavior remain green.
- `docs/ARCHITECTURE.md` describes the resulting single public projection
  orchestration seam consistently with ADR-0041.

## Boundaries

- Behavior-preserving refactor only: no changes to public routes, HTML or seed
  bytes, cache policy, storage behavior, bind/query behavior, theme policy, CSR
  rendering, or Syndication Feed behavior.
- No changes to the public storage interfaces or their backend implementations.
- No replacement generic callback helper, marker interface, compatibility layer,
  or alternate projector path.
- No conversion of soft projector routes to strict Axum extractors.
- No new domain terminology or architectural decision beyond the consolidation
  already governed by ADR-0041, ADR-0016, and ADR-0128.
- Work from other architecture-consolidation milestone issues is not included.
