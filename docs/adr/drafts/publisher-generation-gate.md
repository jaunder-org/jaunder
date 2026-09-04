# ADR-DRAFT: Fence publisher work with durable hub generations and a kernel gate

- Status: proposed
- Date: 2026-09-03
- Issue: [#1052](https://github.com/jaunder-org/jaunder/issues/1052)

## Context

A feed worker renders outside a database transaction and publishes over HTTP. A
concurrent hub change must invalidate every cached discovery representation and
must win over an attempt that captured the previous hub. Process-local state
cannot fence a CLI process against the server, and a database transaction cannot
remain open across rendering or an HTTP request.

## Decision

Persist one monotonic opaque publisher generation with the configuration. A
publisher snapshot reads feed configuration, site identity, and that generation
coherently. A normalized hub change (including successful malformed-value
repair) updates the value, increments the generation, and deletes every feed
cache entry in one short write transaction; a normalized no-op does none of
those things.

The cache-commit seam compares the snapshot generation within its short write
transaction and returns a typed stale result rather than writing stale output. A
kernel-backed file gate derived from the storage directory is acquired before
both hub mutations and a worker's final cache-commit/publish region. The worker
holds its guard after the short cache transaction through the WebSub request;
configuration mutations wait for that guard, so a successful mutation cannot be
followed by an old-hub ping. No database transaction spans rendering, gate wait,
or HTTP.

The gate assumes one Jaunder installation shares one storage directory and a
filesystem whose advisory file locks are visible to its server and CLI
processes.

## Alternatives considered

A process-local mutex cannot serialize an out-of-process CLI mutation with the
server and loses ABA fencing on restart. Holding a database transaction or
session advisory lock through rendering and HTTP would serialize correctly but
extends SQLite write-lock occupancy and consumes a PostgreSQL session while a
bounded remote operation is outstanding. The file gate provides the required
cross-process lifetime without retaining a database resource.

## Consequences

Every hub editor and worker must use the shared publisher service rather than
writing the configuration key or feed cache directly for this flow. Generation
prevents ABA validation across restarts and across processes. A stalled publish
briefly blocks hub mutation, but the gate is released by normal unwinding,
panic, and process exit.
