# Exclude runtime filesystem state from backups — issue #1715

## Outcome

Directory and archive backups contain recovery content, not Media lock files or
interrupted-upload scratch data. Restoring an older backup does not replay those
runtime artifacts. Retained remote Media remains recoverable.

## Load-bearing decisions

- Exclude exactly the root `media/.locks` and `media/tmp` subtrees from export
  and restore, including empty directory entries and all descendants.
- Preserve `media/cached`, including its empty directory when present: it is a
  location for retained remote Media, not a blanket disposable-cache category.
- Keep the existing exclusion of root `themes/.locks` and `themes/.staging` from
  both export and restore.
- Match exclusions by their location within the Media or theme storage root, not
  by a filename suffix or a name encountered anywhere in the tree. Durable
  nested content named `tmp`, `.locks`, `.staging`, or ending in `.lock` must
  not be accidentally discarded.
- Apply the same policy to directory and archive modes and to both database
  backends. Previous-backup hard-link reuse must not reintroduce excluded paths.
- Older format-1 backups containing these runtime paths remain readable; restore
  ignores those paths without deleting or overwriting target runtime state.
- Preserve the existing restore-target emptiness refusal, database backup table
  selection, compatibility checks, and backup format/schema version policy (ADRs
  0054, 0064, and 0174). No target-cleanup or overwrite mode is introduced.

## Acceptance

- Public CLI backup/restore regression tests run for SQLite and PostgreSQL.
- Seed both empty and populated transient subtrees; prove new directory backups
  and archive member inventories omit the excluded paths entirely.
- Prove durable uploaded and cached Media and theme bytes survive
  backup/restore. Include similarly named nested durable paths and an ordinary
  `.lock` file to demonstrate location-scoped filtering rather than broad name
  filtering.
- Restore directory and archive backups augmented with historical runtime
  entries; prove durable content is restored and excluded runtime entries are
  not materialized at the target.
- Exercise directory backup reuse with a previous backup carrying transient
  entries; prove no excluded paths enter the new backup.
- Existing non-empty restore-target rejection and backup compatibility tests
  remain green. Update maintained backup documentation with the exact policy.
- Execute focused regression tests before broader verification. Test sources,
  minimal fixtures, and current documentation are deliverables; command logs,
  extracted archives, and review evidence stay in ignored/session storage.

## Boundaries

No schema migration, database-table exclusion change, Media lifecycle change,
cache eviction, historical-backup rewrite, or filesystem security redesign. No
new domain term or architectural boundary is needed for this bounded fix.
