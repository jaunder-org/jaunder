# Canonical instants prevent missing Collection Members — issue #1940

> Approval record: human “Approve spec” for `issue-1940-spec-v2`, relayed
> through Manager in background `ccf53f7f-4400-40d4-8cc8-dad2b8cae6b4`; Lead
> acknowledged and accepted this contract. The separate outline gate is also
> approved. Preparation-state descriptions below are historical; this marker
> changes no governing behavior, scope, acceptance criterion or separate merge
> gate.

## Outcome

An unchanged active Post cannot disappear across an AtomPub Collection cursor
because its stored timestamp spells the same instant differently from the
cursor. Comparable SQLite instants have one lossless, chronologically sortable
spelling; Jaunder-owned UTC Org metadata uses the same convention without
changing publication identity.

## Approach and tradeoffs

### Existing wrapper, closed range

Keep `UtcInstant` as the sole Rust owner of parsing, normalization,
representation and application bounds. Use Jiff's existing precision-nine
timestamp printer, not handwritten fractions or callsite formatters. Canonical
output is `2026-09-25T12:00:25.000000000Z` (30 ASCII characters). After offset
normalization, admit exactly `0001-01-01T00:00:00.000000000Z` through
`9999-12-30T22:00:00.999999999Z`, inclusive. Retain pre-1970; reject year
zero/BCE and instants outside Jiff's real upper limit. The human approved the
positive-year restriction with “That's fine”; this does not establish approval
of the migration/failure choices proposed below.

One checked constructor and `TryFrom<Timestamp>` replace infallible inbound
`From<Timestamp>`. Parsing, serde deserialization, SQLite text/civil/numeric
legacy decode, PostgreSQL decode, clock sampling and civil/timezone conversion
must all call that constructor. Keep conversion out to Jiff, not unchecked
re-entry. Keep `now() -> Self` for existing callers; an out-of-domain system
clock fails visibly as an invariant panic with a specific diagnostic, never
clamps or substitutes. Ordinary checked arithmetic reports both Jiff overflow
and application-range failure; existing cutoff-specific saturation uses
application MIN explicitly. Preserve existing leap-second normalization to
`:59`, not leap-second storage. Valid historical offsets/fraction widths and
existing civil UTC input remain readable. Legacy numeric SQLite decode retains
its established Julian-day interpretation, then range validation; never
reinterpret numeric cells as Unix epochs. Display, serde serialization and
SQLite encoding share canonical output. PG stays native microseconds, including
its existing pre/post-2000 quantization behavior; fixed-nine display cannot
recover discarded precision.

A uniform existing `+00:00` encoding could fix ordering in this range too.
Fixed-nine `Z` is chosen for its simple auditable grammar and shared owned-UTC
emission. Encoder-only repair leaves existing rows/defaults/restore broken;
query-time SQLite datetime conversion risks nanosecond loss and index avoidance.
A numeric redesign or generic timestamp registry is unnecessary.

### Explicit comparable-column population

Normalize and enforce these SQLite columns (retain existing nullability):

| Table                                                              | Columns and reason                                                                                                                               |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| posts                                                              | created_at, updated_at, published_at, deleted_at: creation/update/publication cursors and lifecycle state                                        |
| post_revisions                                                     | created_at, updated_at, published_at, deleted_at, captured_at: complete snapshot/copy consistency                                                |
| sessions                                                           | created_at, last_used_at: expiration/cutoff and ordering                                                                                         |
| invites, email_verifications, password_resets                      | created_at, expires_at, used_at: deadline/use/retention consistency                                                                              |
| media                                                              | created_at: library ordering                                                                                                                     |
| idempotency_keys                                                   | created_at: retention cutoff                                                                                                                     |
| feed_events                                                        | next_attempt_at, claimed_at, terminal_at, created_at, regenerated_at, pinged_at: scheduling/leases/dead-letter cursor and coherent lifecycle row |
| feed_cache                                                         | representation_modified_at, generated_at: freshness and representation time                                                                      |
| passkey_credentials                                                | created_at, last_used_at: credential ordering and coherent usage state                                                                           |
| passkey_registration_ceremonies, passkey_authentication_ceremonies | expires_at, claimed_at: expiry/claim consistency                                                                                                 |

This is explicit application SQL, not schema discovery. Do not rewrite users,
subscriptions, audiences, themes or theme_revisions solely for descriptive
creation timestamps; new typed output may canonicalize them, but historical text
has no SQL ordering claim here. Exclude pending_code_migrations.enqueued_at,
private SQLx history and already-numeric theme retention fields. Expanding this
population needs a concrete comparison/copy consumer and an approved contract
update, not an automatic scan.

### Offline transition, enforcement and failure policy (proposed)

Paired SQL migrations only enqueue `normalize_comparable_instants_v1`; they do
not install enforcement. Preserve the existing ascending queue-ID authority.
Earlier pending rendering rebuilds run before normalization without new
triggers, including real changed-rendering updates and their outbox writes.
Never reorder operations, bypass enforcement or hand storage to requests/workers
before drain.

Within the existing single queue-row SQLite `BEGIN IMMEDIATE` transaction: read
stable row identities and raw cells first, normalize ALL selected tables, verify
the complete population, CREATE the fixed named INSERT/UPDATE rejection
triggers, then delete the queue row and COMMIT. Use a storage-local,
identity-first `read_legacy_instant_cells` compatibility reader: fetch identity
plus raw class/value, retain known offset-text/civil-UTC/Julian-number
interpretation, then invoke the checked wrapper with table/column/nonsecret row
identity attached to errors. Do not fetch typed UtcInstant rows before obtaining
identity or weaken the public decoder. Update each row's selected columns
together. New numeric storage rejects; legacy numerics preserve the established
decoded instant, never Unix-epoch reinterpretation. No SQLite date-function
rewrite, table rebuild, FK disabling, UDF or generic registry.

Any malformed/out-of-range cell, unsupported storage class, write/verification,
trigger-DDL or queue-delete error rolls back this whole operation and refuses
open. No skipping, now/NULL replacement, rounding, clamping or partial batch
commits. Known rejected COMMIT must discard/close a still-transactional
connection rather than return it to the pool; reopen proves old physical cells,
pending queue and no newly installed triggers. Cancellation before COMMIT has
that same durable state. Lost acknowledgment after COMMIT is uncertain, not
proof of rollback: reopen must instead find either that old state or canonical
cells, no operation row and the full trigger set, never a mixed state. SQL
migrations and earlier queue operations may already have committed; this is not
whole-upgrade atomicity.

For this one-shot operation, pending work may have no new triggers (normal
upgrade) or the exact full set (an already-initialized restore target); a
partial or different set fails closed. The handler verifies/reuses an identical
full set rather than dropping/bypassing it. With the operation absent/completed,
open and restore require the exact full set; missing triggers are not silently
accepted as completion. Use explicit expected names/definitions for these
tables, not a schema registry.

DB predicates explicitly require TEXT, 30 ASCII characters, separators/digit
positions, nine fractions, terminal Z, Gregorian month/day/leap-year validity,
00–23 hours, 00–59 minutes/seconds, application range and deliberate NULL rules.
Use binary comparison, never locale collation. The PG queue handler validates
the corresponding native population and installs explicit finite-bound
constraints before deleting its operation row, in its existing transaction; no
precision rewrite. PG retains native calendar/type validation, rejects both
infinities, and admits microsecond endpoints `0001-01-01T00:00:00.000000Z`
through `9999-12-30T22:00:00.999999Z`.

Update every application writer omitting a selected defaulted timestamp (notably
outbox enqueue) to bind one sampled typed instant, including batch paths. Keep
incompatible historical SQL defaults only as deliberately rejected raw
omissions: relevant raw INSERTs must supply canonical required fields; no
implicit now default is promised. Test that omission fails, not silently
persists an alternative spelling. Unselected default-driven writers need no
unrelated rewrite. Earlier rendering/outbox paths use these same
coherent-release writer changes; any remaining legacy defaults they exercise
before normalization are normalized before enforcement. Revision copies before
drain completion are also normalized; ordinary post-open copies receive
canonical Posts and preserve exact instants. Canonical encoding, affected
writers, handler and enforcement ship together in one coherent production
commit, not an independently deployable enforcement slice.

At the existing PG backup export seam, selected native timestamp cells must pass
through UtcInstant and its canonical formatter instead of unmodified `to_jsonb`
timestamp text. Preserve stored microseconds exactly, padding output fractions
with zeros, not fabricating lost ns. Keep NULLs and other export cells
unchanged. Same-schema PG-to-SQLite and SQLite-to-PG restore remain supported
with their existing precision difference. No generic export registry or second
formatter.

Restore explicitly validates the entire selected population before database
commit, INCLUDING when a normalization operation is pending and enforcement
triggers are absent. Do not rely on triggers alone or consume queued work during
import. Selected backup timestamp strings must be canonical and in range before
native PG binding can erase their spelling; malformed/noncanonical cells fail
rather than silently normalize. Enforce the exact pending/completed trigger
states above (and PG constraint equivalents) on the target independently of
imported queue rows. Keep exact schema-version compatibility, closed
restore-cell roles, existing transaction/FK checks and media/Theme ownership. No
cross-schema/legacy-backup restoration or new backup framework.

Operator stops the single production instance and excludes other
binaries/writers, retains a verified premigration recovery copy, then runs the
supported offline upgrade. Failure leaves the server stopped: operator restores
that copy with its matching old binary, or obtains an explicitly reviewed row
repair and retries; no automated data repair is authorized. After success,
validate cursor behavior and a fresh backup/restore before the operator retires
old backups/recovery copy. Agents never access production, databases, servers,
credentials or delete backups. Downtime, WAL/disk growth and stricter raw-write
admission are accepted costs.

### Org and architectural contract

`jaunder-datetime.el` owns UTC parsing/normalization/canonical emission for
JAUNDER_DATE_UTC and JAUNDER_SYNCED_AT; publish/pull/reconcile call that seam.
The sync-marker parser accepts historical valid whole-second Z and
offset/fraction spellings as well as new fixed-nine Z markers. Preserve
nanoseconds with existing string-aware precision handling, not a lossy Emacs
clock conversion. Keep local DATE, named timezone/alias and DST-fold identity
unchanged; no artificial edits.

After separate spec and outline approvals, commit the approved contracts and
numberless ADR draft/architecture projection BEFORE any source/test
implementation. Explicitly supersede ADR-0153/0182 physical-byte preservation,
native full instant range, unchecked inbound conversion and restricted
arithmetic provisions only as needed here. Extend ADR-0226's offline exception
to atomic normalization and handler-owned enforcement DDL, retaining its
lock/queue/failure semantics. ADR-0174 exact-schema restore and ADR-0169 typed
binds remain; distinguish strict DB timestamp rejection from post-restore domain
diagnostics. No new domain term.

## Acceptance (future proof, not performed during preparation)

- Public seams for TDD: UtcInstant
  construction/parsing/serde/display/arithmetic; storage open/upgrade and
  existing storage traits/cursors; same-schema backup/ restore; AtomPub
  Member/Collection HTTP behavior; existing Emacs datetime, publish/pull and
  reconcile entrypoints. No new public testing API is required.
- Reproduce human Post 5/266 bound-SQL exclusion, then show Member 5 appears
  under the same cursor and complete Collection refresh. Evidence proves this
  mismatch, not that every reported orphan shares its cause.
- Traverse every timestamp cursor with small pages and equal-time ties: public
  timelines both ways, drafts, scheduled Posts, Manage Posts/Collection and
  feed-event dead letters. No missing/duplicate IDs on an unchanged dataset.
- Compare SQL ordering/equality to UtcInstant order across zero/1/999ns,
  fraction widths, second/day/year boundaries, offsets, pre-epoch and
  application endpoints. Reject normalized offset crossings, year zero/BCE and
  invalid calendars; cover every ingress, leap normalization, arithmetic
  overflow and cutoff saturation.
- Prove all selected typed/default-driven/batch writers, raw INSERT/UPDATE,
  omission, NULL/storage-class/shape/calendar/range rejection and restore
  bypass. Preserve IDs, semantic edit times, mutation versions, nulls,
  timezones, fold, exact SQLite nanoseconds and Revision snapshots; no extra
  revisions or events.
- Use the existing bounded MigrationDatabase fixture (storage/src/migrations.rs)
  to upgrade both immediately preceding and pre-0041 schemas. Include an older
  pending rebuild with legacy Post times whose rendered output genuinely changes
  and an outbox-default write ahead of normalization. Verify ascending
  completion; do not claim recovery of precision already lost by historical
  migration 0041.
- With checked public decoding installed, the identity-first reader reports
  table, column and row for malformed text, BLOB/unsupported classes and Julian
  numeric zero (BCE). Inject mid-conversion, trigger creation and queue-delete
  failures. Use the existing rejecting queue-delete trigger pattern and a
  bounded test-only before/after-COMMIT pause in the existing drain, not general
  fault infrastructure: a fixture-only deferred FK violation makes COMMIT
  actually fail; cancellation before COMMIT and suppressed acknowledgment after
  successful COMMIT distinguish rollback from uncertainty. Close/reopen
  connections and assert raw physical cells, pending row and exact trigger set
  for each durable outcome, then retry as needed.
- Verify indexes/FKs, raw/default/batch writes, and explicit restore validation
  with pending work and absent triggers, completed work/full triggers, and
  rejected partial/missing-completed sets. Run actual same-schema PG→SQLite and
  SQLite→PG export/restore with NULLs and pre-2000 fractions. Preserve stored PG
  micros; SQLite→PG retains the existing quantization, not a nanosecond-parity
  claim.
- Both backends: range/serde/HTTP behavior; PG ±infinity rejection, inclusive
  microsecond endpoints and one-step-outside values, plus pre/post-2000
  submicroseconds. Emacs old/new sync markers and publication precision/fold
  round-trip.

## Boundaries and approval state

Issue: https://github.com/jaunder-org/jaunder/issues/1940 (Bug/P1, milestone
22). Managed branch discussion/emacs-client-interface and assigned checkout stay
intact. No implementation, tests/builds, commits or source/config changes in
preparation. This revised proposal incorporates the supplied cold reviews, not
human approval. Lead must accept the dispositions, resolve any remaining
soundness findings, then route the spec approval brief through Manager. After
spec approval, reconcile and review the triggered outline before its separate
approval brief. No nested reviewer ran in this revision task. Security, #1898
and #1933 are independent; coordinate only real shared hunks.
