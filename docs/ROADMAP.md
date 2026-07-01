# Roadmap

## ✅ Completed Milestones

### M0: Project structure and tooling

Project foundation established with clean workspace organization and CI
enforcement.

### M1: Deployment bootstrap

`jaunder init` and guided setup implemented.

### M2: User management

Authentication, registration, and profile management (display name, bio, avatar)
implemented.

### M3: Email and SMTP

SMTP integration and email verification flows established.

### M4: PostgreSQL backend

Full parity between SQLite and PostgreSQL backends implemented and tested.

### M5: Simple blog

Post creation and public timelines (local and user-specific) implemented.

### M6: Backup and operations

CLI-based `backup` and `restore` commands implemented for both database and
media.

### M7: Media handling

Local media upload, serving, and storage management implemented.

---

## 🚀 Future Milestones

### M8: Published feeds

Each user's posts are published as RSS, Atom, and JSON Feed at canonical
per-user URLs. Covers feed generation, incremental updates as posts are created,
edited, or deleted, and correct handling of the unified content model's fields
in each format's native schema.

### M9: ActivityPub — subscribe and read

Local users can follow remote AP accounts. Covers WebFinger handle resolution,
AP actor discovery and caching, fetching remote objects, and verifying inbound
activities (Create, Announce, Update, Delete) via HTTP Signatures. Inbound
content is fanned out to the following users' private content layers and appears
in their federated home timeline. Thread context is fetched and stored for AP
content, with a thread view surfaced in the UI. No outbound federation — Jaunder
consumes AP content but does not yet present itself as an AP server.

### M10: Per-feed retention

Users can configure a retention window per source (e.g. "keep only the last 60
days of items from this feed"). Items that have been bookmarked, liked, replied
to, or reposted are exempt from pruning. Pruning runs on a background schedule.

### M11: Read state, search, and notifications

Tracks per-user read/unread state on all content, with unread counts and
jump-to-first-unread in the UI. Full-text search over a user's private content
layer. Notifications for follows, replies, mentions, boosts, and likes, surfaced
in a notifications view in the web UI and optionally delivered via email or Web
Push (VAPID).

### M12: ActivityPub — publish

Jaunder operates as a full ActivityPub server when `site.federation_domain` is
configured. Each local user gets an AP actor with inbox, outbox, followers, and
following collections. Outbound posts are delivered to followers' inboxes as
signed `Create` activities. Handles inbound Follow, Undo, and Accept; delivers
`Update` and `Delete` for edited and deleted posts. Remote users can follow and
interact with local users.

### M13: Account and profile management

A dedicated account management area in the web UI and native API consolidating
the social graph features accumulated across prior milestones: following and
followers lists with individual removal, follow request approval for locked
accounts, block and mute list management, per-source settings (retention window,
mute), notification preference configuration, and AP account settings
(manuallyApprovesFollowers).

### M14: Native API and mobile clients

Stabilize and document the native API as a first-class interface covering all
application capabilities. Build mobile clients targeting the native API,
providing an alternative to the default web front-end for iOS and Android.

### M15: Content interactions and moderation

Reply, boost/repost, quote-post, like, and bookmark interactions across all
source types (AP, AT, feeds). User-level controls: block, mute, content-warning
handling. Operator-level controls: defederation list, AT labeler integration.
Post editing and deletion with full revision history retained.

### M16: Mobile push notifications

APNs and FCM push delivery for notification events, enabling direct push to
native iOS/macOS and Android clients. Each mechanism is enabled independently
when its credentials are present in site config.

### M17: Feed ingestion

Users can follow RSS, Atom, and JSON Feed sources. Covers source discovery
(direct URL or auto-discovery from a blog's HTML), polling on an adaptive
schedule, WebSub subscription for push delivery, and fan-out to each following
user's private content layer. When replying to a feed item, Jaunder sends a
WebMention to the source if it advertises a WebMention endpoint. The federated
home timeline shows everything a user follows across all source types.

### M18: OPML import and export

Users can bulk-import feed subscriptions from an OPML file and export their
current subscriptions as OPML.

### M19: AT Protocol PDS

Jaunder operates as an AT Personal Data Server when `site.at_pds_domain` is
configured. Each user is provisioned a DID and AT handle; posts are committed to
a signed MST repo and exposed via the standard `com.atproto` XRPC endpoints.
Jaunder subscribes to the AT Jetstream relay for near-real-time delivery of
followed AT accounts' content, and exposes a `subscribeRepos` firehose so that
relays can crawl it.

### M20: Mastodon compatibility API

Implements the Mastodon client REST API as a compatibility shim, allowing
existing Mastodon clients to connect to a Jaunder instance without modification.
Covers the subset of endpoints needed for timelines, posting, follows,
notifications, and account management.

### M21: OAuth 2.0

Jaunder acts as an OAuth 2.0 authorization server, allowing first-party and
third-party clients to request scoped tokens via the standard authorization code
flow. Also satisfies the AT Protocol OAuth profile required for authenticating
external AT clients against the PDS.

## Not yet scheduled

- Cross-backend data migration
- Full-text search
- Tag discovery (tag cloud, trending tags, tag autocomplete)
- Evaluate Org-mode parser options (orgize limitations, tree-sitter-org, custom
  parser)
