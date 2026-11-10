# ADR-DRAFT: A Post's publication instant is displayed in its latest editing timezone

- Status: proposed
- Date: 2026-11-08
- Issue: [#1921](https://github.com/jaunder-org/jaunder/issues/1921)

## Context

A Post's absolute publication instant is currently interpreted and displayed
independently of the timezone in which its editor authored or last edited it.
The approved #1921 contract requires the latest supported named IANA editor
timezone to travel with the Post across storage, web, AtomPub, and the Emacs Org
client. A timezone change must not alter publication identity, ordering, or
permalink dates. Org additionally needs to preserve exact instant precision and
DST-fold identity when its unchanged wall time is re-expressed in a newer
editing zone.

## Decision

Each Post stores one required supported named IANA timezone: the latest accepted
editor timezone. Every immutable Post revision stores the timezone of its
snapshot. A timezone-only update is a real change: it creates a revision of the
complete old Post state and stores the new zone on the current Post. Existing
Post and revision rows are explicitly backfilled to UTC; the paired SQLite and
PostgreSQL migrations preserve parity. Valid named aliases are retained exactly
as accepted; numeric offsets and `Etc/Unknown` are not persisted zones.

The timezone selects local rendering of the unchanged absolute publication
instant. It does not change `published_at`, timeline ordering, permalink-date
identity, or audit timestamps. Current Post Published displays use its timezone;
revision Published displays use the corresponding revision snapshot's timezone.
Current-schema backup/restore includes both fields and keeps ADR-0174's
exact-schema compatibility rule; no cross-version conversion is implied.

The web API and AtomPub carry optional timezone input. Explicit valid transport
input takes precedence over a recognized named `JAUNDER_DATE_TZ` in raw Org.
When omitted, create uses UTC and reports the fallback; update preserves its
current zone. Present empty or invalid values reject. AtomPub represents the
zone as `j:timezone` in the existing Jaunder namespace and advertises the
version-1 `timezone` feature. Standard Atom `<published>` remains the
independent absolute instant.

For Emacs local-document preparation only, `JAUNDER_DATE_UTC` is the
prior-instant comparison marker. If local DATE civil fields equal that instant
rendered in the recorded old zone, preserve the exact marker (including
fractional precision and later-fold identity) and render DATE in the latest
editor zone. A changed DATE is interpreted in the latest zone under ADR-0182.
Missing/invalid baseline requires pull/reconciliation or stopping before PUT;
only a confirmed write updates the local zone/DATE pair. Raw Org ingress remains
governed by ADR-0155: its optional `JAUNDER_DATE_UTC` asserts the final
effective publication instant, marker-free input remains compatible, and
structured Atom `<published>` precedence is unchanged. The Emacs local
comparison is not raw-ingress behavior.

## Consequences

- All selected publication displays use the saved Post/revision zone and
  date-specific abbreviation; absolute instants remain authoritative for
  ordering and identity.
- Browser and Emacs capture prefer supported named IANA identities, retaining
  aliases. If unavailable, the current device's exact integer whole-hour UTC
  offset within ±12 hours inclusive selects a supported `Etc/GMT` zone, with
  POSIX-reversed sign (`UTC+1` becomes `Etc/GMT-1`; zero uses `Etc/GMT`). A
  visible warning identifies this as an inferred fixed offset, not a regional
  timezone or historical/future DST identity. Missing, fractional, out-of-range
  or unsupported offset capture instead sends explicit UTC with a distinct
  visible warning; offsets are never rounded. Emacs also requires local fixed
  zone conversion support. This capture-only fallback changes neither server
  named-zone validation nor absolute instants. Older clients retain omission
  semantics; malformed supplied values do not silently fall back.
- Org can refresh a wall-time/zone pair after a web edit without changing the
  instant, and failed or uncertain writes do not checkpoint local metadata.
- Both storage backends, immutable revision paths, and current-schema
  backup/restore must preserve timezone state. Production reads, imports, and
  migration execution are not authorized by this decision.
- The bounded local-original extractor reports local ID/zone exceptions only;
  server-absent IDs require a separately supplied authorized read-only
  inventory.

<!--
Shipping an ADR includes updating docs/ARCHITECTURE.md (and CONTEXT.md when
the ubiquitous language changes) in the same change. Later addenda are past
tense; see docs/adr/0127-architecture-view-materialized-from-adrs.md.
-->
