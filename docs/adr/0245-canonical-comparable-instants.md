# ADR-0245: Canonical comparable instants belong to the existing time wrapper

- Status: accepted
- Date: 2026-11-11
- Issue: [#1940](https://github.com/jaunder-org/jaunder/issues/1940)

## Context

Human-run production evidence showed an active AtomPub Member omitted by a
Collection cursor: SQLite compared stored whole-second `Z` text against an
otherwise equal `+00:00` cursor. Decoding both spellings to one instant cannot
repair SQL ordering. Existing SQL defaults, Revision copies and raw restore can
retain mixed spellings; changing only the encoder is insufficient.

The human approved the spec and separate storage outline through Manager. This
draft records the delivered Rust/storage and Emacs implementation; its
`proposed` status follows the numberless-draft promotion lifecycle, not an
outstanding human design gate. Implementation does not establish production
migration: offline cutover and production validation remain operator-owned.

## Decision

Keep `common::time::UtcInstant` over Jiff as the only Rust owner of instant
parsing, normalization, checked construction, arithmetic bounds and canonical
output. Use Jiff's precision-nine printer, not another type or callsite
formatters. After offset normalization, admit `0001-01-01T00:00:00.000000000Z`
through `9999-12-30T22:00:00.999999999Z`, inclusive. Pre-1970 remains supported;
year zero/BCE and values beyond Jiff's actual maximum do not. Preserve existing
leap second normalization to `:59` and existing local-time gap/fold rules.

All inbound paths, including serde, SQLx decode, civil conversion, clock
sampling and replacement of inbound `From<Timestamp>` with checked `TryFrom`,
validate through that wrapper. Conversion out to Jiff remains available.
Ordinary checked arithmetic rejects library or application overflow; named
cutoff saturation uses application MIN. `now() -> Self` visibly panics on an
out-of-domain clock rather than fabricating an instant. Valid legacy offset
text, civil UTC and established Julian-number interpretation remain readable
within range. Public decode stays checked, never a migration bypass.

Display, serde and SQLite encoding share fixed-nine UTC `Z` output, for example
`2026-09-25T12:00:25.000000000Z`. Under binary ordering, this representation is
chronological and preserves SQLite nanoseconds within the approved positive-year
range. PostgreSQL remains native microseconds, including existing pre/post-2000
quantization; it gains finite bounds from `0001-01-01T00:00:00.000000Z` through
`9999-12-30T22:00:00.999999Z`, rejecting both infinities. No precision redesign.

Normalize and enforce this explicit SQLite population, retaining nullability:

| Table                                                              | Columns                                                                         |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------- |
| posts                                                              | created_at, updated_at, published_at, deleted_at                                |
| post_revisions                                                     | created_at, updated_at, published_at, deleted_at, captured_at                   |
| sessions                                                           | created_at, last_used_at                                                        |
| invites, email_verifications, password_resets                      | created_at, expires_at, used_at                                                 |
| media, idempotency_keys                                            | created_at                                                                      |
| feed_events                                                        | next_attempt_at, claimed_at, terminal_at, created_at, regenerated_at, pinged_at |
| feed_cache                                                         | representation_modified_at, generated_at                                        |
| passkey_credentials                                                | created_at, last_used_at                                                        |
| passkey_registration_ceremonies, passkey_authentication_ceremonies | expires_at, claimed_at                                                          |

These fields support comparison or coherent Revision/lifecycle copies. Do not
rewrite descriptive User, subscription, audience, Theme or Theme Revision times,
queue enqueue metadata, private SQLx history or numeric theme retention fields.

Paired SQL migrations only enqueue `normalize_comparable_instants_v1`. Existing
ascending queue authority stays: earlier rendering rebuilds and outbox writes
run without new enforcement. In one existing offline queue transaction, read
stable identities/raw cells, normalize ALL selected population, verify, CREATE
fixed SQLite INSERT/UPDATE rejection triggers, delete the operation row and
COMMIT. The storage-local `read_legacy_instant_cells` compatibility reader
obtains identity before interpreting text/civil/Julian cells and calling checked
construction, so errors identify table/column/nonsecret row without weakening
public decode. PG validates its native population and installs explicit
finite-bound constraints in its corresponding queue transaction. No queue
reordering, generic registry, UDF, table reconstruction, FK disabling or
enforcement bypass.

SQLite predicates reject wrong storage class, shape, NULL placement, calendar,
time fields and range, including Gregorian leap-day validity; they require 30
ASCII characters, nine fractions and terminal `Z`, with binary comparison.
Affected application writers bind typed sampled instants, including single/batch
outbox enqueue; raw omission of incompatible required defaults deliberately
fails. Encoding, writers, normalization, enforcement and portable export/restore
form one coherent production commit, not a separately deployed partial
enforcement slice.

Pending normalization permits absent enforcement on upgrade or an identical full
set on an initialized restore target. Partial/different sets fail; an identical
set is verified/reused, never dropped. Completed/absent operation requires the
exact full set on open/restore; absence is not silently treated as completion.
Malformed/out-of-range cells and write/DDL/delete failures roll back this
operation and block open; earlier SQL migrations/queue work may already have
committed. A rejected COMMIT must not return a still-transactional connection to
the pool. Cancellation/uncertain acknowledgment is reconciled from physical
cells, queue row and enforcement after reopen, never inferred to mean rollback.

At the existing PG backup export seam, selected native timestamps pass through
UtcInstant canonical output, preserving stored micros and NULLs, never inventing
lost ns. Restore validates canonical selected cells before native PG binding can
erase spelling and validates the entire selected population before commit, even
with pending work and absent triggers. Imported queue rows neither disable
validation nor get consumed during import. Exact-schema compatibility,
backend-portable restore, closed restore-cell roles and existing FK/media/Theme
ownership remain; no cross-schema or indefinite legacy-backup restore framework.

`jaunder-datetime.el` owns canonical UTC Org emission for JAUNDER_DATE_UTC and
JAUNDER_SYNCED_AT. Publish/pull/reconcile reuse it and accept historical valid
sync spellings. Preserve publication nanoseconds, local DATE, named timezone/
alias and DST-fold identity; do not create artificial edits/checkpoints.

This supersedes only the physical-byte/schema-preservation, unchecked inbound
conversion, unrestricted instant range and arithmetic-restriction portions of
[ADR-0153](0153-storage-owned-instants.md) and
[ADR-0182](0182-jiff-time-model.md) needed for this transition. Their shared
domain type, Jiff ownership, precision and timezone contracts remain. Extend
[ADR-0226](0226-offline-code-migration-queue.md)'s narrow offline exception to
the selected population and handler-owned enforcement DDL, atomic with queue
completion. Preserve its locking, order and fail-closed startup semantics.
[ADR-0169](0169-typed-storage-bind-admission.md)'s typed binds and closed
restore roles remain;
[ADR-0174](0174-backup-format-and-schema-compatibility.md)'s exact-schema
compatibility remains. Strict timestamp admission is a precommit DB invariant,
not a replacement for other post-restore domain diagnostics.

## Consequences

A uniform existing `+00:00` encoder could also sort correctly in this range, but
fixed-nine `Z` gives a simple auditable grammar and consistent owned-UTC output.
Query-time date conversion risks precision/index loss; a numeric redesign or
generic framework adds unnecessary machinery. Normalization changes physical
bytes, not exact instants, IDs, semantic edit times, mutation versions, nullable
state or publication zone/fold, and does not create Revisions or events.

The single operator coordinates offline cutover and excludes old writers.
Startup may be expensive and grow WAL/disk use. Malformed data blocks startup:
recover with the verified premigration copy and matching binary, or explicitly
reviewed row repair and retry; never skip, clamp, round, replace with now/NULL
or auto-repair. Keep that recovery copy until a validated fresh backup/restore,
after which the operator may retire old backups. Agents receive no
production/database/server/ credential access or backup-deletion authority from
this decision.

Regression coverage includes the synthetic human-cursor reproducer and every
timestamp cursor's ties/small pages, all ingress/range/arithmetic cases, genuine
earlier changed rendering/outbox upgrade, malformed/failure/cancellation/reopen
proof, raw/default writes, strict absent-trigger restore, actual same-schema
restores both directions, PG infinity/endpoints/precision and historical Org
marker/publication round trips.

<!--
Shipping an ADR includes updating docs/ARCHITECTURE.md (and CONTEXT.md when
the ubiquitous language changes) in the same change. The architecture projection
records the delivered system; final conformance and operator cutover are separate.
-->
