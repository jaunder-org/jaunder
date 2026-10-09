# Issue #1716: Batch-scoped Collection reads for reconciliation pulls

Issue: <https://github.com/jaunder-org/jaunder/issues/1716>

## Outcome

Confirmed pull and keep-remote batches stop enumerating the entire AtomPub
Collection between selected Posts. The number of complete Collection walks
needed by the operation is independent of the number of selected Posts, while
fresh per-Post checks continue protecting local replacement.

## Load-bearing decisions

- Scope reuse to one confirmed pull or keep-remote invocation, one configured
  root, and one active blog. Do not reuse the report's preview as fresh remote
  operation evidence or retain operation evidence for a later batch.
- Acquire complete remote Member evidence at most once when the batch needs it.
  Retain exhaustive pagination, malformed-page/cycle rejection, and duplicate
  remote-ID rejection. Never use a partial enumeration as successful evidence.
- Share only remote Collection evidence, not a frozen local inventory. Re-read
  local identity/uniqueness at each matched-row revalidation boundary, including
  a last local scan after Media finalization, immediately before replacement.
  Earlier successful creation or rename must not make later local checks stale.
- Preserve reviewed Member identity and strong ETag checks, staging, final
  remote revalidation, reviewed local path/digest, clean visiting buffers,
  destination no-overwrite checks, Media integrity, and recoverable atomic
  replacement/rename under ADR-0200 and ADR-0211. Collection ETags remain
  preview evidence under ADR-0209, never authorization to replace a Post.
- Pull-time Local Post Link localization must not cause extra complete walks
  between rows. Retain ADR-0201's unique remote and current local target proof;
  absent or invalid proof leaves canonical URLs unchanged.
- Reuse intentionally stops checking the whole Collection for newly arising
  malformed duplicate Entries after successful acquisition. Supported server
  writes cannot create duplicate Post IDs; fresh selected-Member checks still
  detect its deletion or changed reviewed representation. No paginated snapshot
  or HTTP/filesystem transaction is claimed.
- A failed Collection acquisition is a failed batch-scoped proof, not an empty
  Collection or a successful fallback to stale preview data. Rows requiring it
  remain unsuccessful without Post replacement; retain ordered results and do
  not retry enumeration independently for every row.
- Preserve sequential execution, confirmation, cancellation between Posts,
  independent row failures, and retained recovery results. The final report
  refresh uses a separate new inventory, including its own Collection walk.
- Record the reuse/safety trade-off in a numberless ADR draft and project it
  into the architecture view; document the behavior in the Emacs client guide.

## Acceptance

- Exercise actual confirmed multi-row pull and keep-remote paths with the
  deterministic 100-Member/four-page Collection fixture. Three matched pulls use
  four operation page GETs, not twelve; three keep-remote resolutions use four,
  not twenty-four. Count the final four-page report refresh separately: each
  successful full batch totals eight Collection page GETs, excluding the report
  initially opened before confirmation. Selected Member GETs remain.
- Cover server-only and mixed pull batches containing Org HTTP(S) links: link
  localization does not add a Collection walk per Post, preserves canonical URLs
  without valid target proof, and retains current-local target checks.
- Show that remote ETag/identity changes or deletion and local duplicate IDs,
  identity/path/digest changes, modified buffers, destination collisions, and
  Media changes arising between rows still block the affected replacement.
  Keep-remote retains both initial and final row revalidation boundaries.
- For matched pull and keep-remote, introduce a duplicate local Post ID after
  the preceding Member check or during Media finalization. The last pre-install
  local scan must block replacement and preserve the reviewed local Post.
- Cover duplicate remote IDs, bad pagination/pages, acquisition failure,
  cancellation, later-batch retry, distinct roots/blogs, and failed final
  refresh. No partial/stale evidence authorizes replacement, and terminal
  recovery results survive refresh failure.
- Run focused regression tests, the complete pure ERT suite, and applicable live
  ERT pull/conflict tests. Keep request-count/timing captures in ignored or
  session storage; commit regression fixtures and tests, not run reports.

## Boundaries

No server/protocol/storage changes, cross-batch cache, concurrent execution,
background prefetch, weakened overwrite safeguards, or historical conflict
repair. Push, keep-local, interactive merge, standalone pull, and publish-time
Local Post Link resolution retain their contracts. UI changes requested by #1718
and #1721 remain separate issue cycles.
