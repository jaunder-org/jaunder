# #1617 — Responsive reconciliation fetches

## Outcome

Fetching selected server-ahead Posts from the Emacs Protocol Client gives
continuous, interpretable foreground progress rather than appearing frozen after
its initial count. A slow or failed transport step terminates with an actionable
row outcome; measured, safety-preserving improvements reduce any proven
avoidable work without promising to eliminate necessary fresh scans.

## Load-bearing decisions

- Keep `jaunder-reconcile` and its selected-row fetch (`f`) as foreground Emacs
  commands. Do not build a general asynchronous Emacs execution framework or a
  Rust helper in this issue.
- Measure the current selected-row path with a repeatable large-collection
  fixture before changing it. Separate Member transport, Media acquisition,
  local inventory, remote Collection pagination, final revalidation,
  installation, and final report refresh in the evidence; do not attribute the
  reported minute-long wait to one stage without a repro.
- Preserve fresh unique-match proof immediately before each matched replacement,
  including remote uniqueness across Collection pages, and the strong-ETag,
  local-byte/path, clean-buffer, and destination checks in ADR-0200. Collection
  pages are not a batch snapshot (ADR-0209): do not reuse remote uniqueness
  evidence from an earlier Post. Optimize only work proven redundant under that
  rule. If the mandatory full scans dominate, document the cost and propose a
  separate decision rather than weakening safety.
- Before each selected Post, identify its batch position and Post ID. Show
  distinct stages before blocking: Member staging (including Media acquisition),
  fresh Collection verification with page number advanced on each completed
  page, final Member revalidation, local installation, and final report refresh.
  Stage transitions and page progress must reach the display before the next
  blocking call; after each Post show success, blocked, or failed. Do not imply
  elapsed-time heartbeat during a synchronous request that cannot paint one.
- Bound each HTTP connection attempt to 15 seconds and fail a read that averages
  less than one byte per second for 60 seconds (curl's low-speed limit). A
  timeout is a row-local transport failure with Post/stage context, not success.
  Earlier successes and later eligible rows remain intact; cancellation remains
  between Posts. No automatic retry of mutations or automatic overwrite after an
  unknown outcome. A whole batch may take longer than these per-request
  inactivity bounds, especially across many Collection pages or for large Media
  that continues to deliver bytes.
- Preserve the existing one-time ETag rebaseline workflow, canonical validators
  through proxies, and compatibility with older servers lacking Collection
  Member ETags (ADR-0209/0210). The initial report and unrelated push, delete,
  and conflict behavior are not redesign targets.

## Acceptance

1. A deterministic host-runnable fixture with 100 Collection Members across four
   25-Member pages and three selected server-ahead Posts records per-stage time
   and HTTP request counts before and after changes, including 12 fresh
   Collection page requests for those three matched-post verifications (plus any
   initial/final report fetch). It demonstrates any safe reduction in other work
   or records that the required scans dominate, with the latter an evidence-only
   acceptable outcome—not a claim of batch speedup.
2. A selected fetch visibly names each Post ID, batch position, and current
   stage before blocking. Four completed verification pages produce four page
   updates per Post; report refresh is separately labelled. Tests distinguish
   success, blocked, read timeout, cancellation, and refresh failure, and
   demonstrate the 15-second connect and 60-second low-speed read policy without
   waiting real time. Document where detailed failures appear and how to safely
   resume a partial batch.
3. Pure and live Emacs regression tests demonstrate that a changed remote ETag,
   duplicate/changed Post identity, changed local bytes, modified visiting
   buffer, and occupied destination still block a matched replacement; Media
   verification, partial-success reporting, and older-server fallback still
   work.
4. Focused Emacs tests and the applicable project verification gate pass. Record
   the measured before/after stage counts or timings alongside the change; do
   not promise a production wall-clock speedup from a synthetic fixture alone.

## Boundaries

No background task scheduler, new daemon or Rust CLI, automatic conflict
resolution, bulk overwrite, new server protocol, or opportunistic ETag
rewriting. A single in-flight synchronous request cannot be cancelled or made
interactive by this work. If fresh remote uniqueness dominates batch time,
report that limitation and seek a separate design decision instead of shipping
an unsafe shortcut.
