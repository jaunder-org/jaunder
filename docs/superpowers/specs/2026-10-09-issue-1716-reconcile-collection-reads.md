# Issue #1716: Operation-owned Collection evidence for reconciliation

Issue: <https://github.com/jaunder-org/jaunder/issues/1716> Related reproducer:
<https://github.com/jaunder-org/jaunder/issues/1722>

## Outcome

All confirmed reconciliation batch-changing actions use one operation-owned flow
instead of rebuilding the complete AtomPub Collection inside individual row
preflights or nested publishing. A single selected Post is the same flow with a
batch size of one. Collection work is bounded independently of selection size,
without weakening mutation authorization or hiding recovery outcomes.

## Load-bearing decisions

- Cover push, pull, keep-local, keep-remote, and delete, including nested Local
  Post Link resolution. Refactor shared discovery, evidence lifetime, and
  revalidation into one deep operation-level module, not broader action-specific
  cache bindings. Keep dependencies exact under ADR-0016 and retain ADR-0047's
  configured-root/blog resolution; no heterogeneous dependency bundle crosses
  the module's interface.
- One confirmed operation owns evidence for one root, active origin and User.
  Acquire at most one complete Collection traversal when discovery or global
  identity/link proof is needed, independently of the report's preview. Reuse
  may begin at first need, but proof must exist before its dependent mutation.
  Push without Post links, delete, and immediately ineligible/blocked rows must
  not introduce unnecessary discovery reads merely to force two traversals.
- Retain exhaustive pagination, malformed-page/cycle rejection, and duplicate
  remote-ID rejection. Distinguish unacquired, complete-empty, complete, and
  failed acquisition. Partial or failed acquisition never authorizes mutation,
  becomes an empty/default Collection, or falls back to stale preview evidence.
  Retain its diagnostic; do not retry discovery independently for every row.
- Keep discovery separate from authorization. Preserve each action's reviewed
  identity/strong-ETag requirements, targeted Member checks, conditional PUT or
  DELETE, eligibility, and local source authority. Never silently adopt a newer
  remote ETag from discovery. Ordinary push continues publishing current
  authored source; explicit conflict choices retain their reviewed snapshots.
- Re-read current local identity and uniqueness at matched-row revalidation
  boundaries, rather than caching a local inventory across rows. Matched pull
  and keep-remote require a last local uniqueness scan after Media finalization,
  immediately before the final digest/buffer/destination guard and replacement.
  Preserve clean visiting buffers, no-overwrite, Media integrity, installed-byte
  sync checkpoints, and recoverable atomic replacement/rename under ADR-0200 and
  ADR-0211; durable create intent/replay remains governed by ADR-0199.
- Operation-owned remote discovery cannot remain authoritative for an identity
  changed by an earlier write. Confirmed create/update/delete must update or
  invalidate that identity's discovery/link evidence before later consumers,
  even if subsequent local write-back or rename fails. Unknown write outcomes
  invalidate affected assumptions rather than being treated as unchanged.
- Restore invalidated evidence only from valid authoritative response data or
  fresh targeted Member reads, never another per-row Collection traversal. Fresh
  target proof must retain exact identity/edit URI, canonical slug and singleton
  valid harvested alternate href. Later rows referencing changed, deleted,
  uncertain, newly created or renamed targets must use current valid proof or
  fail closed. Do not invent a missing Post ID or guess a permalink.
- Preserve ADR-0201's direction-specific link rules: publish aborts visibly when
  required target proof is invalid; pull preserves canonical URLs without valid
  localization proof. Current local target identity/path/slug and uniqueness
  remain necessary. Authored links are not rewritten to repair a missing target
  path after another row's rename. Discovery does not authorize Post mutation.
- Preserve sequential execution, confirmation, cancellation between Posts,
  independent row failures, and ordered terminal results. A lost write response
  is unknown, not a safe automatic update/delete retry; confirmed remote commit
  followed by failed local completion is partial success, not rollback. Existing
  safe keyed-create recovery remains intact. Independent rows may continue, but
  a dependent row cannot assume uncertain evidence is valid.
- Discard operation evidence before a separate authoritative final report
  refresh, including completion, cancellation and row failures. Refresh failure
  retains terminal recovery results and the reviewable report. New invocations,
  roots, blogs and nested operations own independent evidence.
- Interactive merge never holds operation freshness across human editing.
  Preparation and completion use distinct short-lived operation scopes; finish
  reacquires needed evidence and repeats its existing reviewed-state checks.
  Preserve Ediff scratch ownership, cancellation and terminal refresh behavior.
  Preparation does not add a redundant report refresh when no Post changed.
- Supported server writes cannot duplicate primary-key Post IDs. Sharing one
  discovery walk accepts less repeated detection of newly faulty global
  duplicate Entries after acquisition; selected-Member and fresh local checks
  remain authoritative. Neither pagination nor a batch is an atomic snapshot or
  an HTTP/filesystem transaction. Record this trade-off and operation ownership
  in a numberless ADR draft and the architecture/client guides.

## Acceptance

- Exercise public confirmed commands for all five actions with one and several
  selected Posts and a multi-page Collection. Count opening the report, the
  operation, targeted Member work, and final refresh separately. Three matched
  pulls, keep-remote choices, keep-local choices, and link-bearing pushes over
  the 100-Member/four-page fixture use at most four operation page GETs plus
  four refresh page GETs, not complete walks per row/preflight/link pass. Push
  without links and delete skip unused discovery. Changing Collection size is
  measured as at most one operation walk and one final walk, not identical page
  counts.
- Exercise blocked/failed rows, empty evidence, duplicate remote Entries,
  cycles, malformed/non-2xx pages and transport failures. No partial/stale proof
  permits a dependent mutation, no row retries failed discovery, and final
  refresh is separate even after failure. Prove same-report later retry,
  cancellation, independent failures, root/blog/nested-operation isolation and
  retained results when final refresh fails.
- Inject remote ETag/identity drift or disappearance and local duplicate IDs,
  identity/path/digest changes, dirty buffers, destination collisions and Media
  changes during staging and between rows. Both matched pull and keep-remote
  reject a duplicate introduced after their prior Member check or during Media
  finalization, preserving the reviewed Post. Required targeted reads and
  conditional requests must remain real in these proofs.
- Exercise earlier-row create/update/delete/canonical rename, partial local
  completion after remote commit, and lost write responses with later cross-Post
  links. Later rows consume fresh valid target proof or refuse unsafe
  assumptions; no full Collection reacquisition, guessed URL, false rollback or
  unsafe retry. Retain server-only/mixed pull link coverage and delayed-pull
  sync convergence.
- Prove merge completion does not reuse preparation evidence after remote/local
  changes during editing; retain scratch and recovery outcomes on blocked,
  partial or unknown completion. A single-row operation shares the batch path.
- Replace obsolete private-cache tests with observable operation/command tests
  where the refactor supersedes those seams. Run focused red/green proofs,
  complete pure ERT and applicable live ERT publish/pull/conflict tests. Compare
  request counts and bounded timing evidence without claiming synthetic timings
  reproduce production latency. Keep run evidence in ignored/session storage;
  commit only regression consumers/fixtures and maintained contracts/docs.

## Boundaries

No server/storage/protocol change, long-lived cache, concurrent row execution,
background prefetch, invented rollback, historical conflict repair, or weaker
Post/Media safeguards. Standalone publish/pull retain their contracts; their
nested helpers participate only when a reconciliation operation owns the scope.
UI changes in #1718 and #1721 remain separate cycles. The shared fix's PR must
include `Closes #1716` and `Closes #1722`; neither issue closes before merge.
