# Spec — #4: Back up every table by deriving the set from the live schema

**Issue:** [#4](https://github.com/jaunder-org/jaunder/issues/4) **Status:**
proposed **Decision record:** ADR draft to be authored — "Backup target set
auto-derived from the live schema; restore defers FK checks."

## Problem

`storage/src/backup.rs` exports and restores only the tables named in the
hand-maintained `TABLES_IN_EXPORT_ORDER` (`backup.rs:24`) — 10 tables. Any table
added by a later migration is silently omitted from backups unless someone
remembers to edit that list.

The reported symptom (issue #4): the seven content-visibility tables —
`channels`, `subscription_statuses`, `target_kinds`, `subscriptions`,
`audiences`, `audience_members`, `post_audiences` — were never added, so a
restore drops them. A restored post has **no `post_audiences` rows**, so under
the resolution filter it is Private (hidden from everyone but the author), and
all subscriptions/audiences are lost.

**The bug is a class, not an instance.** Enumerating the live schema against the
list shows the visibility tables are not the only omissions:

| Table               | In list today? | Correct disposition                                                                                                                |
| ------------------- | -------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `user_config`       | **no**         | **must back up** — user settings (latent data-loss bug)                                                                            |
| `media`             | **no**         | **must back up** — blob metadata; the media _files_ are mirrored separately, but without the table the restored files are orphaned |
| 7 visibility tables | **no**         | **must back up** — the reported bug                                                                                                |
| `feed_events`       | **no**         | **must back up** — durable pending-regeneration/retry state                                                                        |
| `feed_cache`        | **no**         | exclude — regenerable HTTP response cache                                                                                          |

So `user_config` and `media` are two more silent data-loss bugs of the same root
cause. Adding names to the list one issue at a time does not close the class;
the next migration reopens it.

## Approach

**Stop maintaining a list of targets. Derive the target set from the live schema
at backup time, and make restore order-independent so derivation needs no
hand-encoded ordering.**

Two moving parts:

1. **Membership is derived, not declared.** Replace `TABLES_IN_EXPORT_ORDER`
   with runtime enumeration of the live schema (both backends already query it
   in `existing_export_tables`) minus an explicit `TABLES_EXCLUDED_FROM_BACKUP`
   denylist. Every table is backed up **by default**; the only manual act left
   is a _deliberate exclusion_, which is documented and test-guarded. A future
   migration that adds a table is covered automatically.

2. **Restore is order-independent on both backends.** The manual list encoded a
   foreign-key-safe insert order for Postgres (SQLite restore already runs with
   `PRAGMA foreign_keys = OFF` + a final `foreign_key_check`). Rather than
   compute a topological sort, make the Postgres FKs
   **`DEFERRABLE INITIALLY IMMEDIATE`** and have restore wrap the bulk import in
   `SET CONSTRAINTS ALL DEFERRED`. Every FK is still checked — batched to
   `COMMIT` — so a referentially-broken restore fails the whole transaction and
   rolls back (the same guarantee, and the same end-of-transaction shape as
   SQLite). With order irrelevant, "export the live tables in any deterministic
   order" is correct, so no topological sort is needed.

   _Rejected alternative:_ `SET session_replication_role = replica` disables FK
   triggers with no schema change, but it performs **no** commit-time recheck
   (constraints are never marked `NOT VALID`, so nothing re-validates the loaded
   rows), and needs elevated DB privilege. It would silently import broken data
   — the opposite of what a restore wants. Deferral keeps the integrity check.

### Concretes

- **Derived membership (`backup.rs`).** Delete `TABLES_IN_EXPORT_ORDER`. Add
  `pub(crate) const TABLES_EXCLUDED_FROM_BACKUP: &[&str] = &["_sqlx_migrations", "feed_cache"];`.
  Both backends' `existing_export_tables` change from "intersect the live schema
  with the static list, preserving list order" to "take the live schema tables,
  drop `sqlite_%` internal tables (SQLite) and anything in the denylist, sort
  **alphabetically** for a deterministic manifest." The alphabetical order is
  purely for reproducible manifests; correctness no longer depends on it.

- **Deferrable Postgres FKs (migration `0023`).** A Postgres migration alters
  every existing FK to `DEFERRABLE INITIALLY IMMEDIATE` via a `DO` block that
  loops over `pg_constraint WHERE contype = 'f'` and issues
  `ALTER TABLE … ALTER CONSTRAINT … DEFERRABLE INITIALLY IMMEDIATE` for each —
  so no constraint names are hand-enumerated. `INITIALLY IMMEDIATE` means normal
  app operation is byte-for-byte unchanged: checks stay immediate except inside
  a transaction that explicitly defers. A **matching SQLite `0023`** is added to
  keep the two backends' migration versions in lockstep (the manifest's
  `schema_version` is `MAX(version)`, and the cross-backend interop tests
  compare it); it is a no-op, because SQLite restore already disables FK
  enforcement per-connection and needs no schema change.

- **Postgres restore defers (`postgres/backup.rs`).** After `BEGIN`, issue
  `SET CONSTRAINTS ALL DEFERRED` before the import loop. Constraint violations
  now surface at `COMMIT`; `map_restore_error` already maps SQLSTATE class `23`
  to `ConstraintViolation`, so the error mapping is unchanged. SQLite restore
  already runs FK-off, so it needs no deferral change.

- **Restore replaces, not appends (both backends).** Because the backup set now
  includes the migration-seeded lookups (`channels`, `subscription_statuses`,
  `target_kinds`), restoring into a freshly-initialized target — whose
  migrations already seeded those rows — would duplicate-key on INSERT. Restore
  therefore `DELETE`s each table before inserting its rows, making it an
  authoritative replace rather than assuming an empty target. This is safe under
  the FK-off (SQLite) / deferred (Postgres) import transaction, where
  delete/insert order is immaterial and integrity is checked once at the end.
  For a fresh target it is a no-op on every table except the three seeded
  lookups (identical rows cleared and re-inserted). Keeping the lookups in the
  backup set — rather than denylisting them — means that if a lookup ever gains
  user-writable rows they are carried, instead of silently dropped (which would
  reintroduce exactly the data-loss class this issue closes).

- **Tighten the restore emptiness guard (`server/src/commands.rs`,
  `storage/src/db.rs`).** `ensure_restore_target_empty` currently refuses only
  when `database_has_users` is true — it misses a database that holds data but
  no users (site config set before signup, unused invites, a populated feed
  cache). Replace it with `storage::database_is_empty`, which enumerates the
  live schema and returns false if **any** table other than the migration-seeded
  lookups
  (`MIGRATION_SEEDED_TABLES = ["channels", "subscription_statuses", "target_kinds"]`)
  holds a row. Those three are the only tables non-empty in a pristine `init`,
  so a fresh target still passes. `database_has_users` (used only by this guard)
  is removed. A dual-backend guard test asserts a fresh `init` is
  `database_is_empty` **and** that exactly those three tables are seeded — so a
  future migration that seeds a new table, or a lookup that gains rows, trips
  loudly instead of silently weakening the check.

- **Row ordering is derived, not mapped (`backup.rs::order_by_clause`).** The
  hand-written per-table key map with a `rowid` fallback breaks for any
  auto-included table: Postgres has no `rowid`, so `ORDER BY rowid` on a new
  table errors. Replace the map with **primary-key-derived ordering**: order by
  the table's PK columns (introspected — SQLite `PRAGMA table_info` `pk`,
  Postgres primary-key columns), falling back to **all columns** for a PK-less
  table. This reproduces today's ordering for the existing 10 tables (their map
  entries already _are_ their keys) and handles `post_audiences`, which has **no
  PK** (only partial unique indexes) via the all-columns fallback.

- **Guardrail test — nothing silently unbacked.** A harness test (per backend)
  enumerates the live schema tables and asserts each is either in the derived
  backup set **or** in `TABLES_EXCLUDED_FROM_BACKUP`. A future migration that
  adds a table then fails CI until its coverage is a deliberate decision (back
  it up — automatic — or add it to the denylist with a reason). This is the
  anti-regression framework issue #4 (b) asked for, but structural: there is no
  ordered list left to forget.

- **Discipline test — the ordering hazard can't return.** A Postgres harness
  test asserts every FK in the live schema is `condeferrable` (query
  `pg_constraint WHERE contype = 'f' AND NOT condeferrable` → must be empty), so
  a future migration cannot reintroduce a non-deferrable FK that would break the
  order-independent restore.

- **Fidelity test — the reported bug, closed.** A round-trip test (per backend):
  seed a post with a non-public audience + subscriptions/audience_members,
  export, restore into a fresh DB, and assert the visibility rows survive and
  the post resolves to a **non-author** viewer (not silently Private). Add
  analogous round-trip assertions for `user_config` and `media` rows.

## Manifest / versioning

`BackupManifest.tables` is written from the derived set at export and replayed
verbatim at restore (`restore` iterates `manifest.tables`, not a fresh
derivation), so an old backup restores exactly what it captured even if the
denylist later changes. `schema_version`/`schema_checksum` handling is
unchanged; `schema_checksum` already hashes every non-internal table, so it
needs no edit. The `0023` migrations bump both backends to `schema_version = 23`
in lockstep.

## Out of scope / follow-ups

- No cross-backend restore work (SQLite backup → Postgres restore) beyond
  keeping `schema_version` aligned; that remains governed by the existing
  version/checksum checks.
- No `--force` restore flag. Restore keeps its hard refusal of a non-empty
  target (now stricter); a force-overwrite mode is a separate feature, not part
  of this issue.
- `#136`'s backup contract tests can, once this lands, drop their author-viewer
  workaround and assert audience/visibility fidelity for non-author viewers
  directly. Coordinating that is follow-up on #136, not this issue.

## Risks

- **Migration touches every existing FK.** Mitigated by
  `ALTER CONSTRAINT …  DEFERRABLE INITIALLY IMMEDIATE` (in-place, no
  drop/recreate, no data movement) and `INITIALLY IMMEDIATE` (runtime behavior
  unchanged outside the restore txn). Verified by the discipline test and the
  existing migration round-trip tests.
- **Auto-derive changes what is backed up** (adds `media`, `user_config`,
  `feed_events`, and the visibility tables; excludes only `feed_cache` and
  `_sqlx_migrations`). The denylist is the single reviewed decision point; the
  guardrail test makes any omission loud.
