# #1824: Posts source-block readiness

**Status:** Approved for implementation by the User. Direct provenance: User
response `#1824: approved` to the reviewed minimal Posts-test repair proposal
(Manager assignment brief). Independent proposal review found no findings. This
approval covers the unchanged proposal below; it is not per-PR merge approval.

---

## #1813 Posts failure diagnosis

This reviewed, User-approved proposal is authorized for implementation within
its stated boundary. Scope is the existing
`published Org source blocks are distinct and readable on narrow screens` test
and its fixture/readiness. Do not edit product CSS, broaden the locator, relax
assertions, change retry/budget/population, or alter the cold permalink entry.

## Reconciled diagnosis

Queue 40030970355 / head `0f1df9ada6a17915d48aec243d0ac37b1ec41532`,
`posts.spec.ts:139`:

- Direct #1813 evidence: initial `overflowX` assertion got `""`; test had
  already found expected code text/token in the permalink. The trace shows Post
  creation 413, permalink `posts/get` and both stylesheet requests succeed, and
  captured application CSS includes `.j-post-body pre { overflow-x: auto; }`.
  Yet its protocol trace does **not** record the locator's `isConnected`,
  `ownerDocument`/frame, computed-style return, or matched CSS rules at
  evaluation time. Consequently the detached-seed explanation is not
  independently proven by #1813 alone.
- New corroborating evidence supplied from PR #1823 diagnostic of run
  40033475729: both attempts' trace snapshots show evaluation begins with
  projector-seeded fallback, then the fetched PostCard replaces it during the
  evaluation interval; retry evaluates fetched card #414 (initial #413). This
  materially strengthens a detached seed `<pre>`/stale Locator race as the
  leading explanation for `getComputedStyle(...).overflowX === ""`. The direct
  isConnected bit still was not captured. Confidence: **medium-high leading
  inference**, not direct instrumentation or proof that no live-card defect
  exists.
- The test's negative checks (`background` not transparent, `padding` not `0px`)
  accept empty strings, so they did not prove that the queried element had
  application styling. They must remain, but should be made positive/retrying at
  the fetched PostCard; this preserves the actual styling intent rather than
  suppressing the empty result.
- Retry 1 POST created Post #414; prior attempt created #413. The strict-mode
  failure occurs at `localCode` after navigation to anonymous Local
  (`posts.spec.ts:216-219`), not on Home. The anonymous Local surface
  legitimately contains both persistent public Posts from separate attempts. The
  retry error-context capture also shows an authenticated Home snapshot with
  just one Post by the displayed current User; that is a distinct page/surface
  and is not evidence of a Home ownership leak. `createPostViaApi` performs one
  create and returns `post_id`/permalink, with no cleanup registration; both
  public Posts legitimately persist for Local. Resolve Local's intended Post
  selection by returned identity while preserving the assertion that this
  subject is present and that Local contains public Posts. Do not use
  `.first()`, weaken strictness, delete/cleanup public Posts, or add population
  changes.
- Exact source comparison: #1708 changed setup from compose/follow-permalink to
  API-create/cold-permalink, but did not change the styling expectation. No
  causal blame for #1708 is justified. Theme and app CSS responses are
  successful; no production CSS patch is proposed.

Evidence detail: diagnosis and CI receipt supplied in the #1824 planning
assignment; the 40030970355 report/trace/screenshots were retained with that
diagnosis. Cross-run provenance: Manager-supplied #1752 diagnostic receipt for
PR1823 run 40033475729, producer artifact 12538115360; it identifies
evaluated-node replacement timing but explicitly lacks direct `isConnected`
instrumentation.

## Minimal test-only proposal

1. Preserve `createPostViaApi` + its returned `{post_id, permalink}`, and
   preserve exactly one cold `registeredPage(permalink)` document entry
   (ADR-0111); do not return to UI composer setup or add a second boot.
2. After navigation, wait/assert for the **fetched** PostCard identity from the
   returned `post_id` (existing `#j-post-${post_id}` ID is emitted by
   `PostCard`; use a retrying locator assertion such as
   `expect(page.locator(`#j-post-${post_id}`)).toBeVisible()` before acquiring
   `.j-post-body pre`). Confirm this readiness distinguishes fetched PostCard
   from the projector seed; do not use generic body/text/token presence as
   readiness because both fallback and fetched content have those.
3. Read CSS through retrying Locator assertions on the identity-scoped fetched
   card, and explicitly reject empty values: `overflow-x` must equal `auto`;
   background must be nonempty and nontransparent; padding must be nonempty and
   nonzero. An empty computed background or padding string is a failure, not a
   pass. Ensure each retry queries the returned-ID card rather than a previously
   detached element handle. Keep current code-text/token checks and narrow
   viewport geometry checks intact.
4. Keep existing Home, anonymous permalink, and anonymous Local checks. Scope
   the intended Local subject assertions to the returned `post_id`, preserving
   strict identity without selecting `.first()` or loosening the locator. The
   two public Posts from separate attempts may legitimately coexist on Local;
   this is explanatory evidence, not a new population assertion. Do not add
   cleanup or change the test population.
5. Rollback boundary: if the fetched connected PostCard still lacks positive
   overflow/background/padding or fails narrow-screen geometry, stop—this
   indicates a live style/behavior contract violation, not a test race. Revert
   only the test synchronization change and route a separately scoped product
   fix decision; no CSS patch is authorized by this proposal.

## Proof plan (not run)

- First focused implementation proof:
  `devtool run -- cargo xtask e2e-local posts.spec.ts:139`. This existing route
  is host SQLite/Chromium; it exercises browser readiness/style and returned-ID
  Local subject selection, but it is **not** evidence for the failed PostgreSQL
  lane or VM behavior.
- Mandatory later: fresh PostgreSQL/Chromium current-CI proof, preserving traces
  for every attempt that actually occurs (a green run need not retry or
  manufacture a retry), proving fetched PostCard identity before styles are
  sampled, positive nonempty computed CSS and geometry, and returned-ID Local
  subject selection while preserving existing Home and anonymous-permalink
  checks. No SQLite result substitutes for this proof.
- No test, VM, CI, source change, requeue, or land action was performed in the
  diagnosis. No test command was executed there.

Canonical ADRs considered: ADR-0111 one document boot per Page; ADR-0184 Theme
Package/CSS contract. Domain vocabulary follows `CONTEXT.md` (Post, Home,
Local).
