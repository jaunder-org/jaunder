# ADR-DRAFT: Trusted proxy client-IP derivation

- Status: proposed
- Date: 2026-09-21
- Issue: [#1609](https://github.com/jaunder-org/jaunder/issues/1609)

## Context

Jaunder runs as plain HTTP behind an external reverse proxy
([ADR-0008](../0008-deployment-model.md)). Its supported single-host stack puts
Caddy on the public network and loopback-bound Jaunder behind it
([ADR-0196](../0196-single-host-nixos-deployment-stack.md)). The server can
therefore observe the proxy's socket address, but it has no supported trust
boundary for attributing a request to the client address reported by that proxy.

A forwarding header is caller-controlled text unless the direct Transport Peer
is trusted. Always accepting its leftmost value would let a direct client spoof
an address; always ignoring it prevents future IP-aware controls and accurate
request diagnosis behind the required deployment topology. The two common header
families also differ in grammar and may be supplied together with conflicting
chains.

Client IP addresses are personal, unbounded telemetry values. Existing
observability policy forbids collecting raw PII merely because it is available
([ADR-0011](../0011-unified-observability.md)), while process configuration must
be resolved once at the executable boundary and injected as typed state
([ADR-0144](../0144-process-configuration-cli-contract.md),
[ADR-0158](../0158-peripheral-process-configuration.md)). The minimal NixOS
module is itself an operator compatibility surface whose option names, defaults,
and process-variable mappings require compatibility review
([ADR-0142](../0142-declarative-nixos-deployment-package-outputs.md)).

## Decision

Jaunder distinguishes two request identities:

- the **Transport Peer**, the direct socket endpoint and root of trust; and
- the **Effective Client IP**, an IP-only attribution derived from trusted proxy
  evidence.

`jaunder serve` accepts an explicit list of trusted proxy addresses or CIDRs
through repeatable `--trusted-proxy` flags or comma-separated
`JAUNDER_TRUSTED_PROXIES`. Members are trimmed. An absent, empty, or
whitespace-only whole environment value means no trusted proxy; an empty member
inside a nonempty list is invalid. Bare addresses denote one host, duplicates
and overlaps normalize to one trust set, and any invalid configured value fails
startup. IPv4-mapped IPv6 addresses canonicalize to plain IPv4 before trust
matching and chain comparison.

This decision evolves ADR-0142's public minimal-module surface with
`services.jaunder.trustedProxies`, a string list defaulting to empty. The module
omits `JAUNDER_TRUSTED_PROXIES` for an empty list and otherwise serializes its
members with commas. The owned stack sets exactly `127.0.0.1/32`, its immediate
Caddy hop, and removes inbound `Forwarded` before proxying; Caddy owns the
trusted-proxy-aware `X-Forwarded-For` chain. Deployment-specific CDN ranges
remain Caddy configuration and never become application defaults.

Resolution begins with the Transport Peer. Jaunder reads no forwarding header
unless that peer is trusted, then walks the reported chain from right to left
while each current hop is trusted. The first untrusted hop becomes the Effective
Client IP. If the chain is absent, unusable, longer than 32 hops, or contains
only trusted hops, the Transport Peer's IP remains effective. Without a
Transport Peer, no forwarding evidence is usable and no Effective Client IP is
produced. These conditions never reject the HTTP request.

Both RFC 7239 `Forwarded` and `X-Forwarded-For` are supported. Their accepted
node forms reduce to canonical IPv4 or IPv6 addresses; ports are discarded, and
hostnames, obfuscated or unknown nodes, zone identifiers, and malformed forms
are unusable. When both families are present, their canonical ordered chains
must agree. A malformed family or disagreement discards all forwarded evidence.
Vendor-specific headers, PROXY protocol, forwarded host/scheme, and trace
context are separate contracts and are not interpreted here.

Jaunder delegates CIDR handling, supported header grammar, address
canonicalization, Axum transport-peer extraction, and right-to-left trust-chain
resolution to the audited `axum-client-addr` 0.2 crate. A narrow Jaunder policy
layer owns only the stricter cross-header agreement, hop bound, bounded outcome
classification, and request context that preserves the Transport Peer beside the
Effective Client IP.

Neither address is authentication or authorization evidence. No current
application control consumes the Effective Client IP. HTTP telemetry may record
a closed resolution outcome, but it records no IP address and no raw header
content. A future IP-aware control or durable address sink requires a separate
policy decision.

## Consequences

Direct requests remain unable to spoof their client attribution, and existing
deployments retain their behavior until an operator explicitly configures a
trusted proxy. Multi-hop deployments gain one deterministic, vendor-neutral
resolution policy shared by native and NixOS operation.

The application depends on a young, narrowly scoped resolver crate. Pinning its
version, retaining Jaunder's conformance tests, and keeping the stricter policy
adapter at the boundary limit that supply-chain and semantic risk without
reimplementing general forwarding grammar.

Caddy and any upstream CDN must sanitize or normalize forwarding evidence before
it reaches Jaunder. The supported stack owns that immediate boundary; custom
operators own theirs. Clearing the trusted-proxy list is a safe rollback that
returns every request to Transport Peer attribution.

Preserving both identities increases request-context plumbing, but not telemetry
or storage cardinality. Address-based abuse controls, audit logs, host/scheme
reconstruction, dynamic vendor ranges, and PROXY protocol remain excluded.
