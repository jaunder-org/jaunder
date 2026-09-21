# Jaunder

Jaunder is a single-binary, self-hosted social reader and publishing server.
This glossary captures the domain language unique to Jaunder so that code, docs,
and conversation stay consistent.

## Language

### Registration

**Registration Policy**: The instance-wide rule for admitting new Users and
authorizing Invitation issuance. It is exactly `Closed` (nobody registers or
issues Invitations), `OperatorInvites` (an Invitation is required and only
operators issue one), `MemberInvites` (an Invitation is required and any
authenticated User issues one), or `Open` (anyone registers directly and
Invitations are unavailable). _Avoid_: `InviteOnly`, which hides who may issue
an Invitation, and using `Closed` to mean operator-issued registration.

**Invitation**: A single-use, expiring capability issued by an authorized User
and delivered directly to a prospective User. Holding a valid Invitation permits
registration only under an invitation-based Registration Policy; it does not
make the holder an operator. _Avoid_: invitation request (there is no public
request or approval queue), registration code.

### Media

**Media Upload Capability**: The instance-wide permission to create new Media
through either the web interface or the AtomPub Media Collection. When disabled,
existing Media remains readable and deletable, but every new upload is forbidden
and upload discovery is hidden. It is independent of maximum file size and user
quota. _Avoid_: upload limit (a byte limit is a quantity, not a capability), web
uploads (the policy is cross-protocol), magic zero.

### Presentation

**Local**: The public, viewer-independent web timeline at `/`, containing
currently published public Posts originating on the Jaunder instance. It is the
signed-out landing surface; a browser carrying an authentication marker
redirects to Home before Local content paints. _Avoid_: Home (the authenticated
publishing cockpit), feed (reserved for syndication).

**Home**: The authenticated publishing cockpit at `/app`, containing the current
User's own published Posts and inline composer. It is not a followed source or
reading timeline. _Avoid_: Feed (reserved for Syndication Feeds), Local (the
public instance timeline).

**Site Tagline**: The optional operator-controlled plain-text description of
Local. It appears in Local presentation and describes site-wide and site-tag
Syndication Feeds; it does not describe Home or an individual User's
publication. _Avoid_: Post summary, User bio, promotional hero.

**Style Contract**: The versioned semantic HTML surface shared by Jaunder's
built-in and custom public themes. It guarantees accessible source order,
landmarks, and named concept hooks, not incidental wrapper nesting or sibling
positions. _Avoid_: template API (themes cannot replace the document), DOM
snapshot (incidental structure is not contractual).

**Theme Package**: A portable, non-executable custom public theme containing a
versioned manifest, one CSS entry point that Jaunder validates and scopes, and
optional package-local font or raster-image assets. A stored Theme Package
belongs to the operator or one author; owner Media bindings remain instance data
and do not travel with it. _Avoid_: template, plugin, skin (the package is CSS
and assets, not code or a viewer preference).

### Publishing

**Post**: A unit of authored content owned by one local user, carrying a body in
a specific authoring format, an optional title, a slug, tags, and a publication
state (draft until published). An active Post is identified publicly by its
permalink; a Deleted Post releases that public identity while retaining its
internal Post ID. _Avoid_: Article, entry (reserve "Entry" for the AtomPub wire
object), note.

**Post Shortcode**: A bounded, complete-line construct in Markdown or Org Post
source that asks Jaunder to render one supported provider-owned presentation
without storing generated markup. Unknown or invalid constructs remain literal,
and the native source round-trips unchanged. _Avoid_: shortcode engine (there is
no general template language), embed code (authors do not supply the rendered
iframe), Org macro (the behavior is not client-specific).

**Rendered Title**: A Post title's persisted, `ammonia`-sanitized, inline-only
HTML projection in the Post's authoring format. Web headings and markup-capable
Syndication Feed fields use it directly; plain-text feed fields derive readable
text by stripping that projection with `ammonia` and decoding entities with
`html-escape`. CSR trusts server-authored Rendered Title bytes exactly as it
trusts rendered body HTML. The authored Post title remains canonical for
editing, slugs, metadata, and AtomPub. _Avoid_: formatted title (does not name
the persisted projection), rendered heading (the heading also contains
presentation-owned structure).

**Media Record**: One persistent per-user record for one exact stored media
identity, representing media its user controls in their library. A qualifying
Post reference may materialize its author's independent record from an existing
source row; it never pins or transfers control of another user's record. It
persists until its owner explicitly deletes it. _Avoid_: treating a Post
reference as ownership of another user's Media Record.

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

**Passkey**: A named, independently revocable WebAuthn credential a User enrolls
for browser sign-in. It is a discoverable public-key credential that lets the
authenticator identify the account and must verify the User; it neither replaces
the account password nor authenticates machine-facing APIs. _Avoid_: security
key (Passkeys may be synced), passwordless account (password recovery remains),
Session (a successful Passkey assertion creates a Session).

**Username**: A case-insensitive local account identifier accepted as ASCII
`[a-z0-9_-]+`. Input is normalized to lowercase; that canonical form is stored,
compared, serialized, displayed, and used in URLs. _Avoid_: preserving case as a
second username identity or pre-normalizing outside the Username boundary.

**Display Name**: An optional, current human-readable label for a User. It is
shown alongside the User's canonical Username but never replaces that identity
in URLs, lookup, comparison, or protocol credentials. Casing is preserved, and
changing it changes how all of the User's Posts are presented. _Avoid_: treating
a Display Name as a second Username or as a per-Post publication snapshot.

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

**Local Post Link**: A relative Org file link from one locally managed Post to
another Post file in the same configured Jaunder root. The path must resolve
exactly; the Emacs Protocol Client never searches for a matching filename. The
local source keeps the relative link, while publication uses the target Member's
server-advertised public URL; pull restores the relative form only when local
ID, slug, filename, and remote Member identity agree exactly. _Avoid_: deriving
a permalink from a filename or slug, searching for a target, treating an invalid
`.org` link as Media.

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
- A **User** may hold many **Passkeys**, each revocable independently; a Passkey
  authenticates the browser and then creates an ordinary **Session**.
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
