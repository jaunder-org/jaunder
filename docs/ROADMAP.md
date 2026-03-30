# Roadmap

## M0: Project structure and tooling

Rename workspace crates to names that will age well, install git hooks for pre-commit and pre-push checks, and configure CI as the authoritative enforcement gate. The goal is a clean, well-guarded foundation before any application code is written.

## M1: Deployment bootstrap

The `jaunder init` command: reads bootstrap config (bind address and storage path from CLI args or `JAUNDER_*` environment variables), runs the initial guided setup, creates the database, and applies the initial schema migrations. Without this milestone there is no runnable instance.

## M2: Email and SMTP

SMTP site config (relay host, port, credentials, sender address). Optional email address on user accounts with verification flow. Enables password reset email delivery (used in M3) and email notification delivery (used in M10).

## M3: User management

User registration (username + password), login returning an opaque device token, and Bearer token authentication middleware. User profile: display name, bio, and avatar image. Active sessions view with individual token revocation. Registration policy: open, invite-only, or closed, with invite generation via CLI or web interface. Password reset: username-keyed, time-limited token delivered via email (requires SMTP configured in M2).

## M4: Simple blog

Users can write posts and read them back. Builds on the auth layer from M2: publishing (Markdown in, HTML out), cursor-paginated public local timeline at `/` and per-user timeline at `/~:username/`, and a basic web UI wired to all of the above.

## M5: Backup and operations

CLI commands for `backup` and `restore`. Automatic scheduled backup to a configurable path capturing both the database and local media directory. Operator warning surfaced in the web interface until backup destination is configured.

## M6: Media handling

User media upload and local serving. Optional per-user caching of consumed media attachments at ingestion time, protecting against link rot.

## M7: Published feeds

Each user's posts are published as RSS, Atom, and JSON Feed at canonical per-user URLs. Covers feed generation, incremental updates as posts are created, edited, or deleted, and correct handling of the unified content model's fields in each format's native schema.

## M8: ActivityPub — subscribe and read

Local users can follow remote AP accounts. Covers WebFinger handle resolution, AP actor discovery and caching, fetching remote objects, and verifying inbound activities (Create, Announce, Update, Delete) via HTTP Signatures. Inbound content is fanned out to the following users' private content layers and appears in their federated home timeline. Thread context is fetched and stored for AP content, with a thread view surfaced in the UI. No outbound federation — Jaunder consumes AP content but does not yet present itself as an AP server.

## M9: Per-feed retention

Users can configure a retention window per source (e.g. "keep only the last 60 days of items from this feed"). Items that have been bookmarked, liked, replied to, or reposted are exempt from pruning. Pruning runs on a background schedule.

## M10: Read state, search, and notifications

Tracks per-user read/unread state on all content, with unread counts and jump-to-first-unread in the UI. Full-text search over a user's private content layer. Notifications for follows, replies, mentions, boosts, and likes, surfaced in a notifications view in the web UI and optionally delivered via email or Web Push (VAPID).

## M11: ActivityPub — publish

Jaunder operates as a full ActivityPub server when `site.federation_domain` is configured. Each local user gets an AP actor with inbox, outbox, followers, and following collections. Outbound posts are delivered to followers' inboxes as signed `Create` activities. Handles inbound Follow, Undo, and Accept; delivers `Update` and `Delete` for edited and deleted posts. Remote users can follow and interact with local users.

## M12: Account and profile management

A dedicated account management area in the web UI and native API consolidating the social graph features accumulated across prior milestones: following and followers lists with individual removal, follow request approval for locked accounts, block and mute list management, per-source settings (retention window, mute), notification preference configuration, and AP account settings (manuallyApprovesFollowers).

## M13: Native API and mobile clients

Stabilize and document the native API as a first-class interface covering all application capabilities. Build mobile clients targeting the native API, providing an alternative to the default web front-end for iOS and Android.

## M14: Content interactions and moderation

Reply, boost/repost, quote-post, like, and bookmark interactions across all source types (AP, AT, feeds). User-level controls: block, mute, content-warning handling. Operator-level controls: defederation list, AT labeler integration. Post editing and deletion with full revision history retained.

## M15: Mobile push notifications

APNs and FCM push delivery for notification events, enabling direct push to native iOS/macOS and Android clients. Each mechanism is enabled independently when its credentials are present in site config.

## M16: Feed ingestion

Users can follow RSS, Atom, and JSON Feed sources. Covers source discovery (direct URL or auto-discovery from a blog's HTML), polling on an adaptive schedule, WebSub subscription for push delivery, and fan-out to each following user's private content layer. When replying to a feed item, Jaunder sends a WebMention to the source if it advertises a WebMention endpoint. The federated home timeline shows everything a user follows across all source types.

## M17: OPML import and export

Users can bulk-import feed subscriptions from an OPML file and export their current subscriptions as OPML.

## M18: AT Protocol PDS

Jaunder operates as an AT Personal Data Server when `site.at_pds_domain` is configured. Each user is provisioned a DID and AT handle; posts are committed to a signed MST repo and exposed via the standard `com.atproto` XRPC endpoints. Jaunder subscribes to the AT Jetstream relay for near-real-time delivery of followed AT accounts' content, and exposes a `subscribeRepos` firehose so that relays can crawl it.

## M19: Mastodon compatibility API

Implements the Mastodon client REST API as a compatibility shim, allowing existing Mastodon clients to connect to a Jaunder instance without modification. Covers the subset of endpoints needed for timelines, posting, follows, notifications, and account management.

## M20: PostgreSQL backend

Adds PostgreSQL as a second storage backend behind the existing database trait abstraction. Targets heavier multi-user deployments and high-volume AP instances. Full-text search is implemented via `tsvector` columns and GIN indexes rather than SQLite FTS5.

## M21: OAuth 2.0

Jaunder acts as an OAuth 2.0 authorization server, allowing first-party and third-party clients to request scoped tokens via the standard authorization code flow. Also satisfies the AT Protocol OAuth profile required for authenticating external AT clients against the PDS.
