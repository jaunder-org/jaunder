# ADR-DRAFT: WordPress-compatible permalink alias

- Status: proposed
- Date: 2026-09-10
- Issue: [#1429](https://github.com/jaunder-org/jaunder/issues/1429)

## Context

Jaunder's canonical Post permalink identifies both its User and its
date-and-slug identity: `/~username/YYYY/MM/DD/slug`. WordPress-compatible
inbound links omit the User component (`/YYYY/MM/DD/slug`). Date and slug are
only unique within a User, so a bare path can name zero, one, or several Posts.
Treating the first match as authoritative would make a database ordering
decision part of the public URL contract and could disclose which Users have
matching Posts.

The public projector already treats malformed or absent public routes as its
no-store SPA shell miss. Canonical permalink navigation is deliberately `~`-only
in the CSR under [ADR-0076](../0076-no-full-load-spa-navigation.md): in-app
navigation must not add a full document load merely to hand a route back to the
server. Compatibility URLs are therefore an inbound HTTP concern, not a second
client route or a second permalink identity.

## Decision

- Accept only an inbound `GET /YYYY/MM/DD/slug` compatibility path. Parse it as
  a complete permalink date-and-slug shape, then search active Posts visible to
  an anonymous viewer across all Users.
- When exactly one Post matches, return a same-origin HTTP `302` to that Post's
  canonical `/~username/YYYY/MM/DD/slug` permalink. Preserve the original query
  string unchanged and send `Cache-Control: no-store` on the redirect.
- When the path is malformed or zero or multiple anonymous-visible active Posts
  match, return the existing no-store public SPA shell miss. Do not redirect,
  select a match by storage order, or expose why resolution failed.
- Jaunder emits only canonical `~`-prefixed permalink links. The CSR continues
  to mount and navigate only canonical `~` permalink routes; it never links to
  or client-routes the compatibility path.

This revisits ADR-0076's concern that a bare five-segment URL must not become an
in-app route. It does not discard that decision: a cold or externally initiated
HTTP request may use this narrowly server-owned alias, while a live SPA session
continues to use router-managed canonical navigation.

## Consequences

- Existing WordPress-style inbound links work when their visible active Post is
  unambiguous, without weakening User-qualified canonical identity.
- Ambiguous and unavailable aliases have the same public shell-miss surface as
  other malformed or absent public routes, and both successful aliases and
  misses remain non-cacheable.
- Query parameters survive the canonicalization hop exactly, so callers may
  attach tracking or other query data without Jaunder interpreting or rewriting
  it.
- The server owns compatibility resolution. Feed, AtomPub, rendered content, and
  CSR links remain canonical, and ADR-0076's no-full-document-load rule remains
  unchanged.
