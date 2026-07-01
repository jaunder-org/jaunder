# ADR-0001: Pluggable Storage Backends

- Status: accepted
- Deciders: mdorman, Gemini CLI
- Date: 2026-05-14

## Context and Problem Statement

Jaunder needs to support different deployment scales, from personal instances to
high-volume multi-user environments. A single storage engine may not be optimal
for all use cases.

## Decision Drivers

- Scalability: Support for both small and large deployments.
- Ease of Use: Zero-config setup for small instances.
- Flexibility: Ability to use established database systems for power users.

## Considered Options

- SQLite Only: Simple, but may hit limits at high volume.
- PostgreSQL Only: Powerful, but adds operational overhead for small instances.
- Pluggable Strategy: Abstract storage behind traits and support both.

## Decision Outcome

Chosen option: "Pluggable Strategy", because it allows SQLite to be the default
for "zero-config" personal use while providing PostgreSQL as an option for
heavier deployments.

### Implementation Details

- Storage logic is abstracted behind traits (e.g., `UserStorage`,
  `SessionStorage`).
- The application logic only sees the traits, not the concrete implementations.
- Transactions spanning multiple traits are handled via free functions that
  accept a raw pool.

## Consequences

- Good: SQLite provides a low barrier to entry.
- Good: PostgreSQL provides a path for scaling.
- Bad: Developers must maintain parity between two migration trees and two
  storage implementations.
