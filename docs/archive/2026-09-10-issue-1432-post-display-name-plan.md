# Post Display Name Implementation Outline

> Execute with `jaunder-iterate`, delegating through `jaunder-dispatch` when
> useful. This outline exists because the feature changes the cross-backend Post
> projection and the serialized projector/CSR presentation contract.

## Scope

In:

- Carry the User's current optional Display Name through every Post read and
  Post presentation payload.
- Render Display Name plus canonical handle, with a handle-only fallback,
  through the shared Post renderer.
- Prove backend parity, projector/CSR coincidence, Style Contract cardinality,
  current-profile freshness, and visible browser behavior.

Out:

- Post persistence or migrations, identity/routing changes, new author lookups,
  non-Post chrome, author links, Syndication Feed output, and AtomPub output.

## Task outline

- [x] Task 1: Carry current User presentation through Post reads
  - Contract: `PostRecord` carries the author Username and optional Display Name
    from the existing `users` join; `RenderedPost` carries the same optional
    Display Name in the projector/CSR seed and server-function payloads. Every
    Post projection supplies the field in the same statement as the Post.
  - Verification: focused dual-backend storage behavior proves presence and
    absence on direct and listing reads; DTO conversion and serde behavior prove
    the presentation payload without changing feed or AtomPub author output.

- [x] Task 2: Render Post identity consistently across surfaces
  - Contract: the shared `PostView` renders a populated `author-name` followed
    by `@username`; without a Display Name it renders one hidden, empty
    `author-name` and one visible handle. The projector and CSR adapter use that
    same view and renderer.
  - Verification: host renderer tests prove both identity states, exact Style
    Contract hook cardinality, and projector/CSR coincidence; a focused browser
    flow proves Display Name presentation, handle-only fallback, and a profile
    change appearing on an existing Post after reload.

## Risk checks

- Every SQL path decoded as `PostRecord`, including publication, Draft,
  Scheduled, tag, and syndication-selection paths, projects
  `users.display_name`; no path performs a second User lookup.
- SQLite and PostgreSQL use the same nullable typed Display Name boundary.
- `RenderedPost` constructors, fixtures, and serialized seed consumers migrate
  together; no compatibility alias or default masks a missing caller.
- Style Contract version 1 retains exactly one `author-name` and one
  `author-handle` hook per Post header.
- Initial projector markup and mounted CSR content remain byte-identical.
- Syndication Feed and AtomPub serialized author identity remain canonical
  Username-only behavior.
- Run focused red/green lanes per task, then `devtool run -- cargo xtask check`
  before each commit; use the focused `e2e-local` scenario for browser proof.
