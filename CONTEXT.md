# Jaunder

Jaunder is a single-binary, self-hosted social reader and publishing server.
This glossary captures the domain language unique to Jaunder so that code, docs,
and conversation stay consistent.

## Language

### Publishing

**Post**: A unit of authored content owned by one local user, carrying a body in
a specific authoring format, an optional title, a slug, tags, and a publication
state (draft until published). Identified publicly by its permalink. _Avoid_:
Article, entry (reserve "Entry" for the AtomPub wire object), note.

**Default Post Format**: A per-user preference naming the authoring format
(`Markdown`, `Org`, or `Html`) used as the web composer's default and as the
interpretation for AtomPub `type="text"` content. Real HTML
(`type="html"`/`xhtml"`) always overrides to `Html` regardless of this setting.

**App Password**: A named, individually-revocable credential a user mints for a
non-browser client (e.g. MarsEdit) to authenticate against machine-facing APIs.
It is not the user's login password; it is an opaque token presented as the
password in HTTP Basic auth. _Avoid_: API key, access token (it reuses
session-token infrastructure but is user-facing as a "password").

### AtomPub (RFC 5023)

**Member** / **Entry**: The AtomPub wire representation of a single resource in
a Collection — an Atom `<entry>` XML document. In Jaunder, a Member Entry maps
to exactly one **Post**. _Avoid_: using bare "Entry" to mean a Post; an Entry is
the protocol serialization of a Post.

**Collection**: An AtomPub-addressable, paginated set of Members. In Jaunder, a
user's Collection is their set of Posts.

**Service Document**: The AtomPub discovery document (`app:service`) that
advertises a user's available Collections and the media types each accepts.

### Syndication

**Syndication Feed**: The public, unauthenticated Atom/RSS/JSON feed (M8)
consumed by arbitrary feed readers. Always serialized as rendered HTML. Distinct
from an AtomPub **Collection**, which is authenticated and editor-facing.
_Avoid_: calling this "the feed" without qualification when an AtomPub
Collection is also in play.

**`feed_*` scope**: The `feed_*` identifier family — `feed_url`, `feed_cache`,
`feed_events` (and the planned inbound `source_feeds`) — refers **only** to
syndication feeds (RSS, Atom, JSON Feed). "Feed" is not a synonym for a
publication, a followed source in general, or an inbound reading timeline;
ActivityPub actors and AT records are **not** "feeds." _Avoid_: treating
`feed_url` as a universal publication/source identity — identity is per-entity.

### Clients

**Protocol Client**: Third-party software that talks to Jaunder over an open
protocol: a feed reader consuming a **Syndication Feed**, or an AtomPub editor
(MarsEdit, the Emacs client) working a **Collection**. May be consumer-facing or
owner-facing, but always confined to the protocol surface. _Avoid_: bare
"client" for these — unqualified "client" is reserved for software running the
planned `jaunder-client` runtime (see `docs/hub-architecture.md` §8).

## Relationships

- A **User** _is_ the publication: there is deliberately no
  blog/site/publication entity, and Posts group only by their author.
- A **User** owns one publishing **Collection** of **Posts**.
- An AtomPub **Member Entry** is the wire form of exactly one **Post**.
- A **User** may hold many **App Passwords**, each revocable independently.
- A **Post** appears in two unrelated Atom surfaces: the public **Syndication
  Feed** (as rendered HTML) and the user's AtomPub **Collection** (in native
  source form for lossless round-trip).

## Flagged ambiguities

- "Entry" is overloaded: in AtomPub it is the XML wire object; in casual use it
  can mean a Post. Resolved: **Post** = the stored domain object;
  **Entry/Member** = its AtomPub serialization.
- "Feed" is overloaded: the public **Syndication Feed** (HTML, for readers) and
  the AtomPub **Collection** feed (native source, for editing) are different
  documents with different audiences. They are deliberately separate
  serializers, not one shared path. A **third sense to avoid**: the
  inbound/normalized _reading timeline_ is also loosely called a "feed," but it
  is not a syndication feed and carries no `feed_*` naming — `feed_*` is
  syndication-only (RSS/Atom/JSON).
- "Blog" names nothing: there is no blog entity — the **User** is the
  publication (see Relationships). Do not introduce one casually.
- "Client" is overloaded: feed readers, AtomPub editors, and the planned rich
  apps are all casually "clients." Resolved: **Protocol Client** = third-party
  software on an open protocol; unqualified "client" = software running the
  planned `jaunder-client` runtime.
