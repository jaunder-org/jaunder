# Structural write transactions

Issue: #363

## Outcome

Every one of the 48 audited application storage mutation APIs participates in an
explicit caller-owned write scope, so callers can compose storage traits
atomically without a pool or arbitrary SQL. Pre-commit operation failure cannot
leave a committed database mutation; unacknowledged commit is explicitly
indeterminate. Server mutation outputs distinguish confirmed from indeterminate
success, and both backends retain their required transaction and locking
semantics.

## Load-bearing decisions

- `WriteScope` is a concrete, homogeneous backend-erased value minted only by
  the runtime-selected backend factory at the composition root. It owns
  transaction acquisition, callback execution, commit-on-`Ok`, rollback/drop
  behavior, and commit-outcome classification. The composition root injects it
  separately beside the exact storage traits into server context and workers; no
  downstream code constructs, locates, or selects a scope. `WriteScope` exposes
  scope entry only—never storage lookup or SQL—and server functions, workers,
  and CLI paths enter it explicitly around the smallest coherent set of database
  mutations rather than automatically at the whole-function boundary.
- The scope callback receives a sealed, backend-erased mutable write transaction
  capability. It is object-safe so existing `Arc<dyn XStorage>` dependencies
  remain object-safe, but it is neither a SQL executor nor constructible by
  application callers.
- Every one of the 48 audited application storage mutations requires that
  capability. There is no auto-committing pool form, standalone sibling,
  compatibility overload, or generated paired method. A one-operation write
  still enters `WriteScope` and calls the same transaction-required mutation.
- The runtime-selected backend adapter creates the concrete transaction behind
  backend-erased application traits; a mismatch is an internal wiring error.
- PostgreSQL uses a tracked SQLx transaction. SQLite write scopes use SQLx's
  tracked custom-begin transaction with `BEGIN IMMEDIATE`; this supersedes
  ADR-0021's single-statement autocommit preference for the 48 audited APIs,
  preserving their no-deferred-read-to-write-upgrade invariant. An unfinished
  scope must never return an open transaction or write lock to the pool.
- SQLite write scopes stay narrow. Validation, rendering, ordinary password
  hashing, filesystem work, and network work happen before acquisition or after
  completion. Only the existing ADR-0022 claim-before-Argon2 cases may retain
  expensive work inside the write scope.
- The internal scope result algebra distinguishes rollback-confirmed operation
  failure, confirmed commit, and commit-indeterminate. Operation callback
  failure means the transaction was not committed; callback success causes
  exactly one commit attempt. A commit acknowledgement failure is indeterminate
  because PostgreSQL may have committed before the connection failure became
  observable; it is never reported as a rollback-confirmed operation error.
- `WriteScope` records its bounded outcome—rollback-confirmed operation failure,
  confirmed commit, or commit-indeterminate—on the narrowest span that owns the
  decision, as required by ADR-0147.
- Server mutation wire outputs map confirmed and indeterminate commits to
  successful typed `MutationOutcome<T>` variants. Only rollback-confirmed
  operation failure remains an ordinary error. Clients invalidate and revalidate
  for either outcome variant, while the indeterminate variant remains visibly
  error-like to the caller rather than masquerading as confirmed success.
- Preparation functions and values are context-neutral: callers normally prepare
  before acquiring the write scope, but a load-bearing atomic sequence may call
  the same preparation inside it. Reusable mutation has one transaction-required
  implementation; private concrete-connection helpers may share SQL and
  bindings, but no second pool-backed mutation implementation is allowed.
- `AtomicOps` retains exactly its two existing workflows:
  `create_user_with_invite` and `confirm_password_reset`. It no longer owns
  transactions; each workflow consumes the caller's capability and must not
  expand into an omnibus replacement for caller composition.
- #874 and #909 are proving slices, not external prerequisites. The #874 safe
  tracked-drop slice lands before or atomically with #909; only then does
  `set_post_tags` consume the common capability while preserving SQLite
  `BEGIN IMMEDIATE` and PostgreSQL row-lock behavior. This ordering supersedes
  the issue's recorded prerequisite chain.
- Child issues and vertical pull requests may land under #363, but each selected
  mutation cuts over both backends, every caller, and its tests together. The
  global invariant waits for complete conversion and removal of old paths.
- Database/filesystem composition does not pretend to be a distributed
  transaction. Upload removes a newly created file on callback failure but
  retains it after confirmed or indeterminate commit. Delete reclaims only after
  confirmed commit; failure or indeterminate commit retains the file as a safe
  orphan rather than risk removing a possibly referenced file. Durable orphan
  reconciliation and crash cleanup remain separate.
- No macros, generic operation-command framework, public generic executor, or
  effect system. Explicit `WriteScope::run` is the visible commit boundary.

## Acceptance

- A compile-time check or compile-fail test demonstrates that each of the 48
  audited application storage mutations cannot be called without the sealed
  write transaction capability.
- Composition-root wiring has a direct proof that only the backend factory mints
  `WriteScope`, and that server context and workers receive it separately beside
  their exact storage traits; no downstream storage lookup or scope construction
  is possible.
- No final production or test-support caller persists an audited application
  storage mutation through a bare pool, a storage-owned transaction, or an
  independently committing convenience method.
- A dual-backend test mutates one concern, injects an error in a later concern,
  and observes that neither mutation committed.
- A dual-backend test drops or unwinds an uncommitted scope and observes both no
  committed data and a usable subsequent writer. The SQLite case proves no open
  `BEGIN IMMEDIATE` transaction returns to the pool.
- The existing post-tag locking, snapshot, and injected-error suites pass after
  `set_post_tags` moves to the shared capability; its write statements have no
  bare-pool route.
- SQLite transaction tests demonstrate tracked `BEGIN IMMEDIATE` behavior for
  the pinned SQLx version. PostgreSQL tests retain required row locks and
  rollback-on-drop behavior.
- Cross-trait email verification and user-email update run in one scope; failure
  of the user update leaves the verification code unused on both backends.
- Registration composes user creation or invite consumption with session
  creation in one scope; a session failure leaves no user and does not consume
  the invite.
- Feed-event batching retains bounded transaction size and does not hold a write
  scope across rendering, worker loops, or WebSub calls.
- User insertion and password-reset claim each have one mutation body shared by
  standalone one-operation scopes and larger atomic workflows; their SQL is not
  duplicated to obtain transaction composition.
- Media tests cover callback failure, confirmed commit, and indeterminate commit
  for upload and delete: they never remove a possibly referenced file, and
  post-commit reclamation cannot change the handler result to an ordinary error.
- Commit failure is observable as an indeterminate outcome distinct from an
  operation error, the owning scope span records the bounded outcome
  determinant, and the revalidation decision covers the indeterminate outcome.
- Server and client tests exercise both typed `MutationOutcome<T>` variants:
  confirmed and indeterminate both invalidate relevant data, while indeterminate
  remains visibly error-like rather than a confirmed result.
- Both SQLite and PostgreSQL integration suites pass throughout each completed
  vertical cutover and at final completion.
- The final structural census finds 48 transaction-required audited application
  storage mutation methods, including exactly the two `AtomicOps` workflows; it
  finds no obsolete mutation paths or remaining production mutation caller
  outside a write scope. Administrative database lifecycle operations are not
  part of this census. Only then is the interim #362 heuristic retired.
- Implementation ships a tracked numberless ADR draft for this decision and a
  truthful `docs/ARCHITECTURE.md` projection; neither artifact is created before
  this specification is approved.

### Authoritative mutation census

The universal invariant and this census apply only to the 48 audited application
storage mutation APIs below. They do not apply to administrative database
lifecycle operations: migrations, whole-store backup restore, and PostgreSQL
bootstrap retain their top-level transaction or connection ownership and are not
composable application mutations.

- `AudienceStorage`: `create_audience`, `rename_audience`, `delete_audience`,
  `add_member`, `remove_member`.
- `EmailVerificationStorage`: `create_email_verification`,
  `use_email_verification`.
- `FeedCacheStorage`: `upsert`, `delete`.
- `FeedEventStorage`: `enqueue`, `enqueue_many`, `claim_pending_batch`,
  `mark_regenerated`, `mark_pinged`, `mark_failed`, `mark_exhausted`.
- `InviteStorage`: `create_invite`.
- `MediaStorage`: `create_media`, `try_delete_media`.
- `PasswordResetStorage`: `create_password_reset`, `use_password_reset`.
- `PostStorage`: `create_post`, `create_posts`, `update_post`, `publish_post`,
  `soft_delete_post`, `unpublish_post`, `set_post_tags`.
- `SessionStorage`: `create_session`, `authenticate`, `revoke_session`.
- `SiteConfigStorage`: `set`, `delete`, `set_identity`, `set_backup_config`,
  `set_default_audience`, `set_feeds_config`.
- `SubscriptionStorage`: `subscribe`, `unsubscribe`.
- `UserConfigStorage`: `set`, `delete`.
- `UserStorage`: `create_user`, `authenticate`, `update_profile`, `set_email`,
  `set_password`.
- `AtomicOps`: `create_user_with_invite`, `confirm_password_reset`.

## Boundaries

- No database schema or stored-data migration.
- Administrative database lifecycle operations—migrations, whole-store backup
  restore, and PostgreSQL bootstrap—remain outside the universal application
  mutation invariant. They retain top-level transaction or connection ownership
  and are not composable application mutations.
- No transaction spanning filesystem, SMTP, WebSub, or another remote system.
- No general crash-recovery, cleanup queue, or transactional-outbox framework.
- No compatibility shim survives a selected method's vertical migration; #363
  completes only after every child migration lands.
