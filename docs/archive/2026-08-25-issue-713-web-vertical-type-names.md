# #713 — remove redundant vertical nouns from public web types

Issue: [#713](https://github.com/jaunder-org/jaunder/issues/713). Milestone:
Code quality ratchet.

## Outcome

Three public, non-wire types use names relative to their owning web vertical:
`auth::User`, `auth::Rejection`, and `tags::InputState`. Their behavior and
module ownership remain unchanged.

## Load-bearing decisions

- Rename `auth::AuthUser` to `auth::User`, `auth::AuthRejection` to
  `auth::Rejection`, and `tags::TagInputState` to `tags::InputState`.
- The vertical module path supplies the removed noun, extending #684's naming
  rule from server functions and wire DTOs to these three non-wire types.
- Cross-vertical consumers spell the generic authenticated-user type as
  `auth::User`; they do not import a bare `User`. This preserves the context
  that the shorter canonical name intentionally delegates to its module path.
- Auth-internal code may use bare `User` and `Rejection`; tags-internal code may
  use bare `InputState` because the enclosing vertical already supplies context.
- Update current source documentation and non-historical project documentation,
  including `docs/ARCHITECTURE.md` and live design drafts, to the new names.
  Accepted ADRs and `docs/archive/` remain unchanged historical records.
- No glossary or ADR change: this is a source naming cleanup, not a new domain
  concept or durable architectural choice.

## Acceptance

- The public web module surface exports `auth::User`, `auth::Rejection`, and
  `tags::InputState`, with no compatibility aliases for the old names.
- Every production and test caller compiles against the new names; Auth user
  references outside the auth vertical remain path-qualified.
- Existing auth extraction, rejection projection, and tag-input state tests pass
  without changed behavioral expectations.
- Current source documentation, architecture documentation, and live design
  drafts name the new types; accepted ADRs and archived planning documents are
  untouched.

## Boundaries

- Pure rename: no wire identifiers, routes, status codes, authentication
  behavior, tag-input transitions, visibility, or module ownership changes.
- No new tests, static gate, compatibility re-export, deprecation path, glossary
  entry, or ADR unless implementation reveals an uncovered behavior change.
