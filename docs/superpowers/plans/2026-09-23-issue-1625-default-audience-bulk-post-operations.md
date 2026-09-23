# Default Audience and Bulk Post Operations Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` only for an isolated
> task. This outline exists because the approved spec adds persisted preference
> semantics and a multi-Post transaction with optimistic-concurrency and
> backend-parity requirements.

## Scope

In:

- Site, User, and Effective Default Audience storage, authorization, web
  controls, and web/AtomPub create precedence.
- Compact Manage Posts read/filter/pagination and exact confirmation snapshots.
- A paired storage migration and startup backfill for the shared normalized
  title/slug search projection.
- Atomic Change Audience and Delete operations preserving ordinary Post
  lifecycle and publication side effects.
- Focused dual-backend, HTTP, browser, accessibility, and visual proof.
- The approved spec, hierarchical-default-audience draft, glossary, architecture
  projection, and the related AtomPub draft wording already authored on this
  branch.

Out:

- Deleted Post management, restore, purge, other bulk actions, durable jobs,
  operation caps, plugins, and external-client Named Audience discovery.
- A schema migration for the User Default Audience itself: the existing
  `user_config` key/value table represents the optional closed preference.
- Changes to `docs/DESIGN.md`; this feature realizes existing publishing goals
  rather than changing them.

## Task outline

- [ ] Task 1: Resolve and expose hierarchical defaults on every creation path
  - Contract: add a validated `UserConfigKey` for an optional `DefaultAudience`;
    storage exposes get/set-or-clear operations and one Effective Default
    Audience resolver over exact `UserConfigStorage` and `SiteConfigStorage`
    dependencies. Absence inherits the site value; malformed user data is a
    decode error; absent/malformed site data remains Private.
  - Contract: `/admin/site` owns the operator-only Site Default Audience card;
    `/profile` owns the authenticated User's Public/Subscribers/Private/Use site
    default control. Web composer and AtomPub creation call the same resolver;
    explicit audience input still wins.
  - Contract: keep Site and User writes independent; neither setting mutation
    edits existing Posts or emits Post lifecycle side effects.
  - Verification: dual-backend `UserConfigStorage` tests cover absent, valid,
    clear, malformed, and database-error cases; existing Site Default Audience
    tests retain malformed/absent fallback and database-error propagation. HTTP
    tests cover authorization and typed wire rejection. Web and AtomPub create
    tests prove explicit → User → Site → Private precedence, malformed-user
    rejection, and site-storage-error propagation. Use focused
    `cargo xtask test-local` filters for `user_config`, `web::profile`,
    `web::site`, `web::posts::create`, and `atompub` before the commit boundary.

- [ ] Task 2: Deliver the Manage Posts read model and stable selection snapshots
  - Contract: introduce a management request carrying publication-state,
    audience-target, normalized title/slug text, cursor, and page size. Each
    backend applies every filter in storage and returns one bounded
    `updated_at DESC, post_id DESC` keyset page with its complete Audience
    Selections; no ordinary page request loads or scans an unbounded result into
    host memory.
  - Contract: paired migration `0041` adds a nullable normalized Post search
    projection. A shared Rust derivation case-folds title plus slug after
    Unicode whitespace normalization; create/update maintain it atomically.
    Startup completes legacy NULL rows before serving requests by reading
    bounded chunks and issuing one batched write per chunk through each dialect.
    Both listing queries and all-matching snapshot resolution search only that
    projection, so SQLite and PostgreSQL share byte-identical text semantics.
  - Contract: expose a compact row DTO with Post ID, mutation version,
    established fallback label, slug, Draft/Scheduled/Published state, complete
    Audience Selection, and updated time. The mutation version changes on every
    meaningful Post mutation.
  - Contract: selection intent is either explicit Post IDs or all matching one
    management filter. Confirmation resolves either intent server-side into a
    `BulkSelectionSnapshot`: exact owner-scoped `(PostId, mutation version)`
    targets in canonical Post-ID order plus selected count. The browser carries
    that exact snapshot into execution; later matching or unrelated Posts do not
    join or invalidate it.
  - Verification: migration/startup tests prove legacy backfill, idempotence,
    stale-candidate rejection, and projection maintenance on create/update.
    Query tests pin all three publication states,
    Public/Subscribers/Private/Named membership, normalized title/slug matching,
    titleless/textless labels, keyset stability, bounded page size, and backend
    parity. HTTP tests prove owner scoping, cross-page explicit selection,
    all-matching resolution, and exclusion of Deleted Posts.

- [ ] Task 3: Compose atomic bulk Post mutations inside one write scope
  - Contract: a storage-owned bulk service accepts the authenticated User,
    `BulkSelectionSnapshot`, operation, and one request clock. One set-based
    validation/lock call consumes the complete canonically ordered snapshot and
    rejects any owner/active/version mismatch before mutation; all work then
    remains in the same `WriteScope` transaction or rolls back.
  - Contract: add bulk-aware, backend-parity storage primitives rather than
    looping ordinary per-Post writers. A fixed set of batched statements
    captures one complete revision plus audience/tag/media children for every
    materially changed target, applies Change Audience or soft deletion
    set-wise, and enqueues all required feed/WebSub evidence through batched
    storage. Change Audience validates one complete selection before the write
    scope and excludes equal target sets from every write; Delete retains
    ordinary Deleted Post semantics.
  - Contract: return selected and materially changed counts only after commit.
    Missing, deleted, unauthorized, stale, or failed targets abort the complete
    transaction; no metrics or success invalidation may claim a rolled-back
    mutation.
  - Verification: dual-backend integration tests prove deterministic lock order,
    target-count-independent batched write calls, multi-target success, mixed
    changed/no-op counts, complete revisions and audience children, feed-event
    parity, and rollback for stale, missing, unauthorized, and induced mid-batch
    failure. Run the focused storage/Post lifecycle lane before the commit
    boundary.

- [ ] Task 4: Build the compact Manage Posts interaction on the shared contracts
  - Contract: register private route `/posts/manage`, add authenticated
    navigation, and split host-testable filter/selection/confirmation state from
    wasm-only components. Reuse the Audience picker for complete replacement; do
    not create a second Audience Selection model.
  - Contract: selection persists across pages; Select all matching replaces it
    with the server-resolved snapshot. Confirmations show operation, exact
    count, and the complete replacement Audience Selection or explicit Deleted
    Post scope; Delete requires count entry at ten or more. Pending execution
    disables every duplicate-submit path. Success reports selected/changed
    counts and refreshes the filtered page; conflicts preserve no success state
    and require refresh.
  - Contract: before presentation mutation, capture the existing `/app`,
    `/drafts`, and honest `/posts/manage` not-found baseline with deterministic
    data at 1440×900 and 390×844. Recapture the approved After states after the
    final presentation change.
  - Verification: host tests cover filter/selection transitions, confirmation
    thresholds and scope text, pending gates, and result rendering. Component,
    e2e, accessibility, and visual assertions pin the confirmation's complete
    target/scope, visible compact fields, keyboard-operable controls, actionable
    failures, and no horizontal page overflow at both viewports.

- [ ] Task 5: Prove the complete browser flow and reconcile documentation
  - Contract: add focused Playwright coverage for operator and User defaults,
    inherited-value display, compact filtering, cross-page selection, snapshot
    conflict, full Audience replacement/no-op, Delete safeguards, pending state,
    and selected/changed success counts. Use semantic waits and one document
    boot under `jaunder-e2e` rules.
  - Contract: keep `CONTEXT.md`, the proposed hierarchical-default-audience ADR,
    `docs/ARCHITECTURE.md`, and the AtomPub audience draft consistent with the
    delivered behavior; do not edit generated `docs/README.md` or promote the
    ADR draft.
  - Verification: run focused `cargo xtask e2e-local` for the owning spec, then
    the relevant broad gate selected by `jaunder-ship`. If new server functions
    change the inventory, regenerate and verify `docs/coverage/server-fns.json`
    only from the authoritative SQLite/Chromium capture. Present review-only
    visual pairs with identical fixture, viewport, theme, and auth conditions:
    Before uses `/app` and `/drafts`; After uses `/posts/manage`. Do not create
    a new committed visual-snapshot variant for this comparison.

## Risk checks

- Default resolution is one shared rule: explicit > User > Site > Private;
  malformed User configuration never widens through inheritance.
- Every ordinary management page remains a bounded storage-filtered keyset
  query; the paired search-projection backfill completes before requests and
  preserves byte-identical filter semantics across backends.
- Confirmation snapshots contain exact targets and versions; execution never
  reinterprets an all-matching filter or absorbs a later Post.
- Bulk writes use set-based validation and batched statements, one request clock
  and transaction, and preserve semantic no-op, revision, feed-event, rollback,
  and bounded SQLite write-lock invariants without per-row write loops.
- The web boundary injects exact storage handles and `WriteScope`; no state
  bundle or `StorageFactory` crosses the composition root.
- Every new HTTP endpoint receives backend-parametric integration and e2e
  coverage; every user-visible state retains keyboard and automated
  accessibility proof.
- No lint, coverage, accessibility, or test suppression is introduced without
  explicit approval.
