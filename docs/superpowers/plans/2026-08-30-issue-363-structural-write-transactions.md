# Structural Write Transactions Implementation Outline

> Execute with `jaunder-iterate`, delegating individual tasks through
> `jaunder-dispatch` when useful. This outline exists because the change
> replaces the storage transaction boundary across both dialects and 48 mutation
> APIs.

## Review header

**Scope — in:** a factory-minted `WriteScope`; a sealed backend-erased mutable
write capability; conservative commit-indeterminate outcomes; caller-owned
transaction composition; all 48 audited mutations and every caller; both
backends; server/client revalidation; media ordering; #874 and #909; a proposed
ADR draft, architecture projection, and retirement of #362's interim heuristic.

**Scope — out:** migrations, backup/restore transaction ownership, PostgreSQL
bootstrap, distributed transactions, crash cleanup, a general executor or
command framework, and compatibility mutation paths.

**Tasks:**

1. Make SQLite custom-begin transactions tracked and drop-safe (#874).
2. Establish the write-scope contract and prove it through post tags (#909).
3. Cut over identity, credential, registration, and session mutations.
4. Cut over publishing, audience, subscription, and transactional event writes.
5. Cut over configuration, cache, and bounded worker mutations.
6. Cut over media with conservative database/filesystem ordering.
7. Close the structural census, documentation, and interim-gate removal.

**Key contracts:** only backend factories mint `WriteScope`; `WriteScope::run`
is the visible commit boundary; its callback receives the sealed capability;
operation failure is rollback-confirmed, while every commit error is
conservatively commit-indeterminate; application storage mutation methods have
only the capability-taking form; server wires expose typed `MutationOutcome<T>`;
the owning scope span records the bounded outcome.

**Risk checks:** preserve SQLite `BEGIN IMMEDIATE`, bounded lock occupancy, and
no open transaction on drop; preserve PostgreSQL row locks; keep Argon2 only in
the ADR-0022 claim windows; do not hold a scope across rendering, filesystem,
SMTP, worker loops, or WebSub; retain possibly referenced media after an
indeterminate commit; keep every vertical dual-backend and object-safe.

## Scope

Implement the approved
[specification](../specs/2026-08-29-issue-363-structural-write-transactions.md).
Each task cuts selected methods over across traits, both dialects, production
and test-support callers, and focused tests together. No task leaves a
pool-backed or independently committing compatibility form behind.

## Task outline

- [x] **Task 1: Make custom SQLite write transactions tracked and drop-safe.**
  - Contract: replace raw test-support `BEGIN IMMEDIATE` guards with SQLx 0.8.6
    `Connection::begin_with("BEGIN IMMEDIATE")`; an unfinished guard owns a
    tracked `Transaction` whose drop starts rollback before reuse.
  - Homes: `storage/src/test_support.rs` and its focused guard tests.
  - Verification: dropping or unwinding `PostWriteLock` commits nothing and a
    subsequent writer in the same dual-backend environment remains usable.

- [x] **Task 2: Establish the common write scope and post-tag proving slice.**
  - Contract: backend factories in
    `storage/src/{db,sqlite/open,postgres/open}.rs` mint a concrete
    backend-erased `WriteScope` beside exact storage handles; downstream code
    can enter it but cannot construct it, locate storage, or execute arbitrary
    SQL. The callback receives a sealed mutable capability. Callback `Err` is
    rollback-confirmed; callback `Ok` causes one commit attempt; any commit
    error maps conservatively to commit-indeterminate. The scope records its
    bounded outcome on its owning span.
  - Cutover: `PostStorage::set_post_tags` and every production/test-support
    caller consume the capability. Preserve SQLite `BEGIN IMMEDIATE`, PostgreSQL
    post-row and slug-ordered tag locks, snapshot ordering, and injected-error
    behavior. Remove the separate transaction bodies and bare statement route.
  - Outcome surface: introduce the typed `MutationOutcome<T>` algebra. No
    standalone Task-2 client endpoint consumes post tags, so client revalidation
    remains unimplemented rather than adding unused scaffolding.
  - Decision records: add the numberless proposed ADR draft and project the
    approved direction into `docs/ARCHITECTURE.md`.
  - Verification: dual-backend scope tests cover callback rollback, drop/unwind,
    later-writer usability, confirmed commit, injected commit-indeterminate, and
    the ADR-0147 bounded determinant; post-tag locking/error suites retain their
    existing focused coverage.

- [x] **Task 3: Compose identity, credential, registration, and session
      writes.**
  - Cutover: all mutations on `EmailVerificationStorage`, `InviteStorage`,
    `PasswordResetStorage`, `SessionStorage`, `UserStorage`, and the two
    `AtomicOps` workflows; migrate web registration/auth/email/password-reset/
    session/profile APIs, CLI user/invite/app-password callers, seed helpers,
    and their integration fixtures.
  - Contract: `AtomicOps` keeps only `create_user_with_invite` and
    `confirm_password_reset`, consumes the caller capability, and shares each
    mutation body with one-operation scopes. Registration composes user or
    invite-backed creation with session creation; email verification composes
    code use with `set_email`; login composes authentication's write with
    session creation. Preserve ADR-0018/0022 timing and claim-before-Argon2
    ordering.
  - Outcome surface: the first real mutation outputs consume
    `MutationOutcome<T>`; their client decision fold invalidates for confirmed
    and indeterminate outcomes while presenting indeterminate as error-like.
  - Verification: dual-backend injected-later-failure tests prove registration
    leaves no user/session and consumes no invite, and email update leaves the
    code unused; server/client tests prove typed outcomes and revalidation.

- [x] **Task 4: Compose publishing, audience, subscription, and event writes.**
  - Cutover: the remaining six `PostStorage` mutations, all five
    `AudienceStorage` mutations, both `SubscriptionStorage` mutations, and
    `FeedEventStorage::enqueue_many`; migrate `post_service`, web and AtomPub
    callers, test-support seeding, and server/storage integration fixtures.
  - Contract: Post mutation plus affected Syndication Feed events share the
    smallest coherent scope. Rendering, media-reference preparation, and other
    expensive work precede acquisition. Batched events remain bounded; tag and
    post locking/revision semantics remain unchanged.
  - Verification: both-backend Post lifecycle, revision, audience, subscription,
    AtomPub, and event-hook suites pass; a later event failure rolls back the
    Post mutation; no selected caller retains an independent commit.

- [x] **Task 5: Bound configuration, cache, and worker write scopes.**
  - Cutover: both `FeedCacheStorage` mutations, the remaining six
    `FeedEventStorage` mutations, all six `SiteConfigStorage` mutations, and
    both `UserConfigStorage` mutations; migrate site/backup APIs, CLI
    configuration, feed worker/regeneration, server fixtures, and storage tests.
  - Contract: each worker claim/status/cache write has its own smallest coherent
    scope. No scope spans rendering, a worker loop, WebSub publication, or
    network work; `enqueue_many` chunking and write-lock occupancy remain
    bounded.
  - Verification: dual-backend feed/config/cache suites and worker integration
    tests prove bounded acquisition, rollback on operation failure, and no
    foreign work under the scope.

- [ ] **Task 6: Preserve media under confirmed and indeterminate commits.**
  - Cutover: `MediaStorage::create_media` and `try_delete_media`,
    `storage::MediaManager`, web and AtomPub upload/delete handlers, seed and
    fixture callers, and media integration/e2e coverage.
  - Contract: upload prepares filesystem state before the scope, removes a newly
    created file on callback failure, and retains it after confirmed or
    indeterminate commit. Delete reclaims only after confirmed commit; operation
    failure or indeterminate commit retains a safe orphan. Post-commit reclaim
    failure is reported diagnostically but cannot rewrite the database outcome.
  - Verification: manager and dual-backend tests cover the complete
    upload/delete outcome matrix and fresh reference checks; server/client/e2e
    tests prove list and usage invalidation while indeterminate remains visibly
    error-like.

- [ ] **Task 7: Enforce the complete structural invariant and close
      documentation.**
  - Contract: the structural check enumerates exactly the 48 audited application
    mutation methods and proves each requires the sealed capability; it rejects
    production mutation callers outside `WriteScope`. Administrative lifecycle
    operations remain explicitly excluded. Remove obsolete transaction helpers,
    pool mutation paths, aliases, and duplicated mutation bodies.
  - Cutover: retire #362's interim ast-grep heuristic only after the census is
    clean; make the ADR draft and `docs/ARCHITECTURE.md` projection truthful to
    the final implementation.
  - Verification: both backend integration suites, server/client contract tests,
    structural census, changed endpoint e2e coverage, and the repository ship
    gate pass with no obsolete caller or compatibility path.

## Execution order

Tasks are intentionally sequential. Task 1 supplies safe SQLite drop behavior;
Task 2 fixes the shared capability and outcome contracts; Tasks 3–6 migrate
disjoint verticals against that contract; Task 7 may remove the interim gate
only after all 48 methods and callers have crossed the boundary. Each completed
task enters `jaunder-commit`; no lint suppression or compatibility shim is
authorized.
