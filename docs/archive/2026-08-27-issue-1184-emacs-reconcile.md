# Issue #1184 — Preview and report Post reconciliation

## Outcome

Add a per-blog `jaunder-reconcile` command that compares one configured Org
directory with its AtomPub Collection, leaves a persistent report of every
inventory and divergence state, and optionally pulls only server-only Posts
after one confirmation.

## Load-bearing decisions

- Reconciliation consumes #1185's side-effect-free inventory. It does not
  duplicate Collection pagination, directory scanning, or local/remote joining.
- For every matched Post, reconciliation fetches the current Member. A
  successful response must carry a strong ETag; server change is exactly
  inequality with the strong ETag stored in `JAUNDER_SYNCED`. Local change is
  exactly a file mtime more than two seconds later than the UTC instant in
  `JAUNDER_SYNCED_AT`.
- A matched Post is `unclassifiable` with no offered action at the first failed
  prerequisite in this order: Member fetch, current ETag, stored ETag, sync
  instant, then file mtime. Stable reasons are `member-transport-error`,
  `member-not-found` for `404`, `member-http-error` with its status,
  `current-etag-invalid`, `stored-etag-invalid`, `synced-at-invalid`, and
  `file-mtime-unreadable`; each `*-etag-invalid` covers missing, malformed, or
  weak values. Transport detail may accompany its stable reason. One failed
  Member does not suppress other rows or replace an existing report before the
  new report is complete.
- Matched Posts are classified as `unchanged`, `server-ahead`, `local-ahead`, or
  two-sided `conflict`. Server-ahead rows give pull guidance; local-ahead rows
  give publish guidance; neither is changed by reconciliation.
- Inventory-only rows remain distinct: `orphan`, `local-draft`, `server-only`,
  and `inventory-conflict`. One inventory-conflict row and count unit represents
  one D1 connected conflict group; it preserves every conflict kind and every
  implicated local path, Member, ID, and slug. It is never folded into two-sided
  divergence.
- `*Jaunder Reconcile*` remains available after display, cancellation, and
  application. Sections use the shared contract's state order; rows preserve D1
  inventory order. The buffer shows deterministic counts and rows for every
  state, all conflict details, stable unclassifiable reasons, and one-sided
  guidance.
- The preview set is exactly the inventory's server-only Posts. Confirmation is
  requested once for that set. On confirmation, #1182's pull operation rechecks
  each destination and applies only that previewed set; occupied destinations
  remain blocked rather than overwritten or renamed.
- Network work uses the active blog selected by the existing longest-prefix
  directory-to-blog boundary and the existing authenticated AtomPub transport.
  An unconfigured root or transport use without an active blog fails loudly.
- Reconciliation never publishes a Post, resolves divergence, overwrites a
  matched local file, or interprets a missing local file as deletion.

## Acceptance

- Pure ERT fixtures cover all four matched mtime/ETag combinations and the exact
  two-second boundary. Separate and overlapping failures cover Member transport
  and HTTP errors, missing/malformed/weak current and stored ETags,
  missing/malformed sync time, and unreadable mtime, proving the specified
  first-reason precedence.
- Pure ERT covers every inventory-only class and overlapping connected conflict
  group, with exact states, group counts, preserved details, and guidance.
  Buffer-level assertions prove deterministic sections, counts, rows, reasons,
  and guidance remain unchanged after cancellation and application.
- Pure ERT proves the previewed server-only identity set is the set offered for
  application; cancellation performs no pull. Command-level fixtures with nested
  configured roots prove longest-prefix selection scopes inventory, Member
  requests, rows, and pulls to one Collection, and that no active blog fails
  loudly.
- Live ERT proves current Member ETags are fetched, confirmation pulls only the
  previewed server-only Posts through the existing safe pull behavior, an
  occupied destination stays unchanged and blocked, and the report remains.
- Existing ERT continues to prove Collection inventory and deterministic pull
  behavior at their owning seams; reconciliation tests do not replace those
  contracts.

## Boundaries

No recursive scan, automatic server-ahead pull, automatic publish, batch
resolution, merge or last-write-wins policy, hand-deletion propagation, media
download/localization, destination suffixing, or explicit Post deletion is in
scope. The AtomPub wire contract and server behavior do not change.
