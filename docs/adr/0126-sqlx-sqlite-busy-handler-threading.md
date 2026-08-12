# ADR-0126: sqlx-sqlite busy-handler blocking is thread-scoped, not runtime-scoped

- Status: accepted
- Date: 2026-08-11

## Context

Concurrency tests that hold a SQLite write lock while a second connection
contends need to know what a block in SQLite's busy handler blocks: the async
runtime, or just a thread. Under a current-thread tokio runtime (the
`#[tokio::test]` default), a runtime-scoped block would deadlock the test.

## Decision

Rely on the verified driver property: sqlx-sqlite runs each connection on its
own dedicated OS thread, so a call waiting in SQLite's busy handler blocks that
thread, not the runtime. Concurrency tests may therefore run under the default
current-thread flavor; switching them to `flavor = "multi_thread"` is
unnecessary.

## Consequences

- Lock-contention tests stay on the default runtime flavor and cite this draft
  instead of re-deriving the argument.
- Switching sqlx to an in-runtime SQLite driver would turn these blocks into
  hangs; any such migration must revisit every contention test.
