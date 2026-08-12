# ADR-0115: Restore clears every table before loading any

- Status: accepted
- Date: 2026-08-11

## Context

Backup restore loads tables from a manifest sorted alphabetically, not
FK-topologically. On Postgres, `SET CONSTRAINTS ALL DEFERRED` defers foreign-key
_checks_ to COMMIT, but does **not** suppress `ON DELETE CASCADE` _actions_: a
per-table delete-then-load could fire a cascade that wipes rows already loaded
for an earlier table.

## Decision

Both backends restore as clear-then-load: DELETE every manifest table first,
then load every table. Restore is an authoritative replace of a database the
emptiness preflight already proved unused, so the full clear loses nothing. On
SQLite, FK enforcement is off during restore (a DELETE never cascades); the
split is kept anyway so the two backends' restore shape stays identical. On
Postgres, every FK is still checked once at COMMIT, matching SQLite's
end-of-import `foreign_key_check` — a referentially broken restore fails the
whole transaction.

## Consequences

- Table order in the manifest never matters, for clearing or loading.
- Any new backend must implement the same two-phase shape.
