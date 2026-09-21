# Design

## Operational Model

`Jaunder` ships as a single binary (see
[ADR-0008](adr/0008-deployment-model.md)) and takes its basic config from the
command line or environment variables.

Getting an instance up and running is designed to be simple:

```
jaunder init # initial, guided database setup
jaunder serve
```

Additional configuration is performed via the web interface or CLI. Jaunder does
not implement HTTPS directly, expecting to run behind a reverse proxy for TLS
termination.

### NixOS deployment stack

For a supported single-host production composition, import
`nixosModules.jaunder-stack` and declare its application host:

```nix
{
  imports = [ inputs.jaunder.nixosModules.jaunder-stack ];

  services.jaunder.stack = {
    enable = true;
    hostName = "jaunder.example.com";
    database = "sqlite"; # or "postgresql" for the host's local instance
  };
}
```

The stack makes Caddy the only public listener on ports 80 and 443, with
automatic HTTPS for the application host. It trusts only its immediate loopback
Caddy hop (`127.0.0.1/32`) for client-address forwarding, removes inbound
`Forwarded` before proxying, and lets Caddy construct the `X-Forwarded-For`
chain. Each application or optional observability host name must be a DNS
hostname of at most 253 ASCII characters: dot-separated, nonempty labels of at
most 63 ASCII letters, digits, or hyphens, each beginning and ending with a
letter or digit. The optional observability host must differ from the
application host after lowercasing, because same-host Caddy definitions collide
and could authenticate or replace the application route. It runs production-mode
Jaunder, the OpenTelemetry Collector, VictoriaMetrics, VictoriaLogs, and
VictoriaTraces. The collector and all Victoria services, including their
built-in UIs, remain loopback-only. The collector sends metrics, logs, and
traces directly to the native `/metrics`, `/logs`, and `/traces` ingestion
prefixes over loopback; this traffic never traverses Caddy.

### Trusted reverse proxies

Jaunder trusts no forwarding header by default. For a custom NixOS deployment,
set only the addresses or CIDRs that can directly connect to Jaunder:

```nix
services.jaunder = {
  enable = true;
  prod = true;
  bind = "127.0.0.1:3000";
  trustedProxies = [ "127.0.0.1/32" ];
};
```

This projects the comma-separated process setting `JAUNDER_TRUSTED_PROXIES`. An
empty list (the default) omits it entirely, so a direct request cannot select an
Effective Client IP with `Forwarded` or `X-Forwarded-For`.

The directly connected proxy is responsible for sanitizing forwarding evidence.
A custom Caddy route must remove `Forwarded` and let Caddy append the immediate
peer to `X-Forwarded-For`:

```caddyfile
jaunder.example.com {
  reverse_proxy 127.0.0.1:3000 {
    header_up -Forwarded
  }
}
```

When a CDN precedes Caddy, configure Caddy—not Jaunder—with the CDN's exact
trusted ranges and strict right-to-left forwarding trust:

```caddyfile
{
  servers {
    trusted_proxies static 203.0.113.0/24 2001:db8::/32
    trusted_proxies_strict
  }
}
```

Replace the example ranges with the CDN's current published ranges. Normalize an
accepted provider-specific client header at Caddy into the standard forwarding
chain only after that trust check. Jaunder does not interpret `CF-Connecting-IP`
or other vendor-specific headers, and CDN ranges must not be added to
`trustedProxies`: Jaunder still trusts only its immediate Caddy Transport Peer.

To roll back forwarding attribution, clear `services.jaunder.trustedProxies`,
rebuild the host, and restart the service. Jaunder then ignores every forwarding
header and uses the Transport Peer again. This setting is diagnostic context,
not authentication or authorization evidence.

The default loopback UIs are:

- VictoriaMetrics: `http://127.0.0.1:8428/metrics/`
- VictoriaLogs: `http://127.0.0.1:9428/logs/`
- VictoriaTraces: `http://127.0.0.1:10428/traces/`

For remote operator access without publishing those UIs, forward all three ports
from a trusted admin machine, then use the same `127.0.0.1` URLs locally:

```bash
ssh -L 8428:127.0.0.1:8428 -L 9428:127.0.0.1:9428 -L 10428:127.0.0.1:10428 operator@example-host
```

#### Optional HTTPS observability host and Basic Auth

An optional distinct HTTPS operator host publishes only the three prefixed
observability routes. It follows the DNS hostname contract above and must remain
distinct from the application host after lowercasing. Configure both Basic Auth
fields when setting it. The username must match the exact ASCII grammar
`[A-Za-z0-9._-]+`:

```nix
services.jaunder.stack.observability.hostName = "observe.example.com";
services.jaunder.stack.observability.basicAuth.username = "operator";
services.jaunder.stack.observability.basicAuth.passwordHash = "$2b$..."; # bcrypt, or an $argon2id$... hash
```

`passwordHash` is a literal evaluated Nix string embedded in generated Caddy
configuration and the Nix store; it is not a runtime-file secret. Protect the
plaintext while generating the hash, use a strong password because an exposed
hash permits offline guessing, and store only the generated hash in host Nix
configuration.

Before running either command below, ensure a compatible Caddy binary is
installed and available on `PATH` on a trusted admin machine. Generate the hash
interactively so the plaintext does not appear in the command line or shell
history, then unset it:

```bash
read -rs -p 'Password: ' password; printf '\n'; printf '%s' "$password" | caddy hash-password --algorithm bcrypt; unset password
read -rs -p 'Password: ' password; printf '\n'; printf '%s' "$password" | caddy hash-password --algorithm argon2id; unset password
```

With the host configured, use `https://observe.example.com/metrics/`,
`https://observe.example.com/logs/`, and `https://observe.example.com/traces/`.
Caddy authenticates before proxying; collector ingestion continues directly over
loopback and carries no Basic Auth.

#### Database, retention, persistence, and maturity

The database choice defaults to SQLite. PostgreSQL mode additively enables the
host's ordinary PostgreSQL service and ensures the `jaunder` role and database.
Jaunder connects as the `jaunder` system user through the peer-authenticated
`/run/postgresql` Unix socket. The stack chooses no package and assigns no
PostgreSQL listener, authentication, or global policy. On the pinned NixOS
module, `enableTCPIP = false` still retains a localhost TCP listener; the stack
adds no non-loopback listener, host HBA rule, firewall opening, or other network
exposure. Keep existing PostgreSQL package, listener, HBA, and global-policy
declarations in their owning host configuration.

Use native service options for retention rather than stack options, for example:

```nix
services.victoriametrics.retentionPeriod = "14d";
services.victorialogs.extraOptions = [ "-retentionPeriod=14d" ];
services.victoriatraces.retentionPeriod = "30d";
```

The initial native retention is 31 days for metrics and 7 days for logs and
traces. Victoria data is persistent operational evidence, not Jaunder
application data: `jaunder backup` and `jaunder restore` neither include nor
recover it. Back up the telemetry stores separately if their retained evidence
is required.

VictoriaTraces is upstream work in progress. This stack supports fresh
deployment and same-version restart, not compatible on-disk upgrades or stable
third-party query APIs.

## Interfaces

- **CLI**: Administrative tasks (setup, backup/restore, configuration).
- **Interoperability APIs**: ActivityPub (including WebFinger), AT Protocol
  (XRPC).
- **Compatibility APIs**: Mastodon client API shim.
- **Native API**: The authoritative interface for all Jaunder capabilities.
- **Web frontend**: The default interactive interface (built with Leptos).

## UI & User Experience

### Timelines

Jaunder provides several views for consuming content. Every web Post timeline
uses cursor-based pagination (see [ADR-0004](adr/0004-pagination-strategy.md))
and lets the viewer choose Newest or Oldest publication order through URL state
(see [web Post timeline ordering](adr/0190-web-post-timeline-ordering.md)):

- **Local** (public `/`): Viewer-independent public Posts originating from local
  Users; it is the signed-out landing surface.
- **Home** (authenticated `/app`): The signed-in User's own published Posts and
  inline composer. Auth-marked visits to `/` redirect here before Local paints.
- **User timeline** (public): Original Posts by a specific local User.
- **Site-tag timeline** (public): Visible Posts carrying a Tag across the site.
- **User-tag timeline** (public): Visible Posts carrying a Tag from one local
  User.

Every Post on these public surfaces and on its permalink carries a Copyright
Declaration derived from its creation year, its author's current public name,
and that User's current Content License. Public Syndication Feed items expose
the same rights metadata without changing Post content.

#### Read State

Jaunder tracks read/unread state per item in each user's content layer (see
[ADR-0006](adr/0006-storage-isolation.md)). Items are marked read automatically
as they are scrolled past.

### Account and Profile Management

Each user manages their own profile and social graph through a dedicated account
area:

- **Profile**: Display name, bio, avatar, and a publication-wide Content License
  that defaults to All Rights Reserved and applies retroactively to every Post.
- **Source management**: Adding/removing feeds, AP actors, and AT accounts.
- **Lists**: Following, Followers, Blocks, and Mutes.
- **Sessions**: Individual revocation of device tokens.
- **Passkeys**: Enrollment and independent revocation of named browser
  credentials; password login and recovery remain available.

## Functional Architecture

### Unified Content Model

Jaunder normalizes data from diverse protocols into a unified core while
retaining high-fidelity raw payloads (see
[ADR-0005](adr/0005-unified-content-model.md)).

### Ingestion & Federation

Jaunder prioritizes real-time delivery via push mechanisms (ActivityPub Inbox,
WebSub, AT Jetstream) and falls back to adaptive polling for other sources (see
[ADR-0010](adr/0010-protocol-integration.md)).

### Retention & History

Consonant with its role as a high-fidelity reader, Jaunder retains an immutable
history of edits and preserves content locally even if it is deleted from the
source (see [ADR-0009](adr/0009-edit-delete-policy.md)).

### Media Handling

User-uploaded media is served directly by the binary (see
[ADR-0003](adr/0003-asset-management.md)). Media linked in external content can
be optionally cached per-user to protect against link rot.
