# WordPress-compatible permalinks implementation outline

> Execute with `jaunder-iterate`, dispatching slices with `jaunder-dispatch`.
> This outline exists because the feature adds a public HTTP route and a
> cross-backend visibility-sensitive storage resolution contract.

## Scope

In:

- Resolve the inbound bare date-and-slug form across anonymously visible Posts.
- Redirect one unique match to its existing canonical permalink.
- Prove storage parity, HTTP behavior, and one cold browser entry.
- Keep the approved spec, proposed ADR, architecture view, and flow docs
  aligned.

Out:

- Schema changes, global permalink uniqueness, username inference, emitted bare
  links, CSR routing, or compatibility aliases for other resources.

## Task outline

- [x] Task 1: Add bounded cross-User permalink-alias resolution to Post storage.
  - Contract: `PostPermalinkAliasMatch` distinguishes `Missing`,
    `Unique(Username)`, and `Ambiguous`;
    `PostStorage::resolve_post_permalink_alias(date: PermalinkDate, slug: &Slug, now: UtcInstant) -> Result<PostPermalinkAliasMatch>`
    applies anonymous visibility without exposing candidate rows.
  - Verification: dual-backend `#[apply(backends)]` storage tests cover unique,
    absent, ambiguous, private, subscriber-only, draft, future, and Deleted
    Posts with
    `devtool run -- cargo xtask test-local -- -p storage permalink_alias`.
- [x] Task 2: Add the server-owned alias route and exact redirect response.
  - Contract: valid `GET /YYYY/MM/DD/slug` consumes the storage resolution;
    `Unique` returns `302` plus canonical same-origin `Location`, the untouched
    raw query, and `Cache-Control: no-store`; `Missing`, `Ambiguous`, and parse
    rejection use the existing shell miss; storage failures remain sanitized
    boundary `500`s.
  - Verification: backend-parametric projector integration tests cover status,
    Location, query ordering/repetition/empty values, no-query output, no-store,
    all miss classes, privacy, ambiguity, Unicode path encoding, and failure.
- [ ] Task 3: Prove the compatibility entry through the browser and reconcile
      public documentation.
  - Contract: the browser performs one cold entry through the HTTP alias and
    lands on the canonical `~username` route; no client route or emitted link
    uses the alias. Documentation retains the proposed draft path and current
    canonical-route language.
  - Verification: focused `devtool run -- cargo xtask e2e-local <spec:line>`
    observes a rendered canonical `~username` href, the alias redirect's
    canonical final URL and Post content, and an in-app bare-path navigation
    remaining a CSR miss; `devtool run -- cargo xtask check` certifies the
    complete issue before commit.

## Risk checks

- Query at most two qualifying rows; never pick by backend/storage ordering and
  never allocate or return an unbounded candidate collection.
- Apply the same UTC date, active lifecycle, and anonymous audience predicates
  on SQLite and PostgreSQL; private, subscriber-only, draft, future, and Deleted
  Posts cannot make an alias resolvable.
- Emit exactly `302 Found`; Axum's temporary redirect helper is `307` and does
  not satisfy the contract.
- Preserve raw query bytes without allowing a caller-controlled origin or header
  injection; serialize Unicode canonical paths into a valid `Location` value.
- A redirect is always no-store, and zero/multiple matches remain
  indistinguishable shell misses; infrastructure failure is never erased into a
  miss.
- Migrate every `PostStorage` implementation and test double, preserve canonical
  permalink generation, and leave the CSR route table unchanged.
