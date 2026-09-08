# Centralized Public Projection Orchestration Implementation Outline

> Execute with `jaunder-iterate`, delegating through `jaunder-dispatch` when
> useful. This outline exists because the change creates a durable projector
> seam with a typed interface whose route-policy and response-ownership
> contracts must remain coherent during the cutover.

## Scope

In:

- One `PublicProjector` module with exact Post, User, Theme, and Shell
  dependencies.
- Closed typed operations for permalink, site timeline, profile, site tag, and
  user tag.
- Centralized anonymous fetch, theme resolution, document/cache construction,
  shell mapping, and error observability.
- Clean migration of every public projector handler and deletion of the partial
  callback seam.
- Focused regression coverage for route-policy distinctions not already isolated
  by projector integration tests.
- Projection of the resulting seam into `docs/ARCHITECTURE.md`.

Out:

- Changes to public bytes, routes, cache semantics, storage interfaces, theme
  policy, CSR rendering, or Syndication Feed behavior.
- Strict extractors for soft projector routes.
- Compatibility wrappers, parallel orchestration paths, new domain vocabulary,
  or a new ADR.
- Work owned by other architecture-consolidation issues.

## Task outline

- [x] Task 1: Lock the user-tag failure-policy distinction
  - Contract: a failure in the listing-stage User lookup is reported as
    swallowed and returns the no-store shell; a later theme-owner User lookup
    failure is emitted once as a boundary failure and returns a sanitized,
    non-cacheable `500`.
  - Verification: focused server projector integration coverage proves both
    outcomes and observability classifications without depending on private
    implementation text.

- [x] Task 2: Establish the seam on boundary-failing routes
  - Contract: at this stage, `PublicProjector` accepts exactly the Post, Theme,
    and Shell dependencies plus the closed `Permalink` and `SiteTimeline`
    operations and request headers. Its typed internal outcome distinguishes
    cacheable presentation, semantic shell miss, and boundary failure; its
    single response path owns cache negotiation and final response construction.
  - Contract: permalink and site-timeline operations preserve anonymous fetches,
    `PageSeed` construction, author-versus-site theme ownership, absent-Post
    shell misses, boundary-error disposition, and stable observability contexts.
    Their handlers retain only soft decoding and operation selection.
  - Verification: focused permalink, listing, and caching integration coverage
    preserves content, seeds, ETags, conditional `304` responses,
    authenticated/anonymous byte identity, themes, shell misses, and one-time
    boundary observability.

- [x] Task 3: Migrate the soft-failure listing routes
  - Contract: this task extends `PublicProjector` to its final exact Post, User,
    Theme, and Shell dependencies and closed `Permalink`, `SiteTimeline`,
    `Profile`, `SiteTag`, and `UserTag` operations. Profile, site-tag, and
    user-tag operations preserve their distinct malformed-route, unknown-User,
    cacheable-empty, swallowed-error, theme-owner, fallback, and boundary-error
    policies.
  - Contract: profile and user-tag handlers retain soft path decoding and select
    typed operations without callbacks or policy flags. User-tag's listing-stage
    and theme-owner User lookups keep their different failure dispositions.
  - Verification: focused listing and tag integration coverage proves all three
    routes' content, themes, cache behavior, shell outcomes, and swallowed
    versus boundary observability.

- [ ] Task 4: Complete the clean cutover
  - Contract: `username_page_response`, route-supplied fetch callbacks,
    duplicated handler orchestration, and superseded document helpers are
    removed. `projector/mod.rs` remains assembly-only with explicit re-exports,
    and no second serializer, response mapper, or projection path survives.
  - Verification: the complete server projector integration surface—listing,
    permalink, tags, and caching—passes as one regression set.

- [ ] Task 5: Project and gate the consolidated architecture
  - Contract: `docs/ARCHITECTURE.md` names the single public projection
    orchestration seam consistently with ADR-0041, ADR-0016, and ADR-0128;
    `CONTEXT.md` and the ADR log remain unchanged.
  - Verification: documentation formatting/parity checks and `cargo xtask check`
    pass after the intended tree is complete. Focused browser runs for
    `timeline-cls.spec.ts`, `unicode-slug.spec.ts`, and `theme.spec.ts` prove
    the listing, permalink, projector-to-CSR, and effective-theme surfaces.

## Risk checks

- All public data fetches cross an explicit `ViewerIdentity::Anonymous` seam; no
  handler authentication or cookie state enters projection output.
- Cacheable responses retain identical bytes, ETags, conditional `304` behavior,
  and public cache headers; shell and failure responses remain non-cacheable.
- Soft route decoding stays in handlers so malformed inputs remain shell misses
  rather than extractor `400` responses.
- Profile unknown-User behavior remains a cacheable empty profile, while
  user-tag unknown-User behavior remains a shell miss.
- User-tag's listing-stage and theme-owner User lookups retain different failure
  dispositions.
- Site ownership, author overrides, and fallback theme ownership remain
  unchanged for every route.
- Each swallowed or boundary failure is observed once under its existing stable
  context.
- No second serializer, callback seam, response mapper, or public projection
  path survives the cutover.
- Every work item reaches `jaunder-commit` after its focused evidence; no lint
  suppression is introduced without explicit approval, and commits carry no
  `Co-Authored-By` trailer.
