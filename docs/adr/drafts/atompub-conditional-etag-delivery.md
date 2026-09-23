# ADR-DRAFT: Preserve AtomPub Post validators across response encoders

- Status: proposed
- Date: 2026-09-23
- Issue: [#1641](https://github.com/jaunder-org/jaunder/issues/1641)

## Context

The server computes a strong content ETag for each AtomPub Post representation
and accepts it unchanged as `If-Match` for conditional writes. A production
Caddy `encode zstd gzip` directive rewrites that ETag with a coding suffix on a
compressed response. The Emacs Protocol Client stores the resulting wire ETag in
`JAUNDER_SYNCED`; its next otherwise-current conditional PUT receives `412`,
because Jaunder compares against its canonical unsuffixed validator. Caddy does
not rewrite `If-Match` for upstream writes. AtomPub is a distinct,
round-trippable editing surface rather than a public Syndication Feed
([separate serialization surfaces](../0015-atompub-serialization-surfaces.md)).

The reverse proxy belongs outside Jaunder, including in the single-host NixOS
stack ([deployment stack](../0196-single-host-nixos-deployment-stack.md)), and
operators may add their own encoding directives. Repairing only the supplied
Caddy configuration would leave other installations vulnerable; accepting an
intermediary's private suffix at the server would weaken conditional-write
identity.

## Decision

Jaunder marks every `/atompub/*` response `Cache-Control: no-transform`,
preserving any other cache restrictions. A compliant response encoder leaves
AtomPub bytes and canonical strong ETags untouched, including when clients
advertise Zstandard. Supported Caddy configurations additionally exclude
`/atompub/*` from response encoding while retaining compression elsewhere. These
are independent defenses: the server owns the HTTP contract, while the operator
owns the Caddy routing policy.

The Emacs Protocol Client requests `Accept-Encoding: identity` on its
authenticated AtomPub transport as a further safeguard. It continues to store
and replay the strong ETag it receives without suffix stripping. Jaunder's
comparison with the canonical ETag and its `412` response for genuinely stale
`If-Match` remain unchanged. The transport continues to use `plz`/curl
([Emacs HTTP transport](../0038-emacs-http-transport-plz-not-url-el.md)).

## Consequences

AtomPub responses no longer benefit from intermediary compression, including XML
documents and media served on `/atompub/*`. Public Syndication Feeds, web pages,
and assets remain eligible for compression. Custom proxies that ignore
`no-transform` still require an operator path exception; Jaunder does not treat
arbitrary coded ETags as valid write preconditions. Previously saved coded
validators are outside this decision and are not silently repaired.
