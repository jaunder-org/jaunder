# Design

## Operational model

`Jaunder` ships as a single binary, takes its basic config from the command line or environment variables, and getting an instance up and running should be as simple as:

```
jaunder init # initial, guided database setup
jaunder serve
```

Additional configuration can then be done via the web interface, or through additional CLI calls if that is more appropriate.

`Jaunder` does not do its own https, it expects to run behind a reverse proxy to provide it.

## Interfaces

- **CLI**: easy access to basic administrative tasks — initial setup, backup/restore, setting configuration values
- **Interoperability APIs**: ActivityPub (including WebFinger for handle resolution), Authenticated Transfer Protocol
- **Compatibility APIs**: Mastodon client APIs
- **Native API**: the most complete interface that covers all aspects of the software's capabilities
- **Web front-end**: the interface that ships with the system by default, operates via the Native API, and handles all available functionality, both regular and administrative
- **Mobile clients**: alternative clients written to the native API

## UI

### Timelines

Jaunder exposes three timeline views:

- **Local timeline** (public, `/`): All original posts published by local users, in chronological order. Accessible to unauthenticated visitors.
- **User timeline** (public, `/~:username/`): All original posts published by a single local user, in chronological order. Accessible to unauthenticated visitors.
- **Federated timeline** (authenticated, `/`): For an authenticated user, the home route returns all items from every source the user follows — across all protocols and source types — in chronological order.

Sort direction (newest-first or oldest-first) is user config, applied to all timeline views.

The underlying API uses cursor-based pagination so that the web front-end can implement endless scroll without discrete page numbers. Clients receive an opaque cursor alongside each batch of results and pass it back to fetch the next batch.

#### Read State

Jaunder tracks read/unread state per item in each user's content layer. Timelines display unread counts and support jumping to the first unread item. Items are marked read automatically as they are scrolled past; users can also mark items read or unread manually. The unread count is also surfaced in the notification badge and via the API.

### Account and Profile Management

Each user has a dedicated account management area, accessible via the web interface and the Native API. This area covers all user-centric configuration and social graph management:

- **Profile**: Display name, bio, avatar image. When AP is enabled: whether the account requires manual follow approval (`manuallyApprovesFollowers`). When AT PDS mode is enabled: AT handle settings.
- **Source management**: Add and remove followed sources (feeds, AP actors, AT accounts). Configure per-source settings (retention window, mute).
- **Following list**: View and remove followed sources and accounts.
- **Followers list**: View the AP and AT accounts that follow the user; remove individual followers.
- **Follow requests**: For locked accounts, approve or deny pending follow requests.
- **Block and mute lists**: View and manage blocked actors/domains and muted accounts.
- **Notification preferences**: Configure which event types trigger notifications and which delivery mechanisms (email, Web Push, APNs, FCM) are active.
- **Active sessions**: View and individually revoke device tokens.

## Low-level architecture

### Configuration

**Bootstrap config** (CLI args, `JAUNDER_*` env vars, sensible defaults): the minimal values needed to locate the database, start the server, and store media. Exactly two values:
- bind address and port
- local storage path (media stored in a subdirectory, database in the directory if using SQLite, backups in a subdirectory by default)

**Runtime config** is stored in the `site_config` table in the database, read on each use with no in-process caching, so changes take effect immediately without a restart. It has two namespaces:

- **Site config** (keys prefixed `site.`): instance-wide settings, readable and writable by operators via the CLI or web admin interface.
- **User config** (keys prefixed `user.<username>.`): per-user preferences, stored in the same table with the same access semantics. Users manage their own settings through the account management interface.

Instance-level site config that does not have a more specific home below: site name; registration policy (open / invite-only / closed — invitations generated via CLI or web interface).

### Storage

#### Backends (Pluggable)

The storage layer is abstracted behind a trait. Implementations:

1. **SQLite**: Default. Single file. Suitable for personal instances and small groups. Full-text search is implemented via SQLite FTS5.
2. **PostgreSQL**: For heavier multi-user deployments, high-volume AP instances, or power users with large follow graphs. Full-text search is implemented via `tsvector` columns and GIN indexes.

#### Data Architecture: Shared Ingestion, Private Copies

To be good network citizens, external sources are fetched **once per feed**, regardless of how many local users follow them. However, each user's view of that content is fully private:

- **Ingestion layer** (shared): raw fetched content, feed metadata, WebSub subscriptions, AP actor caches, AT DID/actor caches
- **User content layer** (private): per-user copies of feed items, read state, bookmarks, likes, replies, reposts, notification records

All user-layer tables carry a `user_id` column. Queries never cross user boundaries.

#### Retention

- Default: retain all content indefinitely.
- Per-feed pruning: users can configure a retention window per source (e.g., "keep only the last 60 days of items from this high-volume feed"). Bookmarked, liked, replied-to, or reposted items are exempt from pruning.
- **Future goal**: tiered archival of older content to secondary/cold storage. The mechanism and storage targets are not yet specified; the retention model is designed to not preclude this.

### Unified Content Model

#### Philosophy

Every item is stored in two forms at write time:

1. **Raw payload**: The original protocol data (AP JSON-LD, RSS & Atom XML, JSON Feed JSON) stored without alteration. This is the source of truth. If normalization logic ever changes, all items can be re-derived from the raw copy.
2. **Processed form**: The normalized unified representation (core fields + protocol extension blob) stored alongside the raw payload. This is what the API reads from, so normalization work is paid once at ingestion/creation, not on every read.

Neither form is discarded. The raw payload is never modified after storage. See [Post Editing and Deletion](#post-editing-and-deletion) for additional detail.

#### API Representation

Every content item exposed through the API has:

**Core (protocol-agnostic):**
```
id            — Jaunder-internal UUID
source_url    — canonical URL of the item at its origin
source_type   — "activitypub" | "rss" | "atom" | "json_feed" | "at"  (rss, atom, and json_feed are distinct values identifying the raw wire format)
author        — { name, url, avatar_url }
published_at  — RFC 3339 timestamp
title         — optional; absent from short-form social posts (AP Notes, AT posts); typically present for RSS, Atom, and JSON Feed items
body_text     — plain text rendering
body_html     — sanitized HTML rendering
attachments   — list of { url, type, alt_text }
tags          — list of strings
```

**Protocol extension (typed union):**
- `activitypub`: visibility scope, content_warning, boost_of, in_reply_to, ap_object_url, sensitive flag
- `feed`: guid, enclosures, categories, feed_title
- `at`: rkey, cid, did, langs, reply_root_uri, reply_parent_uri, quote_of_uri, via_did, labels

RSS, Atom, and JSON Feed are treated as a single protocol family: ingestion behavior (polling, WebSub), source discovery, and the normalized extension schema are identical for all three. They are distinguished only by `source_type`, which records the original wire format so that raw payloads can be correctly reprocessed if normalization logic changes.

Clients that only care about the unified core can ignore extensions. Clients building Mastodon-style UIs can use AP extensions directly.

### Authentication & Authorization

#### User Authentication (Inbound)

- Local username + bcrypt-hashed password.
- Each user account may optionally carry an **email address**. Email is not required; users who omit it are warned at registration time that self-service password reset will not be available to them.
  - If an email address is provided, it will be verified.
  - If an email address is provided, it can be used for notifications.
- On successful login, Jaunder issues a long-lived **device token** (opaque, stored as a hash). Tokens are scoped to a device/client and can be individually revoked.
- **Password reset**: initiated by supplying a **username** (not an email address) to `POST /api/v1/auth/password-reset/request`. If an email address is on file, a time-limited single-use reset token is sent there and consumed via `POST /api/v1/auth/password-reset/confirm`. If no email address is on file, the endpoint responds with a clear message saying so — this is safe to disclose because it reveals nothing about any particular email address. Without an email address on file the user must contact the instance operator directly.
- Email addresses are not required to be unique; two accounts may share an address without creating any ambiguity, since reset lookup is keyed on username.
- Jaunder can act as an OAuth 2.0 provider, allowing first-party and third-party clients to request tokens via the standard OAuth flow.

Device token TTLs are site config. Email-dependent features (password reset, email notifications) require SMTP site config to be set (relay host, port, credentials, sender address); these features are disabled until SMTP is configured.

#### HTTP API Auth

All API endpoints require a `Bearer <device_token>` header, except:
- `GET /.well-known/webfinger` (public)
- `GET /users/:id` (AP Actor endpoint, public)
- `POST /users/:id/inbox` (AP inbox, verified via HTTP Signatures)
- `GET /.well-known/atproto-did` (AT DID document, public)
- `GET /xrpc/com.atproto.repo.describeRepo` (AT repo metadata, public)
- `GET /xrpc/com.atproto.sync.*` (AT repo sync endpoints, public)

### Feed Ingestion

#### Strategy: Push-First with Adaptive Polling

Jaunder prioritizes real-time delivery over polling:

1. **ActivityPub inbox delivery**: When federation is enabled, remote AP servers push activities directly to Jaunder's inbox. No polling needed for followed AP accounts.
2. **WebSub (PubSubHubbub)**: For RSS, Atom, and JSON Feed sources that advertise a WebSub hub, Jaunder subscribes to the hub for near-real-time push delivery.
3. **AT Protocol Jetstream**: For followed AT accounts, Jaunder subscribes to the configured AT Jetstream URL — a JSON WebSocket stream — filtered to the relevant DIDs, for near-real-time push delivery of AT events. No polling needed for followed AT accounts.
4. **Adaptive polling**: For sources with no push mechanism, Jaunder polls on an adaptive schedule. The polling interval is estimated from the feed's historical update frequency, bounded by a configurable minimum and maximum (site config; defaults: no faster than 15 minutes, no slower than 24 hours for active feeds).

The WebSub callback base URL and the AT Jetstream URL are also site config.

#### Source Discovery

When a user adds a source, they can provide:
- A direct RSS, Atom, or JSON Feed URL
- A blog or website URL — Jaunder inspects the HTML for `<link rel="alternate" type="application/rss+xml">`, `application/atom+xml`, and `application/feed+json` to auto-discover the feed
- An ActivityPub actor URL
- An `@user@domain` handle — resolved via WebFinger
- An OPML file for bulk import of feed subscriptions
- An AT handle (`@user.bsky.social`) — resolved via DNS TXT record or `/.well-known/atproto-did` to a DID, then via the PLC directory to the user's PDS
- An AT actor DID URI (`at://did:plc:...`) directly
- A keyword/name search via AP directories or relay search services

### ActivityPub Federation (Operator-Optional)

When the `site.federation_domain` site config key is set, Jaunder operates as a full ActivityPub server.

#### Actor Model

Each Jaunder user gets an AP Actor at `https://<domain>/users/<username>` with:
- `type: Person`
- `inbox`, `outbox`, `followers`, `following`, `publicKey`
- User-configurable `manuallyApprovesFollowers` flag (locked account)

#### Visibility Scopes

Posts created in Jaunder support all standard AP visibility levels:
- **Public**: addressed to `as:Public`; appears in public timelines
- **Unlisted**: delivered to followers; not in public timelines
- **Followers-only**: addressed to the followers collection
- **Direct**: addressed to mentioned actors only

#### HTTP Signatures

Jaunder uses the **Cavage HTTP Signatures draft** (the de facto standard across Mastodon, Misskey, Pleroma, Pixelfed, etc.) for signing outbound AP requests and verifying inbound ones.

The signing implementation is abstracted behind a `HttpSigner` trait to allow future addition of **RFC 9421** (HTTP Message Signatures) when ecosystem support matures. Inbound verification will accept both formats when RFC 9421 support is added.

#### Inbox Processing

The inbox handler:
1. Verifies the HTTP Signature of the incoming request.
2. Parses the Activity type (`Create`, `Announce`, `Like`, `Follow`, `Undo`, `Delete`, `Update`, etc.).
3. Fans out activities to affected users' content layers.
4. Queues outbound side-effects (e.g., `Accept` in response to `Follow`, delivery of replies to followers).

### AT Protocol PDS (Operator-Optional)

When the `site.at_pds_domain` site config key is set, Jaunder operates as an AT Personal Data Server (PDS).

#### Identity Model

Each Jaunder user is provisioned a DID (`did:plc` by default) registered with the PLC directory, and an AT handle at `<username>.<at_pds_domain>` verified via DNS TXT record. Users may bring their own handle by pointing their domain's DNS TXT record to their DID.

#### Repo Model

Each user's AT content is stored in a signed Merkle tree (MST) repo per the `com.atproto.sync` spec. Jaunder manages per-user signing keys. Repos are served via the standard `com.atproto.repo.*` and `com.atproto.sync.*` XRPC endpoints.

#### Relay Sync

Jaunder exposes a `com.atproto.sync.subscribeRepos` firehose endpoint that relays subscribe to, and calls `com.atproto.sync.requestCrawl` on the configured AT PDS relay URL to prompt initial crawling. This makes Jaunder users' posts visible in the broader AT network, including the Bluesky AppView.

#### Visibility

AT Protocol has no equivalent to AP's visibility levels. All records committed to a user's AT repo are public on the open firehose by default; there is no followers-only or direct-message primitive in the base protocol. When a user publishes a post in Jaunder, AP visibility settings apply only to the AP delivery path. AT delivery is always public.

#### Supported Lexicons

Jaunder implements the Bluesky app lexicons required for interoperability:
- `app.bsky.feed.post` — posts
- `app.bsky.feed.like` — likes
- `app.bsky.feed.repost` — reposts
- `app.bsky.graph.follow` — follows
- `app.bsky.actor.profile` — user profiles

#### OAuth 2.0 Authentication

Jaunder implements the AT Protocol OAuth 2.0 profile for authenticating external AT clients against the PDS.

### Publishing

#### Original Posts

The default post visibility is site config (instance-wide default) and can be overridden per user as user config.

Users compose posts in **Markdown** with optional rich content attachments (images, video links). Jaunder:
- Stores the source Markdown.
- Renders to sanitized HTML for AP `Note` objects and the user's published feeds.
- Delivers the AP `Create` activity to followers' inboxes.
- Commits the post as an `app.bsky.feed.post` record to the user's AT repo (when AT PDS mode is enabled). If the post body exceeds AT Protocol's 300-grapheme limit, the AT record contains a truncated excerpt and a link to the full post at its Jaunder URL.
- Publishes the post to the user's feeds in all supported formats:
  - RSS at `https://<domain>/users/<username>/feed.xml`
  - Atom at `https://<domain>/users/<username>/feed.atom`
  - JSON Feed at `https://<domain>/users/<username>/feed.json`
- Makes the rendered post available with a unique URL.

#### Reposting / Boosting

Users can repost content from any consumed source:
- For AP content: sends an AP `Announce` activity, attributed to the original author.
- For AT content: commits an `app.bsky.feed.repost` record to the user's AT repo.
- For RSS, Atom, or JSON Feed content: creates an AP `Announce` with a link back to the original URL. Since these formats have no native repost mechanism, this is a Jaunder-originated activity that surfaces the original.

#### Quote Post

Users can quote-post content from any consumed source, adding their own commentary alongside the quoted item:
- **AP content**: Creates a new `Note` using the de-facto `quoteUri` extension pointing to the original object URL.
- **AT content**: Commits an `app.bsky.feed.post` record with an `app.bsky.embed.record` embedding referencing the quoted AT URI.
- **RSS, Atom, or JSON Feed content**: Creates a new post with a link and excerpt from the original item, published as a standard AP `Note` with `quoteUri`.

#### Media Attachments (Own Posts)

User-uploaded media is stored locally by Jaunder and served from `https://<domain>/media/<hash>`. Supported types: images (JPEG, PNG, GIF, WebP, AVIF), video links.

### Post Editing and Deletion

#### Outbound (user's own posts)

- **Edit**: Users can edit their own posts after publishing. Jaunder updates the stored Markdown and re-renders all derived forms.
  - AP: sends an `Update` activity to followers' inboxes with the revised `Note` object.
  - AT: overwrites the record in the user's MST repo (AT records are mutable by rkey).
  - Feeds: the updated post is reflected in the user's RSS, Atom, and JSON Feed outputs.
- **Delete**: Users can delete their own posts.
  - AP: sends a `Delete` activity to followers' inboxes.
  - AT: deletes the record from the user's MST repo (committed as a tombstone).
  - Feeds: the item is removed from the user's published feeds.

#### Inbound (posts from followed sources)

Consistent with the **high-fidelity** design goal, Jaunder retains all versions of received content rather than silently overwriting or discarding:

- **AP `Update` activity**: The revised `Note` is stored as a new immutable revision alongside all prior versions. The current version is shown in the UI; all prior versions are retained and accessible.
- **AP `Delete` activity**: The item may be hidden from the user's timeline and feed views (configurable), but the previously stored content (and any prior revisions) is retained in the database.
- **AT record edits and deletes**: Handled equivalently — new record versions are stored alongside prior ones; deleted records are hidden from the timeline while the stored data is retained.
- **RSS, Atom, and JSON Feed**: These formats have no native edit or delete mechanism. If a re-fetched item has changed content for a previously known GUID, Jaunder stores the new version alongside the previous one. Removed items are not detectable by Jaunder; they simply stop appearing in future fetches.

### Content Interactions

#### Reply

- **To an AP post**: Jaunder sends a `Create` activity with `inReplyTo` set to the AP object URL. Delivered to the original author's inbox and Jaunder's followers.
- **To an AT post**: Commits an `app.bsky.feed.post` record with `reply.root` and `reply.parent` set to the appropriate AT URIs.
- **To an RSS, Atom, or JSON Feed item**: The reply is stored locally. Jaunder also creates an AP post with `inReplyTo` pointing to the item's URL. If the source is itself an AP-enabled blog (e.g., WordPress + AP plugin), the reply will be delivered and may surface as a comment on the original post. Jaunder additionally sends a **WebMention** to the item URL if the source advertises a WebMention endpoint.

#### Boost / Repost

See "Publishing" above.

#### Like / React

- **AP**: sends a `Like` activity to the author's inbox.
- **AT**: commits an `app.bsky.feed.like` record to the user's AT repo.
- **RSS/Atom/JSON Feed**: stored locally only (no upstream mechanism).

#### Quote Post

See "Publishing" above.

#### Bookmark

Private local save. No federation. Exempt from retention pruning.

### Threading

AP conversations have native thread structure via `inReplyTo` and `replies` collections. Jaunder fetches and stores thread context when ingesting AP content.

AT conversations have native thread structure via `reply.root` and `reply.parent` URI fields. Jaunder resolves thread context from the origin PDS when ingesting AT content.

For RSS, Atom, and JSON Feed, Jaunder synthesizes thread linkage by matching AP posts' `inReplyTo` URLs against known feed item URLs. This allows AP replies to a blog post to appear alongside the feed item in Jaunder's UI.

The UI attempts to provide a thread view for any content that supports it.

### Content Moderation

#### User-Level Controls

- **Block**: reject/drop all content from a specific AP actor, AT DID, or domain. Applied at ingestion time.
- **Mute**: ingest content but hide it from the user's timeline. Can be unmuted.
- **Content warnings**: AP `summary`/`sensitive` fields and AT label-based sensitivity markers are propagated; users can collapse CW'd or sensitive-labelled content by default.

#### Operator-Level Controls

- **Defederation list**: instance-wide block of AP domains and AT DIDs/domains. Applied before any user-level processing. No content from blocked sources reaches any user.
- **AT labeler**: optionally subscribe to one or more AT labeler services; apply label-based filtering rules to ingested AT content.

### Notifications

Jaunder generates notifications for the following events:

| Event          | Trigger                                                                                                        |
|----------------|----------------------------------------------------------------------------------------------------------------|
| New follower   | AP: incoming `Follow` activity; AT: incoming `app.bsky.graph.follow` record targeting the user                 |
| Follow request | AP: incoming `Follow` when account has `manuallyApprovesFollowers` set (pending approval)                      |
| Reply to post  | AP: incoming `Create` with `inReplyTo`; AT: incoming `app.bsky.feed.post` with `reply.parent` targeting a user's post |
| Mention        | AP: incoming post with `@`-mention; AT: incoming post with a `mention` facet referencing the user's DID        |
| Boost of post  | AP: incoming `Announce`; AT: incoming `app.bsky.feed.repost` targeting a user's post                          |
| Like of post   | AP: incoming `Like` activity; AT: incoming `app.bsky.feed.like` targeting a user's post                        |

Notifications are stored per-user and exposed via the API. Jaunder also delivers notifications via the following push mechanisms; each is enabled independently when its site config credentials are present:

- **Email**: uses the SMTP site config shared with the authentication system.
- **Web Push (VAPID)**: requires a VAPID key pair in site config (generated automatically at `jaunder init`). Usable by the web front-end and any client that registers a push subscription.
- **APNs**: requires an APNs certificate and private key in site config. Enables direct push to Apple devices for native iOS/macOS clients.
- **FCM**: requires an FCM API key in site config. Enables push via Firebase Cloud Messaging for native Android clients.

Which event types trigger notifications, and which delivery mechanisms are active, are user config. The notification delivery architecture is designed to accommodate additional push mechanisms over time.

### Media Handling

| Content type                     | Storage                                       |
|----------------------------------|-----------------------------------------------|
| Media uploaded by the user       | Stored locally; served from Jaunder           |
| Media linked in consumed content | Link-only by default                          |
| Consumed media (user opt-in)     | Optionally cached locally per user preference |

Media caching for consumed content is user config and defaults to off. When enabled, Jaunder fetches and stores media attachments at ingestion time, protecting against link rot.

### Multi-User Behavior

A Jaunder instance can serve multiple users. Key behaviors:

- **Shared ingestion**: If users A and B both follow `https://example.com/feed.xml`, Jaunder fetches it once and fans out to both users' content layers.
- **Private copies**: Each user's items, read state, interactions, and notifications are stored separately with no cross-user access.
- **Shared AP actor cache**: Remote AP actor metadata (profile, public key) is cached once and reused across users.
- **Shared AT DID/actor cache**: Remote AT DID documents and PDS metadata are cached once and reused across users.
- **AP inbox fan-out**: Activities delivered to the instance inbox are routed to the appropriate user(s) based on addressing.
- **AT Jetstream fan-out**: Events received from the relay's Jetstream subscription are routed to all local users who follow the relevant AT accounts.

### Search

Jaunder provides full-text search over a user's content layer. Search is strictly scoped to the requesting user — no query ever returns content from another user's layer.

Indexed fields:
- `title` (where present)
- `body_text`
- `author.name`
- `tags`

Search results are returned in relevance order and include items from all source types (`activitypub`, `rss`, `atom`, `json_feed`, `at`). The search index is maintained incrementally as content is ingested or deleted.

### Backup and Restore

Jaunder performs automatic backups on a configurable schedule. A backup captures both the database and the local media directory into a single `.tar.gz` archive. The destination path, schedule (cron expression), and archive retention count are site config. Until the destination path is configured, Jaunder surfaces a persistent warning to the operator (fulfilling the "Easy maintenance" design goal).

Manual backup and restore are also available via the CLI:

```
jaunder backup   # write a backup archive immediately
jaunder restore  # restore from a specified archive
```

