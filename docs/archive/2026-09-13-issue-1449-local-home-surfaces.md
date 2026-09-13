# Local and Home Surfaces Implementation Outline

> Execute with `jaunder-iterate`, delegating through `jaunder-dispatch` when
> useful. This outline exists because the approved spec changes auth/session
> redirect semantics and supersedes part of an accepted architecture decision.

## Scope

In:

- Route every authenticated entry path to Home while preserving the anonymous,
  cacheable Local projector.
- Make Local client reads public-only and viewer-independent on both storage
  backends.
- Cut user-facing navigation, copy, tests, flow docs, glossary, design, and
  architecture prose over to the approved Local/Home vocabulary.
- Record the authenticated-root redirect as the proposed draft decision at
  `docs/adr/0193-authenticated-root-redirects-home.md`.

Out:

- Combined public/private timelines, session-aware projection, Local opt-out, or
  a redirect preference.
- Followed-source aggregation, read state, richer cockpit behavior, storage
  schema, audience-policy, pagination, or ordering changes.
- Syndication Feed endpoints, discovery metadata, protocol labels, or `feed_*`
  storage identifiers.

## Task outline

- [x] Task 1: Route authenticated browser transitions directly to Home
  - Contract: `PREPAINT_SCRIPT` redirects a structurally valid auth marker from
    `/` to `/app` before paint, carrying only recognized `order=oldest`; the
    obsolete `HomeRedirectPreference` key and script branch are deleted, and the
    `csr/index.html` twin remains byte-identical. A live session missing its
    marker restores it during reconciliation and replaces `/` with `/app` once.
    Login and registration success plus authenticated brand/root navigation
    target `/app` directly; authenticated navigation contains Home and no Local
    or bare Feed item. Logout still clears the marker before returning to public
    `/`. `/app` remains the sole authority for session confirmation, so a stale
    marker reaches `/login` without a private fetch or loop.
  - Verification: focused `web` host tests prove the script/key registry,
    redirect URL mapping, and signed-out/authenticated navigation inventories.
    Browser cases cover malformed JSON plus missing or invalid marker usernames
    remaining on Local. `devtool run -- cargo xtask e2e-local auth.spec.ts`
    proves login, registration, logout, brand navigation, and authenticated nav;
    `devtool run -- cargo xtask e2e-local authed-flash.spec.ts` proves
    pre-paint, stale-marker, missing-marker recovery, and oldest-order
    redirects; the matching `password_reset.spec.ts` flow lands on Home.

- [x] Task 2: Make the Local query intrinsically public-only
  - Contract: `web::timeline::fetch_local_timeline` no longer accepts a viewer;
    it applies `ViewerIdentity::Anonymous` internally for row visibility and
    decoration. Migrate its projector, server-function, theme-preview, and test
    callers atomically. `list_local_timeline` preserves existing request
    credential validation, including rejection of invalid explicit credentials,
    but a successfully resolved identity cannot affect the Local query. Keep
    `PostStorage::list_published`, audience resolution, Home/profile/tag
    queries, cursor shape, page sizing, and publication ordering unchanged.
  - Verification: the dual-backend
    `local_timeline_enforces_visibility_for_viewer` integration scenario proves
    anonymous, author, subscriber, stranger, and valid bearer-authenticated
    requests return the same public Post set in both order directions, while an
    invalid bearer credential remains rejected. Existing dual-backend Local
    pagination/projector cacheability tests remain green. Focus with
    `devtool run -- cargo xtask test-local -- -p jaunder -E 'test(/local_timeline_enforces_visibility_for_viewer/)'`
    and the matching `web` unit surface.

- [x] Task 3: Complete the Local/Home copy and documentation cutover
  - Contract: Home chrome describes the User's published Posts without bare Feed
    wording. Update remaining live source documentation, browser scenario names
    and assertions, and flow documents; retain legitimate Syndication Feed prose
    and internal API identifiers. Keep ADR-0044 as immutable history, with the
    new draft and `docs/ARCHITECTURE.md` explicitly marking only Decision 5
    superseded. Keep `CONTEXT.md` and `docs/DESIGN.md` aligned; do not edit
    generated `docs/README.md`.
  - Verification: focused `web` host tests and
    `devtool run -- cargo xtask e2e-local posts.spec.ts` prove Home copy,
    anonymous Local pagination, and Home own-Post isolation. Documentation
    formatting, links, ADR bundle, flow parity, and terminology checks pass
    through the normal `jaunder-commit` gate.

## Risk checks

- The root projector and its cache key remain anonymous and never inspect the
  session or auth marker.
- Missing and stale markers follow their distinct approved paths; neither can
  fetch Home data before `/app` confirms the session.
- Same-document navigation does not rely on the document-level script rerunning,
  and logout cannot be redirected back to Home by a marker it should clear.
- Root redirect canonicalization preserves only oldest-first timeline state and
  drops malformed, newest, and unrelated query parameters.
- Malformed JSON and markers with missing or invalid usernames remain on Local;
  only the canonical marker shape redirects before paint.
- Every `fetch_local_timeline` reference migrates with its signature; storage's
  shared viewer-aware policy remains unchanged.
- SQLite and PostgreSQL exercise the same public-only Local contract.
- User-facing bare Feed references are removed only from the Home surface;
  Syndication Feed behavior and terminology remain intact.
- The proposed ADR draft, architecture projection, `CONTEXT.md`,
  `docs/DESIGN.md`, and live flow documents agree before the branch gate runs.
