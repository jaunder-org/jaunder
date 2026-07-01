# Design

## Operational Model

`Jaunder` ships as a single binary (see
[ADR-0008](decisions/0008-deployment-model.md)) and takes its basic config from
the command line or environment variables.

Getting an instance up and running is designed to be simple:

```
jaunder init # initial, guided database setup
jaunder serve
```

Additional configuration is performed via the web interface or CLI. Jaunder does
not implement HTTPS directly, expecting to run behind a reverse proxy for TLS
termination.

## Interfaces

- **CLI**: Administrative tasks (setup, backup/restore, configuration).
- **Interoperability APIs**: ActivityPub (including WebFinger), AT Protocol
  (XRPC).
- **Compatibility APIs**: Mastodon client API shim.
- **Native API**: The authoritative interface for all Jaunder capabilities.
- **Web frontend**: The default interactive interface (built with Leptos).

## UI & User Experience

### Timelines

Jaunder provides several views for consuming content, all using cursor-based
pagination (see [ADR-0004](decisions/0004-pagination-strategy.md)):

- **Local timeline** (public): Original posts by local users.
- **User timeline** (public): Original posts by a specific local user.
- **Federated timeline** (authenticated): Combined feed from all sources a user
  follows.

#### Read State

Jaunder tracks read/unread state per item in each user's content layer (see
[ADR-0006](decisions/0006-storage-isolation.md)). Items are marked read
automatically as they are scrolled past.

### Account and Profile Management

Each user manages their own profile and social graph through a dedicated account
area:

- **Profile**: Display name, bio, and avatar.
- **Source management**: Adding/removing feeds, AP actors, and AT accounts.
- **Lists**: Following, Followers, Blocks, and Mutes.
- **Sessions**: Individual revocation of device tokens.

## Functional Architecture

### Unified Content Model

Jaunder normalizes data from diverse protocols into a unified core while
retaining high-fidelity raw payloads (see
[ADR-0005](decisions/0005-unified-content-model.md)).

### Ingestion & Federation

Jaunder prioritizes real-time delivery via push mechanisms (ActivityPub Inbox,
WebSub, AT Jetstream) and falls back to adaptive polling for other sources (see
[ADR-0010](decisions/0010-protocol-integration.md)).

### Retention & History

Consonant with its role as a high-fidelity reader, Jaunder retains an immutable
history of edits and preserves content locally even if it is deleted from the
source (see [ADR-0009](decisions/0009-edit-delete-policy.md)).

### Media Handling

User-uploaded media is served directly by the binary (see
[ADR-0003](decisions/0003-asset-management.md)). Media linked in external
content can be optionally cached per-user to protect against link rot.
