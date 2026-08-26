# Issue #754: Keep draft summary labels derived

## Outcome

Titleless unpublished Post rows continue to display a fallback label derived
from the Post body at read time. The behavior is unchanged, but its ownership
and the reason for deliberate recomputation become explicit.

## Load-bearing decisions

- `summary_label` is presentation metadata for an unpublished Post row, not Post
  identity, authored summary content, or historical state.
- The canonical stored `PostBody` remains the source of the fallback label.
  Reading an unpublished Post derives the label from its first non-blank body
  line through the existing boundary-aware `PostSummary` truncation rule.
- The draft query already loads the body and is bounded to 51 records to emit at
  most 50 rows. Avoiding that bounded derivation does not justify another stored
  value.
- Persistence would add a SQLite/PostgreSQL migration and backfill plus
  freshness obligations across ordinary writes, raw seed paths, and direct
  backup restore. Those costs and stale-state risks outweigh the repeated
  first-line scan.
- A changed derivation rule may intentionally change existing rows immediately;
  the fallback label has no stability guarantee independent of the Post body.
- The decision is recorded in the method documentation and current architecture
  view. It does not require a new ADR because it retains the existing storage
  boundary rather than introducing a durable architectural choice.

## Acceptance

- The fallback-label method documentation states that read-time recomputation is
  deliberate and gives the bounded-query and freshness rationale.
- The architecture view distinguishes the authored optional Post summary from
  the body-derived unpublished-row fallback label and records that the latter is
  not stored.
- Existing fallback-label behavior and boundary-aware truncation tests remain
  unchanged and green.
- `cargo xtask validate` passes.

## Boundaries

- No database column, migration, backfill, cache, trigger, or background job.
- No change to fallback-label text, truncation, pagination, or unpublished-row
  UI.
- No change to authored `PostRecord.summary`, Post revisions, backup format, or
  restore behavior.
- No glossary change and no ADR.
