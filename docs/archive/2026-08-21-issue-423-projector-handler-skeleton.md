# Issue #423: projector profile/user-tag handler skeleton

## Outcome

The public projector's profile and user-tag routes keep their current HTTP
behavior while the duplicated username-page projection skeleton is expressed
once. The two handlers remain thin route adapters: parse soft path inputs, run
the route-specific anonymous fetch, build the route-specific `PageSeed`, and
return either a cacheable projected document or the SPA shell.

## Load-bearing decisions

- Preserve ADR-0041's projector contract: public projector handlers fetch as
  `ViewerIdentity::Anonymous`, produce byte-identical cacheable documents only
  for anonymous-public content, and serve the SPA shell when the URL has no
  anonymous-public projection.
- Preserve the projector-vs-AtomPub soft-path boundary from ADR-0063: malformed
  public route segments are handled inside the projector handler and fall back
  to the SPA shell, not axum's pre-handler 400.
- Extract only the shared username-page control flow. Route-specific pieces stay
  parameters: the fetch operation, the swallowed-error context, and the
  `PageSeed` constructor.
- Keep the subtle unknown-user asymmetry intact:
  - profile uses `fetch_user_posts`; a valid but unknown Username still yields
    an empty page, so `/~unknown` is cacheable.
  - user-tag uses `fetch_user_posts_by_tag`; an unknown Username yields an
    error, so `/~unknown/tags/<tag>` serves the SPA shell.
- Document that asymmetry at the extracted helper so future cleanup does not
  "normalize" the two route outcomes.
- Error reporting remains equivalent to today: storage errors collapsed to shell
  fallback still pass through `report_swallowed` with the route's existing
  context.
- This is a pure code-quality refactor. No route, status, header, cache-control,
  ETag, seed shape, or rendered HTML behavior changes.

## Acceptance

- The diff removes the duplicated profile/user-tag
  `fetch -> PageSeed -> cacheable-or-shell` response shape rather than moving it
  into two new copies.
- Existing projector tests still pass.
- A focused regression proves profile's valid unknown Username remains cacheable
  while user-tag's valid unknown Username remains a shell fallback, or existing
  tests already cover both behaviors explicitly.
- Malformed Username and malformed Tag projector paths still serve the SPA shell
  rather than 400.
- No public API, storage schema, or web client contract changes appear in the
  diff.

## Boundaries

- Do not change `fetch_user_posts` or `fetch_user_posts_by_tag` semantics.
- Do not broaden this into a projector-wide response abstraction for permalink,
  site timeline, or site tag handlers unless required to avoid duplicating the
  exact issue scope.
- Do not introduce a new trait or boxed future solely to make the helper
  generic; prefer the smallest boring Rust shape that keeps the handlers
  readable.
- Do not alter cache policy, seed serialization, or CSR boot behavior.
