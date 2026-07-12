# ADR-0064: Backup target set auto-derived from the live schema; restore defers FK checks

- Status: accepted
- Date: 2026-07-11
- Issue: [#4](https://github.com/jaunder-org/jaunder/issues/4)

## Context

`storage/src/backup.rs` exported and restored only the tables named in a
hand-maintained `TABLES_IN_EXPORT_ORDER`. A migration that added a table but did
not also edit that list silently dropped the table from every backup — a
silent-data-loss class, not an isolated bug. Issue #4 is one instance (the seven
content-visibility tables; a restored post lost its `post_audiences` rows and
became Private); auditing the schema against the list surfaced two more
(`user_config`, `media`).

The list encoded two things: **which** tables to export, and — for Postgres,
whose restore inserts with foreign keys enforced immediately — a
**foreign-key-safe insert order**. SQLite restore already sidesteps ordering by
importing with `PRAGMA foreign_keys = OFF` and validating once at the end via
`foreign_key_check`. Both backends already introspect the live table list; only
the ordering constraint kept the set from being derived automatically.

## Decision

**Derive the backup target set from the live schema instead of declaring it, and
make restore order-independent so no ordering has to be encoded.**

- **Membership** is `live tables − TABLES_EXCLUDED_FROM_BACKUP` (an explicit
  denylist: `_sqlx_migrations`, `feed_cache`), minus SQLite-internal `sqlite_%`
  tables, sorted alphabetically for a reproducible manifest. Every table is
  backed up by default; the only manual act left is a deliberate, reviewed
  exclusion.
- **Order independence.** Postgres foreign keys are made
  `DEFERRABLE INITIALLY IMMEDIATE`, and restore issues
  `SET CONSTRAINTS ALL DEFERRED` so rows load in any order with every FK still
  checked — batched to `COMMIT`, matching SQLite's end-of-transaction
  `foreign_key_check`. With order irrelevant, no topological sort is needed, and
  row ordering within each table's dump derives from the columns (all columns,
  schema order) rather than a per-table key map.
- **Restore is authoritative, not additive.** It `DELETE`s each table before
  inserting, so the migration-seeded lookups (`channels`,
  `subscription_statuses`, `target_kinds`) — which a freshly-initialized target
  already holds — do not duplicate-key. This runs only behind the emptiness
  guard (below), so it never overwrites live user data.
- **Two guards keep the invariants fail-closed.** A golden guardrail test pins
  the exact backed-up set (and the total table count), so any schema change
  trips it whether a new table is auto-included or denylisted. An
  FK-deferrability discipline test asserts no non-deferrable Postgres FK
  survives.
- **The restore emptiness guard is tightened.** `ensure_restore_target_empty`
  now refuses unless _every_ table except the migration-seeded lookups is empty
  (`database_is_empty`), replacing the narrower `database_has_users` — so a
  database holding data but no users is no longer silently overwritten.

## Consequences

- New tables are backed up automatically; a migration adding one needs no backup
  code change. Exclusions are the single reviewed decision point, and the
  guardrail test forces that decision to be conscious.
- A schema-discipline burden is introduced — future foreign keys must be
  `DEFERRABLE` — but it is made fail-closed by the discipline test rather than
  left to memory.
- Restore changes from "assume an empty target" to "authoritatively replace
  behind a strict emptiness guard." There is no `--force` overwrite mode;
  restoring into a non-empty database remains a hard refusal.
- **Rejected — a topological sort of the FK graph:** unnecessary once order is
  irrelevant, and brittle to future cycles or self-referential FKs that deferral
  handles for free.
- **Rejected — `SET session_replication_role = replica`:** it disables FK
  triggers with no schema change but performs no commit-time recheck (the
  constraints are never marked `NOT VALID`, so nothing re-validates the loaded
  rows) and needs elevated privilege — it would silently import
  referentially-broken data.
