# ADR-DRAFT: Persistent Browser Sessions with rolling inactivity expiry

- Status: proposed
- Date: 2026-10-28
- Issue: [#1845](https://github.com/jaunder-org/jaunder/issues/1845)

## Context

Browser-session cookies could disappear when a browser session ended, although
server credentials remained valid until revoked. Browser continuity needs a
persistent cookie and authoritative server expiry, without expiring App
Passwords. Legacy rows have no reliable purpose discriminator; neither labels
nor the request's authentication transport establishes their purpose.

## Decision

New Sessions have durable Browser or Machine kind. Existing rows remain Unknown,
non-expiring and revocable. Browser Sessions expire at
`last_used_at <= now - 2_592_000 seconds`, with no separate absolute lifetime.
Machine and Unknown Sessions never expire under this policy. This qualifies
[ADR-0014](../0014-atompub-authentication.md) only for lifecycle classification;
tokens remain transport-interchangeable and authorization precedence is
unchanged.

Authentication samples UTC after acquiring its serialization lock and persists
activity only at the existing 60-second threshold. Expired or revoked Sessions
cannot be revived. Only acknowledged commits authenticate and permit cookie
emission. Browser establishment and cookie-authenticated committed activity set
the same token's persistent cookie through the integer-second committed activity
cutoff. Throttled requests and explicit Authorization never renew ambient
cookies. HttpOnly, SameSite=Lax, Path=/, configured Secure and logout clearing
remain intact.

This partially supersedes
[ADR-0167](../0167-bounded-transient-data-retention.md)'s Session exclusion:
only expired typed Browser Sessions join its existing startup/daily pass. One
explicit UTC instant governs the pass. Domain-owned statements delete at most
500 rows each, drain the eligible backlog, and release locks between batches.
There is no global transaction or new schedule. Failure is reported with a
bounded, PII- and secret-free domain classification; other domains continue and
a later pass retries remaining rows. All other ADR-0167 policies remain
unchanged.

## Consequences

Active Browser Sessions can continue indefinitely; legacy credentials require a
fresh sign-in to adopt browser persistence and idle expiry. Legacy and Machine
rows are not automatically reclaimed. Physical deletion is an optimization, not
an authentication prerequisite. Browser proof observes receipt and use of a
persistent cookie, not universal restart behavior under every privacy policy.
There is no configurable retention setting, token rotation, App Password expiry,
or generic retention framework.
