# ADR-DRAFT: Jiff time model

- Status: proposed
- Date: 2026-09-06
- Issue: [#1272](https://github.com/jaunder-org/jaunder/issues/1272)

## Context

`UtcInstant` is the domain type for an absolute instant at web and storage
boundaries. ADR-0072 introduced that boundary seam and ADR-0153 extended it
through storage, but their implementation commitments left it backed by Chrono.
ADR-0056 also selected Chrono for the browser datetime helper. Those choices now
leave direct first-party Chrono and chrono-tz dependencies, and their source
uses, throughout the time model.

The replacement must preserve the contracts that are already observable. Feed
and AtomPub adapters still encounter Chrono-backed upstream Atom/RSS model
types; removing direct first-party Chrono must not require replacing those
upstream protocol crates. SQLite and PostgreSQL must retain their schemas,
stored values, precision, and timezone semantics, including SQLite's exact
timestamp text. Local-time conversion needs bundled IANA time-zone data. Its
seams retain their existing policies: the HTML datetime-local path normalizes a
gap as browsers do, while strict paths — including Org — choose the earlier
instant in an ambiguous fold and reject gaps. Legacy persisted values may exceed
the range accepted by the replacement library.

## Decision

All direct first-party `chrono` and `chrono-tz` dependencies and source use are
removed. Chrono may remain transitively through unavoidable third-party
implementation dependencies. Atom/RSS protocol adapters may use upstream string
surfaces to convert their Chrono-backed models at that boundary; Chrono does not
re-enter a first-party public or domain seam.

`common::time::UtcInstant` remains the domain type for an absolute instant and
wraps `jiff::Timestamp`. It retains `value()` and `From`-style Jiff escape
hatches, transparent RFC 3339 serde, display, parsing, ordering, and `now()`; it
does not grow a general calendar or arithmetic API. `PermalinkDate` uses Jiff's
civil `Date`, and public Syndication Feed time seams use domain or Jiff types
rather than Chrono types.

`UtcInstant` parses RFC 3339 with Jiff Temporal parsing directly. This
intentionally broadens only the `UtcInstant` text/serde seam; it does not
replace the fixed outer grammars of `PermalinkDate` (civil `YYYY-MM-DD`), HTML
datetime-local (minute with optional seconds), or Org (structured
`DATE`/weekday/time/TZ). Its canonical emitted form is UTC with a `Z` suffix; a
leap-second `:60` input normalizes to `:59`. Each time-domain type uses its
native Jiff range: civil `Date` spans `-009999-01-01` through `9999-12-31`,
while `Timestamp` spans `-009999-01-02T01:59:59Z` through
`9999-12-30T22:00:00.999999999Z`. No preflight scan is added: a legacy value
outside its type's range fails at the decode or restore boundary that encounters
it.

The application always bundles IANA TZDB. HTML datetime-local keeps its
browser-normalizing, non-strict conversion for gaps. Strict local-time seams,
including Org, continue to choose the earlier instant in an ambiguous fold and
reject a nonexistent local time in a gap.

Storage retains its existing PostgreSQL and SQLite schemas, precision, timezone
semantics, and exact SQLite timestamp text. It uses `jiff-sqlx` 0.2 with SQLx
0.9 plus a deliberate bridge for the physical compatibility that upstream
integration alone does not provide.

This decision supersedes only the Chrono-specific implementation portions of
[ADR-0056](../0056-web-canonical-colocated-leptos.md),
[ADR-0072](../0072-timestamps-cross-boundary-as-utcinstant.md), and
[ADR-0153](../0153-storage-owned-instants.md). Their retained domain and
boundary decisions remain in force.

## Consequences

- The direct time-library surface becomes Jiff while `UtcInstant` continues to
  prevent implementation types from leaking through first-party interfaces.
- RFC 3339 input compatibility is deliberately wider only for `UtcInstant`,
  while output remains a canonical UTC representation; callers that relied on
  rejected formerly-invalid Temporal forms must accommodate the accepted forms.
- Fixed outer grammars retain their current forms: `PermalinkDate` is civil
  `YYYY-MM-DD`, HTML datetime-local is minute with optional seconds, and Org is
  structured `DATE`/weekday/time/TZ.
- Database data stays physically compatible, but an out-of-range legacy instant
  fails only when its normal decode or restore path reaches it.
- Bundled TZDB makes local-time behavior independent of host zoneinfo. HTML
  datetime-local retains browser-normalizing non-strict gap conversion; strict
  seams, including Org, retain earlier-fold selection and gap rejection.
- Shipping the bundled TZDB increases the `-Oz` raw WASM artifact from the
  previous 2 700 875-byte reference to 3 102 495 bytes. The committed ceiling is
  deliberately recalibrated to 3 200 000 bytes, retaining 3.1% headroom while
  remaining below remeasured `-Os` (3 236 295) and `-O2` (3 279 507).
- Unavoidable third-party implementation dependencies can still bring Chrono
  transitively. Atom/RSS protocol models remain isolated at adapters instead of
  defining Jaunder's domain, web, storage, or feed-public time seams.
