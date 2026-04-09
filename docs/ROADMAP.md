# Roadmap

## M0: Project structure and tooling

Rename workspace crates to names that will age well, install git hooks for pre-commit and pre-push checks, and configure CI as the authoritative enforcement gate. The goal is a clean, well-guarded foundation before any application code is written.

## M1: Deployment bootstrap

The `jaunder init` command: reads bootstrap config (bind address and storage path from CLI args or `JAUNDER_*` environment variables), runs the initial guided setup, creates the database where supported by the selected backend, and applies the initial schema migrations. Without this milestone there is no runnable instance.

## M2: User management

User registration (username + password), login returning an opaque device token, and Bearer token authentication middleware. User profile: display name, bio, and avatar image. Active sessions view with individual token revocation. Registration policy: open, invite-only, or closed, with invite generation via CLI or web interface.

## M3: Email and SMTP

SMTP site config (relay host, port, credentials, sender address). Optional email address on user accounts with verification flow. Password reset: username-keyed, time-limited token delivered via email (requires SMTP configured in M3). Enables email notification delivery (used in M11).

## M4: PostgreSQL backend

Adds PostgreSQL as a second database backend selectable at runtime. The backend is chosen via the `--db` bootstrap flag (or `JAUNDER_DB` environment variable), which accepts a URL encoding both engine and connection parameters (`sqlite:./data` or `postgres://...`); SQLite remains the default for small deployments. All M1–M3 application features are fully supported on both backends. The implementation uses separate SQLite and PostgreSQL storage implementations sharing traits, helper functions, and migration structure where practical, while each backend retains its own concrete pool type and native SQL types. A `jaunder migrate-db --to <url>` command transfers all data from the current backend to a new one, allowing operators to move between SQLite and PostgreSQL (or back) without data loss. Full-text search uses `tsvector` columns and GIN indexes on the PostgreSQL backend instead of SQLite FTS5.

From M4 onward, persisted-feature milestones must maintain backend parity: ship both migration
trees in the same change, implement storage-trait changes on both backends before merge, and add
tests for both backends unless a temporary backend-specific deferral is documented explicitly.

## M5: Simple blog

Users can write posts and read them back. Builds on the auth layer from M2: publishing (Markdown in, HTML out), cursor-paginated public local timeline at `/` and per-user timeline at `/~:username/`, and a basic web UI wired to all of the above.

## M6: Backup and operations

CLI commands for `backup` and `restore`. Automatic scheduled backup to a configurable path capturing both the database and local media directory. Operator warning surfaced in the web interface until backup destination is configured.

## M7: Media handling

User media upload and local serving. Optional per-user caching of consumed media attachments at ingestion time, protecting against link rot.

## M8: Published feeds

Each user's posts are published as RSS, Atom, and JSON Feed at canonical per-user URLs. Covers feed generation, incremental updates as posts are created, edited, or deleted, and correct handling of the unified content model's fields in each format's native schema.

## M9: ActivityPub — subscribe and read

Local users can follow remote AP accounts. Covers WebFinger handle resolution, AP actor discovery and caching, fetching remote objects, and verifying inbound activities (Create, Announce, Update, Delete) via HTTP Signatures. Inbound content is fanned out to the following users' private content layers and appears in their federated home timeline. Thread context is fetched and stored for AP content, with a thread view surfaced in the UI. No outbound federation — Jaunder consumes AP content but does not yet present itself as an AP server.

## M10: Per-feed retention

Users can configure a retention window per source (e.g. "keep only the last 60 days of items from this feed"). Items that have been bookmarked, liked, replied to, or reposted are exempt from pruning. Pruning runs on a background schedule.

## M11: Read state, search, and notifications

Tracks per-user read/unread state on all content, with unread counts and jump-to-first-unread in the UI. Full-text search over a user's private content layer. Notifications for follows, replies, mentions, boosts, and likes, surfaced in a notifications view in the web UI and optionally delivered via email or Web Push (VAPID).

## M12: ActivityPub — publish

Jaunder operates as a full ActivityPub server when `site.federation_domain` is configured. Each local user gets an AP actor with inbox, outbox, followers, and following collections. Outbound posts are delivered to followers' inboxes as signed `Create` activities. Handles inbound Follow, Undo, and Accept; delivers `Update` and `Delete` for edited and deleted posts. Remote users can follow and interact with local users.

## M13: Account and profile management

A dedicated account management area in the web UI and native API consolidating the social graph features accumulated across prior milestones: following and followers lists with individual removal, follow request approval for locked accounts, block and mute list management, per-source settings (retention window, mute), notification preference configuration, and AP account settings (manuallyApprovesFollowers).

## M14: Native API and mobile clients

Stabilize and document the native API as a first-class interface covering all application capabilities. Build mobile clients targeting the native API, providing an alternative to the default web front-end for iOS and Android.

## M15: Content interactions and moderation

Reply, boost/repost, quote-post, like, and bookmark interactions across all source types (AP, AT, feeds). User-level controls: block, mute, content-warning handling. Operator-level controls: defederation list, AT labeler integration. Post editing and deletion with full revision history retained.

## M16: Mobile push notifications

APNs and FCM push delivery for notification events, enabling direct push to native iOS/macOS and Android clients. Each mechanism is enabled independently when its credentials are present in site config.

## M17: Feed ingestion

Users can follow RSS, Atom, and JSON Feed sources. Covers source discovery (direct URL or auto-discovery from a blog's HTML), polling on an adaptive schedule, WebSub subscription for push delivery, and fan-out to each following user's private content layer. When replying to a feed item, Jaunder sends a WebMention to the source if it advertises a WebMention endpoint. The federated home timeline shows everything a user follows across all source types.

## M18: OPML import and export

Users can bulk-import feed subscriptions from an OPML file and export their current subscriptions as OPML.

## M19: AT Protocol PDS

Jaunder operates as an AT Personal Data Server when `site.at_pds_domain` is configured. Each user is provisioned a DID and AT handle; posts are committed to a signed MST repo and exposed via the standard `com.atproto` XRPC endpoints. Jaunder subscribes to the AT Jetstream relay for near-real-time delivery of followed AT accounts' content, and exposes a `subscribeRepos` firehose so that relays can crawl it.

## M20: Mastodon compatibility API

Implements the Mastodon client REST API as a compatibility shim, allowing existing Mastodon clients to connect to a Jaunder instance without modification. Covers the subset of endpoints needed for timelines, posting, follows, notifications, and account management.

## M21: OAuth 2.0

Jaunder acts as an OAuth 2.0 authorization server, allowing first-party and third-party clients to request scoped tokens via the standard authorization code flow. Also satisfies the AT Protocol OAuth profile required for authenticating external AT clients against the PDS.
