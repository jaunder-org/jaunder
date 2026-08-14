# Spec — #689: typed Syndication Feed and AtomPub titles

- Issue: [#689](https://github.com/jaunder-org/jaunder/issues/689)
- Milestone: Domain-value type safety (newtypes)
- Governing decisions:
  [ADR-0063](../../adr/0063-domain-value-newtype-convention.md) and
  [ADR-0101](../../adr/0101-infallible-kind-is-invariant-first.md)
- Protocol reference:
  [RFC 5023 §8](https://www.rfc-editor.org/rfc/rfc5023#section-8)
- Date: 2026-08-14

## Problem

`FeedMetadata` carries its title and optional description as `String` while its
sibling `FeedItem` already carries typed post metadata. The title is not merely
a flattened `SiteTitle`: Jaunder composes it from the site title and the
Syndication Feed surface, but that rule currently lives in a server helper that
takes `&str` and returns `String`.

The same review found three AtomPub presentation values flattened to `String`:
the Service Document Workspace title, each Service Document Collection title,
and the Posts Collection feed title. RFC 5023 requires human-readable titles but
assigns Workspaces no protocol semantics and does not prescribe their text. The
current username and `Posts`/`Media` labels are therefore Jaunder policy, not
protocol identities.

Bare strings make blank wire titles representable, lose the distinction between
these separate presentation roles, and leave their composition rules at call
sites. ADR-0063's type-identity and cross-boundary criteria apply; ADR-0101 says
a value for which blank text is invalid must use a validating constructor.

## Decision

### Shared title invariant

Add five distinct string-backed domain values in `common`:

- `FeedTitle`: a public Syndication Feed document title.
- `FeedDescription`: optional descriptive text for a Syndication Feed.
- `WorkspaceTitle`: the human-readable title of an AtomPub Workspace.
- `CollectionTitle`: the human-readable title of a Collection declaration in an
  AtomPub Service Document.
- `CollectionFeedTitle`: the human-readable title of an AtomPub Collection feed.

Each uses the standard ADR-0063 `StrNewtype` trailer and a validating `FromStr`.
Construction trims surrounding whitespace and rejects empty or whitespace-only
input. The types remain distinct: sharing an XML element name (`atom:title`) or
primitive representation does not make the values interchangeable.

`FeedMetadata.description` remains optional as `Option<FeedDescription>`. `None`
is the only absent-description state; `Some` always contains nonblank text. The
production construction site remains `None`; renderer capability for a future
real description source remains intact.

### Composition belongs to the value

Each derived title has an infallible constructor whose typed inputs prove the
nonblank invariant and whose output exactly preserves today's wire text:

- `FeedTitle` composes `&SiteTitle` with `&FeedSurface`:
  - site: `{site_title}`;
  - site tag: `{site_title} — #{tag}`;
  - user: `{site_title} — @{username}`;
  - user tag: `{site_title} — @{username} #{tag}`.
- `WorkspaceTitle` is constructed from `&Username` and currently renders exactly
  the username. The distinct type records that this is Jaunder's presentation
  policy, not that an RFC 5023 Workspace is account identity.
- `CollectionTitle` owns the two current Service Document labels, `Posts` and
  `Media`, through named constructors.
- `CollectionFeedTitle` owns the Posts Collection feed composition
  `{username}'s posts` from `&Username`.

The constructors create only values known valid from typed inputs; arbitrary
external or test text still goes through `FromStr`. Composition is not copied
into server call sites.

### Propagation and boundary conversion

- `FeedMetadata.title` becomes `FeedTitle` and `description` becomes
  `Option<FeedDescription>`.
- `ServiceDocument.workspace_title` becomes `WorkspaceTitle`.
- `CollectionDecl.title` becomes `CollectionTitle`.
- `FeedMeta.title` becomes `CollectionFeedTitle`.
- Server construction sites pass typed inputs to the named constructors instead
  of formatting or flattening to `String`.
- RSS, Atom, JSON Feed, AtomPub Service Document, and AtomPub Collection feed
  serializers retain the newtypes until the external serializer API requires
  text, then borrow or convert at that boundary.

No endpoint, media type, serialized field name, title spelling, whitespace, or
presence rule changes for existing production output.

## Reviewed sibling values

The raw pre-validation accumulator named by #689 no longer exists:
`common::atompub::entry` now delegates XML parsing to `atom_syndication`
(ADR-0089). Its remaining `Option<String>` is the `j_slug` extension accessor,
not title metadata. Collection accept media types remain `Vec<String>`; they are
separate protocol tokens outside this title-focused issue. No reviewed title or
description field remains primitive.

## Tests

- Each newtype accepts representative nonblank text, trims surrounding
  whitespace, and rejects empty and whitespace-only text.
- Each named composition constructor produces the exact current string for all
  of its variants:
  - all four `FeedSurface` variants;
  - Workspace from a username;
  - Posts and Media Collection labels;
  - Posts Collection feed from a username.
- Renderer tests assert exact serialized title text for RSS, Atom, JSON Feed,
  the Workspace and both Collections in the Service Document, and the AtomPub
  Posts Collection feed.
- Syndication renderer tests pin description presence for both states: `None`
  produces RSS's required empty description and omits Atom `subtitle` and JSON
  Feed `description`; representative `Some(FeedDescription)` text is serialized
  unchanged by all three formats.
- A focused server test confirms Syndication Feed regeneration still emits the
  same composed title through the typed constructor.

## Acceptance criteria

1. `FeedMetadata.title` and `FeedMetadata.description` expose `FeedTitle` and
   `Option<FeedDescription>`; no production caller constructs either field from
   a bare `String`.
2. `ServiceDocument.workspace_title`, `CollectionDecl.title`, and
   `FeedMeta.title` expose `WorkspaceTitle`, `CollectionTitle`, and
   `CollectionFeedTitle`; their server construction sites use the named typed
   constructors.
3. Empty and whitespace-only values cannot be constructed through any of the
   five types' public parsing boundary; accepted input is outer-trimmed.
4. The title-composition rules exist once in `common`, consume `SiteTitle`,
   `FeedSurface`, or `Username` as applicable, and reproduce every current wire
   spelling exactly.
5. Exact serialized title assertions pin RSS, Atom, JSON Feed, the AtomPub
   Workspace and both Service Document Collections, and the AtomPub Posts
   Collection feed to their existing text.
6. Description tests pin both presence states: `None` retains RSS's required
   empty description while omitting Atom `subtitle` and JSON Feed `description`;
   `Some` emits its text unchanged in all three formats.
7. The sibling scan records that ADR-0089 removed the former raw Atom parser
   accumulator and that Collection accept media types remain primitive.
8. The repository's applicable check gate passes.

## Non-goals

- Changing how titles are worded, localized, or selected.
- Fetching a user's optional `DisplayName` for an AtomPub Workspace title.
- Adding a configured Syndication Feed description source.
- Typing AtomPub accept media types or extension-marker strings.
- Changing storage schemas, endpoints, or wire formats.
- Adding or amending an ADR: this is a direct application of ADR-0063 and
  ADR-0101, not a new architectural trade-off.
