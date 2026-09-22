# Trusted reverse proxies

Issue: [#1609](https://github.com/jaunder-org/jaunder/issues/1609)

## Outcome

Operators can declare the reverse-proxy addresses or CIDRs that Jaunder trusts.
Every HTTP request retains its direct Transport Peer and, only across that
explicit trust boundary, derives an Effective Client IP from standard forwarding
headers. Existing installations continue to trust no forwarding headers.

## Load-bearing decisions

- The Transport Peer is the direct socket endpoint. It is the root of every
  trust decision and is never replaced by a forwarded value.
- The Effective Client IP is an IP address only. A forwarded port is not client
  identity; the Transport Peer retains its full socket address separately.
- With no configured trusted proxy, Jaunder ignores `Forwarded` and
  `X-Forwarded-For` and uses the Transport Peer's IP.
- Trust is configured for `jaunder serve` through repeatable
  `--trusted-proxy <IP-or-CIDR>` arguments and the comma-separated
  `JAUNDER_TRUSTED_PROXIES` environment variable, following the existing
  flag-over-environment precedence contract.
- Environment-list members are trimmed. An absent, empty, or whitespace-only
  whole value means an empty list; once any member is present, every
  comma-delimited member must be nonempty. A trailing comma, leading comma, or
  empty interior member is invalid configuration.
- A bare IPv4 or IPv6 address denotes one host. Empty configuration trusts no
  proxy. Invalid configuration fails startup rather than weakening the trust
  boundary. Duplicate and overlapping ranges are permitted and normalize to the
  same trust set.
- IPv4-mapped IPv6 addresses are canonicalized to plain IPv4 before trusted-set
  matching and cross-header comparison. A mapped peer or header node therefore
  matches the corresponding IPv4 host/CIDR and compares equal to the plain IPv4
  form.
- Resolution starts at the Transport Peer and walks a supplied chain from right
  to left only while the current hop is in a configured trusted range. The first
  untrusted hop is the Effective Client IP.
- If every forwarded hop is trusted, no demonstrably untrusted client endpoint
  exists; resolution falls back to the Transport Peer's IP rather than naming a
  proxy as the client.
- Jaunder supports RFC 7239 `Forwarded` and `X-Forwarded-For`. Standard quoted
  or bracketed IP forms and unambiguous optional ports are accepted, then
  reduced to IP addresses. Hostnames, `unknown`, obfuscated nodes, zone
  identifiers, malformed syntax, and unusable hops are not client evidence.
- Multiple field lines form one ordered chain. Each supplied header family is
  limited to 32 hops; Jaunder never silently truncates a longer chain.
- When both header families are present, their canonical ordered IP chains must
  agree. A malformed family or disagreement makes all forwarded evidence
  untrusted. Requests still proceed using the Transport Peer rather than failing
  at the HTTP boundary.
- When the Transport Peer is unavailable, Jaunder ignores forwarding headers,
  exposes no Effective Client IP, and continues the request.
- Jaunder uses `axum-client-addr` 0.2 as the audited CIDR, header-grammar,
  address canonicalization, and right-to-left resolution implementation. A
  narrow Jaunder-owned policy layer adds cross-header agreement, the hop bound,
  resolution classification, and simultaneous preservation of the Transport
  Peer. Jaunder does not maintain a second general forwarded-header parser.
- Request telemetry may record only a bounded resolution outcome such as socket,
  forwarded, malformed, conflict, over-limit, or transport-unavailable. It
  records neither IP address nor raw forwarding-header content.
- The Effective Client IP is not authentication, authorization, Session,
  secure-cookie, origin, scheme, host, TLS, or trace-context evidence. PROXY
  protocol and vendor-specific client-IP headers are outside this contract.
- The Effective Client IP and Transport Peer are available as typed request
  context, but no existing application control consumes either identity.
  Introducing an IP-aware control requires its own explicit policy review.
- The minimal NixOS module evolves ADR-0142 with
  `services.jaunder.trustedProxies`, a `listOf str` defaulting to `[]`. It joins
  the list with commas into `JAUNDER_TRUSTED_PROXIES`; an empty list omits the
  variable. The owned single-host stack sets exactly `[ "127.0.0.1/32" ]`, its
  hard-coded immediate Caddy-to-Jaunder hop. Its Caddy route removes inbound
  `Forwarded` and relies on Caddy's trusted-proxy-aware `X-Forwarded-For`
  handling, so caller-controlled competing evidence does not reach Jaunder.
- For a CDN in front of Caddy, Caddy owns vendor-specific normalization and must
  use explicit CDN ranges with strict right-to-left trust. Jaunder continues to
  trust only its immediate Caddy hop; no vendor range enters application
  defaults.
- The durable security contract is recorded in
  `docs/adr/0205-trusted-proxy-client-ip.md` and projected into the current
  architecture view.

## Acceptance

- A direct request cannot change its Effective Client IP with forwarding headers
  under the default or any configuration that does not trust its Transport Peer.
- A configured single proxy and a multi-hop trusted chain produce the expected
  Effective Client IP while retaining the original Transport Peer.
- Resolution stops at the first untrusted hop; an all-trusted chain falls back
  to the Transport Peer.
- Regression coverage exercises IPv4, IPv6, IPv4-mapped IPv6, bare-address and
  CIDR configuration, repeated fields, multiple values, standard optional-port
  forms, absent headers, malformed and opaque values, conflicting header
  families, more than 32 hops, and missing transport identity.
- CLI integration coverage proves default, repeated flags, environment,
  precedence, trimmed members, absent/empty/whitespace-only values, rejected
  leading/trailing/interior empty members, and other invalid-startup behavior
  without mutating the parent process environment.
- Native server integration coverage proves that the real serving path supplies
  Transport Peer context and that request context preserves both identities.
- NixOS evaluation coverage proves the `services.jaunder.trustedProxies` type,
  empty default/omission, comma serialization, the stack's exact `127.0.0.1/32`
  value, and removal of inbound `Forwarded` at its Caddy-to-Jaunder boundary.
- Telemetry coverage proves only bounded resolution outcomes can be emitted and
  that raw addresses and header values are absent.
- Operator documentation covers the default-deny boundary, a custom Caddy
  deployment, the supported stack, CDN-before-Caddy layering, and rollback by
  clearing the trusted-proxy list.

## Boundaries

- This work adds no authentication, authorization, rate limiting, banning,
  auditing, or durable address retention.
- It does not derive host, scheme, secure-cookie policy, or URL construction
  from forwarding headers.
- It adds no support for PROXY protocol, Unix-socket peer identity, mTLS proxy
  identity, vendor-specific client-IP headers, dynamic vendor range discovery,
  or hot-reloaded trust configuration.
- It does not expose client addresses through application APIs, logs, traces,
  metrics, or stored records.
