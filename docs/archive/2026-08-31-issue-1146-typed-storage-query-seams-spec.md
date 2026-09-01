# Typed storage query bind seams

## Outcome

Storage query construction accepts only approved domain values or explicit
persistence-role values at compile time. Raw sqlx bind methods become a
structurally detected bypass, so stripping a domain value to a primitive before
or across a helper boundary is not a valid normal storage path.

This change composes with the `WriteScope` and `WriteTransaction` architecture
landed by #1280. It changes value admission to queries, not transaction
ownership, SQL behavior, schemas, or public runtime behavior.

## Load-bearing decisions

- Govern every sqlx value-admission door reachable from `storage/src`, not only
  the doors currently used: `Query`, `QueryAs`, and `QueryScalar` `.bind(...)`;
  `QueryBuilder` and `Separated` `.push_bind(...)`; and the bypass APIs named
  below.
- Keep native sqlx query and builder types. Do not introduce owning query
  wrappers or forward their execution/fetch interfaces.
- Add typed extension methods for the normal path: `bind_storage(...)` for query
  values and `push_storage_bind(...)` for query builders.
- The extension methods delegate directly to sqlx and add no allocation,
  conversion, clone, query storage, or runtime branch.
- A sealed, storage-owned approval trait determines which values may cross those
  methods. Its approved set is explicit and fails closed.
- The approval trait is independent of a database backend. Existing sqlx
  `Encode` and `Type` bounds still decide whether an approved value is
  representable for a particular database and lifetime.
- The registry explicitly approves repository domain types and persistence-role
  types. It does not approve primitive or foreign representation families merely
  because sqlx can encode them.
- Controlled generic approval may preserve an approved value through references,
  `Option<T>`, and collection forms required by existing queries. It must not
  turn an unapproved leaf value into an approved bind.
- Values with application semantics use shared domain types through caller
  inputs, storage trait signatures, records, helpers, and binds. Examples
  include role/status/verification facts that are currently flattened to
  booleans.
- Once a shared domain type is introduced, apply ADR-0063 consistently: do not
  retain primitive aliases, parallel fields, compatibility conversions, or
  flattened repository-owned surfaces.
- Values that exist only because of persistence representation remain
  storage-owned role types. Examples include catalog identifiers, serialized
  aggregates, stored payloads, backup row JSON, and restore values.
- Persistence-only role types are created inside the storage seam from
  application/domain inputs and remain typed through every inner helper and
  bind. They do not leak into unrelated application interfaces.
- Reuse existing role types such as `RowCount`, `Exists`, stored feed values,
  stored configuration values, catalog values, and backup values when their role
  exactly matches the bound column.
- Add a new role type only for a distinct legitimate sink that remains primitive
  today. Do not add generic `StorageText`, `StorageBool`, `StorageI64`,
  timestamp, JSON, or byte wrappers.
- Role-specific types prevent arbitrary same-representation substitution at Rust
  helper and storage-trait boundaries. The generic bind seam proves repository
  approval, not correspondence between a value and a particular SQL placeholder;
  exact helper signatures and query review retain that responsibility.
- Storage helper and trait signatures that eventually bind a value carry its
  exact domain or persistence-role type. A helper must not accept a stripped
  primitive and re-admit it at the query site.
- Administrative backup and restore use explicit catalog/restore role types,
  including a closed representation for runtime JSON-derived restore values.
  They receive no raw-bind escape merely because ADR-0164 excludes them from the
  application mutation census.
- PostgreSQL-only array binds remain dialect-specific and use the existing
  `PgHasArrayType` capability. The typed bind seam does not invent a SQLite
  array abstraction.
- `WriteScope`, `WriteTransaction`, backend connection ownership, commit
  outcomes, and the 48-method mutation census remain unchanged.
- Replace the current `sqlx-newtype-bind` responsibility with a source-level
  raw-admission detector. Its governed syntax includes method calls named
  `bind`, `try_bind`, `push_bind`, `push_bind_unseparated`, or `with_arguments`;
  UFCS forms of those sqlx methods; `Arguments::add`;
  `Arguments`/`IntoArguments` implementations or direct native argument
  construction; and `query_with`, `query_as_with`, `query_scalar_with`,
  `__query_with_result`, and `__query_scalar_with_result`, including imported
  aliases.
- Forbid sqlx query macros under `storage/src`: sqlx 0.8.6 expands them through
  native `Arguments::add` and hidden result constructors outside the source AST.
  No such macro is present in the governed tree at the time of this decision.
- `bind_storage(...)` on `Query`, `QueryAs`, and `QueryScalar`, and
  `push_storage_bind(...)` on `QueryBuilder` and `Separated`, are the only
  normal value-admission APIs. `push_bind_unseparated` and all prebuilt-argument
  constructors remain forbidden because the current storage population does not
  need typed equivalents.
- The detector allows raw sqlx admission only at the typed extension
  implementation's exact delegation sites. It has no primitive-bind marker
  allowlist or module, region, administration, dialect, or test escape. Existing
  `sqlx-newtype-bind:allow` markers are removed with their raw binds.
- The detector parses Rust syntax, fails if governed input cannot be parsed,
  covers inline tests and test-support code under its root, and documents its
  exact population and limits per ADR-0085. It is deliberately source-level: it
  recognizes governed method/path/import shapes and local aliases, but does not
  claim rustc type resolution or visibility into arbitrary proc-macro expansion.
- Ordinary raw sqlx query constructors remain available because they admit SQL
  text, not values. Constructors that accept prebuilt arguments do not.
- Record this durable query-seam decision as a tracked proposed ADR with its
  `docs/ARCHITECTURE.md` projection. Correct ADR-0071 and ADR-0085's stale
  descriptions of the bind gate and cite the new decision using promotion-safe
  links.
- Consider `CONTEXT.md` for any shared domain concepts introduced by the
  migration; persistence-only role names do not belong in the glossary.

## Closed bind-role census

The following census is exhaustive for primitive or representation-stripped bind
values under `storage/src` at spec time. All already-typed IDs, slugs, tags,
media values, timestamps, limits, offsets, stored payloads, references, options,
and PostgreSQL arrays keep their current types and receive only the mechanical
method rename.

| Current role and locations                                                                                                                                                                                          | Owner and resulting type                                                                                                                                                                                                        | Required surface migration                                                                                                                             |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| User operator bit (`users.rs`)                                                                                                                                                                                      | shared domain `OperatorStatus`, promoted from the existing storage decode role                                                                                                                                                  | Replace booleans in user records, user/atomic storage traits, helpers, and callers.                                                                    |
| Email verification bit (`users.rs`)                                                                                                                                                                                 | shared domain `EmailVerified`, promoted from the existing storage decode role                                                                                                                                                   | Replace booleans in records, storage traits, helpers, and callers.                                                                                     |
| Forced media deletion (`media.rs`, both dialect builders)                                                                                                                                                           | shared domain `MediaDeleteMode`                                                                                                                                                                                                 | Replace `force: bool` through the caller, `MediaStorage`, `MediaDialect`, and bind.                                                                    |
| Update-post publication-clear bit (both post dialects)                                                                                                                                                              | storage-owned `PostPublicationClear`                                                                                                                                                                                            | Derive from `PublishUpdate` inside each dialect and keep the persistence bit internal.                                                                 |
| Permalink date comparison text and tag-slug prefix pattern (`posts.rs`)                                                                                                                                             | storage-owned `PermalinkDateText` and `TagSlugPrefixPattern`                                                                                                                                                                    | Construct beside their queries; helpers accept the exact role.                                                                                         |
| Media advisory-lock integer (`posts.rs`, PostgreSQL media/posts)                                                                                                                                                    | storage-owned `MediaAdvisoryLockKey`                                                                                                                                                                                            | Make the media key functions return the role type and carry it through media lock helpers.                                                             |
| Bounded media-reference snapshot limit (`posts.rs`)                                                                                                                                                                 | storage-owned `MediaReferenceSnapshotLimit`                                                                                                                                                                                     | Type the sentinel-bearing query limit constant; its public `usize` truncation limit remains unchanged.                                                 |
| Generic site/user configuration strings (`site_config.rs`, `user_config.rs`)                                                                                                                                        | existing `StoredSiteConfigValue` and `StoredUserConfigValue`                                                                                                                                                                    | Construct within their storage methods; inner setters no longer accept `&str` values.                                                                  |
| Subscription status lookup token (`subscriptions.rs`)                                                                                                                                                               | storage-owned `SubscriptionStatusName`                                                                                                                                                                                          | Convert from `SubscriptionStatus` at the lookup seam; bind only the lookup role.                                                                       |
| Feed failure diagnostics and retry attempts (both dialects and lifecycle fixtures)                                                                                                                                  | existing `StoredFeedDiagnostic` and `FeedEventAttempts`                                                                                                                                                                         | Production and fixture helpers construct the existing roles before binding.                                                                            |
| Rendered syndication-feed body (`feed_cache.rs`)                                                                                                                                                                    | existing `StoredFeedBody`                                                                                                                                                                                                       | Convert the closed representation's body inside `FeedCacheStore::upsert`; do not carry `&str` to the bind.                                             |
| Backup catalog table/database names (PostgreSQL backup/teardown)                                                                                                                                                    | existing `CatalogTableName` plus new storage-owned `CatalogDatabaseName`                                                                                                                                                        | Catalog and teardown helpers accept their exact identifier roles.                                                                                      |
| Runtime PostgreSQL/SQLite restore cells (both backup dialects)                                                                                                                                                      | closed storage-owned `RestoreBindValue`, dispatching to `RestoreText` and exact null/boolean/integer/real/JSON role values                                                                                                      | The dynamic restore binder owns the exhaustive match; no branch binds a primitive.                                                                     |
| PostgreSQL template-database lookup name and creation lock (`test_support.rs`)                                                                                                                                      | test-only `TemplateDatabaseName` and `TemplateDatabaseLockKey`                                                                                                                                                                  | Type both constants and their exact catalog/advisory-lock binds; do not reuse production catalog or media-lock roles.                                  |
| PostgreSQL array-bridge tag seed (`postgres/open.rs`)                                                                                                                                                               | existing `Tag`                                                                                                                                                                                                                  | Parse the fixture seed as `Tag`; no raw seed string remains.                                                                                           |
| Deliberately corrupt email, invite code, session hash, username, tag slug, post slug, post format, and media filename fixtures (`email.rs`, `invites.rs`, `sessions.rs`, `users.rs`, `posts.rs`, `test_support.rs`) | test-only storage roles named for the target column: `CorruptEmailAddress`, `CorruptInviteCode`, `CorruptSessionTokenHash`, `CorruptUsername`, `CorruptTagSlug`, `CorruptPostSlug`, `CorruptPostFormat`, and `RawMediaFilename` | Fixture helpers construct only their column-specific corruption role; these types are approved leaves, not a primitive or generic corrupt-text escape. |

Any newly discovered primitive or stripped representation is a spec/census
defect: classify it explicitly as shared-domain or persistence-owned before
implementation proceeds.

## Acceptance

- Every sqlx bind under `storage/src` uses `bind_storage` or
  `push_storage_bind`; the only raw bind calls are the typed seam's exact
  delegation sites.
- Query, query-as, query-scalar, direct query-builder, and separated-builder
  call shapes all retain their current SQL, execution/fetch behavior, output
  typing, and ownership.
- Existing domain values bind directly without flattening to strings, integers,
  booleans, timestamps, JSON, bytes, or other foreign representations.
- Every currently legitimate primitive bind is replaced by an existing or new
  role-specific domain/persistence type; no generic primitive-family sink is
  introduced.
- Storage trait and record surfaces use shared domain types for
  application-semantic values introduced by this work, with all callers migrated
  cleanly.
- Persistence-only representations are constructed within storage and carried
  through typed internal helper signatures to the bind seam.
- A helper parameter cannot launder a stripped domain value: primitive
  parameters and locals are not accepted by the typed bind methods, and exact
  role signatures prevent cross-role helper calls.
- Dynamic PostgreSQL and SQLite backup/restore paths bind through explicit
  catalog/restore types while preserving their schema-driven runtime behavior.
- PostgreSQL array binds remain typed and working; SQLite code gains no
  array-only dependency or capability.
- Compile-time contract coverage proves representative approved domain,
  persistence, reference, optional, and collection values compile at the seam.
- Negative compile-time coverage proves representative `String`, integer,
  boolean, timestamp, JSON, and byte values do not satisfy the approval trait.
  Exact helper/storage-trait signature tests or ordinary compiler checks cover
  wrong-role substitution; the generic approval trait does not claim
  SQL-placeholder identity.
- Gate fixtures prove raw `.bind`, `.try_bind`, `.push_bind`,
  `.push_bind_unseparated`, prebuilt-argument APIs, direct argument admission,
  aliases, and sqlx query macros fail; the seam's exact delegations pass;
  orphan/invalid exemptions cannot appear; and unreadable governed code fails
  closed.
- The old primitive-expression classifications and `sqlx-newtype-bind:allow`
  markers are deleted rather than retained beside the new invariant.
- Affected ordinary queries retain focused dual-backend storage coverage.
  Backup/restore behavior remains covered in `server/tests/misc/commands.rs` for
  same-backend round trips, negatives, and archive handling, and in
  `server/tests/misc/backup_interop.rs` for both single-hop cross-backend
  directions plus the PostgreSQL → SQLite → PostgreSQL → SQLite four-hop cycle.
- Static checks and the full host test surface pass without lint suppressions or
  coverage exclusions added to avoid the migration.
- ADR-0071, ADR-0085, the new proposed ADR, and `docs/ARCHITECTURE.md` agree on
  typed bind ownership, residual gate responsibility, QueryBuilder coverage, and
  stated limitations.

## Boundaries

- No schema, migration, SQL statement, selected column, bind order, transaction
  boundary, isolation behavior, or mutation outcome change.
- No redesign of `WriteScope`, `WriteTransaction`, `Backend`, dialect traits,
  storage trait ownership, or dependency injection.
- No owning wrapper around sqlx query objects and no repository replacement for
  sqlx execution/fetch interfaces.
- No blanket approval of all `Encode + Type` values, foreign primitives, or
  representation types.
- No raw-bind allowlist, source marker, administrative escape, dialect escape,
  test escape, or region-scoped exemption.
- No requirement to type SQL literals that are part of SQL text rather than
  bound values.
- No query-result/decode refactor except clean-cutover migration required when
  this work introduces a shared domain type used on both sides of storage.
- No user-visible API, HTTP, UI, or error-policy change.
- No attempt to infer SQL column meaning from query text; approval comes from
  Rust value types, and the residual gate detects bypass syntax rather than
  column semantics.
