# Typed storage query bind seams implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for isolated slices.
> This outline exists because the change establishes a durable storage boundary,
> spans both SQL dialects, and has one shared integration seam.

## Scope

In:

- Add the approved native-sqlx typed bind extension and sealed approval
  registry.
- Replace every governed raw value admission with typed domain or
  persistence-role values.
- Replace the marker-based bind check with the approved fail-closed bypass
  detector.
- Preserve backend parity, dynamic backup/restore behavior, SQL, bind order, and
  transaction ownership.
- Record the decision through `jaunder-adr` and `jaunder-adr-projection`.

Out:

- Schema, migration, SQL-text, decode-policy, transaction, `WriteScope`, or
  `WriteTransaction` redesign.
- Owning sqlx wrappers, blanket `Encode + Type` approval, primitive-family
  sinks, or raw-bind exemptions.
- UI, HTTP, or public runtime behavior changes.

## Task outline

- [x] Task 1: Establish the typed bind seam
  - Contract: `storage/src/sql.rs` owns the sealed approval trait and the
    `bind_storage` implementations for native `Query`, `QueryAs`, and
    `QueryScalar`, plus `push_storage_bind` for native `QueryBuilder` and
    `Separated`. The methods delegate once to sqlx without allocation,
    conversion, clone, storage, or branch. `storage/src/lib.rs` exposes only the
    crate surface needed by storage modules.
  - Contract: approval is explicit and backend-independent; reference,
    `Option<T>`, and required collection forms preserve approval only when their
    leaf type is approved. Existing typed values needed by positive fixtures
    enter the registry here; later tasks add their newly defined types at the
    single integration boundary in Task 5.
  - Verification: positive compile contracts cover representative domain,
    persistence, reference, optional, and PostgreSQL collection values; paired
    negative compile contracts reject primitive leaves. Follow the repository’s
    positive-plus-`compile_fail` rustdoc convention and prove the fixtures
    participate in the storage doctest lane.

- [x] Task 2: Migrate application-semantic user and media surfaces
  - Ownership: `storage/src/{users,helpers,atomic,media,media_manager}.rs`, both
    atomic dialects, both media dialects, and every caller found through LSP
    references. Do not edit `storage/src/sql.rs` or xtask gate files in this
    task.
  - Contract: promote `OperatorStatus` and `EmailVerified` through records,
    traits, helpers, atomic registration, and callers; introduce
    `MediaDeleteMode` through caller, `MediaStorage`, and `MediaDialect`
    surfaces. No boolean aliases or compatibility conversions remain.
  - Contract: the owned user/media bind sites receive the exact domain types,
    including the user corruption fixture role, and delete their now-orphaned
    `sqlx-newtype-bind:allow` markers, but retain the old raw method spelling
    until Task 5.
  - Verification: affected user registration/verification and guarded/forced
    media deletion behavior pass on both backends; exact signatures reject
    transposed role values at compile time.

- [x] Task 3: Type ordinary query roles without changing behavior
  - Ownership: posts and both post dialects, including the advisory-key
    callsites in PostgreSQL media; site/user configuration; subscriptions; feed
    cache; feed events and both feed-event dialects; email, invites, and
    sessions corruption fixtures. This task starts after Task 2 because it
    revisits PostgreSQL media. Do not edit `storage/src/sql.rs`, backup
    dialects, `storage/src/test_support.rs`, or xtask gate files.
  - Contract: implement the approved census roles: `PostPublicationClear`,
    `PermalinkDateText`, `TagSlugPrefixPattern`, `MediaAdvisoryLockKey`,
    `MediaReferenceSnapshotLimit`, direct `InstanceId`/`MediaReferenceKind`,
    stored configuration values, `SubscriptionStatusName`, `StoredFeedBody`,
    `StoredFeedDiagnostic`, `FeedEventAttempts`, and column-specific corruption
    roles in the owned files.
  - Contract: persistence roles are constructed beside their storage query or in
    an exact inner helper. Application/domain inputs and SQL/bind order remain
    unchanged. Owned sites delete their now-orphaned bind markers but retain the
    old raw method spelling until Task 5.
  - Verification: affected post, configuration, subscription, feed-cache, and
    feed-event tests pass with `#[apply(backends)]` where parity applies;
    corruption fixtures still insert the intended invalid column value and still
    exercise decode rejection.

- [x] Task 4: Type administrative restore and test-database roles
  - Ownership: `storage/src/backup/**`, both backup dialects, PostgreSQL
    teardown, `storage/src/test_support.rs`, and `storage/src/postgres/open.rs`.
    Do not edit `storage/src/sql.rs` or xtask gate files.
  - Contract: `RestoreBindValue` is a closed dispatcher over exact
    null/boolean/integer/real/text/JSON roles and reuses `RestoreText`; catalog
    table/database, template-database name/lock, raw media-filename fixture, and
    the PostgreSQL array-bridge seed’s existing `Tag` use their exact census
    types. No generic restore or corrupt-text sink exists.
  - Contract: dynamic restore retains the current schema-driven branch behavior
    and backend-specific representations. Owned sites delete their now-orphaned
    bind markers but retain the old raw method spelling until Task 5.
  - Verification: `server/tests/misc/commands.rs` preserves same-backend round
    trips, negative cases, and archive handling;
    `server/tests/misc/backup_interop.rs` preserves both cross-backend
    directions and the PostgreSQL → SQLite → PostgreSQL → SQLite four-hop cycle.

- [x] Task 5: Close the registry and activate the residual gate
  - Dependency: Tasks 2–4 must be integrated first. This task exclusively owns
    `storage/src/sql.rs`, the repository-wide bind-method rename,
    `xtask/src/steps/sqlx_newtype_bind_check.rs`, and any unchanged gate
    registration in `xtask/src/lib.rs`.
  - Contract: add every census type and all already-typed bind leaves to the
    explicit approval registry, then migrate all `storage/src`
    query/query-as/query-scalar/builder/separated sites to `bind_storage` or
    `push_storage_bind`. Verify that no `sqlx-newtype-bind:allow` marker
    remains.
  - Contract: the detector rejects the complete approved source population: raw
    bind/try-bind/push-bind/unseparated/with-arguments methods and UFCS,
    direct/native arguments and `Arguments::add`, prebuilt-argument
    constructors, imported aliases, sqlx query macros, orphan exemptions, and
    unreadable or unparsable governed input. Only the typed seam’s exact raw
    delegations pass.
  - Verification: inline xtask fixtures prove each allowed and rejected shape;
    the live `sqlx-newtype-bind` host step passes against the whole governed
    tree; focused storage tests from Tasks 2–4 remain green after the mechanical
    cutover.

- [x] Task 6: Record the architecture and run the branch confidence gate
  - Contract: create a tracked numberless proposed ADR with `jaunder-adr`;
    project it into `docs/ARCHITECTURE.md` with `jaunder-adr-projection`;
    correct ADR-0071 and ADR-0085 so the bridge, explicit approval registry,
    QueryBuilder coverage, residual detector, and source-analysis limitations
    agree. Use promotion-safe links.
  - Contract: update `CONTEXT.md` only if the shared application-semantic types
    add vocabulary not already represented; persistence-role names stay out of
    the glossary.
  - Verification: documentation cross-references and generated architecture
    tables remain valid; `cargo xtask validate --no-e2e` supplies the final
    hermetic non-e2e confidence before ship review. Each implementation task
    reaches `jaunder-commit` only after its narrower evidence; the commit hook
    owns that task’s single precommit run.

## Cross-task contracts

- Task 4 may run in parallel with Task 2 after Task 1 because their files and
  contracts are disjoint. Task 3 starts after Task 2 because it owns the
  advisory-key type and exact PostgreSQL media callsite in a file Task 2 also
  changes. None may edit the central approval registry or gate; Task 5 is the
  integration owner for that shared mutation boundary.
- Every production or fixture role named by the approved spec’s closed census
  must have one owner. A newly discovered primitive/stripped value stops that
  slice until the spec census classifies it.
- The extension keeps native sqlx result types and execution/fetch APIs. No task
  may compensate for a type migration by adding a primitive conversion at a
  lower helper.
- Existing SQL text, placeholder order, selected columns, backend-specific array
  capability, and the 48-method structural mutation census are invariants, not
  refactor opportunities.

## Risk checks

- Run LSP references before changing each exported record, trait method, or
  shared domain type; cleanly migrate every caller and remove obsolete primitive
  paths.
- Prove the approval registry rejects primitive leaves without claiming that a
  generic approved type identifies a SQL placeholder; exact helper and trait
  signatures carry wrong-role safety.
- Keep sqlx query macros forbidden under `storage/src`; the source gate cannot
  inspect their generated `Arguments::add` calls.
- Preserve PostgreSQL-only `PgHasArrayType` behavior without exposing it to
  SQLite.
- Preserve same-backend and cross-backend backup bytes/values, including null,
  boolean, integer, real, text, JSON object/array, and four-hop
  interoperability.
- Add no lint suppression, coverage exclusion, compatibility alias, deprecated
  path, raw-bind marker, or administrative/test escape.
- Use no `Co-Authored-By` trailer.
