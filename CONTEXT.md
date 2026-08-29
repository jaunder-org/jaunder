# Jaunder

Jaunder is a single-binary, self-hosted social reader and publishing server.
This glossary captures the domain language unique to Jaunder so that code, docs,
and conversation stay consistent.

## Language

### Publishing

**Post**: A unit of authored content owned by one local user, carrying a body in
a specific authoring format, an optional title, a slug, tags, and a publication
state (draft until published). An active Post is identified publicly by its
permalink; a Deleted Post releases that public identity while retaining its
internal Post ID. _Avoid_: Article, entry (reserve "Entry" for the AtomPub wire
object), note.

**Default Post Format**: A per-user preference naming the authoring format
(`Markdown`, `Org`, or `Html`) used as the web composer's default and as the
interpretation for AtomPub `type="text"` content. Real HTML
(`type="html"`/`xhtml"`) always overrides to `Html` regardless of this setting.

**Default Audience**: The instance-wide audience applied when a new Post has no
explicit audience. It is exactly `Public`, `Subscribers`, or `Private`; a Named
audience is per-author and cannot be an instance-wide default.

**Deleted Post**: A locally authored Post retained under a deletion tombstone
but absent from active web, Syndication Feed, and AtomPub Collection surfaces.
Deletion is not physical erasure. _Avoid_: using Deleted Post for inbound
deletion activity or promising purge.

**Post Revision**: An immutable prior full-state snapshot of a locally authored
Post, readable only by its owner. Distinct from an AtomPub **Entry** and from
inbound `ajr_entry_versions`. _Avoid_: edit event (a no-op write creates no
revision), backup (revisions are included in backups but are not backups).

**App Password**: A named, individually-revocable credential a user mints for a
non-browser client (e.g. MarsEdit) to authenticate against machine-facing APIs.
It is not the user's login password; it is an opaque token presented as the
password in HTTP Basic auth. _Avoid_: API key, access token (it reuses
session-token infrastructure but is user-facing as a "password").

**Username**: A case-insensitive local account identifier accepted as ASCII
`[a-z0-9_-]+`. Input is normalized to lowercase; that canonical form is stored,
compared, serialized, displayed, and used in URLs. _Avoid_: preserving case as a
second username identity or pre-normalizing outside the Username boundary.

### AtomPub (RFC 5023)

**Member** / **Entry**: The AtomPub wire representation of a single resource in
a Collection — an Atom `<entry>` XML document. In Jaunder, a Member Entry maps
to exactly one **Post**. _Avoid_: using bare "Entry" to mean a Post; an Entry is
the protocol serialization of a Post.

**Collection**: An AtomPub-addressable, paginated set of Members. In Jaunder, a
user's Collection is their set of active Posts; Deleted Posts are omitted.

**Service Document**: The AtomPub discovery document (`app:service`) that
advertises a user's available Collections and the media types each accepts.

### Syndication

**Syndication Feed**: The public, unauthenticated Atom/RSS/JSON feed (M8)
consumed by arbitrary feed readers. Always serialized as rendered HTML. Distinct
from an AtomPub **Collection**, which is authenticated and editor-facing.
_Avoid_: calling this "the feed" without qualification when an AtomPub
Collection is also in play.

**WebSub Publish Ping**: An outbound `hub.mode=publish` notification from
Jaunder as publisher to the configured **WebSub Hub**, naming a Syndication Feed
URL as its topic. It announces a representation change but carries no content.
_Avoid_: bare WebSub when publisher-side notification could be confused with the
planned inbound WebSub subscription leg; bare hub when the WebSub Hub could be
confused with Jaunder's planned hub architecture.

**`feed_*` scope**: The `feed_*` identifier family — `feed_url`, `feed_cache`,
`feed_events` — refers **only** to syndication feeds (RSS, Atom, JSON Feed), and
only on the **outbound** side (Jaunder producing its own feeds). "Feed" is not a
synonym for a publication, a followed source in general, or an inbound reading
timeline; ActivityPub actors and AT records are **not** "feeds." _Avoid_:
treating `feed_url` as a universal publication/source identity — identity is
per-entity.

**`ajr_*` scope**: The **inbound** syndication family (**A**tom / **J**SON Feed
/ **R**SS ingestion — `docs/feed-reading.md`): `ajr_feeds` (followed fetch
units), `ajr_follows`, `ajr_fetches`, `ajr_entries`, `ajr_entry_versions`,
`ajr_channel_versions`. "ajr" is unfamiliar but unambiguous — deliberately
distinct from the outbound `feed_*` family so an identifier's direction is
always legible. _Avoid_: `feed_*` names for inbound machinery, and
"subscription" naming for follows (the outbound `subscriptions` table is
_subscribers to me_).

### Clients

**Protocol Client**: Third-party software that talks to Jaunder over an open
protocol: a feed reader consuming a **Syndication Feed**, or an AtomPub editor
(MarsEdit, the Emacs client) working a **Collection**. May be consumer-facing or
owner-facing, but always confined to the protocol surface. _Avoid_: bare
"client" for these — unqualified "client" is reserved for software running the
planned `jaunder-client` runtime (see `docs/hub-architecture.md` §8).

**Local Media Copy**: A durable media file downloaded by the Emacs Protocol
Client into a configured root's `local-media/` directory so a pulled Post is
previewable offline. It is verified against the serving Jaunder instance and
content hash, may be reused across Posts, and must travel with the Post files
during backup or synchronization. It is managed content, not an evictable cache.
The configured root is trusted, author-owned local state; symlinks are rejected
during path creation and immediately before mutation, while replacement after
that final check is outside Emacs Lisp's dirfd-free threat model. _Avoid_:
cache, external media (the source is the configured Jaunder instance), temporary
download.

**Markdown Pull Semantics**: The Emacs Protocol Client uses pinned upstream
`cmark-el` as the authority for CommonMark link, image, autolink, code, raw
block, container, fence, and paragraph semantics. Jaunder maps only bounded
block source positions back to exact destination spans; it does not maintain a
second CommonMark parser.

## Relationships

- A **User** _is_ the publication: there is deliberately no
  blog/site/publication entity, and Posts group only by their author.
- A **User** owns one publishing **Collection** of **Posts**.
- A **User** has exactly one canonical **Username**.
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
