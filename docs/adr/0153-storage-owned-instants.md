# ADR-0153: Storage-owned instants use UtcInstant

- Status: accepted
- Date: 2026-08-25
- Issue: [#748](https://github.com/jaunder-org/jaunder/issues/748)

## Context

`common::time::UtcInstant` is the domain type for an absolute UTC instant. Its
serde-transparent RFC 3339 representation and `FromStr` normalization already
make it the type for instants that cross the web boundary, while
[`ADR-0072`](0072-timestamps-cross-boundary-as-utcinstant.md) deliberately left
storage and other internals as raw `chrono::DateTime<Utc>`.

That exception makes the storage boundary the conspicuous hole in the typed
path: records, storage traits, SQL row shapes, cursors, inputs, dialect
implementations, backup metadata, and fixtures can strip an instant back to its
implementation type. It also leaves storage-owned timestamp fields unlike the
other domain fields that travel with them. The risk is not limited to the
current `PostRecord` and media record: a partial conversion would merely move
the raw seam to the next storage-owned shape.

Chrono is softly deprecated in favor of Jiff, so naming the domain instant in
public interfaces reduces the future blast radius of a clock-library migration.
This is deliberately a modest benefit: `UtcInstant` remains a Chrono-backed
wrapper and this decision neither migrates to Jiff nor promises complete
implementation isolation. As reviewed for this decision, Jiff has no native SQLx
integration, so changing libraries would not remove the storage-bridge work.

## Decision

All storage-owned representations of an **absolute instant** use
`common::time::UtcInstant`, including exported records and traits as well as
private SQL rows, cursor values, inputs, dialect implementations,
`BackupManifest`, and storage fixtures. Role-specific wrappers over `UtcInstant`
remain where they already communicate a distinct role; this migration does not
flatten them into the common type.

`UtcInstant` remains a minimal Chrono-backed wrapper. It retains its `value()`
accessor, existing `From` conversions, transparent serde, parsing, and display,
and owns the `now()` constructor so callers do not repeat its backing-library
implementation. It also gains the plain `SqlxBridge` required to encode/decode
the wrapped instant on SQLite and Postgres, and `PartialOrd`/`Ord`; it gains no
wider arithmetic or calendar convenience API.

Every newly migrated storage shape receives dual-backend coverage. The bridge
must preserve the existing SQL schema and physical values: timestamp precision,
timezone semantics, and SQLite/Postgres behavior remain unchanged.

This decision supersedes **only** ADR-0072's exception that left storage
internals as raw `DateTime<Utc>`. ADR-0072's cross-web-boundary semantics,
RFC-3339 wire representation, UTC normalization, and browser local-to-UTC
conversion remain in force. It also updates ADR-0027's storage-owned instant
types: public-read APIs continue to receive `now` explicitly, and
`PublishUpdate::Publish` continues to carry its optional publication instant,
but both use `UtcInstant` rather than raw `DateTime<Utc>`. The visibility
predicate and all scheduling, backdating, and restart behavior are unchanged.

This applies only to absolute instants owned by storage. Durations, local
wall-clock values, `SystemTime` suffixes, SQL physical types and values, and
non-storage protocol representations remain unchanged.

## Alternatives considered

- **Keep raw `DateTime<Utc>` inside storage.** This preserves ADR-0072's former
  exception but leaves the storage boundary as an implementation-type seam and
  forces any later Chrono migration across public storage interfaces.
- **Migrate only the records named in #748.** This narrows the first diff but
  creates mixed storage shapes and leaves private rows, cursors, backup
  metadata, and fixtures as untyped escape hatches.
- **Migrate to Jiff now.** Jiff is the intended long-term Chrono successor, but
  this decision is not a library migration and Jiff currently lacks a native
  SQLx bridge; coupling the two changes would add unrelated representation and
  backend risk.
- **Use a storage-local wrapper.** A second wrapper would require conversion at
  every storage edge and would not make `UtcInstant` the common domain type.

## Consequences

- Storage APIs expose the same absolute-instant type as the web boundary, so an
  instant remains typed from decode through records and callers instead of
  changing representation at storage.
- SQLite and Postgres each exercise every migrated shape, including the
  `UtcInstant` SQLx round trip; backend parity is part of the change, not a
  follow-up.
- Existing database columns, timestamp precision, timezone behavior, backup
  value semantics, duration handling, local-wall-clock handling, `SystemTime`
  suffixes, and protocol-specific representations do not change.
- The project takes a small SQLx bridge and ordering surface on `UtcInstant`,
  while keeping its Chrono implementation visible through `value()` and its
  existing conversions until a separate decision changes that implementation.
- `CONTEXT.md` remains unchanged: this is architecture and implementation
  vocabulary, not a new or changed domain term.
