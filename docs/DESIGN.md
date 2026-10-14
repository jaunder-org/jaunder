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

#### AtomPub encoding and conditional writes

An AtomPub Post's strong `ETag` is its canonical `If-Match` validator for later
PUT and DELETE. Caddy's `encode zstd gzip` rewrites strong ETags on coded
responses, which makes an otherwise-current write fail with `412` when the
client replays the received value. Jaunder sends `Cache-Control: no-transform`
on `/atompub/*` responses so compliant encoders do not change the bytes or
validator. If you add response compression to the application's Caddy virtual
host, also exclude the AtomPub routes explicitly:

```caddyfile
@nonAtomPub not path /atompub/*
encode @nonAtomPub zstd gzip
```

Do not keep an additional unconditional `encode` on that virtual host; it would
defeat the path exception. Other application routes and public Syndication Feeds
can still be compressed. A custom proxy must likewise honor `no-transform` or
exclude `/atompub/*`; never rewrite `If-Match` or teach the server to accept a
proxy-specific ETag suffix. The opt-in live regression
`server/tests/proxy/atompub_etag.py` accepts Jaunder and Caddy binary paths
(`--jaunder`, `--caddy`) plus `--emacs`. It tests the original encoded false
`412`, then isolates the server header and proxy path exception, and finally
tests both advertised encodings and an Emacs `local-ahead` push against the
combined configuration. This is the
[AtomPub validator delivery decision](adr/0210-atompub-conditional-etag-delivery.md).

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

An optional distinct HTTPS operator host publishes the three prefixed
observability routes. It follows the DNS hostname contract above and must remain
distinct from the application host after lowercasing. Every external request,
including query APIs and unmatched paths, passes hostname-wide Basic Auth before
proxying. Collector ingestion stays credential-free on loopback.

##### Recommended runtime-file configuration

```nix
{
  imports = [ inputs.jaunder.nixosModules.jaunder-stack ];
  services.jaunder.stack = {
    enable = true;
    hostName = "jaunder.example.com";
    observability = {
      hostName = "observe.example.com";
      basicAuth = {
        credentialsFile = "/run/observability-auth/users";
      };
    };
  };
}
```

This example relies on the unchanged defaults: SQLite, file algorithm `bcrypt`,
and empty inline `username`/`passwordHash`. Set
`basicAuth.credentialsFileAlgorithm = "argon2id"` only when using Argon2id
entries. `credentialsFile` defaults to `null`. Supply an absolute runtime
filename **string**, not a Nix path literal, glob, `builtins.readFile`, or
secret content interpolated into Nix. Evaluation/build never reads or copies the
file contents; only runtime references enter generated configuration, units and
store artifacts. When enabling the observability host, exactly one complete
source is required: this file with both inline fields empty, or a valid inline
username/hash pair. Conflicting and incomplete declarations fail evaluation.

File mode requires the host's configured `systemd.package` to be version **260
or newer**, not merely a recent `system.stateVersion`. Older configured packages
are rejected; there is no silent fallback. Lifecycle proof uses pinned systemd
261.2 and Caddy 2.11.4, not a promise of execution on every consumer's nixpkgs.

##### Entry format and private provisioning

The file must be nonempty ASCII, with exactly two whitespace-separated tokens
per line: `username encoded-hash`. Usernames match `[A-Za-z0-9._-]+`. No blank
lines, comments, directives, braces, imports, or third token are allowed; a
final newline is fine. All entries use the selected `credentialsFileAlgorithm`:

- `bcrypt` accepts complete `$2a$` or `$2b$` hashes, cost 04–31.
- `argon2id` accepts `$argon2id$v=19$m=...,t=...,p=...$salt$hash`. Costs are
  positive decimal integers; memory is at least eight times parallelism. Memory
  and iterations are at most 4294967295; parallelism at most 16777215. Salt/hash
  use canonical **unpadded standard Base64** (not URL-safe Base64), decoding to
  at least 8/4 bytes respectively. The salt is embedded in the PHC hash, never a
  separate third token.

Prefer Caddy-generated hashes. On the host, use a compatible Caddy binary on
`PATH`, disable shell tracing, and hash through stdin so plaintext is neither an
external command argument nor shell-history text. This Bash example creates or
atomically replaces a root-owned file without displaying its hash:

```bash
set +x
sudo -v
sudo install -d -o root -g root -m 0700 /run/observability-auth
read -rs -p 'Password: ' password; printf '\n'
if hash=$(printf '%s' "$password" | caddy hash-password --algorithm bcrypt); then
  unset password
  sudo install -o root -g root -m 0600 /dev/null /run/observability-auth/users.new &&
    printf 'operator %s\n' "$hash" | sudo tee /run/observability-auth/users.new >/dev/null &&
    sudo mv /run/observability-auth/users.new /run/observability-auth/users
else
  unset password
  printf 'Hash generation failed; credentials not replaced\n' >&2
fi
unset hash
```

For Argon2id, use `--algorithm argon2id` and set the file algorithm accordingly.
Check that each provisioning command succeeds before proceeding. Use a strong
password: possession of a hash still permits offline guessing. Do not paste
credential contents into logs, Nix configuration, or tracked fixtures.

Systemd's manager reads the source; it can be `root:root 0600` inside a
`root:root 0700` directory. Caddy need not read that source directly. Keep the
source stable during the lifecycle operation and prefer runtime locations over
home directories hidden by service hardening. `/run` is volatile: manual
provisioning must happen before starting Caddy after each boot; use durable
protected operator storage or boot-time secret provisioning for unattended
hosts.

Manual provisioning, sops-nix and agenix are all supported without depending on
one manager. For either secret manager, configure its decrypted output filename,
root ownership and restrictive permissions, then point `credentialsFile` at that
runtime filename string. Order the **actual unit that finishes provisioning**
before `caddy.service`: set `systemd.services.caddy.after` and `requires` to
that unit when service-based provisioning needs explicit ordering. Confirm its
name and boot/rotation behavior in your installed secret-manager module; do not
assume that importing a manager, ordering alone, or a Nix string creates the
file. Never give Caddy write access to the source or loosen permissions for
unrelated readers merely to satisfy startup.

##### Reload, restart, rotation and failures

The stack adds one `LoadCredential` named `jaunder-observability`, providing a
private read-only copy to Caddy, and imports that literal copy inside Basic
Auth. Strict copy validation runs before startup and inside `ExecReload`
**after** systemd refresh and **before** Caddy adaptation/submission. It rejects
missing credential environment/copy, skipped setup, unreadable/empty/malformed
entries and algorithm mismatch; no source fallback or empty unauthenticated
route exists. Caddy's importer/provisioner alone does not establish valid
credential contents.

`RefreshOnReload=credentials` preserves normal unprivileged shared-Caddy reload,
without a privileged helper or disabling reload. Unlike unrefreshed
`LoadCredential`, this supported setting allows **reload to rotate
credentials**. Atomically replace the source using your provisioning method,
then either:

```bash
sudo systemctl reload caddy.service
# Or refresh through a full service restart:
sudo systemctl restart caddy.service
```

Source replacement alone does not activate new authentication. With normal Caddy
reload enabled, successful reload refreshes the copy and applies current Caddy
configuration, including other virtual hosts sharing the service. Restart also
refreshes the copy, but interrupts that shared service. If your host explicitly
disables Caddy reload, use restart. Test old credentials are denied and new ones
work at the frontends and query APIs after the chosen operation.

A failed refresh or strict validation causes reload to fail while retaining the
previous **parsed authenticated configuration** in the healthy main process.
This is not a credential-mount rollback guarantee: a rejected new copy may
already be mounted. Repair the source/provisioning or configuration, then retry
reload or restart. Invalid startup fails rather than opening HTTPS without
protection; do not restart a healthy instance with known-invalid credentials as
a recovery shortcut. Inspect service status/journal without printing secret
contents.

The goal is reducing exposure to unrelated local/store readers, not protection
from root or the trusted Caddy process. Root and Caddy can access credentials;
Caddy's protected runtime autosave and admin configuration may contain imported
hashes. Protect its state and admin access as part of the same trust boundary.

##### Compatible inline configuration

The existing `username` and `passwordHash` string options retain empty defaults;
set both for inline mode and leave `credentialsFile = null`. Complete `$2a$`/
`$2b$` bcrypt and `$argon2id$` hashes select their algorithm automatically;
`credentialsFileAlgorithm` affects only file mode. Inline mode does not require
systemd 260 and remains compatible, but its literal evaluated hash enters the
Caddy configuration and Nix store, allowing offline guessing by store readers.
Prefer file mode for production. Hash interactively through stdin as above and
store only the hash, never the plaintext, if choosing this tradeoff.

Use `https://observe.example.com/metrics/`, `https://observe.example.com/logs/`,
and `https://observe.example.com/traces/`. The separate hostnames, native
prefixes, loopback backends, credential-free collector ingestion and application
trusted-proxy boundary are unchanged.

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
