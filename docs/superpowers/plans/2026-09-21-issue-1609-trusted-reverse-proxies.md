# Trusted reverse proxies implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for a bounded task
> when useful. This outline exists because trusted forwarding headers create a
> public process-configuration and request-security boundary.

Spec: `docs/superpowers/specs/2026-09-21-issue-1609-trusted-reverse-proxies.md`

## Scope

In:

- Typed trusted-proxy configuration, bounded forwarding-chain policy, request
  context, and resolution outcome.
- CLI/environment and real Axum serving integration.
- Minimal and stack NixOS projections, Caddy boundary, tests, and operator
  documentation.
- The proposed ADR, architecture projection, and domain terms already authored
  with the spec.

Out:

- Any address-based application control or raw-address telemetry/storage.
- Host/scheme reconstruction, vendor headers, PROXY protocol, dynamic range
  discovery, and hot reload.

## Task outline

- [x] Task 1: Deliver the trusted-proxy policy module as a deep, host-testable
      seam.
  - Contract: a typed immutable trust configuration resolves headers plus an
    optional Transport Peer into request context containing the unchanged peer,
    optional Effective Client IP, and a closed resolution outcome.
    `axum-client-addr` 0.2 owns CIDR/header grammar and right-to-left traversal;
    the Jaunder adapter owns dual-header agreement, the 32-hop bound,
    mapped-IPv4 equality, and fail-closed fallback.
  - Verification: focused Rust tests cover every address, chain, conflict,
    malformed/opaque, over-limit, all-trusted, duplicate/overlapping range, and
    missing-peer case from the spec; run
    `devtool run -- cargo xtask test-local -- -p jaunder trusted_proxy`.

- [ ] Task 2: Carry the policy through process configuration and the real HTTP
      serving path.
  - Contract: `serve` resolves repeatable `--trusted-proxy` or trimmed
    comma-separated `JAUNDER_TRUSTED_PROXIES` once, then injects the typed
    snapshot through the composition root. Axum supplies
    `ConnectInfo<SocketAddr>`; middleware preserves it and inserts the derived
    context before request observability records only the closed outcome.
  - Verification: child-process CLI tests prove precedence, empty-whole-value
    semantics, rejected empty members, invalid startup, and no ambient mutation;
    native integration tests prove direct-spoof resistance, real transport-peer
    availability, simultaneous identity preservation, and absence of raw
    IP/header telemetry.

- [ ] Task 3: Project the operator contract through NixOS and deployment
      documentation.
  - Contract: `services.jaunder.trustedProxies` is `listOf str`, defaults to
    `[]`, omits the environment variable when empty, and otherwise comma-joins
    it. The stack sets `[ "127.0.0.1/32" ]`, strips inbound `Forwarded`, and
    leaves trusted `X-Forwarded-For` production to Caddy; CDN ranges stay in
    strict Caddy configuration.
  - Verification: extend `jaunder-stack-module` evaluation assertions for option
    type/default/serialization and exact Caddy/Jaunder projection; run
    `devtool run -- nix build -L --accept-flake-config .#checks.x86_64-linux.jaunder-stack-module`.
    Operator docs demonstrate custom Caddy, supported stack, CDN layering, and
    rollback.

## Risk checks

- An untrusted or unavailable Transport Peer must make every forwarding header
  inert.
- No malformed, conflicting, over-limit, or all-trusted chain may partially
  salvage a caller-selected hop.
- Middleware ordering must derive request context before the HTTP span records
  its bounded outcome while retaining Axum's original `ConnectInfo`.
- Raw peer/effective addresses and header values must not enter logs, spans,
  metrics, responses, storage, or application authorization.
- The CLI/env contract must follow ADR-0144 and ADR-0158; the NixOS option
  evolution must preserve ADR-0142's compatibility rules.
- Caddy must be the only trusted immediate stack hop; CDN/vendor knowledge must
  not leak into Jaunder defaults.
