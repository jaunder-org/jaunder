# Architecture

This document is the **materialized view** of the repository's architectural
decision log: the single authoritative statement of the architecture as it is
_now_, folded from the ADRs in [docs/adr/](adr/) (see
[the materialized-view ADR](adr/0127-architecture-view-materialized-from-adrs.md)).
The ADRs are the immutable events — each records why a decision was made, pinned
to its moment; this view records what is currently true, and every claim cites
the decision(s) that established it. Read this to learn the system; open a cited
ADR only when you need the _why_.

Two conventions keep it honest:

- **Citations.** A claim with no ADR citation is un-ADR'd reality — accurate,
  but awaiting a recorded decision or a correction.
- **Current vs. committed.** Sections describe built reality; decisions that are
  made but not yet realized appear under **Committed direction** subheadings,
  never mixed into the present tense.

This file is updated in the feature change that commits a new ADR draft; the
post-merge promoter later rewrites its draft citation to the accepted numbered
path. It is also periodically re-derived from the full log to catch drift.
Process — how to build, verify, and land work — lives in
[CONTRIBUTING.md](../CONTRIBUTING.md); the domain glossary lives in
[CONTEXT.md](../CONTEXT.md).

## Workspace

Jaunder is a full-stack Rust application: an [Axum] server with a
client-side-rendered [Leptos] frontend
([ADR-0002](adr/0002-frontend-framework.md),
[ADR-0040](adr/0040-web-rendering-leptos-csr.md)), deployed as a single binary
([ADR-0008](adr/0008-deployment-model.md)) over a pluggable SQLite/PostgreSQL
storage layer ([ADR-0001](adr/0001-storage-backends.md)), with an Emacs blogging
client and an AtomPub API as first-class publishing surfaces.

Shared code is split by compile target and target reachability, not convenience:
`common` is the dual-target domain crate, `host` is its strictly host-focused
sibling, and `client` is the browser-infrastructure peer. `host` never compiles
to wasm, so it uses host facilities without the `#[cfg]` gating `common` would
demand ([ADR-0058](adr/0058-host-crate-layering.md),
[common/host target-reachability closure](adr/0159-common-host-target-closure.md)).
`client` holds raw browser glue (`web_sys`/`js_sys`/`wasm_bindgen` and wasm-side
Leptos plumbing), plus two host-testable browser contracts; it contains no
application domain types. `web` and `csr` depend on `client`, never the reverse
([ADR-0069](adr/0069-client-crate-wasm-only-home.md)). Proc-macros live apart
from all three, in `macros`
([ADR-0062](adr/0062-macros-crate-proc-macro-home.md)).

| Crate          | Target      | Responsibility                                                                                                                                                                                                                                                                                                                                                                                                                     |
| -------------- | ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `common`       | host + wasm | Dual-target domain types and operations reached by CSR or another dual-target consumer: validated newtypes including `ProfferedPassword`, `RenderedHtml`, `PostFormat`, ETag, Org normalization, croner, `BackupSchedule`, and the Syndication Feed grammar (`FeedFormat`, `FeedSurface`, `canonicalize`); its optional host-only `sanitize` capability establishes the `RenderedHtml` invariant without entering the CSR closure. |
| `storage`      | host        | Storage traits, record types, SQL migrations, and the SQLite/PostgreSQL backends ([ADR-0019](adr/0019-generic-storage-backend-via-dialect.md)).                                                                                                                                                                                                                                                                                    |
| `server`       | host        | The `jaunder` binary: Axum router, CLI, background workers, integration tests.                                                                                                                                                                                                                                                                                                                                                     |
| `web`          | host + wasm | Leptos components and `#[server]` functions — the UI and its server halves, split host/wasm at the file level ([ADR-0070](adr/0070-web-vertical-wasm-only-component-files.md)).                                                                                                                                                                                                                                                    |
| `csr`          | wasm        | The client-side-rendering entry point: mounts `web` in the browser ([ADR-0041](adr/0041-public-projector-and-csr-client.md)).                                                                                                                                                                                                                                                                                                      |
| `host`         | host        | Strictly-host-focused shared code: error carrier, capture dir, auth/token parsing, `Password`/`StoredPasswordHash` and hash operations, rendering/`RenderOutput`/media extraction/ETag construction, AtomPub wholesale, host-only Syndication Feed machinery, `SiteConfigKey`/`UserConfigKey`, invites, process telemetry, metrics, and SMTP relay configuration.                                                                  |
| `client`       | host + wasm | Browser infrastructure: `localStorage`, dialogs, DOM/file-upload glue, reactive revalidation, CSR performance marks, and bounded client telemetry ([ADR-0069](adr/0069-client-crate-wasm-only-home.md)).                                                                                                                                                                                                                           |
| `macros`       | build-time  | The workspace's proc-macro home: newtype, `text_enum`, sqlx-bridge and server-fn derives ([ADR-0062](adr/0062-macros-crate-proc-macro-home.md)).                                                                                                                                                                                                                                                                                   |
| `test-support` | host        | A seed binary linking `storage` for out-of-process e2e seeding ([ADR-0046](adr/0046-test-support-seed-binary.md)).                                                                                                                                                                                                                                                                                                                 |

Every `client` module that touches the browser carries
`#[cfg(target_arch = "wasm32")]`, so a host build of the crate is an
all-but-empty rlib with no coverage-measured browser glue. Two
transport-independent contracts compile and are tested on the host:
`client::perf` owns its mark-name table, while `client::telemetry` owns the
one-flight reporter over injected console and transport seams. The reporter may
consume only the closed, data-free `common::client_telemetry` wire types; the
crate remains free of application-domain DTOs, newtypes, and enums. Irreducible
browser primitives run separately in headless Chromium through the Linux-only
`wasm-tests` Nix check, while user-flow coverage remains in Playwright e2e.
`wasm-tests` is behavioral pass/fail execution and does not add wasm lines to
the host coverage denominator
([ADR-0069](adr/0069-client-crate-wasm-only-home.md)).

Two sibling trees are outside the root workspace, each its own cargo workspace:
`xtask/` (the host-only dev/CI driver, also named in the root
`exclude = ["xtask"]`) and `tools/` (members `devtool`, `coverage`, `doctests`).
Those boundaries are execution/ownership boundaries, not a claim that every
`tools/` crate is absent from every Nix derivation
([Cargo workspace execution boundaries](adr/0141-cargo-workspace-execution-boundaries.md)).
`elisp/` (the Emacs client,
[ADR-0031](adr/0031-elisp-separately-tested-subproject.md)) and `end2end/`
(Playwright) are covered in their sections.

**Package-metadata ownership is deliberately partial.** The root
`[workspace.package]` owns the version, edition, and license inherited by its
nine members (`client`, `common`, `csr`, `host`, `macros`, `server`, `storage`,
`test-support`, and `web`). The independent `tools/` workspace owns its version,
edition, and non-publish setting for its three members (`coverage`, `devtool`,
and `doctests`), but has no workspace license. `xtask/` is a standalone
single-package workspace, so its package metadata remains direct. Likewise,
`test-support` keeps its release exception — direct `publish = false` — rather
than inheriting the root's publish policy. These ownership points follow the
three workspace roots; they are not one repository-wide metadata workspace.

Across every one of those trees, a `mod.rs` states its module's surface and
holds nothing else: `mod`/`pub mod` declarations, `use`/`pub use` re-exports,
`//!` documentation, and attributes — never a `fn`, type, `impl`, `const`,
`macro_rules!`, or an inline test module
([mod.rs assembles the module surface](adr/0128-mod-rs-assembles-module-surface.md)).
Code lives in a sibling file the `mod.rs` declares and re-exports, by an
explicit list rather than a glob, so the file a reader opens first is a map
rather than a haystack. The rule is workspace-wide and deliberately **not**
machine-enforced: whether an item earns its own file is a cohesion judgement a
syntactic check would get wrong in both directions, so it is carried by review.
It generalises the per-vertical rule
[ADR-0070](adr/0070-web-vertical-wasm-only-component-files.md) established for
`web/`, which is unchanged.

[Axum]: https://github.com/tokio-rs/axum
[Leptos]: https://leptos.dev/

## Storage

Jaunder supports two pluggable database backends — SQLite (the zero-config
default) and PostgreSQL (for heavier deployments) — behind object-safe storage
traits; application code sees only the traits, never a concrete backend
([ADR-0001](adr/0001-storage-backends.md)). The backend is selected at runtime
by URL scheme: `DbConnectOptions` (`storage/src/db.rs`) parses `sqlite:` vs
`postgres://` and `open_database`/`open_existing_database` dispatch accordingly.
Each backend has its own migration tree under
`storage/migrations/{sqlite,postgres}`; the two trees carry identical numbered
filenames (currently `0001`–`0025`), and maintaining that parity — same
migrations, same behavior — is the accepted cost of the pluggable strategy
([ADR-0001](adr/0001-storage-backends.md)).

### Crate layout and the generic store pattern

The `storage` crate is organized by domain: each trait-home root module
(`users.rs`, `posts.rs`, `sessions.rs`, `invites.rs`, `media.rs`,
`subscriptions.rs`, `audiences.rs`, `site_config.rs`, `user_config.rs`,
`email.rs`, `password.rs`, `feed_cache.rs`, `feed_events.rs`) holds one
object-safe `XStorage` trait plus its record and input structs (e.g.
`PostRecord`, `PostCursor`). The crate also hosts orchestration that is
persistence work rather than a trait — `post_service.rs` (post create/update
over `PostStorage`, shared by the web and AtomPub front-ends) and
`media_manager.rs` (content-addressed upload, relocated from `server` in #517).
The trait bodies are implemented once by a generic `XStore<DB>` bounded on
public `Backend: sqlx::Database` (`storage/src/backend.rs`, implemented for
`Sqlite` and `Postgres`). `Backend` carries the `db.system` span constant and
adapts sealed `WriteTransaction` capability to the concrete connection.
Crate-private `AppStateBackend: Backend`, implemented only for those two
backends, converts a pool into a backend-erased `WriteScope` exclusively for
generic `AppState` composition. Downstream code can run the factory-minted scope
but cannot name that trait or construct one from a pool, preserving ADR-0164's
downstream-construction invariant without changing ADR-0019's public marker
surface. Backend-specific SQL is isolated in per-trait `XDialect` impls under
`storage/src/{sqlite,postgres}/*.rs`. Traits with no divergence need no dialect
at all. Neither `Backend` nor `AppStateBackend` carries sqlx bind/executor
bounds — each store impl restates exactly the subset it uses
([ADR-0019](adr/0019-generic-storage-backend-via-dialect.md)). Span names are
backend-agnostic (`storage.posts.*`) with `db.system` distinguishing the
backend. Pure-SQL helpers shared by both dialects live in `storage/src/sql.rs`
and `storage/src/helpers.rs`.

Backup is the deliberate exception to the dedup: `storage/src/sqlite/backup.rs`
and `storage/src/postgres/backup.rs` are kept as separate implementations
because dump/restore is fundamentally backend-specific and a shared store would
be a thin shell over a near-total dialect
([ADR-0019](adr/0019-generic-storage-backend-via-dialect.md)).

### Dependency injection and AppState

`storage::AppState` (`storage/src/app_state.rs`) is a bundle of thirteen
`Arc<dyn *Storage>` handles and the factory-minted, sealed `WriteScope`. One
generic `make_app_state<DB>(Pool<DB>)` builds it for both production backends;
its crate-private `AppStateBackend` bound converts the pool into the
backend-erased scope. It holds storage dependencies only; services (mailer,
WebSub client, background workers, and the media manager) are constructed in
`server` and injected per-consumer as constructor parameters, and there is no
services bundle. The durable invariant: no type may be both a heterogeneous
dependency holder and passed beyond the composition root
([ADR-0016](adr/0016-dependency-injection-and-appstate.md)).

The web layer takes most dependencies per-trait via Leptos context and receives
`WriteScope` and `MediaContentLocks` as separate context values.
`server::provide_app_state_contexts` (`server/src/context.rs:27`) publishes
twelve of the handles (all but `feed_cache`, which no `#[server]` fn needs) plus
the separately injected scope. Each ordinary server fn fetches exactly what it
uses—`expect_context::<Arc<dyn UserStorage>>()`,
`expect_context::<WriteScope>()`, or
`expect_context::<Arc<MediaContentLocks>>()`. The helper lives in `server`, not
`storage`, because using Leptos context as the DI mechanism is an
application-wiring decision
([ADR-0016](adr/0016-dependency-injection-and-appstate.md)).

Media operations use one deeper seam. The router composition root constructs a
single `Arc<MediaManager>` from explicit media, Post, site-configuration, write
scope, content-lock, instance-identity, and ownership-resolver dependencies,
then injects that same manager independently into Axum extensions and Leptos
context. AtomPub and web upload/delete entry points supply only transport policy
and input. For deletion, the manager loads one bounded global reference
snapshot, resolves ownership before acquiring the content lock, and carries the
same immutable evidence through guarded deletion, file reclamation, and
owner-reference reporting
([ADR-0016](adr/0016-dependency-injection-and-appstate.md),
[ADR-0154](adr/0154-media-reference-live-ownership.md)).

Nothing in the codebase now pins reactive-owner lifetime for this:
`server_boundary` (`web/src/error/server.rs:99`) is a thin error-projection
wrapper that awaits the body and maps `InternalError → WebError`. The
owner-pinning machinery was dismantled in two steps: `server_resource` went in
#515, then `owner_ancestry_strong` and the `owner_lifetime` tests in #594.
Dropping component SSR left only one server-fn invocation path —
`leptos_axum::handle_server_fns_with_context` on `POST /api/…`, which holds a
parentless root owner strong for the whole future by itself. The ADR-0016
#89/#124/#138 addenda that described that pinning are explicitly marked
superseded-and-historical inside the ADR
([ADR-0016](adr/0016-dependency-injection-and-appstate.md)).

The two cross-store account mutations are functions owned by `storage`, not an
injected operation holder. Registration performs a read-only invite precheck,
creates the user, then conditionally claims the invite with that user's ID; a
concurrent loser rolls its inserted user back. Password reset composes
reset-token claiming, password replacement, and whole-user session revocation.
Each receives the caller's mutable `WriteTransaction` plus only the exact
object-safe storage traits it uses. Invalid reset capabilities use their typed
client-validation mapping
([account mutations compose storage primitives](adr/0166-account-mutations-compose-storage-primitives.md)).

### Query and transaction discipline

- **Cursor pagination.** Timeline and collection listings paginate by keyset
  cursor, never offset: `PostCursor`/`CollectionCursor` (`storage/src/posts.rs`)
  round-trip through an opaque wire pair, giving fixed-cost queries that are
  stable under concurrent inserts ([ADR-0004](adr/0004-pagination-strategy.md)).
- **SQLite transactions.** SQLite dialect code avoids read-then-write deferred
  transactions (the shared→reserved lock upgrade that yields unretryable
  `SQLITE_BUSY` under WAL concurrency). Audited application mutations receive
  the transaction minted by `WriteScope`; its SQLite adapter opens
  `BEGIN IMMEDIATE`, while PostgreSQL reaches the required serialization with
  its operation-specific row locks, including `SELECT … FOR UPDATE`
  ([ADR-0021](adr/0021-sqlite-transaction-discipline.md)).
- **Bounded write-lock occupancy.** SQLite has one write lock and `busy_timeout`
  polls rather than queues, so churn — not hold length — is what starves a
  writer. Two further rules therefore hold on any path SQLite can execute: no
  per-row write loops (a fan-out issues **one** batched storage call), and no
  CPU-heavy or foreign-I/O work between a write transaction's first write and
  its commit. `FeedEventStorage::enqueue_many`
  (`storage/src/feed_events.rs:260`) is the reference implementation — one
  write-first transaction around the single-row INSERT — and the feed worker
  calls it in `ENQUEUE_CHUNK`-bounded batches (`server/src/feed/worker.rs:108`)
  so batch size is capped by construction
  ([ADR-0092](adr/0092-sqlite-bounded-write-lock-occupancy.md)). ADR-0022's
  Argon2-inside-the-claim-window is the one documented exception.
- **Slug-ordered tag locks.** A transaction that will touch several `tags` rows
  sorts them by slug before acquiring any lock, so every transaction takes the
  row locks in one global order and concurrent `set_post_tags` reconciles cannot
  deadlock on Postgres (`storage/src/posts.rs:397`). The sort is `sort_by_key`,
  not `sort_unstable_by_key`, because a desired set may carry two labels sharing
  a slug and the first occurrence's casing must win. SQLite is unaffected
  (`BEGIN IMMEDIATE` locks database-wide); the rule is shared so the backends
  stay identical ([ADR-0125](adr/0125-slug-ordered-tag-lock-acquisition.md)).
- **Blocking in SQLite's busy handler is thread-scoped.** sqlx-sqlite runs each
  connection on its own OS thread, so a call parked in the busy handler blocks
  that thread, not the async runtime. Lock-contention tests therefore stay on
  the current-thread flavor `#[tokio::test]` defaults to
  (`storage/src/posts.rs:2903`); moving sqlx to an in-runtime SQLite driver
  would turn those blocks into hangs
  ([ADR-0126](adr/0126-sqlx-sqlite-busy-handler-threading.md)).
- **Cost ordering.** When an operation is gated on a high-entropy secret (invite
  code, reset token), the secret is validated with a cheap lookup _before_
  expensive work (Argon2 hashing); enumerable identifiers (usernames) get the
  opposite, timing-equalized treatment
  ([ADR-0022](adr/0022-validate-before-expensive-work.md)).

### Backup and restore

`storage::export_backup`/`restore_backup`
(`storage/src/backup/orchestration.rs`) implement a portable dump: a
`manifest.json` plus one NDJSON file per table under `db/`, together with the
media tree, written either as a directory or as a gzipped tar archive built
in-process with the `tar` and `flate2` crates. The backed-up table set is
auto-derived from the live schema — every table minus the explicit
`TABLES_EXCLUDED_FROM_BACKUP` denylist (`_sqlx_migrations`, `feed_cache`;
`storage/src/backup/format.rs`) and SQLite-internal tables, sorted for a
reproducible manifest — so a migration that adds a table needs no backup code
change; server contract tests pin the exact set. Consequently the complete
`post_revisions` scalar rows, their immutable
`post_revision_tags`/`post_revision_audiences` children, and revision-qualified
`post_media` rows travel with every whole-store backup, without a
revision-specific export path; typed restore validation covers their domain
fields ([ADR-0136](adr/0136-local-post-lifecycle.md),
[ADR-0064](adr/0064-backup-target-auto-derivation.md)).

Restore is authoritative and order-independent: both backends clear every target
table in a first pass, then load all rows in a second, with FK enforcement
suspended for the load — Postgres FKs are `DEFERRABLE` (migration
`0024_defer_foreign_keys`) and restore issues `SET CONSTRAINTS ALL DEFERRED`;
SQLite imports under `PRAGMA foreign_keys = OFF` and runs `foreign_key_check`
before `COMMIT`. Clearing every table before loading any is what makes the
alphabetically-sorted manifest safe: `SET CONSTRAINTS ALL DEFERRED` defers FK
_checks_ but not `ON DELETE CASCADE` _actions_, so a per-table delete-then-load
could cascade away rows already loaded for an earlier table. SQLite cannot
cascade with FKs off, but keeps the split anyway so the two restore shapes stay
identical ([ADR-0115](adr/0115-clear-then-load-restore.md)). Restore refuses any
target that is not empty (every table except the migration-seeded lookups;
`storage::database_is_empty`, enforced by `ensure_restore_target_empty` in
`server/src/commands.rs`) — there is no force-overwrite mode
([ADR-0064](adr/0064-backup-target-auto-derivation.md)). Failure is
backend-uniform: a constraint-violating restore returns
`BackupError::ConstraintViolation` and leaves the target unmodified on both
backends
([ADR-0054](adr/0054-backup-test-homing-and-uniform-restore-failure.md)).
Cross-backend interop is value-level — dumps restore across backends with values
preserved to Postgres's microsecond timestamp resolution, but dump _bytes_ are
not canonicalized across backends
([ADR-0054](adr/0054-backup-test-homing-and-uniform-restore-failure.md)).

### Idempotent post creation

Post creation accepts an optional client-supplied
[`IdempotencyKey`](adr/0063-domain-value-newtype-convention.md), so a retried
AtomPub POST does not create a duplicate post — the mechanism decided in issue
[#79](https://github.com/jaunder-org/jaunder/issues/79) as a follow-on to
ADR-0047. At the AtomPub boundary, a missing header, a value rejected by
`HeaderValue::to_str` (including non-ASCII UTF-8 bytes and invalid UTF-8), or
text that is blank after trimming means no key rather than a `400`. A readable,
non-blank value is parsed once into an owned `IdempotencyKey`; typed borrowed
keys carry it through post creation and the owned type is bound for persistence.

The existing `idempotency_keys` table stores the key as `TEXT NOT NULL` and
enforces `UNIQUE(user_id, key)`. Storage serializes each `(user_id, key)` pair
inside the create transaction (`SQLite`'s writer lock; a `PostgreSQL` advisory
lock plus row lock) and applies the request's authoritative cutoff there. A live
mapping returns its selected `PostId` as the replay decision without attempting
new post or feed-event writes; the AtomPub handler fetches that fixed Post
rather than looking the mapping up again after rollback. An expired mapping is
removed and its replacement post and key row are written atomically. Fresh
creation returns `201`; when its original post remains available, same-user key
reuse returns that original post as `200`, even when the new payload differs.
Another user may use the same key independently. The
[bounded transient-data retention decision](adr/0167-bounded-transient-data-retention.md)
replaces indefinite mapping retention with a one-hour semantic replay window: at
`cutoff <= now`, the mapping no longer coordinates a replay, whether or not a
later cleanup pass has physically removed it.

### Testing (summary)

Storage tests are homed by what they prove: backend-common tests run on both
backends via `#[apply(backends)]` and live in the generic home module beside the
`XStore<DB>` they exercise; a single-backend test is presumed a Postgres
coverage gap unless it has a decisive backend-exclusive reason
([ADR-0053](adr/0053-storage-test-homing-and-dual-backend.md)). Backup is tested
as a cross-backend contract at the CLI/server-test level (`server/tests/misc/`),
not in the storage crate
([ADR-0054](adr/0054-backup-test-homing-and-uniform-restore-failure.md)).
Details in the testing section.

### Committed direction

- **Tiered storage isolation.** A shared ingestion layer (raw fetched content,
  feed metadata, actor caches) feeding per-user private content copies — every
  user-layer table carrying `user_id`, queries never crossing user boundaries —
  is the decided architecture for multi-user feed following
  ([ADR-0006](adr/0006-storage-isolation.md)). **Neither tier exists.** Jaunder
  today is a publisher, not a reader: `feed_cache` stores jaunder's _own_
  rendered feed bodies keyed by feed path (`storage/src/feed_cache.rs`, no
  `user_id`), and `subscriptions` records who follows a local author's channel
  (`storage/migrations/sqlite/0019_create_subscriptions.sql`) — outbound WebSub
  delivery, not inbound ingestion. There is no table of fetched external items
  and no fan-out path.
- **`Backend` handle factory (ADR-0016 Phase B).** A factory that mints
  `Arc<dyn *Storage>` handles on demand — held only at the composition root,
  never injected — is decided but not built
  ([ADR-0016](adr/0016-dependency-injection-and-appstate.md)); no such type
  exists in `storage`, and every non-serve CLI command still constructs the full
  `AppState` via `open_existing_database` (`server/src/commands.rs`).

- **Structural write scopes and mutation outcomes.** A factory-minted, sealed,
  backend-erased `WriteScope` is injected separately beside the exact storage
  traits. Only crate-private `AppStateBackend` can mint it during AppState
  composition; downstream code cannot construct a scope from a pool
  ([structural write scopes and mutation outcomes](adr/0164-structural-write-scopes-and-mutation-outcomes.md)).
  Its explicit `run` boundary supplies a sealed mutable `WriteTransaction`
  capability, never storage lookup or arbitrary SQL. The closed audited
  application surface has exactly 48 declarations: Audience (5), Email
  Verification (2), Feed Cache (2), Feed Event (7), Invite (2), Media (2),
  Password Reset (2), Post (7), Session (4), Site Config (6), Subscription (2),
  User Config (2), and User (5). Cross-store account mutations compose these
  capability-taking primitives as storage-owned functions
  ([account mutations compose storage primitives](adr/0166-account-mutations-compose-storage-primitives.md)).
  Each declaration takes `&mut WriteTransaction`; there are no pool-backed,
  auto-committing, standalone, or compatibility mutation paths. The structural
  gate derives the observed declarations, compares them with the closed
  48-method list, rejects unknown, missing, and duplicate declarations, and
  rejects production transaction starts that bypass the
  `WriteScope`/`WriteTransaction` composition. It excludes administrative
  lifecycle work, dialect code, and internal helpers. Callback failure is
  rollback-confirmed; a failed commit acknowledgement is commit-indeterminate.
  Typed `MutationOutcome<T>` preserves that algebra through server responses and
  client revalidation, while the owning scope span records the bounded outcome.
  SQLite scopes retain `BEGIN IMMEDIATE`; PostgreSQL operations retain their
  required row locks; and the post-tag plus media-reconciliation paths retain
  their ordering, rollback-on-drop, and injected-error behaviour. A separately
  injected, cross-process `MediaContentLocks` capability serializes media
  placement and reclamation with Post create/update for each content hash, in
  stable order for multi-reference Posts. Writers acquire it before their short
  database identity-lock scopes and retain it through filesystem cleanup, so no
  database transaction spans filesystem I/O and no Post reference can race file
  removal.

## Content model

A post stores its **source**: a `PostBody` in an author-chosen `PostFormat`
(`Markdown` | `Org` | `Html`, `common/src/render.rs:35`), from which a
module-qualified `host::render` free function derives the stored
`rendered_html`. The two forms feed two deliberately separate serialization
surfaces — Syndication Feeds emit HTML, the AtomPub Collection native source —
detailed in the Protocols section
([ADR-0015](adr/0015-atompub-serialization-surfaces.md)).
`storage/src/posts.rs:42::PostRecord` carries both plus title, `Slug`, summary,
tags, and `created_at`/`updated_at`/`published_at`/`deleted_at`.

`PostRecord.summary` is optional authored Post content. In contrast,
`summary_label` is disposable presentation metadata for a titleless unpublished
row: it is recomputed from the canonical `PostBody` at read time and is never
stored. The bounded unpublished-post query already carries the body; persisting
the label would impose freshness obligations across writes and direct backup
restore.

**A body has at least one non-blank line, and normalization is format-aware**
([ADR-0105](adr/0105-post-body-non-blank-invariant.md)). `PostBody::from_str` is
the one door — the `StrNewtype` derive routes serde and sqlx through it, and
there is no `from_trusted` bypass, so a blank row would fail to decode
(`common/src/post_body.rs:70-82`). The constructor stores **verbatim**; a
separate format-aware seam,
`canonicalize_body(&PostBody, &PostFormat) -> Result<PostBody, InvalidPostBody>`
(`common/src/render.rs:857`), does the normalizing: `Html` is exempt (verbatim
passthrough), `Markdown` and `Org` drop leading all-whitespace lines,
`trim_end()`, then re-append one newline. Interior blank lines and leading
horizontal whitespace are never touched — both are significant to CommonMark.

Every write path converges on **one canonical stored body**. For Org, every
create and update parses the complete leading, case-insensitive Org/Jaunder
metadata block through its first non-keyword top-level element and merges
recognized metadata with structured input. After the whole write is accepted,
every recognized header is removed, including valid mutable metadata displaced
by structured input. Structured presence resolves per field, with lifecycle as
one indivisible merge unit; transport defaults do not manufacture presence.
Structured values win, headers fill only absence, and omission retains the
surface's existing update/default semantics. Unknown Org directives remain body
content. Parsing, precedence, audience authorization, lifecycle and bookkeeping
validation, and stripping are one atomic decision: malformed, conflicting,
foreign-audience, stale, or metadata-only input saves nothing. Recognized
mutable metadata covers title, tags, summary, lifecycle/date-zone data, and
audience targets; identity and sync bookkeeping is checked against
derived/current values, never trusted as input. The full policy is
[server-side Org metadata block canonicalization](adr/0155-server-side-org-metadata-block.md),
which evolves [ADR-0024](adr/0024-server-side-org-canonicalization.md). Clients
synthesize presentation headers on output.

The deep normalization interface is `common::org::normalize_org`: it owns the
Org element boundary, typed metadata parsing, field/lifecycle precedence, date
conversion, and canonical stripping, and returns effective metadata plus
non-authoritative bookkeeping. Web and AtomPub adapters map their wire presence
into that interface; `perform_post_creation`/`perform_post_update` then persist
its canonical result, with module-qualified host free-function ETag construction
and SQLite/PostgreSQL checking final slug/format/time inside the write
transaction before commit or revision creation.

**`RenderedHtml` guarantees "contains no active markup", through a common-owned,
host-only sanitization boundary**
([ADR-0079](adr/0079-rendered-html-sanitization.md)). `common::render::sanitize`
is the only public production API that establishes the invariant; the optional
`sanitize` feature keeps `ammonia` out of CSR/wasm builds. Its field is
crate-private: ordinary application crates have no raw constructor, conversion,
blanket `Deserialize`, or trusted-string rebuild door. Common-private SQLx
decode and field-specific seed/revision DTO deserialization reconstruct the
field directly from Jaunder-owned representations, without re-sanitizing,
copying, or changing stored/rendered bytes. Exact fixtures are available only
through `common::test_support` under `cfg(test)` or `test-support`. The
compiler-backed `rendered-html-compiler-boundary` step uses an isolated
downstream dependency to prove raw construction and that fixture API remain
unavailable in production. SQLx decoding's **wrong-column blessing risk is real
and accepted**: a reviewer must ensure every `RenderedHtml` decode is from the
rendered-HTML column; no spelling marker enforces that judgement
([ADR-0123](adr/0123-rendered-html-storage-decode.md)). `RenderedHtml` stays
common because dual-target consumers reach it; ammonia stays host-only.

**A Post's media references are derived from that sanitized HTML, never
supplied** ([ADR-0090](adr/0090-media-references-extracted-at-render.md)).
`RenderOutput` lives in `host`: its private HTML and `Vec<MediaReference>`
fields and module-qualified `host::render` free function make a value whose
reference set disagrees with its HTML unrepresentable; `into_html` consumes the
pair. Each reference retains its canonical stored-media identity plus its
complete local, absolute HTTP(S), or scheme-relative URL form while rendering
and extraction remain configuration-free
([the live media-reference ownership decision](adr/0154-media-reference-live-ownership.md)).
Relative references are intrinsically local. Immediately before deletion,
absolute references are probed with bounded ordinary reqwest HEAD requests;
scheme-relative forms first inherit the current canonical `site.base_url`
scheme. Reqwest owns DNS, configured proxies, redirects, TLS, and pooling; the
trusted-author model deliberately leaves no Jaunder address policy, DNS
resolution, socket pinning, or redirect implementation. Every response
identifies its persistent database instance through `X-Jaunder-Instance`; a
matching identity is owned, a completed response with a different or absent
identity is foreign, and request failure or an ambiguous header is unknown and
fails closed.

Persisted `post_media` rows use an exact subject key: `Current(post_id)` or
`Revision(post_id, revision_id)`. A meaningful Post mutation copies its exact
current rows to the newly captured immutable Revision subject before replacing
the current subject; the schemas enforce that a Revision subject names that
Post's Revision. The ownership evidence, locked conditional delete, reclaim
predicate, and owner reporting all carry that full subject key, so a current-row
proof cannot authorize removal across an unseen historical snapshot. The
ordinary guard protects references from active Posts, Deleted Posts, and
Revisions; owner reporting deduplicates their Post IDs. A web force delete may
override the caller's own-reference refusal, but still refuses a delete that
would leave referenced bytes with no media-row accounting; it deliberately may
break archival reconstruction ([ADR-0136](adr/0136-local-post-lifecycle.md),
[the live media-reference ownership decision](adr/0154-media-reference-live-ownership.md)).

`CreatePostInput`/`UpdatePostInput` carry the render output in place of bare
HTML, and the rows land in the Post's own transaction. Publication uses the same
atomic revision/mutation discipline; rendering remains the sole reference
constructor.

**Media is content-addressed, and the layout is spelled once.** `media_path`
(`common/src/media.rs:671`) is the single definition of
`<source>/<p1>/<p2>/<sha256>/<filename>`, and `media_url` (`:689`) is that path
under the `/media/` prefix, returning `RootRelativeUrl` infallibly
([ADR-0080](adr/0080-media-path-naming-correspondence.md)). The filename segment
is percent-encoded with `NON_ALPHANUMERIC` minus the RFC 3986 unreserved marks
`-._~` (`MEDIA_SEGMENT_ENCODE_SET`, `common/src/media.rs:623`), so ordinary
names stay byte-identical and only `?`, `#`, space and friends encode — the
class of bug where a URL validates cleanly but addresses a different file. **The
encoded form is canonical**
([ADR-0084](adr/0084-media-filename-encoded-canonical.md)): a `Filename` _is_
the encoded segment, so the database column, the on-disk name and the URL
segment are the same bytes and nothing encodes at a call site;
`Filename::from_str` enforces `s == encode(decode(s))` plus the safe-leaf oracle
run on the _decoded_ form (`common/src/media.rs:303`), and `Filename::decoded()`
(`:376`) is the single explicit opt-out, for display. Byte equality is what lets
[ADR-0090](adr/0090-media-references-extracted-at-render.md)'s comparison
against names extracted from rendered HTML avoid a transform at a comparison
point, and it is what makes
[ADR-0024](adr/0024-server-side-org-canonicalization.md)'s publish-time link
substitution content-derived.

**The public content-addressed media route is strict at extraction.** One
validated address owns `MediaSource`, `ContentHash`, both redundant hash
prefixes, and a canonical `Filename` parsed from Axum's decoded route segment
through the extractor-private/common-owned decoded-segment door. A malformed
component or prefix mismatch is an Axum 400 before the handler; only a
syntactically valid but absent resource is 404. This supersedes #504 only for
`/media/{source}/{p1}/{p2}/{hash}/{filename}`; projector and Syndication Feed
soft routes retain `SoftPath`. HTTP serve-outcome counts belong to the front
proxy, so the application emits upload-domain outcomes and bytes but no partial
`jaunder.media.served` counter
(`docs/adr/0140-strict-media-address-extraction.md`).

**Slugs never fail and preserve Unicode**
([ADR-0025](adr/0025-unicode-slug-generation.md)). The charset is per extended
grapheme cluster — kept iff the base scalar is alphanumeric, carrying its
combining marks (`base_is_alphanumeric`, `common/src/slug.rs:72`, shared by
`slugify_title` and `Slug::from_str`). `Slug::from_str` is the single chokepoint
— NFC normalization, Unicode lowercasing, the `MAX_SLUG_CHARS` (80-scalar) cap;
an unusable title falls back to a synthesized slug. The slug is frozen while the
Post's current `published_at` is non-null; pulling it back to draft makes the
slug editable on a later update, and scheduling or publishing freezes it again
(`storage/src/{sqlite,postgres}/posts.rs::update_post`,
[scheduled publishing](adr/0027-scheduled-publishing-time-gated-visibility.md),
[current-state slug freeze](adr/0130-current-publication-state-slug-freeze.md)).

**Visibility starts with active/not-deleted eligibility, then applies two
orthogonal predicates on the same reads.** _Time_: an active Post is draft
(`published_at` NULL), scheduled (future), or live (past); every public read
gates `published_at <= now` with `now` an explicit parameter, the feed worker's
`go_live_pass` (`server/src/feed/worker.rs`) makes future-dated go-live
restart-durable for cached feeds, and the posts storage contract carries publish
as an explicit `PublishUpdate { Unpublish, Publish { at } }` to each dialect's
SQL-binding boundary, so scheduling, backdating, and pullback to draft
round-trip ([ADR-0027](adr/0027-scheduled-publishing-time-gated-visibility.md)).
_Audience_: posts target audiences
(`AudienceTarget::{Public, Private, Subscribers, Named}`) stored as
`post_audiences` rows; a viewer is a `ViewerIdentity` (channel identity or
anonymous, `common/src/visibility.rs`) and sees a post iff they are the author
or any targeted audience admits them — OR semantics, failing closed (`Private`
is zero rows). Subscriptions route through the admission seam
(`SubscriptionPolicy`, wired to the auto-approving `OpenSubscriptionPolicy`);
their persisted identity is `SubscriberIdentity { channel_id, subscriber_ref }`.
The reference is opaque inside its channel namespace, non-blank in Rust, and
zero-length-rejected by both schemas. Typed reads validate before projecting
display text; migration aborts rather than inventing or deleting an invalid
identity (`docs/adr/0151-subscriber-reference-invariant.md`,
[ADR-0020](adr/0020-content-visibility-and-subscription-model.md)). The
instance-wide Default Audience is separately the closed
`DefaultAudience::{Public, Subscribers, Private}` value: it cannot be a
per-author `Named` target and widens to `AudienceTarget` only at the web and
AtomPub per-Post boundaries.

**Local Post lifecycle.** A Post row is durable canonical identity and latest
state. Storage treats every meaningful top-level content, tag, audience, media,
publication, unpublication, and soft-delete mutation as one atomic operation: it
locks and canonically compares the complete desired state, then captures exactly
one immutable full prior-state **Post Revision** and its tag, audience, and
media children before applying the change. Scalar snapshots contain authored
source and format, rendered HTML, title, slug, summary, immutable creation time,
prior modification time, and publication/deletion timestamps; child values are
copied rather than linked to mutable tag or audience lookup rows. A semantic
no-op writes neither a Revision nor an updated timestamp. Creation is
revision-free because it has no prior state
([ADR-0136](adr/0136-local-post-lifecycle.md)).

Revision records have no product mutators: only top-level Post mutation and
whole-store backup/restore write them. Authenticated owners can list global
history and a Post's history, including a Deleted Post, and inspect an exact
complete snapshot; unauthenticated callers are rejected at the authentication
boundary and non-owner requests are masked as absent. Both lists use
newest-first immutable Revision-ID keyset pagination with bounded overfetch and
opaque cursors. History is not a revert mechanism.

Soft deletion stamps and retains the Post, all revisions, and child
relationships indefinitely while excluding the current Post from active web
reads, Syndication Feeds, and AtomPub Collections. Active permalink and
syndicated-item identity can be reused by a later Post, with accepted
feed-reader conflation; restore is not promised. There is no product purge. A
future purge must decide the combined Post, Revision, media, and child erasure
policy ([ADR-0136](adr/0136-local-post-lifecycle.md)). ADR-0009 continues to
govern consumed content rather than these local rows.

Cross-cutting values are validated newtypes whose `FromStr` is the single
chokepoint: `Username`, `Slug`, and `Tag` live in `common`; `Password` lives in
`host`. Tagging is keyed on the `Tag` slug (`PostTag`, `post_tag_diff` in
`storage/src/posts.rs`).

**Post-shaped wire types are named for the content weight they carry**, not for
the transaction that produced them
([ADR-0097](adr/0097-post-dto-content-weight-axis.md)). Three tiers: metadata
only (`UnpublishedPost`, `web/src/posts/api.rs:103`), plus the rendered form
(`RenderedPost`, `common/src/seed.rs:39`), plus the authored source
(`AuthoredPost`, `:108`, which _nests_ a `RenderedPost` and adds `PostBody` +
`PostFormat`). One type, `SavedPost` (`web/src/posts/api.rs:78`), serves all
four post mutations. Merging two types is viable only _within_ a tier — a
cross-tier union ships the heavier payload to consumers of the lighter one — and
structural overlap alone does not justify it; the discriminator is whether the
code converts between them.

**Timelines paginate by keyset cursor, not offset**
([ADR-0004](adr/0004-pagination-strategy.md)). `PostCursor`
(`storage/src/posts.rs:187` — `created_at` + `post_id`, for stable ordering) and
`CollectionCursor` (`:197`, `updated_at` + `post_id`, for the editor-facing
collection) are the storage-side cursors; the wire carries an opaque
`PageCursor`, and a listing returns `next_cursor` exactly when another page
exists (`web/src/posts/api.rs:117,425`). `PageSize`
(`common/src/pagination.rs:29`) is clamped 1..=50 with a default of 50. The
offset type `PageOffset` (`:62`) exists only for the media listing, where the
reader may skip.

### Committed direction

Nothing below is built. **There is no ingestion tier**: all 25 migrations
(`storage/migrations/{sqlite,postgres}/`) are publishing-side, and no table
holds fetched remote content.

- **Unified content model for ingested content**
  ([ADR-0005](adr/0005-unified-content-model.md)): a consumed protocol item is
  to be stored twice at write time — the raw payload unaltered as the source of
  truth, plus a normalized processed form (core fields + a protocol-specific
  extension blob) that the API and UI read, so logic changes can be applied
  retrospectively by re-processing the raw payloads.
- **High-fidelity retention for inbound changes**
  ([ADR-0009](adr/0009-edit-delete-policy.md)): a received update stores the
  revised item as a new immutable revision alongside all prior versions, and an
  inbound delete may hide content from active views but never purges it. None of
  this exists. ADR-0009 speaks only of consumed content — "for followed
  sources", "when an update is received" — so the local `post_revisions`
  snapshot above implements nothing it decided, despite the resemblance.
- **Sanitization of foreign HTML** on arrival
  ([ADR-0079](adr/0079-rendered-html-sanitization.md)):
  `common::render::sanitize` establishes the invariant on the host; no inbound
  producer exists yet.
- **Visibility Layers B/C**
  ([ADR-0020](adr/0020-content-visibility-and-subscription-model.md)):
  federation/email delivery channels and authenticated browsing for non-local
  visitors; only Layer A (`local` channel) is built.
- **Domain-value newtype convention**
  ([ADR-0063](adr/0063-domain-value-newtype-convention.md)): a criterion for
  when a value earns a newtype plus a generated standard trailer
  (`StrNewtype`/`IdNewtype` in `macros/`, shipped #413); adoption is still
  rolling out (#17, #14).

## Protocols (AtomPub, feeds, WebSub)

Every protocol leg that exists today is **outbound**: public syndication feeds,
an authenticated AtomPub (RFC 5023) editing interface, and publisher-side WebSub
pings. Nothing is ingested. The feed and the AtomPub Collection are two
deliberately separate serializers, told apart by endpoint and never by
user-agent sniffing ([ADR-0015](adr/0015-atompub-serialization-surfaces.md)).

Every URL that crosses a protocol boundary carries a **role** in its type —
`FeedUrl`, `HubUrl`, `CanonicalUrl`, `ServiceDocUrl`, `HomepageUrl`,
`PermalinkUrl`, … , each a `TaggedUrl<Role>` alias
([ADR-0112](adr/0112-role-tagged-site-urls.md)). This is what stops two adjacent
same-typed URLs being transposed: `send_publish(hub, feed)`,
`render_rsd_document(service, homepage)`, and the `FeedMetadata`
`canonical_url`/`self_url`/`hub_url` fields were all live hazards before the
roles existed.

### Syndication feeds

Public read-only feeds serve arbitrary feed readers, so every item carries the
post's `rendered_html` — Atom `type="html"` and the RSS/JSON Feed equivalents
([ADR-0015](adr/0015-atompub-serialization-surfaces.md)). The CSR-reached
`common::feed` grammar is exactly `FeedFormat`, `FeedSurface`, and
`canonicalize`; the remaining Syndication Feed types and qualified rendering
operations live in `host`. `server/src/feed/handlers.rs` serves the cached
bytes, and `regenerate_feed` rebuilds them. Scheduled posts reach feeds via
`FeedWorker::go_live_pass` (`server/src/feed/worker.rs:84`), which enqueues
regeneration for feeds whose posts crossed their publish time
([ADR-0027](adr/0027-scheduled-publishing-time-gated-visibility.md)).

**Accepted membership target.** Cached membership is to apply anonymous/Public
eligibility before ranking, then select the union of the first `feeds.min_items`
Posts and all Posts at or newer than the inclusive `feeds.min_days` cutoff,
ordered by `published_at DESC, post_id DESC`. Defaults are 20 Posts and 30 fixed
24-hour UTC days. The window is exact when regenerated, not continuously as time
passes. A successful setting mutation is to durably invalidate all cached feeds
before returning; checked overlarge ages mean all history, while corrupt stored
values are errors
([Syndication Feed hybrid-window decision](adr/0139-syndication-feed-hybrid-window.md)).
The union and defaults ship today, but
[SQL ranks before visibility](https://github.com/jaunder-org/jaunder/issues/1051),
so private rows can crowd out the count floor.
[Setting activation, arithmetic, and corrupted values](https://github.com/jaunder-org/jaunder/issues/1053)
remain implementation debt.

**Accepted validation target.** Each cached representation is to carry a strong,
deterministic ETag from every ordered serializer input plus serializer revision;
identical semantic inputs and bytes retain it. A persisted whole-second
representation-modification time is to change only with representation identity
and supply `Last-Modified`. `If-None-Match` uses RFC 9110 weak comparison, lists
and wildcard, and takes precedence over `If-Modified-Since`; matching GET/HEAD
returns 304 without a body, nonmatching GET returns a body, and nonmatching HEAD
returns GET-equivalent headers without one. Current validators and cache
metadata accompany 304. `Cache-Control: public, max-age=300` is a downstream
revalidation policy, not a regeneration promise
([Syndication Feed HTTP-validation decision](adr/0138-syndication-feed-http-validation.md)).
[Current tuple completeness, item-derived timestamp, 304 metadata, and conditional parsing](https://github.com/jaunder-org/jaunder/issues/1054)
remain implementation debt.

The Atom feed document is built by upstream `atom_syndication` through the
host-owned Syndication Feed renderer; RSS goes through the `rss` crate the same
way ([ADR-0089](adr/0089-upstream-atom-document-io.md)).

The authenticated Collection (`server/src/atompub/router.rs:16-33`:
`/atompub/service`, the per-user post collection and member routes, the media
collection and its `{sha}/{filename}` members) serves editing clients — MarsEdit
and the Emacs front-end — authenticating with HTTP Basic app passwords validated
through the ordinary session-token path
([ADR-0014](adr/0014-atompub-authentication.md); details in the auth section).
The same router serves the RSD autodiscovery document at `/~{username}/rsd.xml`
(`server/src/atompub/rsd.rs`), which is how MarsEdit finds the service document.
Unlike the syndication feed, the Collection serializes each post in its native
source form, so a `GET` round-trips losslessly through `PUT`
([ADR-0015](adr/0015-atompub-serialization-surfaces.md)).

Format travels in the standard `atom:content` `type` attribute as a media type
([ADR-0023](adr/0023-atompub-jaunder-wire-extensions.md)): Org is `text/org`,
Markdown is `text/markdown`, Html is the `html` token. Reading is lenient — a
bare `text`, an unrecognized type, or an absent one falls back to the account
`default_post_format`, and `xhtml`/`text/html` defensively map to Html. The
whole policy is the `format_wire` seam: two private pure functions in the
host-owned AtomPub mapping, `format_to_wire` and `wire_to_format`. Only the
namespace and `j:slug` definitions they consume move with the rest of AtomPub;
the behavior remains one-line reversible.

For `text/org` entries, AtomPub create and update use that same full-block
metadata interpretation and canonical metadata-free body as every other Org
ingress; Atom elements remain structured input, not a competing canonical
representation. In particular, explicit Atom metadata wins over a header and the
header can supply only its absence. The authoritative invariant is
[server-side Org metadata block canonicalization](adr/0155-server-side-org-metadata-block.md).

Two Jaunder wire extensions ride the namespace `https://jaunder.org/ns/atompub`
([ADR-0023](adr/0023-atompub-jaunder-wire-extensions.md)): a read-only `j:slug`
on every entry — drafts and scheduled included, incoming values ignored — and
`<j:extension version="1" features="format-media-type slug"/>` in the Service
Document, so clients feature-detect once and degrade gracefully.

`CollectionDecl::accept` models Service Document discovery ranges with the
closed `CollectionAccept` type, separately from concrete uploaded-media
`common::media::ContentType` values. The Posts Collection advertises exactly
`application/atom+xml;type=entry`; the Media Collection advertises exactly
`*/*`. The wildcard therefore exists only at the AtomPub discovery boundary and
can never enter media request parsing or storage.

Atom document I/O is upstream's, not ours
([ADR-0089](adr/0089-upstream-atom-document-io.md)). `atom_syndication` 0.12.10
made bare-`<entry>` I/O public, so the hand-rolled reader and writers are gone:
parsing is `Entry::from_str` at the host call site and serialization is
`Entry::write_to` / `Feed::write_to`. Jaunder's own foreign markup stays
Jaunder's — `app:control/app:draft` and `j:slug` live in the entry's extension
map behind helpers that own each `xmlns:` prefix. `quick-xml` is a direct host
dependency for the non-Atom documents Jaunder writes itself: the Service
Document, RSD, and shared XML helpers. Category discovery is inline: an
applicable Collection declares its open-set `app:categories` terms with
`fixed="no"` in the Service Document. Jaunder does not advertise an
`app:categories href` or serve an out-of-line Categories Document
([inline-only AtomPub category discovery decision](adr/0157-inline-only-atompub-category-discovery.md)).

The crates come from the registry — `atom_syndication` 0.12.10 and `rss` 2.1.
Earlier, [ADR-0043](adr/0043-quick-xml-fork-patch.md) (now **superseded**) had
cleared two RUSTSEC advisories by forking both crates onto `quick-xml` 0.41 and
wiring the forks in through `[patch.crates-io]`, flake inputs, and a crane
vendor override. **That apparatus was deleted outright** — the only surviving
requirement is that the registry releases resolve `quick-xml` ≥ 0.41, as before.

**Accepted publisher target.** Publisher-side WebSub is to cover every Site,
User, Site Tag, and User Tag Syndication Feed URL in RSS, Atom, and JSON. One
optional site-wide hub is advertised in each representation; a **WebSub Publish
Ping** names that exact URL as its topic and carries no content. The trigger is
a protocol-independent change to at least one concrete public feed
representation. Post mutation and affected-feed events are to commit as a
transactional outbox; the worker commits the regenerated cache before
duplicate-safe, at-least-once remote publication. Hub configuration changes are
to invalidate all cached feeds and work late-binds to one coherent current
configuration snapshot. Regeneration and publication use separate bounded
attempt budgets; exhausted regeneration and terminal publication remain
separately inspectable and redrivable
([publisher-side WebSub decision](adr/0137-publisher-side-websub.md)).

**Current publisher behavior.** Production pings through
`WebSubClient::send_publish(&HubUrl, &FeedUrl)`
(`server/src/websub/contract.rs:45`) and reports `Success`, `Exhausted`,
`Failed`, or `NoHub`.
[AtomPub does not enqueue, web enqueue is not atomic, and triggers are coarse](https://github.com/jaunder-org/jaunder/issues/1051).
[Configuration changes do not invalidate caches, worker/regenerator snapshots can differ, configuration access errors can collapse to `NoHub`, HTTP failures retry alike, `Retry-After` is ignored, budgets are shared, and terminal rows lack redrive](https://github.com/jaunder-org/jaunder/issues/1052).

The
[bounded transient-data retention decision](adr/0167-bounded-transient-data-retention.md)
makes completed feed events cleanup-eligible immediately and retains exhausted
events for seven days before making them cleanup-eligible. It is terminal-row
retention, not a recovery or redrive decision for #1052, and does not apply to
`feed_cache`. `HttpWebSubClient` runs in production; noop and file-capture
implementations back tests.

### Committed direction — inbound federation

[ADR-0010](adr/0010-protocol-integration.md) commits Jaunder to becoming a
unified reader across ActivityPub, AT Protocol, and web feeds: push-first
delivery (ActivityPub inbox, WebSub subscriptions, AT Jetstream), adaptive
polling as the fallback, everything normalized into the unified content model
([ADR-0005](adr/0005-unified-content-model.md)).

**None of it is built.** There is no ActivityPub inbox, no Jetstream consumer,
no polling scheduler, and no fetcher; all 25 migrations per backend
(`storage/migrations/{sqlite,postgres}/`) are publishing-side, with no table for
fetched content. Inbound RSS/Atom ingestion is the first slice (issue #282),
with WebSub subscription (#921), ActivityPub (#287), and adaptive polling (#920)
sequenced behind it. Read every sentence in this subsection as intent, not as
description.

## Authentication

Jaunder authenticates over three transports that all resolve to one credential
system: **session cookies** for the web frontend and **Bearer tokens** for API
clients ([ADR-0007](adr/0007-auth-mechanisms.md)), plus **HTTP Basic** carrying
app-specific passwords for AtomPub clients such as MarsEdit
([ADR-0014](adr/0014-atompub-authentication.md)). All three paths converge on
the `auth::User` axum extractor (`web/src/auth/server.rs`), which resolves
identity through the `SessionStorage` trait
([ADR-0007](adr/0007-auth-mechanisms.md)). Header parsing itself lives in the
target-agnostic `host::auth::resolve_credential` (`host/src/auth.rs`), pushed
below `web` as part of the thin-web-shell rollout
([#334](https://github.com/jaunder-org/jaunder/issues/334)).

Any `Authorization` header is authoritative explicit intent and is resolved
before the ambient `session=` cookie. Supported Bearer and Basic credentials
select their authenticated identity; malformed values, unsupported schemes,
failed token lookups, and Basic username mismatches reject without cookie
fallback. Only an absent header permits cookie authentication. After successful
Bearer or Basic authentication, a simultaneous session cookie is expired on the
response across Leptos and raw Axum routes. Optional-auth reads distinguish
absence from failure: absent or stale cookie-only credentials may remain
anonymous, while explicit-credential failures propagate
([explicit Authorization replaces ambient session state](adr/0132-explicit-authorization-replaces-session-cookie.md)).

Leptos server functions obtain the same identity via `require_auth()`
(`web/src/auth/server.rs`), which pulls request `Parts` from context and runs
the `auth::User` extractor; failures map to unauthorized/internal errors through
`auth::Rejection` ([ADR-0007](adr/0007-auth-mechanisms.md)). The operator-only
variant `require_operator()` layers the `is_operator` check on top.

`viewer_identity()` (`web/src/viewer.rs`) applies the optional-auth half of the
same rule for visibility-filtered reads: it returns a local viewer on successful
authentication, anonymous for absent or stale cookie-only credentials, and an
error for every failure attributable to a Bearer or Basic credential.

**Session establishment for the web client is cookie-only**
([ADR-0107](adr/0107-web-session-establishment-is-cookie-only.md)): a
`#[server]` fn on the auth path sets the `HttpOnly` `session` cookie and returns
**no session-token material** in its body. Concretely `register` returns `()`
(`web/src/registration/api.rs:59`) and `login` returns the complete advisory
`SessionUser` identity: the authenticated record's canonical `username` and
`is_operator`, not a credential (`web/src/auth/api.rs:54`). The one deliberate
exception is `create_app_password` (`web/src/sessions/api.rs:62`), which returns
the raw token because showing it once at creation is the whole point of an app
password — that endpoint establishes no browser session. **No web endpoint hands
the browser a bearer token** — though endpoints will still _accept_ one, as
logout does. No machine gate enforces the rule: it is held by
`assert_body_carries_no_token` (`server/tests/web/web_auth.rs:50`), which checks
the success body against the token recovered from `Set-Cookie`, called for
register (`:281`) and login (`:530`, `:564`)
([ADR-0107](adr/0107-web-session-establishment-is-cookie-only.md)). Two limits
worth knowing: it covers those two endpoints only, so a new auth `#[server]` fn
inherits no protection, and it is a substring check, so a re-encoded token would
pass.

### Credentials and sessions

- A session token is 32 cryptographically random bytes, base64url-encoded, and
  is minted already-digested by `host::token::generate_hashed`
  (`host/src/token.rs:64`, called at `storage/src/sessions.rs:160`): the raw
  token and its SHA-256 digest are returned together and only the digest is
  persisted, so the raw value is never stored. On the lookup side
  `host::token::hash` (`:53`) is the **sole** `RawToken → TokenHash` conversion
  (`storage/src/sessions.rs:219`, `password.rs:126`, `email.rs:162`). The
  neighbouring `host::token::generate` (`:28`) mints invite codes, not session
  tokens (`host/src/invite.rs:59` is its only caller). The two are distinct
  newtypes, `common::token::{RawToken, TokenHash}`
  ([#458](https://github.com/jaunder-org/jaunder/issues/458)), and `RawToken`
  carries `#[str_newtype(no_sqlx, no_ord)]` (`common/src/token.rs`) so
  `.bind(raw_token)` does not compile — that opt-out, not a lint, is what keeps
  a raw token out of a query. `RawToken`'s `Debug` is hand-written to redact the
  body ([ADR-0011](adr/0011-unified-observability.md)). The directional
  guarantee and the rejected stronger type-state design are recorded by
  [hash bearer-equivalent tokens before persistence](adr/0133-hash-bearer-tokens-before-persistence.md).
- An **app password** is just a labelled session: minting calls
  `SessionStorage::create_session(user_id, &label)`
  (`storage/src/sessions.rs:70`) — no separate table, no `kind` column, so
  tokens are interchangeable across transports (accepted for the self-hosted
  single-user trust model). Sessions never expire; the `sessions` row is
  `(token_hash, user_id, label, created_at, last_used_at)`. `last_used_at` is
  operator-facing metadata and is bounded-stale: authentication refreshes it
  only when the stored value is more than 60 seconds old, so fresh authenticated
  requests need not become database writers (`storage/src/sessions.rs:164`).
  `label` is a mandatory validated newtype,
  `common::session_label::SessionLabel` — browser logins auto-generate a
  User-Agent/host label, app passwords carry a user-supplied name. Revocation is
  deleting the session in the Sessions UI
  ([ADR-0014](adr/0014-atompub-authentication.md)).

The
[bounded transient-data retention decision](adr/0167-bounded-transient-data-retention.md)
separates permanent sessions and App Passwords from credentials with expiry: an
expired credential remains retained for 24 hours after expiry, while a consumed
credential is cleanup-eligible immediately.

- A token for user X reaches only `/atompub/X/*`. The enforcer is
  `server::atompub::require_user_match` (`server/src/atompub/guards.rs:13`),
  which returns 403 on mismatch and guards every per-user route — directly at
  `posts.rs:135,313` and `media.rs:85,152,181`, and through `owned_post`
  (`posts.rs:227`) at `:253,282,435`. It applies whichever credential was used
  ([ADR-0014](adr/0014-atompub-authentication.md)). Separately, on the Basic
  path `verify_basic_username` (`web/src/auth/server.rs:202`) ties the supplied
  username to the resolved session's user; cookie and Bearer requests pass
  `expected: None` and skip that check, which is why the route guard rather than
  the credential check is what does the scoping. `/atompub/service`
  (`server/src/atompub/router.rs:17`) is authenticated but sits outside the
  per-user tree, and RSD is deliberately unauthenticated (`atompub/rsd.rs:22`).
  Basic sends the token on every request, so the TLS-terminating reverse proxy
  is load-bearing for AtomPub ([ADR-0014](adr/0014-atompub-authentication.md)).

Cookie management is layered:
`web::auth::server::{set_session_cookie, clear_session_cookie}` are Leptos
adapters over the pure header builders
`host::auth::{session_cookie_header, clear_session_cookie_header}`, which emit
`session=<token>; HttpOnly; SameSite=Lax; Path=/` (plus `; Secure` when the
deployment's `CookieSettings` say HTTPS); clearing sets `Max-Age=0`. `HttpOnly`
keeps page JavaScript away from the credential — the protection ADR-0107 exists
to stop the response body from undoing — and `SameSite=Lax` is the XSRF
mitigation ADR-0007 lists among its decision drivers
([ADR-0007](adr/0007-auth-mechanisms.md),
[ADR-0107](adr/0107-web-session-establishment-is-cookie-only.md)).

Explicit-auth cookie retirement uses a request-scoped `SessionCookieRetirement`
marker set by `auth::User` only after Bearer/Basic token and username checks
succeed. Outer router middleware appends the expiry header, preserving any
`Set-Cookie` values already emitted by the handler
([explicit Authorization replaces ambient session state](adr/0132-explicit-authorization-replaces-session-cookie.md)).

### Password hashing

Passwords are hashed and verified with **Argon2id** at the crate-default
parameters (m=19456, t=2) by module-qualified `host::password` free functions
([ADR-0018](adr/0018-constant-time-authentication.md)). Test builds may enable
the `cheap-kdf` feature, which swaps in `Params::MIN_M_COST` with t=1 so the
suite is not dominated by KDF time; verification derives cost from the stored
PHC hash, so it needs no branch. The feature fails closed twice, at different
times:

- **Compile time** —
  `#[cfg(all(feature = "cheap-kdf", not(debug_assertions)))] compile_error!`
  (`host/src/lib.rs`). The guard keys on `debug_assertions`, so an _optimized_
  build carrying the feature fails to build rather than producing a weak-hashing
  artifact; ordinary test builds are unaffected.
- **Startup** — `server/src/main.rs` reads `host::CHEAP_KDF_ENABLED` and, if
  set, prints a `FATAL:` line and exits before CLI parsing. This catches the
  debug-build-in-production case the compile-time guard lets through.

The dependency isolation and both complementary guards are the
[test-only cheap KDF fail-closed policy](adr/0131-test-only-cheap-kdf-fails-closed.md).

### Timing discipline: the entropy dividing line

Two deliberate, opposite orderings govern when the expensive Argon2 work runs,
split by the **entropy of the value being validated**:

- **Enumerable identifier (username): equalize timing.**
  `UserStorage::authenticate` (`storage/src/users.rs:304`) performs an Argon2
  verification against a fixed dummy hash (`dummy_password_hash()`,
  `storage/src/helpers.rs:424` — computed once via `OnceLock` through the real
  module-qualified `host::password` hash free function so it carries production
  parameters, with a hardcoded valid-hash fallback so initialization is
  infallible) before returning `InvalidCredentials`
  (`storage/src/users.rs:350`), closing the username-enumeration timing oracle.
  **Durable invariant: the absent-user path MUST keep this equalizing
  verification** — do not remove it as a "fast path" and preserve it through any
  refactor. The backend dedup is already done: `authenticate` is a single
  generic `UserStore<DB: Backend>` impl (`storage/src/users.rs:212`), so SQLite
  and Postgres cannot drift apart here
  ([ADR-0018](adr/0018-constant-time-authentication.md),
  [ADR-0114](adr/0114-absent-user-timing-equalization.md)). Verify cost is
  derived from the hash string's encoded params, so the dummy hash only
  equalizes if its parameters match a real hash's — asserted by
  `dummy_password_hash_matches_real_hash_parameters`
  (`storage/src/helpers.rs:848`). The fallback constant must likewise be a
  well-formed hash, since a fast `Err` would reintroduce the oracle; it carries
  _production_ parameters, so under a `cheap-kdf` build its parity is not exact
  and no parity test asserts it — an accepted limitation of hard-coding, and why
  the fallback is a last resort
  ([ADR-0114](adr/0114-absent-user-timing-equalization.md)).
- **High-entropy secret (invite code, reset token): cheap-reject first.**
  Storage-owned account mutations compose the rows' primitive traits inside the
  caller-owned `WriteScope`: `account_mutations::register_with_invite` prechecks
  and conditionally claims through `InviteStorage`, with
  `UserStorage::create_user` between those operations; and
  `account_mutations::confirm_password_reset` claims through
  `PasswordResetStorage::use_password_reset`, then calls
  `UserStorage::set_password` and `SessionStorage::revoke_all_for_user`.
  Registration validates the invite with a cheap lookup before hashing, creates
  the user, and then conditionally claims the invite so a concurrent PostgreSQL
  loser rolls back its inserted user. Password reset atomically claims the reset
  token before hashing the new password — it originally hashed first, which
  ADR-0022 recorded as a violation and
  [#60](https://github.com/jaunder-org/jaunder/issues/60) fixed. A ~256-bit
  secret admits no useful timing oracle, and hashing first would turn
  bogus-secret requests into a CPU-exhaustion amplifier while destroying invite
  issuance as a throttle
  ([ADR-0022](adr/0022-validate-before-expensive-work.md)).

The
[bounded transient-data retention decision](adr/0167-bounded-transient-data-retention.md)
governs retention after a credential enters its terminal state; it does not
alter the cheap-reject and atomic-claim security properties above.

Do not apply the equalizing-dummy-hash rule to high-entropy-secret paths, or
cheap-reject to enumerable identifiers — each ADR carries the scope boundary to
the other ([ADR-0018](adr/0018-constant-time-authentication.md),
[ADR-0022](adr/0022-validate-before-expensive-work.md)).

### Username boundary

Usernames are a validated domain newtype, `common::username::Username` (an
exemplar of the [ADR-0063](adr/0063-domain-value-newtype-convention.md)
convention): `FromStr` lowercases the input and rejects anything not matching
`[a-z0-9_-]+` (`common/src/username.rs`), and the serde bridge routes wire
(de)serialization through the same validation, so interior code only ever sees
canonical lowercase usernames. The parser is the **only** place this happens:
server-side entry points do not pre-lowercase. The redundant `.to_lowercase()`
calls that once preceded `Username::from_str` fell out of the typed-wire-arg
work — login under [#414](https://github.com/jaunder-org/jaunder/issues/414),
registration and forgot-password under
[#407](https://github.com/jaunder-org/jaunder/issues/407);
[#67](https://github.com/jaunder-org/jaunder/issues/67) had identified the
redundancy and was later closed as already resolved. What remains is a
client-side convenience — the login, registration, and password-reset forms
lowercase live input for display, not for validation.

The canonical value is stored, compared, serialized, displayed, and used in
URLs. Direct equality is therefore case-insensitive in effect:
`verify_basic_username` compares two already-canonical `Username` values, so an
app-password client may vary ASCII case without changing identity. Unicode and
case-preserving username identities are deliberately excluded
([lowercase-canonical usernames](adr/0134-lowercase-canonical-usernames.md)).

## Web frontend

The web UI is Leptos ([ADR-0002](adr/0002-frontend-framework.md)), rendered
**client-side only** ([ADR-0040](adr/0040-web-rendering-leptos-csr.md)): no SSR,
no hydration, a UI-free server — no reactive page render in the request path,
which structurally eliminates the concurrent-SSR disposal class; server
rendering a reactive component to string is the prohibited trap door back. The
`web` crate does not enable `leptos/ssr`; the feature reaches the build only
because `leptos_axum` requires it (`web/Cargo.toml:53-64`), and shedding that
stack is tracked, not done.

Mounted CSR journey ownership lives in
[`docs/flows/README.md`](flows/README.md): it owns the only route graph. The
reviewed Playwright evidence map stays in
[`docs/coverage/csr-e2e-matrix.md`](coverage/csr-e2e-matrix.md), and the
`flow-docs` xtask step checks the typed route, endpoint, and matrix references
between those two documents without duplicating either artifact here.

### Rendering model: projector + CSR client

The mechanism is "SSR the data, not the components"
([ADR-0041](adr/0041-public-projector-and-csr-client.md)): a thin non-reactive
**public projector** (`server/src/projector/`) renders the anonymous document
for public routes, fetching through explicit-viewer `fetch_*` seams as
`ViewerIdentity::Anonymous` (`server/src/projector/handlers.rs:63-198`), so its
output is byte-identical per URL and therefore CDN-cacheable. The document is
assembled from `web::app::render_head` / `render_shell`
(`server/src/projector/document.rs:7,16-40`), which compose the pure
per-vertical render fns; those live beside the vertical they serve —
`web/src/posts/render.rs`, `timeline/render.rs`, `home/render.rs`,
`sidebar/markup.rs`, `taglist/markup.rs`, `topbar/markup.rs`,
`avatar/markup.rs`, `icon/markup.rs` — not in a central render module. The
document embeds a `PageSeed` JSON blob (`common/src/seed.rs`,
`id="jaunder-seed"`) that the CSR client reads on boot, drops the
projector-painted `#app` container, and mounts over (`csr/src/lib.rs:29-47`);
client-side navigation falls back to the `#[server]` fns, still the data API on
`/api`. Reactive components render their anonymous DOM via `inner_html` of the
_same_ pure fns the projector uses (`web/src/home/component.rs:70`,
`sidebar/component.rs:60-70`, `posts/component/display.rs`), so the CSR mount
causes no reflow: flash-free by coincidence, not markup twins.

Markup is built with **maud's `html!`**
([ADR-0093](adr/0093-web-render-html-macro.md)), and the trusted-HTML invariant
is carried by one crate-local newtype, `web::html::Markup` (`web/src/html.rs`),
which shadows `maud::Markup` inside `web`. The single raw door is
`Markup::from_rendered_html`, which takes a `&RenderedHtml`
(`web/src/html.rs:59`) so the sanitization invariant is what opens it. The
`html-sink` and `raw-html-door` gates read inside macro bodies; compiler privacy
prevents a hand-built `String` from becoming `RenderedHtml` before that raw
door.

The authenticated owner stays flash-free by _enhancement_
([ADR-0044](adr/0044-authenticated-owner-flash-free-enhancement.md)): an
advisory localStorage auth marker, read by an inline blocking `<head>` script
(`web::app::PREPAINT_SCRIPT`, `web/src/app/render.rs:40`), sets
`<html class="authed">` before first paint. The same constant is emitted by the
projector (`server/src/projector/document.rs:32`) and embedded verbatim in
`csr/index.html`, with a host test guarding the drift
(`web/src/app/render.rs:284`). `current_user()` is only a background reconcile;
owner affordances are additive decoration in CSS-reserved slots on the untouched
DOM, never a branch switch. The personalized cockpit is its own route, `/app`;
`/` stays public.

### Crates, features, and the build

`web` is one crate compiling two ways by cargo feature — `csr` (the wasm client)
and `server` (the server-side data-API build; renamed from `ssr`)
([ADR-0041](adr/0041-public-projector-and-csr-client.md)) — declared at
`web/Cargo.toml:50-64`. The `csr` crate is the wasm entry point and owns its own
`mount()`/`main()` (`csr/src/lib.rs:34,54`); there is no `web::mount_csr`.

Cargo features select capabilities; target `cfg`s select platform code. Features
unify within one resolved Cargo graph, so a downstream or dev dependency may
activate a capability for every copy of that crate in that build. The production
boundary is therefore the resolved target graph, not one manifest viewed alone.

| Capability                                     | Enabled by                                                                                           | Build where it belongs        | Purpose                                                                                 | Enforcement                                                                                                             |
| ---------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ----------------------------- | --------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `web/csr` → `client/csr`                       | `csr`                                                                                                | wasm                          | Browser UI and Leptos client plumbing.                                                  | CSR build, wasm clippy/tests, size gate.                                                                                |
| `web/server`                                   | `server`                                                                                             | host                          | Server-function bodies and their Axum/storage dependencies; this is not SSR.            | Host clippy/tests and server-function gates.                                                                            |
| `common/sanitize`                              | `host`                                                                                               | host production               | Adds `ammonia`; establishes the `RenderedHtml` invariant.                               | CSR resolved graph plus wasm build/budget; `rendered-html-compiler-boundary` checks the production constructor surface. |
| `common/sqlx`                                  | `storage`                                                                                            | host production               | Adds common-owned `SQLx` bridges required by trait ownership.                           | `common-host-target-closure` rejects it in CSR.                                                                         |
| `host/sqlx`, `storage/sqlx`                    | Their default features                                                                               | host production               | Enable derive-generated bridge impls; their `SQLx` dependencies are already host-owned. | Host clippy and dual-backend tests.                                                                                     |
| `common/{test-support,test-utils}`             | Downstream dev-dependencies                                                                          | tests only                    | Expose shared fixtures and cross-crate test hooks.                                      | Consumer test builds; compiler boundary keeps fixtures out of default production dependencies.                          |
| `host/{test-support,test-utils,cheap-kdf}`     | `storage`, `web`, and `server` dev-dependencies                                                      | tests only                    | Forward common fixtures and enable host test hooks or cheap password hashing.           | Host and consumer test builds; optimized-build `cheap-kdf` compile guard.                                               |
| `storage/{test-support,test-utils,seed-posts}` | Integration-test dev-dependencies; the `test-support` binary enables only `seed-posts` in production | host tests or the seed binary | Provide the dual-backend harness, mocks/hooks, and the lightweight post-seeding recipe. | `test-local`, backend-pattern gate, and seed-binary smoke tests.                                                        |

`test-support` is overloaded only lexically: the workspace **crate** is the
out-of-process seed/capture executable, while each crate's `test-support`
**feature** exposes that crate's in-process fixtures to downstream tests.
`cfg(test)` exposes fixtures to a crate's own tests; the feature is needed
across a crate boundary. `common-host-target-closure`, the host/wasm compile
lanes, and the isolated `rendered-html-compiler-boundary` check the load-bearing
production boundaries; test gates exercise the explicitly enabled test surfaces.

`cargo xtask build-csr` compiles `csr` to wasm and hands the artifact to
`devtool csr-bundle` (wasm-bindgen + `wasm-opt -Oz`), landing
`jaunder.{js,wasm}` in `target/site/pkg/`
(`xtask/src/steps/build_csr.rs:42-53`). The server compiles in the SPA shell
(`web::app::SPA_SHELL`, itself `include_str!("csr/index.html")` —
`web/src/app/render.rs:51`) and falls back to it for anything the projector and
the static routes do not claim (`server/src/lib.rs:101-111`), keeping
ADR-0003/ADR-0008's single binary intact. The bundle is fetched after the JS
glue and is **not** preloaded: `render_head` carries no `<link rel="preload">`
(`web/src/app/render.rs:93-95`), a measured decision under a pre-registered
abort rule rather than an oversight ([ADR-0121](adr/0121-no-wasm-preload.md)) —
the trial collapsed the serial pre-fetch window but bought no boot total, so it
was reverted. The `WASM_URL` / `GLUE_URL` constants and their drift guards
(`web/src/app/render.rs:63-65,284-298`) survive it, because a preload URL
drifting from the `init()` target would not fail — it would silently
double-download.

The wasm artifact carries a hard budget
([ADR-0106](adr/0106-wasm-raw-size-budget.md)): `cargo xtask validate` fails
when the **raw** byte count of `pkg/jaunder.wasm` exceeds
`WASM_RAW_CEILING_BYTES` (2 340 000 today, `xtask/src/wasm_budget.rs:39`). Raw,
not compressed, because the artifact is a compiler input rather than a download;
the ceiling keeps explicit headroom that sits below what the next weaker
optimisation level would produce, and a unit test asserts that relationship so
widening it is deliberate.

### Module layout — the per-vertical file split

Each vertical splits host and wasm code at the **file** level inside the single
`web` crate ([ADR-0070](adr/0070-web-vertical-wasm-only-component-files.md),
which supersedes ADR-0056 and is a deliberate partial **return** to ADR-0055's
module-level gating — ADR-0055's own status is unchanged, having been superseded
by ADR-0056 before either):

- `mod.rs` — module wiring and re-exports only, no items of its own (now a
  workspace-wide rule rather than a per-vertical one — see
  [Workspace](#workspace));
- `api.rs` — the vertical's request wire types and **every** `#[server]` fn,
  dual-compiled; presentation DTOs that carry their assembly logic may live in a
  sibling model leaf instead;
- `server.rs` — `#[cfg(feature = "server")]` host-only helpers;
- `component.rs` — the `#[component]` UI, declared
  `#[cfg(target_arch = "wasm32")]`;
- plus ungated, host-tested, coverage-measured state/model/logic files
  (`compose_state.rs`, `input_state.rs`, `model.rs`, `state.rs`, `render.rs`,
  …).

`web/src/pages/` is gone. Of the 28 directories under `web/src/`, all have
`mod.rs`, 25 carry a `component.rs`, 15 carry an `api.rs`, and 6 (`audiences`,
`auth`, `error`, `posts`, `subscriptions`, `timeline`) need a `server.rs`. The
three without a `component.rs` — `error`, `reactive`, `taglist` — are a
wire-type home, a primitive, and a pure-markup helper: none has UI, so the
absence is structural, not lag. Two mechanisms enforce the layout. The
`target-arch-placement` xtask check
(`xtask/src/steps/target_arch_placement_check.rs`, policing `web/src`,
`client/src` and `csr/src`) admits a `target_arch` gate in exactly two shapes:
an inner `#![cfg(…)]` in a `lib.rs` — the whole-crate gate `client` and `csr`
use — or an outer attribute on a `mod` **or `use`** item in a `mod.rs` or
`lib.rs`. Anywhere else, including a `component.rs` file header, fails. The
second mechanism is `#[macros::server]`, which hard-errors on any `#[server]` fn
outside `web/src/<vertical>/api.rs` (`macros/src/server_fn.rs:22,69`). ADR-0070
carries forward the two rules the earlier module-level split established: pure
logic keeps a host-tested, coverage-measured home, and no fake-value host stubs,
ever.

### Server-fn surface, DI, and errors

Every `#[server]` fn is written `#[macros::server]`, and the macro derives what
the source already states: the wire `endpoint` `/<vertical>/<fn ident>` and the
ADR-0011 span name, both from the file path and identifier, and it **refuses**
an author-supplied `endpoint` or `name` (`macros/src/server_fn.rs:87-131,176`).
That refusal is the guard, not a gate: with the hand-written literal gone there
is no drift left for a static check to find
([ADR-0120](adr/0120-no-endpoint-drift-check.md)). The wire URL is therefore
`/api/<vertical>/<op>` ([ADR-0082](adr/0082-server-fn-wire-namespace.md)),
served by one axum route, `/api/{*fn_name}` (`server/src/lib.rs:61`). Because
the URL _is_ the ident, the naming rule is a wire rule: the vertical's own noun
is dropped (`audiences::create`, not `create_audience`) and the ident is
verb-led. `/api/*` is the CSR client's private protocol; the public stable API
is AtomPub.

Server fns get their dependencies via per-trait Leptos context, never a bundle —
`expect_context::<Arc<dyn FooStorage>>()`
([ADR-0016](adr/0016-dependency-injection-and-appstate.md)), e.g.
`web/src/audiences/api.rs:66`. The macro also wraps each body in
`crate::error::server_boundary` (`macros/src/server_fn.rs:304`); there is no
hand-written `boundary!` call. Server integration/router tests call
`server/tests/helpers/registrar.rs::ensure_server_fns_registered()`, which
initializes the sole explicit list of
`server_fn::axum::register_explicit::<web::…>()` calls
([ADR-0066](adr/0066-server-fn-test-registrar-guard.md), amended #848). The
host-side `server-fn-registrar` gate parses both the `web` server-fn inventory
and that list, so an omitted registration fails before it can silently 404.
ADR-0016's SSR-era owner-pinning addenda have been retired: the sole server-fn
invocation path, `leptos_axum`'s `/api` handler, holds the owner strong for the
whole call, so no `ScopedFuture` wrapper and no sanctioned `Resource`
constructor exist — components call `Resource::new` directly (13 files across
`web/src`), and no clippy `disallowed-methods` entry bans it — `clippy.toml` has
no `disallowed-methods` entry at all; it only _relaxes_ `unwrap`/`expect` for
tests, which the workspace otherwise denies (`Cargo.toml:141`).

**Revision History is an owner-only web surface.** The `posts` vertical exposes
authenticated `list_history`, `get_post_history`, and
`get_revision_history_detail` server functions for `/history`,
`/posts/{post_id}/history`, and `/posts/{post_id}/history/{revision_id}`
respectively. Storage binds the owner in every global, per-Post, and
exact-detail query; unauthenticated callers are rejected at the authentication
boundary, while absent Posts, foreign Posts, and mismatched Revision/Post pairs
share the same masked result rather than relying on public visibility checks.
The sidebar History entry and each active Post's History action lead to the
cursor-paginated global/per-Post lists; the per-Post screen shows current state
alongside immutable snapshots, and the detail renders its stored rendered HTML
only through the existing trusted sink. Deleted Posts remain reachable only
through these owner checks ([ADR-0136](adr/0136-local-post-lifecycle.md)).

`web/` is a **thin shell**
([ADR-0059](adr/0059-thin-web-shell-error-layering.md)): it keeps only the
leptos UI, the `#[server]` surface, and the wire types. Errors flow through the
one-way T1→T2→T3 pipeline — typed domain errors (`storage`/`common`) → the
operator carrier `host::error::InternalError` (`host/src/error.rs:94`) → the
wire type `WebError` (`web/src/error/wire.rs:12`), via the lossy projection
`project` in `web/src/error/server.rs:68`. T2→T3 is a security boundary made
structural: the operator payload is absent from the type that crosses the wire,
so the masked public boundary
([ADR-0017](adr/0017-error-handling-and-the-public-boundary.md)) cannot leak by
discipline failure.

Wire args are **domain newtypes, validated client-side against the same
newtype's `FromStr`** ([ADR-0065](adr/0065-client-side-domain-validation.md)) —
never a re-implemented rule. The chokepoint is the pure `forms::field_error<T>`
(`web/src/forms/field.rs:11`) driving a parent-owned `Field<T>`
(`web/src/forms/field.rs:22`). Standard labelled fields render through
`<ValidatedInput<T>>` / `<ValidatedTextarea<T>>`; bespoke direct-bind layouts
use the shared bare-input and touched-error primitives so only chrome and
placement stay caller-owned. The chrome the labelled shells wrap themselves in,
`Labelled` (`web/src/forms/component.rs:56`), is deliberately **not** generic
over `T`: it takes the validity as two erased signals (`error`, `touched`)
rather than a `Field<T>`
([ADR-0117](adr/0117-labelled-takes-erased-signals.md)), because a generic
component _with children_ needs its close tag to match the opening generics
token-for-token at every call site. ADR-0117 records as an open question whether
that burden alone still justifies the shape. The visible message is gated on a
`touched` flag; submit is gated disable-until-valid. Typing the arg moves
validation into arg-**decode**, so a malformed request from a non-browser client
fails before the fn body — the defense-in-depth path, not the user path.

[Cohesive request aggregates](adr/0129-request-aggregate-server-function-inputs.md)
are the server-fn boundary rule: multiple caller-supplied values forming one
cohesive operation cross as one typed request aggregate. Wasm forms give
`forms::server_action_submit` one constructor for the generated action input;
the adapter derives disabled state and dispatch from that constructor and owns
pending-state and native-submit/default-prevention wiring. The constructor
assembles parsed fields before `ServerAction` dispatch. Aggregates exclude
ambient request context and injected dependencies; native `<form>` submission
retains submit and Enter-key behavior without `ActionForm`'s string harvest and
redundant client-side decode.

The current population is `LoginRequest`, `RegistrationRequest`,
`CreateInviteRequest`, `ConfirmPasswordResetRequest`, `RenameAudienceRequest`,
`AudienceMembershipRequest`, and `DeleteMediaRequest`. Audience add/remove share
`AudienceMembershipRequest` because their fields and member identity coincide;
ordinary and forced media deletion share `DeleteMediaRequest`, differing only in
its `force` value.

This is a semantic boundary rule, not an arity rule. The remaining `ActionForm`s
each carry one domain value: audience create/delete, email and password-reset
requests, post publish/delete, and subscription subscribe/unsubscribe. Other
multi-argument server fns keep direct parameters for independent settings
(`backup::update_settings`, `profile::update`, `site::update_identity`),
independent lookup/filter/pagination dimensions (`media::list_mine`,
`posts::get`, `posts::list_drafts`, `tags::list`, and the `timeline::list_*`
family), or a separate target plus an already-aggregate payload
(`posts::update`). `posts::create` already takes `PostInputs`, and
`media::upload` takes `MultipartData`. No static check guesses cohesion.

The same migration extends `proffered-secret` without weakening its directional
boundary: an inbound-secret field is admitted only on a `*Request` type named by
a server-function parameter, while a wasm-only vertical `component.rs` may name
one only as `Field<Proffered*>`, its validated input renderer, or an explicit
`parse::<Proffered*>()` for dispatch staging. Returns, response DTOs, helpers,
and every other occurrence remain rejected by the gate.

### Reactive idioms

Revalidation goes through one primitive, `web::reactive::Invalidator`
(`web/src/reactive/invalidator.rs:11`,
[ADR-0060](adr/0060-web-invalidator-revalidation-idiom.md)): committed mutations
`notify()`, resources `track()`; `action::<A>()` is success-gated;
cross-component scopes are per-vertical `invalidator_scope!` newtypes
(`web/src/reactive/scope.rs:28`). Keyed lists whose rows mutate in place or hold
nested state render from a `reactive_stores::Store` (`#[store(key: …)]`) fed by
`client::reactive::patched` (`client/src/reactive.rs:52`, driven by the
vertical's `Invalidator::track`) plus a keyed `<For>` mounted unconditionally;
flat lists stay plain `map`/`collect`
([ADR-0061](adr/0061-web-keyed-list-reactive-store.md)). `audiences` is the sole
adopter so far (`web/src/audiences/component.rs`). A domain newtype used as a
**leaf** field of such a store row is declared as itself and given the derive's
per-field escape hatch, `#[patch(|this, new| *this = new)]`
(`web/src/audiences/model.rs`,
[ADR-0078](adr/0078-reactive-store-domain-newtype-fields.md)) — which keeps
`common` free of a `reactive_stores` dependency. The attribute is for leaves
only; a field wrapping a nested `Store` needs granular descent instead.

A reactive widget splits into a host-compiled state module and a wasm-only view,
and the render decision is a **fold, not a closure**
([ADR-0083](adr/0083-reactive-paint-fold.md)): the state exposes
`paint(context) -> WebResult<Paint>` and the component body is a `Memo` plus a
bare `match`, one arm per variant. Failure travels on `Result`'s error axis;
per-caller variation travels as a data enum
(`NoIdentity { Blank, Redirect(_) }`), never a `ViewFn` prop; chrome that must
survive a transition is emitted from its own memo-gated sibling region rather
than repeated inside each arm. The first instance is
`web/src/timeline/state.rs:62,88,226`. Everything except `Effect::new` and
`spawn_local` lands host-side, so transitions and the decision are
coverage-measured.

A form control's `disabled` state and the payload it dispatches must come from
**one call** ([ADR-0113](adr/0113-submit-gate-owns-its-parse.md)): the dispatch
closure receives an already-validated value and has no error arm to swallow.
`posts::compose_state::submit_gate` (`web/src/posts/compose_state.rs:156`) is
the realization — a plain function, not a component — and it owns the single
`if let Some(body) = body.parsed()` arm so form authors never write one. Gating
on `is_valid()` while taking the payload from `parsed()` is two sources and is
prohibited; the composer's and editor's `slug_override` and `summary` fields are
the named outstanding debt against that target state (#907).

Within a live SPA session there are **no full document loads**
([ADR-0076](adr/0076-no-full-load-spa-navigation.md)): navigation is
`leptos_router` (`use_navigate()` / `<a>`). The `no-full-reload` `devtool check`
definition runs an ast-grep rule from root `ast-grep/`, discovered by root
`sgconfig.yml`, over Rust in exactly `web/src` and `client/src`. It rejects only
the result of a `.location()` method call chained to `replace`, `assign`,
`reload`, or `set_href`; there is no allowlist or exemption file. Its
location-bearing diagnostic directs callers to `leptos_router`'s
`use_navigate()` and [#592](https://github.com/jaunder-org/jaunder/issues/592).
The companion `ast-grep-tests` definition runs the committed native fixtures.
The host xtask static-check mechanism and Nix `static-checks` derivation invoke
both definitions (`devtool check --all --sandbox-cargo` in Nix), rather than
maintaining a host-only source scanner
([proposed devtool ast-grep enforcement](adr/0161-devtool-owns-ast-grep-enforcement.md)).
The SPA user namespace is `~`-prefixed: the permalink route's leading segment is
a custom `TildeUsername` route match (`web/src/route_segments.rs:13`, wired at
`web/src/app/component.rs:151`) that matches only a `~`-leading segment,
mirroring the server's literal-`~` projector routes. The tightening is
deliberately partial — the other username-first routes stay plain param
segments.

The style companion is `docs/web-style-guide.md`.

## Observability

Observability is OpenTelemetry end to end: one trace correlates the e2e runner,
browser, and backend; metrics ride the same exporter; a scoped diagnostics
stream gives e2e failures a low-noise "look here first" artifact. Operational
how-to lives in [observability.md](observability.md).

### Traces

The backend emits spans via `tracing` + `tracing-opentelemetry`; shared
host-process setup lives in `host::telemetry`
([ADR-0011](adr/0011-unified-observability.md),
[ADR-0058](adr/0058-host-crate-layering.md)). The server, production CLI
commands, and `test-support` all hold the same `host::telemetry` guard for
process-wide OTLP setup and shutdown. `server::observability` owns server-scoped
HTTP tracing and e2e diagnostics. `host::telemetry::init_tracing` installs the
OTLP tracer only when `JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT` (fallback
`OTEL_EXPORTER_OTLP_ENDPOINT`) is set; with no endpoint every emit is a no-op,
and exporter-setup failure is non-fatal. `with_http_observability`
(`server/src/observability.rs`) layers the per-request tracing span onto the
router, together with a `tower-http` `x-request-id` that it mints when absent
and propagates onto the response. Inbound W3C `traceparent` headers are
extracted onto the per-request span, so backend spans parent into the caller's
trace. Span fields and metric attributes are exported, so they MUST NOT carry
user PII or secrets — stable identifiers (`user_id`, `error.kind`) only. Branch
determinants follow the same rule: record bounded decisions and stable internal
IDs, never passwords, tokens, raw emails, invite codes, request bodies,
arbitrary source text, or whole-struct dumps. The
[isolated E2E browser-diagnostic payload decision](adr/0168-isolated-e2e-browser-diagnostic-payloads.md)
is a deliberately narrow proposed exception: the disposable Playwright harness
may export raw diagnostic payloads containing synthetic application values, but
production browser code installs no console/page-error listener and production
telemetry remains PII- and secret-free. The exception excludes real-user
deployments and infrastructure credentials; browser diagnostics observe failures
without failing tests.

The
[bounded transient-data retention decision](adr/0167-bounded-transient-data-retention.md)
adds PII- and secret-free structured OpenTelemetry signals at transient-data
state transitions: expiry, consumption, completion, exhaustion, and cleanup. The
operator, rather than Jaunder, owns long-term telemetry retention.

### Server-fn span names are macro-derived

Every `#[server]` fn in `web/src` is written as `#[macros::server]`, which emits
the `#[tracing::instrument]` attribute itself with the name
`web.<vertical>.<ident>` computed from the file path and identifier
(`macros/src/server_fn.rs`) — so no server-fn span name is written in source and
none can drift ([ADR-0011](adr/0011-unified-observability.md), amended
2026-07-30). Hand-written `instrument` names still exist outside that set —
`require_auth` carries one (`web/src/auth/server.rs`) — because they are not
server fns. The macro rejects `level`, `err`, `ret`, and any unmodeled key. It
accepts `fields(…)` only as empty declarations such as
`registration.policy = tracing::field::Empty`; server-function bodies may later
record bounded determinant values into those declared fields.

The macro always hides original parameters with generated `skip_all`, then
records each named, non-skipped parameter only through
`common::trace_field::TraceField::trace_value`. The associated projection type
makes the exact `Debug` representation reviewable; compiler trait resolution is
the default-deny admission check. Implementations retain ADR-0011's four
grounds—intrinsically bounded, operator configuration, already public in a
permalink, or `Username`—and add no generic string, `Debug`, or `Display`
fallback. Authors still write `skip(…)` / `skip_all` as explicit opt-outs.
`server-fn-tracing` now checks source grammar, skip names, pattern-bound
parameters, and declaration-only fields, but classifies no type names
([sink-specific telemetry interfaces](adr/0156-sink-specific-telemetry-interfaces.md)).

Wire parse errors likewise own their sink projections: `user_message` selects
feedback for the submitter and `telemetry_code` supplies a bounded
classification. Intentionally retained third-party detail crosses only through
`common::UserFacingMessage`, whose `Debug` is redacted and which implements
neither `Display` nor `TraceField`. Each `UserFacingMessage::from_external` call
requires an immediately preceding, reasoned `server-fn-wire-arg-error:allow`
marker; the static gate derives the census and rejects stale, shared, unmarked,
or orphan doors. Decode telemetry remains fixed and source-free: public
`invalid request arguments`, `stage = "decode"`, and no submitted value or
detailed user message.

The earlier arrangement — the gate writing the `name = "…"` literal into
`web/src`, and a `server_fn` field on the boundary log event — was retired with
that addendum. Nothing under `web/src` is rewritten by `cargo xtask check` for
span naming any more.

### Decision-path fields

A span name identifies the operation; fields describe the bounded facts and
branch determinants for that operation. Determinants live on the narrowest span
that owns the decision. Parent request/server-function spans carry request-wide
decisions; child spans remain appropriate for called work that is worth timing,
is reused by multiple parents, or has determinants/failures of its own.

Determinant fields are declared per instrumentation site, not globally. A span
declares the fields it may later record, usually with `tracing::field::Empty`,
then records each value when the code reaches the decision. Branch-specific
child span names are avoided: e.g. `web.registration.register` carries
`registration.policy`, `registration.invite_present`, and
`registration.outcome`, while invite-backed registration has the separate child
operation span `storage.account_mutations.register_with_invite`.

`host::error::InternalError` captures a `tracing_error::SpanTrace` at
construction time, while the active span stack still exists. Boundary failures
emit that snapshot as `error.span_trace` beside `error.kind`, `error.class`,
`error.public`, `error.source`, and `error.context`; native swallowed-error
reporting emits the same active span context. Client-swallowed telemetry remains
bounded-client-data-only because the server intake no longer has the browser's
span stack. SpanTrace is operator-only and never crosses the
`InternalError -> WebError` projection. The current retention decision is
collector-side tail sampling: Jaunder emits determinant fields and SpanTrace,
but does not buffer and dump branch logs in-process for successful non-slow
spans
([docs/adr/0147-decision-path-observability.md](adr/0147-decision-path-observability.md)).

### E2e traces

E2E tracing is layered ([ADR-0011](adr/0011-unified-observability.md)): an
automatic `e2e.test` span per test plus opt-in `e2e.flow.*` semantic-phase spans
(`end2end/tests/perf.ts`). Trace context flows via `JAUNDER_E2E_TRACEPARENT`
(`flake.nix` → `end2end/tests/otel.ts`), so browser-side and backend spans share
one trace.

**Capture and attribution are two separate things**
([ADR-0096](adr/0096-e2e-trace-capture-vs-attribution.md)). Client-side perf
capture — `addInitScript`, `exposeBinding`, and the request / navigation /
resource / long-task listeners — attaches once per browser _context_ through
`attachTraceCapture` (`end2end/tests/capture-trace.ts`), called from both the
auto fixture and the `tracedContext` fixture, so every page is instrumented by
one code path. The per-test `traceparent` is applied later, and switches a
phase-tagged sink from `pretest` to `test`, so nothing a fixture does before the
test body lands in `e2e.test`'s arrays.

`e2e.test` keeps its original span id, range, and attributes. Fixture-lifecycle
time is measured by an envelope nested around it in
`end2end/tests/performance.ts`; `fixtures.ts` remains the sole ordered
composition and test-export surface:

```
e2e.test.lifecycle
├── e2e.context_mint
├── e2e.test            (unchanged span id, range, and attributes)
│   └── … server request spans
├── e2e.page            (one per extra tracedContext page)
└── e2e.teardown
```

An `e2e.warmup` child existed under the envelope until the per-test warmup was
removed ([ADR-0099](adr/0099-e2e-does-not-pre-warm.md)); the `pretest` phase it
occupied is now normally empty, and the flow-coverage orphan bucket
([ADR-0081](adr/0081-empirical-server-fn-flow-coverage.md)) with it. A residual
is unmeasurable by construction: Playwright tears fixtures down in reverse
order, so the span build and the OTLP POST run before `context.close()`.

In the e2e VMs an otel-collector writes `otel-traces.jsonl` into the capture dir
([ADR-0057](adr/0057-e2e-capture-dir-contract.md), #332). Each VM also copies
out `playwright-report-<backend>.json` (Playwright's `results.json`) and the
service and system journals alongside the capture tarball, per the
[ADR-0037](adr/0037-e2e-failure-diagnostics-capture.md) rule that artifacts are
copied before the Playwright exit is asserted. `cargo xtask traces analyze`
consumes the trace files offline (see the tooling section).

For host-side gates that reconcile Playwright execution with trace evidence, the
lifted Playwright JSON report defines the executed project population. The
consumer requires an exact project-set match with the trace-derived evidence, so
a project-wide capture blackout cannot disappear from the denominator and an
unexpected project cannot inflate it. Artifacts are still copied before a
Playwright failure is propagated; reconciliation runs only after the E2E
combination succeeds, preserving the primary failure
([Playwright report population authority](adr/0165-playwright-report-defines-trace-gate-population.md)).

### Measurement frames are not mixed

The browser measurements arrive in two clocks, and a decomposition is computed
entirely within one of them
([ADR-0100](adr/0100-measurement-frames-are-not-mixed.md)). The **document
frame** — `performance.mark` and `PerformanceResourceTiming`, relative to that
document's `timeOrigin` — is what `capture-trace.ts`'s `harvestDocument` returns
and the only frame boot analysis decomposes: `bootTotalMs` is `timeOrigin` to
`jaunder.boot.mount_done`; its exclusive parts are `timeOrigin → init_start`,
`init_start → boot.entry`, and the Rust boot phases, which sum to it by
construction. Direct WebAssembly API and enclosing wasm-initialization
durations, their successful path, and wasm resource timing are overlapping
diagnostics, never added to that decomposition. The **Node frame** —
`Date.now()` stamps taken in the Playwright driver — carries `committedMs`,
`mountedMs`, and `commitToMountMs`. `commitToMountMs` is still reported as the
bridge to suite wall-clock but is never decomposed; the difference
`commitToMountMs - bootTotalMs` is reported separately as **frame skew** and
charged to the harness (`frame_skew_ms` in `xtask/src/traces/boot_phases.rs`).
The two frames differ by cross-process, plausibly engine-asymmetric lag, so
mixing them would charge harness IPC to the app's boot phases.

### Metrics

An OTLP `MeterProvider` is installed next to the tracer (`build_otel_meter` in
`server::observability`), behind the same endpoint gate
([ADR-0011](adr/0011-unified-observability.md)). Emits go through the
`host::metrics` facade, which lives in `host` **unconditionally** — no `metrics`
Cargo feature — because `host` is native-only and therefore keeps
`opentelemetry` out of the wasm closure by crate structure
([ADR-0058](adr/0058-host-crate-layering.md)). Helper arguments are bounded
enums, or a `&'static str` drawn from a closed set the call site cannot widen —
`atompub_request`'s `op` comes from a matched-route-plus-method lookup
(`server/src/atompub/router.rs:61`), not from an enum. Either way no call site
can attach caller-supplied text as a label. `init_tracing` returns a
`#[must_use]` `TelemetryGuard` whose `Drop` force-flushes both providers on
every exit path, so one-shot CLI commands export buffered telemetry instead of
silently dropping it; one binding at the `run()` dispatch boundary covers every
command, and export failures are logged, never propagated
([ADR-0011](adr/0011-unified-observability.md)).

Serve-only saturation gauges are registered in `host::metrics` as asynchronous
OpenTelemetry gauges backed by a narrow `SaturationSnapshot`. `jaunder serve`
starts the sampler only when the shared OTLP endpoint gate is configured, and
`PreparedServer` owns both the observable guard and the sampler handle for the
serve lifetime. Every 30 seconds, including an immediate first collection, the
sampler reads feed queue depth, backup last-success time, database pool
saturation, and database-declared media upload bytes into the snapshot.

`jaunder.media.storage_bytes` remains the database-declared upload total for
accounting and quota semantics. The distinct `jaunder.media.filesystem_bytes`
saturation gauge is a physical-drift diagnostic: its periodic collector walks
the complete `<storage_path>/media` tree and sums the logical length of every
regular directory entry. This includes cached, temporary, orphaned, and
future-descendant files; independently hard-linked entries each count their own
length. It does not report allocated blocks, deduplicated physical storage, or
filesystem quota usage.

OpenTelemetry callbacks only synchronously read the snapshot and never query
storage. The filesystem walk runs through Tokio blocking work outside every HTTP
request path; the sampler awaits it before another collection begins, so no two
walks overlap. A missing or unreadable path, traversal or metadata error,
symlink, or non-regular non-directory entry fails the whole filesystem sample:
the collector reports the bounded `server.metrics.media_filesystem_bytes`
diagnostic, clears that snapshot field, and emits no datapoint rather than zero
or a partial value. The database pool observer is produced by storage opening
and retained beside `AppState` at the serve composition root, preserving
ADR-0016's rule that `AppState` remains storage-only while still allowing pool
metrics.

### Errors at the boundary

The carrier owns its boundary observability:
`InternalError::emit_boundary_failure` (`host/src/error.rs`) logs six discrete
tracing fields — `error.kind`, `error.class`, `error.public`, `error.source`
(the preserved typed chain, rendered once), `error.context`, and
`error.span_trace` (the active span stack captured at error construction) — at
the level derived from the error class, and emits the `jaunder.errors` metric.
The metric is an error-event counter whose bounded attributes include kind/class
plus `error.disposition = boundary | swallowed` and
`telemetry.origin = server | client`; it is not a unique-root-cause count and it
does not carry `error.span_trace`. `server_boundary` (`web/src/error/server.rs`)
calls it with `boundary/server`, then performs only the outward wire projection
and returns the masked public error. Which fn failed is not a field: the event
is raised inside the enclosing `web.<vertical>.<ident>` span, and both
configured sinks render span context
([ADR-0011](adr/0011-unified-observability.md),
[ADR-0017](adr/0017-error-handling-and-the-public-boundary.md)).

An unexpected native failure that deliberately preserves the primary result goes
through one `host::error` reporting interface, which couples the fixed warning,
active span trace, and `swallowed/server` metric so a caller cannot forget
either side. Diagnostic self-failure uses only its console/stderr fallback;
routing it through the same reporter would recurse.

The authenticated raw client-telemetry intake accepts one versioned, closed-enum
JSON event of at most 1,024 bytes from the Rust WASM client. A dedicated guard
accepts only the browser `session=` cookie, not Bearer or Basic/app-password
credentials. A per-user token bucket admits a burst of five and refills one per
minute; a one-entry-per-bucket round-robin ring prunes full buckets idle for 15
minutes in bounded 64-entry passes. The dedicated guard does not emit the
general `session_validation` metric. An accepted event becomes the fixed warning
plus a `swallowed/client` metric; a 429 leaves only generic HTTP request
observability and status.

The client logs locally first and permits one credentialed keepalive request in
flight; concurrent events are dropped, never queued or persisted. Delivery is
best-effort and carries no arbitrary source text or user value. This is a
bounded diagnostics transport, not a browser OTel SDK or direct OTLP exporter.
All client data remains untrusted operational evidence.

### Scoped server diagnostics (e2e capture)

`JAUNDER_CAPTURE_DIR` controls app-driven capture
([ADR-0049](adr/0049-app-driven-scoped-server-diagnostics.md),
[ADR-0057](adr/0057-e2e-capture-dir-contract.md)). At the relevant executable or
command root, its raw value is resolved once into an optional, valid-only
`CaptureDirectory`: absent or trim-blank input disables server capture, while a
configured non-Unicode or uncreatable directory is an error that aborts `serve`
or the `test-support` capture command. Construction prepares the directory once;
the constructed value is thereafter usable. `reset-mail` and `capture-path`
require that value and fail loudly when capture is disabled.

Each stream receives only its pure, infallibly projected leaf path; downstream
code neither reads `JAUNDER_CAPTURE_DIR` nor performs capture-directory lookup
or preparation. The filenames are defined once in `host::capture` (`mail.jsonl`,
`websub.jsonl`, `diag.log`). The diag stream is a WARN+-filtered JSON `tracing`
layer plus a panic hook appending `kind: "panic"` JSONL records through its own
`O_APPEND` handle (bypassing `tracing` to avoid deadlock). The shared
`test_support::panic_gate` verifier
([ADR-0032](adr/0032-e2e-zero-panic-gate.md)) receives the diagnostic leaf path,
scans raw bytes from the union of that stream and a required server log, and
de-duplicates by panic location with the scoped record winning. Per combo the
e2e harness tars the directory out as `capture-<backend>.tar.gz` — those three
files plus `otel-traces.jsonl` — into the
[ADR-0037](adr/0037-e2e-failure-diagnostics-capture.md) artifact set.

### Committed direction

- Saturation gauges via async observable callbacks — deferred in
  [ADR-0011](adr/0011-unified-observability.md).
- A configurable diag level and an analyzer over the diag JSONL — left open in
  [ADR-0049](adr/0049-app-driven-scoped-server-diagnostics.md).
- Decomposing `commitToMountMs` at all requires first measuring frame skew per
  engine and subtracting it. Nothing does today; adding one is a decision to
  revisit ([ADR-0100](adr/0100-measurement-frames-are-not-mixed.md)).

## Deployment

Jaunder deploys as a **single self-contained server binary behind an external
reverse proxy** ([ADR-0008](adr/0008-deployment-model.md)). It never terminates
TLS itself — HTTPS is the reverse proxy's job (nginx, Caddy, …), so Jaunder
binds plain HTTP (`--bind`, default `127.0.0.1:3000`, `server/src/cli.rs:267`)
and production exposure is a proxy-configuration concern, not an application
feature.

**What is inside the executable.** Two `rust-embed` trees, so **no external file
is needed to serve the client** ([ADR-0003](adr/0003-asset-management.md)).
Request handling still touches disk for user data — media blobs are opened per
request from the storage path (`server/src/media.rs`) — and the process writes
its sole runtime identity to `<storage>/runtime.json` and may read a PostgreSQL
password file; see "Outside the binary" below.

Startup ownership is an OS-backed exclusive `runtime.lock` keyed only by the
storage directory. `serve` acquires it before transient cleanup and retains it
through shutdown, so two processes cannot clean the same `media/tmp`. Before
cleanup it writes the canonical `<storage>/runtime.json` identity; that initial
reservation is fatal on failure. If the file identifies a live process through
its JSON `pid` plus process start time, startup refuses before cleanup. The
pre-bind reservation uses port zero; discovery consumers treat it as not ready
and reread until the bound nonzero port is published. Address updates are
best-effort but preserve the live reservation on failure. Graceful shutdown
first stops background admission and drains every admitted job and active
measurement, then removes the canonical identity before releasing the lock;
forced process exit removes the identity and lets the OS release the lock. The
e2e and Elisp harnesses read that canonical file for this port handshake. This
retains ADR-0035's discovery contract while the
[bounded transient-data retention decision](adr/0167-bounded-transient-data-retention.md)
qualifies its JSON-as-mutex behavior and removes ADR-0144's runtime-path
override.

- `StaticAssets` (`server/src/assets.rs:3-5`, `#[folder = "assets/"]`) carries
  the base stylesheets `jaunder.css` and `jaunder-themes.css`, mounted at
  `/style` by `axum_embed::ServeEmbed` (`server/src/lib.rs:54,57`), which
  supplies ETag and conditional-request handling.
- `Site` (`server/src/site.rs:33-35`, `#[folder = "$OUT_DIR/site"]`) carries the
  CSR client: `pkg/jaunder.{js,wasm}`, their precompressed `.br`/`.gz` siblings,
  the wasm-bindgen `snippets/`, and the `public/` assets flattened to the site
  root (`public/favicon.ico` → `favicon.ico`, `server/build.rs:125-127`).
  `server/build.rs` stages that tree at compile time from
  `JAUNDER_CSR_BUNDLE_DIR` (Nix) or `target/site/pkg` (host build).

Only the two base stylesheets are embedded. ADR-0003 also anticipated
**user-uploadable** stylesheets served from the storage layer; that was never
built, and nothing in `storage/` or the config-key registry handles CSS.

- The SPA shell is the compile-time constant `web::app::SPA_SHELL`
  (`web/src/app/render.rs:51`, `include_str!` of `csr/index.html`), served as
  the fallback for unknown paths (`server/src/site.rs:126-128`).

`ServeEmbed` does no `Accept-Encoding` negotiation, so `site::serve_site` is a
hand-written handler: it picks br/gzip/identity against the embedded variants,
sets `Content-Type` from the logical path, and emits a per-representation `ETag`
with `304` on `If-None-Match`. Only the data directory and the database live
outside the binary, so **"single binary" holds without qualification.** This was
not always true: until #237 (closed 2026-07-17) the wasm bundle was served from
an on-disk site root by `ServeDir`. The wasm bundle's size is gated separately
on raw bytes ([ADR-0106](adr/0106-wasm-raw-size-budget.md)). Rendering
architecture — leptos-CSR client plus the server-side public projector — is
owned by the web section ([ADR-0040](adr/0040-web-rendering-leptos-csr.md),
[ADR-0041](adr/0041-public-projector-and-csr-client.md)).

**CLI surface.** The `jaunder` binary is also the operations tool
(`server/src/cli.rs:233-382`): `serve` runs the server; `init` prepares the
storage directory and database; `create-pg-db` bootstraps a PostgreSQL database;
`user-create`, `user-invite`, and `app-password-create` manage accounts;
`smtp-test` verifies mail configuration; `backup` (directory or archive mode)
and `restore` round-trip the data, with the backup target auto-derived from the
storage configuration ([ADR-0064](adr/0064-backup-target-auto-derivation.md),
[ADR-0054](adr/0054-backup-test-homing-and-uniform-restore-failure.md)); and
`site-config set/get/list/unset` reads and writes site settings.

**Transient-data cleanup.** The
[bounded transient-data retention decision](adr/0167-bounded-transient-data-retention.md)
requires database-backed transient data to have authoritative semantic expiry at
`cutoff <= now`, with physical removal once at startup and daily thereafter.
Each run receives one explicit `now` and drains eligible backlogs through
repeated fixed-size statements that release locks between batches. A database
cleanup failure is reported, does not stop later domains in the same run, and
retries during the next scheduled run. Before uploads are accepted, startup
clears `media/tmp`; failure to clear it is fatal. The policy excludes durable
Posts, revisions, tombstones, and referenced media; non-expiring sessions and
App Passwords; `feed_cache`; and external captures, and deliberately creates no
generic retention framework.

`site-config` is not a free-form door. Its `key` argument is host-owned
`SiteConfigKey`, so clap rejects an unknown key at parse time, and each key
carries the validator that `set` runs before any row is written
([ADR-0102](adr/0102-config-key-closed-registry.md); the registry macro is
`host/src/config_key.rs:58,139`). `list` is the deliberate exception: it dumps
every stored row, flagging keys outside the registry as `UNKNOWN KEY` and
recognised keys holding unparseable values as `INVALID`, so legacy rows stay
visible.

`posts.default_audience` declares that `DefaultAudience` type directly in the
same registry. `SiteConfigStorage` exposes the closed type at its getter/setter
boundary; an absent or unparseable stored row defensively reads as `Private`,
while database errors propagate. The stored tokens and parser come from the
closed-enum convention rather than a config-specific matcher
([ADR-0091](adr/0091-text-enum-closed-string-enum-convention.md)).

Deployment is configured by clap flags with matching `JAUNDER_*` environment
fallbacks and documented defaults
([process configuration](adr/0144-process-configuration-cli-contract.md), as
qualified by the
[bounded transient-data retention decision](adr/0167-bounded-transient-data-retention.md)).
The process-shape variables are `JAUNDER_BIND` (listen address, `:267`),
`JAUNDER_DB` (database URL, default `sqlite:./data/jaunder.db`, `:41`),
`JAUNDER_STORAGE_PATH` (the data directory, default `./data`, `:33`),
`JAUNDER_ENV` (`dev` | `prod`, `:271`), and `JAUNDER_VERBOSE` (`:25`).
PostgreSQL takes its secret by either `JAUNDER_DB_PASSWORD` or
`JAUNDER_DB_PASSWORD_FILE`; the file source wins over the variable, and either
wins over an embedded URL password. All runtime environment inputs — including
the observability variables covered under [Observability](#observability) — are
resolved once at an executable, command, or test-harness composition root into
narrow typed configuration, then injected into the subsystems that own them.
Library modules neither reread ambient configuration nor receive a general
environment reader or process-config bundle
([peripheral process configuration](adr/0158-peripheral-process-configuration.md)).
`prod` is load-bearing in two places: it sets the `secure_cookies` flag passed
to `create_router` (`server/src/commands.rs:546`, `server/src/lib.rs:32`), and
it disables the dev-only auto-initialization of a missing database on `serve`
(`server/src/commands.rs:501-512`).

**What the flake ships.** `flake.nix` exports `packages.jaunder` (the deployable
server binary), `packages.site`, and `nixosModules.jaunder`
(`flake.nix:247-249`, `1059-1062`). `packages.site` is **no longer a deployment
artifact** — the binary embeds the bundle — and is retained only so
`cargo xtask audit-wasm` can build `.#site` and inspect the bundle for size
analysis (`flake.nix:464-473`,
[declarative NixOS deployment and package outputs](adr/0142-declarative-nixos-deployment-package-outputs.md)).
The `services.jaunder` module (`flake.nix:44-118`) creates a dedicated `jaunder`
user/group, runs under systemd from `StateDirectory=jaunder` with
`WorkingDirectory=%S/jaunder`, passes `bind` and `db` through unconditionally
and `JAUNDER_ENV=prod` only when `prod` is set (`flake.nix:95-101`), runs
`jaunder init --db "$JAUNDER_DB" --skip-if-exists` in `preStart`
(`flake.nix:105`), and starts `jaunder serve`. It has no module option for
PostgreSQL password injection; operators supply `JAUNDER_DB_PASSWORD[_FILE]`
through the service manager when needed. There is no site symlink; the module
comment names #237 as the reason. Two `nixosConfigurations` test VMs
(interactive, PostgreSQL) exist for development only.

## Emacs client

The Emacs client is the reference authoring client: it publishes org-mode
buffers over AtomPub plus the jaunder wire extensions
([ADR-0023](adr/0023-atompub-jaunder-wire-extensions.md); the Protocols section
owns the wire format). It lives in the top-level `elisp/` directory as a single
`jaunder` package, a first-class but separately-tested subproject
([ADR-0031](adr/0031-elisp-separately-tested-subproject.md)) with a self-booting
live-server integration harness, `jaunder-test--with-live-server`
([ADR-0035](adr/0035-elisp-live-integration-harness.md)) — the testing section
owns both. The floor is `Package-Requires: ((emacs "29.1"))`
([ADR-0042](adr/0042-emacs-org-atom-mapping-struct-seam.md)). The
[Elisp stateless coverage gate](adr/0162-elisp-stateless-coverage-gate.md)
censuses every production Protocol Client module and top-level source form
before testing, then reconciles every executable Edebug point with exactly one
LCOV record. A zero-stop form with exactly its single synthetic opening-line
point is structural only for `require`, `provide`, `declare-function`,
`defgroup`, and `cl-defstruct`; or `defvar`, `defconst`, and `defcustom` with an
absent, `nil`/`t`, number, string, character, keyword, quote/function-quote, or
literal vector initializer. Computed calls, variable references,
backquote/unquote, and all other evaluated or unknown initializers remain
measurable or require a marker; an ordinary point or LCOV observation on a
structural candidate fails the guard. Test files, helpers, runners,
vendored/generated sources, and byte-compiled files remain outside the census
and denominator.

`elisp/jaunder.el` is the umbrella entry point — it holds the package headers
and nothing but `require` forms for the feature modules (`elisp/jaunder.el:30`):
the format-neutral entry IR (`jaunder-entry.el`, the
`cl-defstruct jaunder-entry`), blog config and request context
(`jaunder-config.el`), soft authoring warnings (`jaunder-warn.el`), timezone
handling (`jaunder-datetime.el`), the wire encoder/response harvester
(`jaunder-atom.el`), the org document interface (`jaunder-org.el`), HTTP
(`jaunder-transport.el`), the service-document capability probe
(`jaunder-service.el`), media (`jaunder-media.el`), and the user commands
(`jaunder-publish.el`).

### Transport and auth

`jaunder--http-request` (`elisp/jaunder-transport.el:94`) is built on `plz`,
which drives the `curl` binary. `url.el` itself is not used for requests; only
`url-parse` is pulled in, to extract the host for the auth-source lookup
(`transport.el:54`) and to validate a configured base URL
(`config.el:25,118-120`). 4xx/5xx return as a `(:status :headers :body)` plist,
unsignalled; a transport-level `plz-error` carrying a response is converted to
the same plist, and one carrying none re-signals
([ADR-0038](adr/0038-emacs-http-transport-plz-not-url-el.md)). Because `plz`
writes headers into a curl `--config` file without escaping,
`jaunder--curl-header-value` (`elisp/jaunder-transport.el:84`) backslash-escapes
`\` and `"` — without it a strong `ETag` echoed back as `If-Match` is truncated
to nothing and the precondition never reaches the server.

Authentication is the server's app-password Basic scheme
([ADR-0014](adr/0014-atompub-authentication.md); the auth section owns details).
`jaunder--auth-secret` (`elisp/jaunder-transport.el:72`) resolves the app
password through Emacs `auth-source`, keyed on the active blog's URL **host**
(port excluded) and username, requests at most one match, and errors when no
entry matches
([Emacs auth-source App Password storage](adr/0143-emacs-auth-source-app-password-storage.md)).
`jaunder--basic-auth-header` UTF-8-encodes `user:password` before base64 per
RFC 7617.

### Org → Atom mapping

`jaunder--org->atom` (`elisp/jaunder-org.el:125`) takes no arguments: it maps
the current org buffer, non-mutatingly, to a `jaunder-entry` struct. A separate
`jaunder--atom-entry->xml` (`elisp/jaunder-atom.el:34`) renders the wire
`<entry>` by building a `dom` node and calling built-in `dom-print` — the struct
seam keeps the mapping pure-data-testable, catches field-name typos at
byte-compile time, and confines all wire knowledge (namespaces, media types,
element order, the `app:control`/`app:draft` marker) to that one serializer
([ADR-0042](adr/0042-emacs-org-atom-mapping-struct-seam.md)). Per-entry content
carries the `text/org` media type and the server canonicalizes
([ADR-0023](adr/0023-atompub-jaunder-wire-extensions.md); the Protocols section
owns it).

`jaunder--harvest-response-fields` (`elisp/jaunder-atom.el:69`) is the one
response reader — a metadata harvest, not a full Entry parse — returning
`content-src`, `content-type`, `slug`, and `published` from a response Entry via
`libxml-parse-xml-region`. Media URLs come from that harvested `<content src>`
and are never reconstructed client-side, so the server stays authoritative about
URL layout ([ADR-0045](adr/0045-emacs-media-content-src.md)).

Media candidates are body-only Org `file:` and `attachment:` links. `file:`
targets resolve against the live authoring buffer's `default-directory`;
`attachment:` targets resolve through org-attach. Header properties, fuzzy
links, HTTP(S) links, and other non-local link types are not candidates. After
resolution, content type is selected case-insensitively from the deterministic
map `jpg`/`jpeg` → `image/jpeg`, `png` → `image/png`, `gif` → `image/gif`,
`webp` → `image/webp`, `svg` → `image/svg+xml`, `mp3` → `audio/mpeg`,
`ogg`/`oga` → `audio/ogg`, `flac` → `audio/flac`, `wav` → `audio/wav`, `mp4` →
`video/mp4`, `webm` → `video/webm`, and `pdf` → `application/pdf`; unknown and
extensionless names use `application/octet-stream`.

The client also probes the AtomPub service document for the
`<j:extension features="…">` capability list that
[ADR-0023](adr/0023-atompub-jaunder-wire-extensions.md) defines; the probe is
cached per base URL and, when `format-media-type` is absent, emits one
suppressible warning per session per blog rather than blocking the publish
(`elisp/jaunder-service.el:32`, `:69`).

### Publish orchestration

The `jaunder-blogs` defcustom (`elisp/jaunder-config.el:32`) is the sole blog
configuration: it maps directories to `(:base-url :username [:format])` plists,
resolved by longest-prefix match on the buffer's directory
(`jaunder--blog-entry-for`, shared by publish and `jaunder-new-post`) and
validated loudly by `jaunder--resolve-blog` — absolute base URL with a non-empty
host, non-empty username, unmatched directory errors, trailing slashes stripped
([ADR-0047](adr/0047-emacs-publish-orchestration.md)). Alongside it sit three
`jaunder-warn-*` toggles for the soft authoring-hygiene warnings (zone mismatch,
untracked media, missing `format-media-type`), none of which ever block a
publish. Commands bind the private `jaunder--active-blog` special through
`jaunder--call-with-blog`; the transport reads it only through
`jaunder--active-base-url` / `jaunder--active-username`, which error when no
blog is active ([ADR-0047](adr/0047-emacs-publish-orchestration.md)).

The user-facing commands are `jaunder-new-post`, `jaunder-publish`, and
`jaunder-save-draft` (publish forced to `app:draft`).

`jaunder-new-post` resolves its target before creating a local Post, then
collects title, repeated Tag labels, and publication state before writing the
Org metadata block. Tag completion reads the Posts Collection's inline
categories from the authenticated AtomPub Service Document; discovery failures
remain visible but degrade to free-text entry so local authoring stays
available. A prefix argument preserves prompt-free minimal-template creation: it
uses the longest matching blog, rejects a nonempty configuration with no
matching root, and falls back to `default-directory` only when the client has no
configured blogs (`elisp/jaunder-publish.el`, `elisp/jaunder-service.el`).

Publish performs all network mutation before any destructive local change
(`elisp/jaunder-publish.el:307`): map → validate (non-empty body; a `scheduled`
Post needs a future `#+DATE:`) → record the machine zone → media localization →
Entry send → write-back → rename to `<slug>.org`. Media localization first
collects candidates, then aggregates every missing, unreadable, or non-regular
resolved path into one preflight error before warning or uploading; it next
emits the untracked-media warning, uploads each equal resolved path once, and
applies right-to-left positional substitution using the response
`<content src>`. The Entry send is a `POST` create, or a `PUT` when `JAUNDER_ID`
is present, carrying `If-Match` only when the buffer also records a
`JAUNDER_SYNCED` ETag. Write-back persists `JAUNDER_ID` first, from the
`Location` header, before `JAUNDER_SLUG`, `JAUNDER_SYNCED`, `JAUNDER_SYNCED_AT`,
the resolved publish time, and the rename — so any failure, including a `412`
stale-ETag, is recoverable by a plain re-publish
([ADR-0047](adr/0047-emacs-publish-orchestration.md)). Media substitution
applies to the sent body only; the authoring buffer is never modified.

Creates go through `jaunder--create-with-retry`
(`elisp/jaunder-publish.el:279`), which retries a response-less signalled
`plz-error` or a returned 5xx response. `jaunder--http-request` converts a
response-bearing `plz-error` into the ordinary response plist, so a signalled
`plz-error` at this boundary is transport failure; non-transport failures such
as a missing `auth-source` entry propagate immediately
([Emacs auth-source App Password storage](adr/0143-emacs-auth-source-app-password-storage.md)).
Transport retry uses up to three attempts with one- then two-second backoff
under **one** `Idempotency-Key`, so the server dedups the replay. The key is
ephemeral, not stable across invocations: it is a fresh md5 of local entropy per
call, so a later re-publish gets a new key and an edit is never mistaken for a
retry. The server side of that contract was decided in issue
[#79](https://github.com/jaunder-org/jaunder/issues/79) as a follow-on to
ADR-0047 — see the Storage section.

### Pull, reconcile, and durable local media

`jaunder--atom->org` (`elisp/jaunder-pull.el`) maps one authenticated AtomPub
Member response to deterministic Org-file bytes: fixed metadata-header order,
native source body, numeric Post ID, canonical slug, strong ETag sync marker,
and one captured wall-clock/zone pair. `jaunder--pull-member` validates the
inventory identity against the response, blocks an occupied root-level
`<slug>.org` before network work, and installs through a same-directory
temporary file without overwrite. Inventory exhausts Collection pagination and
joins root-level Org files to Members by Post ID; `jaunder-reconcile` reports
divergence without resolving it, previews only server-only pulls, and applies
them after one confirmation. Remote deletion remains the separate explicit,
ETag-guarded `jaunder-delete-post` command.

#### Committed direction: Local Media Copies

Pulled Org, Markdown, and HTML source localizes only format-aware link
destinations that name canonical public media on the active Jaunder origin and
contain no user information or query. Media GETs are anonymous and do not follow
redirects. The authenticated Member response and every media response must carry
exactly one canonical `X-Jaunder-Instance` UUID, and all values must match.
Computed bytes, the strong `"sha256-<hash>"` response ETag, and the canonical
URL hash must agree. External links and non-link URL text remain unchanged
([Local Media Copies](adr/0160-emacs-pulled-media-local-copies.md)). Markdown
localization delegates CommonMark meaning to the pinned upstream `cmark-el` AST.
Because the parser exposes block positions but not inline destination spans,
`jaunder-pull-media.el` retains a bounded lexical source adapter: it scans only
AST-authorized ranges and accepts only exact destinations present in the AST.
The adapter locates bytes but cannot authorize link semantics; it does not
implement block classification such as fences, containers, raw HTML blocks, or
paragraph interruption. Its source-syntax and reference-map compatibility seam
is pinned to the packaged cmark-el revision
([Local Media Copies](adr/0160-emacs-pulled-media-local-copies.md)).

Verified bytes become durable **Local Media Copies** at
`local-media/<sha256>/<decoded-filename>` under the configured root; native link
targets use the canonical percent-encoded filename exactly once to resolve that
leaf. The root is trusted, author-owned local state. Path creation and immediate
mutations reject symlinks and non-directory components, staging is exclusive,
and copies are never overwritten. A malicious replacement after Emacs's final
check remains out of scope because Emacs Lisp has no dirfd-anchored mutation.
Existing copies are hash-verified before reuse. A pull stages and verifies all
distinct media, installs Local Media Copies, rewrites native links to relative
local targets, and atomically installs the Post last. Failure leaves the Post
server-only, so rerunning reconciliation retries it. Verified copies installed
before an ordinary failure or crash remain safe to reuse. There is no rollback,
cache eviction, matched-Post repair, arbitrary external download, or multi-file
transaction promise.

## Domain types and invariants

The rules in this section are cross-cutting: they govern `common`, `host`,
`storage`, `web`, and the CLI alike, so they have no single subsystem home. Ten
accepted ADRs decide them. Filed under whichever subsystem happened to use them
first, several went undocumented for months; they are collected here.

### What earns a newtype, and the trailer it gets

A domain value earns a newtype when at least one of three axes applies
([ADR-0063](adr/0063-domain-value-newtype-convention.md) §1): an **invariant** a
bare primitive cannot express, a **transposition hazard** (another value of the
same primitive type is a plausible mis-pass), or a **trust boundary** (a
semantic guarantee that must not be forged). Consistency alone is not sufficient
justification for introducing one — but §5 makes _adopting_ an existing newtype
mandatory on every field, argument, return, and DTO that carries its value;
flattening it back to a primitive takes express owner approval. A genuinely
polymorphic value is an enum, not a string newtype.

The trailer is generated, never hand-written. Three derives live in the `macros`
crate — `StrNewtype` (`macros/src/str_newtype.rs`), `IdNewtype`
(`macros/src/id_newtype.rs`), `NumNewtype` (`macros/src/num_newtype.rs`). For a
string newtype the derive emits the serde bridge, `Display`,
`AsRef`/`Borrow`/`Deref<str>`, `PartialEq<str>`, owned conversions, and
`PartialOrd`/`Ord`; only the validating `FromStr` stays hand-written, because
the rule is the one per-type part. `Deref<Target = str>` is the single
sanctioned use of deref polymorphism in the repo — it retires the `.as_str()`
tax that made the pre-derive newtypes too thin to propagate. Attribute options
select the profiles (`macros/src/str_newtype.rs:464-479`): `secret` tightens the
surface to a redacting `Debug` plus `AsRef<str>` (`host::Password`), while
`secret, serde` re-opens only the validating serde bridge for the dual-target
inbound `common::ProfferedPassword`; `secret, sqlx` re-adds storability, and
`no_sqlx, no_ord` gives the bearer-token `RawToken` the full ergonomic trailer
minus storability and ordering (`common/src/token.rs:109`). Numeric IDs take the
fixed `IdNewtype` trailer (eight of them, `common/src/ids.rs:15-44`); bounded
numeric values take the parameterized `NumNewtype` one, whose bound is
declarative and re-run by `FromStr`, serde, and the column (`PageSize`,
`PageOffset`, `RowLimit` — `common/src/pagination.rs:29,62,81`), with an opt-in
`clamp` flag (`macros/src/num_newtype.rs:430`) for a public bound that should
coerce rather than reject.

String-backed domain values use one validating generated trailer. The
invariant-first question remains “is there a string this type should refuse?”,
not “does the constructor reject?” — the latter is a property of code already
written, and reading it as evidence about the value mislabelled `PostTitle`,
`PostBody`, and later `SubscriberRef`. A type for which no input is invalid can
use `FromStr::Err = Infallible`; it does not need a separate macro mode.
`#[str_newtype(infallible)]` was removed after its sole production adopter,
`SubscriberRef`, gained its non-blank invariant
(`docs/adr/0151-subscriber-reference-invariant.md`,
[ADR-0101](adr/0101-infallible-kind-is-invariant-first.md)).

ADR-0101 also replaces trusted doors with typed proof wherever a caller can
supply one. `PostSummary` applies that shape directly in its derived-summary
constructors: `from_title` accepts a `PostTitle`, and `from_body_line` accepts a
`PostBody`; each source already proves non-blankness, so these constructors only
coerce the length half of the summary invariant. They share one internal
boundary-aware truncation helper, which prefers sentence then word boundaries
before a hard Unicode-scalar cap.

### Identity and label are two types, not one

A domain value that carries a canonical identity _and_ a preserved presentation
variant at different cardinalities is two composable newtypes, paired only where
both travel ([ADR-0068](adr/0068-tag-identity-label-split.md), which amends
ADR-0063's one-type-per-value shape). Tags are the applied case: `Tag`
(`common/src/tag.rs:19`) is the lowercased canonical slug — one interned row,
the browse key, the dedup key, the SQL key — while `TagLabel`
(`common/src/tag.rs:61`) is the case-preserving label, one per _tagging_.
`TagLabel::slug` (`common/src/tag.rs:86`) is infallible by construction, because
`TagLabel`'s `FromStr` validates through `Tag`'s rule: one validity source
(`TagValidationError`, `common/src/tag.rs:105`), no re-implemented validator.
Equality and dedup on labels go by slug, never by raw casing. The pattern
generalizes; tags are its only adopter so far.

### Closed string enums

A closed string-backed enum is declared with one attribute, `#[text_enum(…)]`
([ADR-0091](adr/0091-text-enum-closed-string-enum-convention.md),
`macros/src/text_enum.rs`). It injects `strum`'s
`AsRefStr`/`Display`/`EnumString`/`IntoStaticStr` and the
`parse_err_ty`/`parse_err_fn` pair, and generates the named parse error, its
parse fn, `Serialize`/`Deserialize`, and — with the opt-in `sqlx` flag
(`macros/src/text_enum.rs:302`) — the storage bridge. It is an attribute rather
than a derive because a derive cannot add attributes to its item, and it must be
the item's first attribute, since an attribute macro sees only what is written
below it. Fourteen enums adopt it, eight of them with `sqlx`: `PostFormat`
(`common/src/render.rs:26`), `TargetKind` and `DefaultAudience`
(`common/src/visibility.rs:43,180`), `MediaSource` (`common/src/media.rs:601`),
`SmtpTlsMode` (`common/src/smtp_tls_mode.rs:18`), the host-owned `UserConfigKey`
and `SiteConfigKey` (`host/src/config_key.rs:206,91`), and host-owned
`FeedEventStatus`.

The attribute owns the _convention_; `strum` owns the _engine_ — token mapping,
`Display`, `FromStr`, `VariantArray`, `EnumMessage`
([ADR-0075](adr/0075-adopt-strum-retire-str-enum.md)). ADR-0075 established that
by retiring the bespoke `StrEnum` derive, which had duplicated ~300 lines of a
crate already in the tree on the false premise that `strum` could not produce a
named, host-registrable parse error. `StrEnum` is deleted: `macros/src/` carries
no `str_enum.rs`. The named `Invalid<Name>` unit error it produced is preserved,
now generated without `thiserror` so an adopting crate needs no dependency
beyond `strum` — its unit-struct shape is load-bearing for `host`'s
`validation_from!`.

### How a typed value crosses a boundary

Values are parsed at the **outermost** boundary — `#[server]` argument and
return types, CLI argument types, storage record fields and trait signatures —
and held inward (ADR-0063 §4).

**The database.** Every derive-based newtype is a first-class column type: the
derives emit a transparent, feature-gated `sqlx::Type`/`Encode`/`Decode` bridge
delegating to the inner value, plus an opt-in Postgres `PgHasArrayType`
([ADR-0071](adr/0071-sqlx-string-newtype-bridge.md)). `query_as` therefore
decodes straight into the newtype, so `query_as::<_, (PostId, TagId, …)>` makes
a swapped destructuring a compile error where two adjacent bare `i64`s made it
invisible. `Decode` re-validates for a string newtype and re-runs the bound for
a `NumNewtype`; it is an infallible wrap for an `IdNewtype`, which has no value
invariant. `Encode` is a storability capability, not a conversion — which is why
`secret` drops the bridge by default and `no_sqlx` exists. Feature isolation
keeps `sqlx` out of the wasm build, guarded by a `compile_error!` in `common`.

At the storage write boundary, the sealed explicit `StorageBind` registry admits
approved domain and persistence-role values by exact Rust type, independently of
each backend's SQLx representation capabilities. References, `Option`, vectors,
and slices preserve only an approved leaf; this retains PostgreSQL's existing
`PgHasArrayType` slice-array capability without adding a SQLite abstraction.
Native `Query`, `QueryAs`, and `QueryScalar` use `bind_storage`; native
`QueryBuilder` and `Separated` use `push_storage_bind`. Those extension traits
are the only normal value-admission APIs and directly delegate to SQLx, keeping
the native execution/fetch surface. The registry does not infer SQL-column
meaning: exact helper and storage-trait signatures retain wrong-role safety
([typed storage bind admission](adr/0169-typed-storage-bind-admission.md)).

`sqlx-newtype-bind` is the residual defense-in-depth detector. It parses every
Rust source file under `storage/src`, including test-support code, and fails
closed if its root or input cannot be read or parsed. It rejects raw bind,
builder-bind, prebuilt-argument, native-argument, and SQLx query-macro syntax;
the sole raw admissions are the typed seam's five exact direct delegations. It
follows source-visible aliases and treats uncertain receiver syntax
conservatively, but does not claim rustc type resolution, call-graph analysis,
arbitrary proc-macro expansion, or SQL-column inference; SQLx query macros are
forbidden under the root for that reason. The separate decode gate structurally
enumerates readable targets and accepts only declaration-backed bridge types,
approved foreign types, or composites whose leaves it polices; it has no
primitive or site-exception path. Intentional persisted values therefore use
explicit role-specific types, and custom row policy decodes a fully policed
intermediate row before conversion
([decision record](adr/0163-sqlx-decode-approval-is-type-only.md)). Since
ADR-0091 there is exactly one bridge implementation,
`macros/src/sqlx_bridge.rs:67`, driven by a `BridgeSpec`; the three newtype
derives, `#[derive(SqlxBridge)]`, and `#[text_enum(sqlx)]` all call it.

Because `Decode` re-validates, a row written under an older grammar or corrupted
can fail it — and on a bulk read **one bad row must not stop the scan**
([ADR-0122](adr/0122-one-bad-row-must-not-stop-the-scan.md)). Scans and lists
decode per row and skip the failures (`list_media` fetches raw rows rather than
`query_as` for exactly this reason, `storage/src/media.rs:275-325`;
`feed_urls_needing_catchup` at `storage/src/posts.rs:1918-1937`), so a single
unusable row costs only itself instead of 500-ing a media list or wedging the
feed worker's `last_tick` forever. Three guardrails bound it: direct single-row
lookups (`get_media`, `find_by_hash`) stay strict; the feed-event claim's
diversion to the purge list is **column-scoped** — only a `feed_url` decode
failure may divert, since `purge_corrupt` DELETEs and a
treat-any-error-as-corrupt wrapper would widen a destructive path from one
column to ten (`storage/src/feed_events.rs:57-79`, #728); and a row whose own id
will not decode fails the batch. Dual-backend tests assert the skip/purge
behaviour per site.

**Time.** `common::time::UtcInstant` is the domain type for absolute UTC
instants at both the web and storage boundaries. It is a minimal Chrono-backed,
instant-backed newtype: transparent serde retains its RFC 3339 wire form,
`FromStr` canonicalizes offsets to UTC for the client-side `Field<T>` path,
`now()` centralizes wall-clock construction, and its existing `value()`/`From`
conversions remain available
([ADR-0072](adr/0072-timestamps-cross-boundary-as-utcinstant.md);
[storage-owned instants use UtcInstant](adr/0153-storage-owned-instants.md)).
Storage records and traits, private rows/cursors/inputs/dialects,
`BackupManifest`, and storage fixtures carry `UtcInstant`; existing
role-specific wrappers over it remain intact. Its plain SQLx bridge and
dual-backend coverage preserve SQLite/Postgres schemas, physical values,
backend-specific precision, and timezone semantics. Public-read APIs likewise
take an explicit `UtcInstant` `now`, preserving ADR-0027's visibility behavior.
`UtcInstant` remains Chrono-backed: Chrono's soft deprecation makes the named
type a smaller future migration seam, not a claim of complete implementation
isolation or a Jiff migration; Jiff has no native SQLx integration. Durations,
local wall-clock values, `SystemTime` suffixes, SQL physical types and values,
and non-storage protocol representations remain outside this decision.

**URLs.** The `url` crate is the sanctioned absolute-URL parser and normalizer,
and it is a direct dependency of `common` (`common/Cargo.toml:24`) — which means
it is compiled for wasm and reachable in the client binary, a cost accepted in
exchange for one correct normalization chokepoint no boundary can bypass
([ADR-0073](adr/0073-url-crate-for-absolute-url-normalization.md)). Hand-rolled
normalization and repurposing `urlencoding` as a parser are ruled out. The
chokepoint is `TaggedUrl<T>`'s `FromStr` (`common/src/tagged_url.rs:106-110`),
which parses through `url::Url`.

Because a URL role costs only a marker struct and a type alias, URLs are an
express exception to ADR-0063 §1's cost model: "consistency alone is not
sufficient justification" must not be cited to argue a role out of existence
([ADR-0112](adr/0112-role-tagged-site-urls.md)). A host-less root-relative
reference is a distinct grammar, not a `TaggedUrl` — it is `RootRelativeUrl`
(`common/src/root_relative_url.rs`), decided under #560 rather than by an ADR.

### Absence is named where it can occur

Where a row can genuinely be absent, the code names it; everywhere else it is
left alone ([ADR-0108](adr/0108-absence-is-named-at-its-source.md)).
`MissingRow { what }` (`storage/src/error.rs:28`) is a standalone error naming
an absent required row, and `RequireRow::require_row`
(`storage/src/error.rs:53,62`) is its one-line partner on an `Option`. It still
pages — a required row that is absent is a real invariant violation — but the
operator learns _which_ row instead of reading `"storage operation failed"` with
`"no rows returned"` buried in the source chain
(`From<MissingRow> for InternalError` routes through `InternalError::server`,
`storage/src/error.rs:34-43`). `fetch_one` stays correct and is not discouraged
where the row is structurally guaranteed — a bare aggregate, `SELECT EXISTS(…)`,
`INSERT … RETURNING` with no `ON CONFLICT` — and the blanket
`From<sqlx::Error> for InternalError` stays, so a `RowNotFound` arriving there
marks a caller defect. This is deliberately **not enforced**: a mechanical ban
on `fetch_one` was built and removed, because it cannot read SQL and so forced
~17 correct calls into `fetch_optional` plus a panic path. Whether a row can be
absent is a per-query judgement.

### The error model

Expected failures are typed, not collapsed into an opaque carrier: discrete
`thiserror` variants in `storage`/`common` enums — `UserAuthError`
(`storage/src/users.rs:60`), `PerformCreationError`
(`storage/src/post_service.rs:292`), `UpdatePostError`
(`storage/src/posts.rs:158`), `MailError` (`common/src/mailer.rs:45`), and some
two dozen more — so matching on `NotFound` versus `Unauthorized` versus
`SlugConflict` remains possible
([ADR-0017](adr/0017-error-handling-and-the-public-boundary.md) §1).

At the point of failure a cause is never flattened to a `String` (§3). A single
concrete source travels via `#[from]`/`#[source]`; a variant that legitimately
wraps unrelated error types carries `#[source] Box<dyn Error + Send + Sync>`,
which stays `downcast_ref`-able so the boundary can still classify a
`sqlx::Error` by SQLSTATE or pool timeout; and where there is genuinely no
underlying error object, the offending _value_ is carried as context rather than
a source being invented.

Preservation also governs continued-after-error paths. Expected validation or
domain rejection may become ordinary control flow; an unexpected infrastructure,
I/O, browser, subprocess, invariant, or decode failure must propagate with its
typed source. If an intentional degradation or preservation of the primary
result requires continuing, the site reports the failure before continuing and
explains why continuation is correct. This is a semantic judgement rather than a
ban on `.ok()`, `unwrap_or`, `let _`, `Err(_)`, or `map_err`, each of which also
expresses legitimate expected control flow
([ADR-0017](adr/0017-error-handling-and-the-public-boundary.md)).

Short-lived `xtask`/`devtool` processes do not install the application OTel
provider: a population/correctness failure fails the command, while a legitimate
ancillary or cleanup failure preserves the primary result and writes its typed
source with static context to stderr.

Internal detail reaches a client only through the masking boundary (§2). The
leaky public constructors `WebError::storage`/`WebError::server` are removed —
`WebError` (`web/src/error/wire.rs:12`) exposes no constructor that serializes a
raw source chain. The operator carrier is `InternalError`
(`host/src/error.rs:94`), which holds `kind`, `class`, `context`, the exact
public message, and the preserved `anyhow` source; the public message it masks
storage and server failures with is fixed (`"storage operation failed"` /
`"server operation failed"`, `host/src/error.rs:161-179`). These are the T1 and
T2 layers of the one-way error pipeline the Web frontend section describes; that
section covers the T2→T3 projection and why the boundary cannot leak by
discipline failure, and is not repeated here.

## Testing

`CONTRIBUTING.md` remains the how-to; this section records the architecture of
the test suites. The gates that run them are the next section.

### The dual-backend harness

The harness — the `Backend` enum, `TestEnv`, per-test DB provisioning, and the
rstest templates — lives inside `storage` as the `test_support` module, gated
`#[cfg(any(test, feature = "test-support"))]` (`storage/src/lib.rs:47-51`)
([ADR-0033](adr/0033-shared-db-test-harness-crate.md)). Its
`storage/src/test_support/mod.rs` facade declares the nine cohesive leaves
(`backend`, `feeds`, `invites`, `mail`, `media`, `post_service`, `postgres`,
`posts`, and `users`) and re-exports their public harness surface
([ADR-0128](adr/0128-mod-rs-assembles-module-surface.md)). `storage`'s own tests
reach it through `cfg(test)`; external test crates enable the `test-support`
feature. A separate crate is impossible: it must return `storage::AppState`, so
`storage`'s tests would dev-depend on a crate that depends on `storage`, and
`storage`'s own test target then links two distinct instances of itself
(`E0308: multiple different versions of crate storage`).

The four templates live with their backend provisioning in
`storage/src/test_support/backend.rs`: `backends` and `backends_matrix` (both
dual; the second is the `#[values]`-based variant), plus `sqlite_only` and
`postgres_only` (`backend.rs:560-590`). The backend axis of `backends_matrix` is
`#[values]`-based because a `#[case]`-based axis cannot coexist with a test's
own named `#[case]` rows; it composes as rows × backends, and the attribute
order is `#[apply(backends_matrix)]`, then the `#[case::name(..)]` rows, then
`#[tokio::test]` ([ADR-0124](adr/0124-rstest-reuse-cross-module-templates.md)).
Each template is `#[export]`ed, so it expands to a name-mangled `macro_rules!`
that a plain `use storage::test_support::backends;` brings into scope and
`#[apply(backends)]` then resolves **by bare name** — no
`#[apply(path::to::template)]` and no `pub use` re-export (ADR-0124;
`storage/src/test_support/backend.rs:572-590`). That is why the templates stay
in `storage::test_support` rather than moving to a consumer.

A storage test is homed by what it proves
([ADR-0053](adr/0053-storage-test-homing-and-dual-backend.md)): a backend-common
contract is written `#[apply(backends)]` and lives in the generic home module
beside the store it exercises, because a dual-backend test inside a dialect file
(`storage/src/sqlite/media.rs`) is self-contradictory. A single-backend test is
_presumed_ a Postgres coverage gap and converted, unless its subject is
backend-exclusive syntax or introspection — Postgres `CREATE ROLE`, SQLite
`PRAGMA`/`sqlite_master`. "Error path", "lazy/closed pool", and "the seed SQL is
written in one dialect" are explicitly not decisive reasons.

The `test-backend-pattern` guard (`xtask/src/steps/test_pattern_check.rs`)
enforces this over **both** `storage/src/` and `server/tests/`: every
`#[tokio::test]` must carry a backend template or one of two exemption markers,
`// guard:no-backend` or `// guard:low-level-db`, each with a reason
(`test_pattern_check.rs:83`). The second also exempts a low-level test from the
dialect-homing rule. It also checks placement — a dual template inside a dialect
directory, or a mismatched single template, is an error — and requires a
`// reason:` on a single-backend keep. Plain synchronous `#[test]` units are
never flagged.

The same reasoning governs test doubles
([ADR-0103](adr/0103-prefer-real-harness-over-mirroring-fake.md)): when a fake
would have to reproduce backend behaviour to be useful, use the real harness
instead. `InMemorySiteConfig` was deleted for that reason and its tests became
dual-backend. `MockSiteConfigStorage` stays, because its call sites assert
_non-interaction_ — a bare `::new()` panics if anything calls it, an assertion a
real store cannot express.

Test-only fault-injection hooks are gated on
`#[cfg(any(test, feature = "test-utils"))]`, not bare `#[cfg(test)]`, so
cross-crate integration tests can drive them dual-backend
([ADR-0026](adr/0026-test-fault-injection-hooks-feature.md); the live hook is
`hash_password`'s at `storage/src/helpers.rs:392`). Note that `test-utils` and
`test-support` are two different features: `test-utils` carries the mocks and
the injection hooks, `test-support` carries the harness.

### Server integration tests

The server integration tests are one binary, not six
([ADR-0067](adr/0067-server-integration-tests-one-binary.md)): `autotests` is
off and a single `[[test]]` target points at `server/tests/main.rs`, which
declares `mod helpers;` once and one module per subsystem (`atompub`, `feed`,
`misc`, `projector`, `storage`, `web`). `helpers` therefore compiles once, and
six crate-level `#![expect(clippy::unwrap_used, clippy::expect_used)]` collapse
into the one at `server/tests/main.rs:9`. The accepted cost is lost build
isolation — a compile error in any subsystem fails the whole target.

Backup is a cross-backend _contract_ (a portable dump), so its tests are homed
here rather than in `storage`
([ADR-0054](adr/0054-backup-test-homing-and-uniform-restore-failure.md)):
`server/tests/misc/commands.rs` holds the per-backend round-trips and negatives,
`server/tests/misc/backup_interop.rs` the cross-backend hops and the four-hop
`postgres→sqlite→postgres→sqlite` cycle — seeded from Postgres on purpose, so
every timestamp is pinned at microsecond precision from the first store and both
same-backend dump pairs are byte-comparable. A constraint-violating restore
fails uniformly on both backends: `BackupError::ConstraintViolation` with the
target unmodified (`storage/src/backup/error.rs`; Postgres maps its SQLSTATE
class at `storage/src/postgres/backup.rs:28`).

### The e2e suite

Each browser e2e check is a NixOS-test VM running Playwright against a real
served instance, one derivation per `{backend}×{browser}` combo (`mkE2eCombo`,
`flake.nix:969`). CI runs `cargo xtask validate --no-e2e` in the static job,
where the authoritative Emacs coverage verdict is decided, plus a
`{sqlite,postgres}×{chromium,firefox}` matrix — each job
`cargo xtask e2e <backend> <browser>` — aggregated by an `e2e-gate` that depends
only on that browser matrix. Branch protection therefore needs two stable names
([ADR-0034](adr/0034-ci-e2e-matrix-distribution.md)). Local
`cargo xtask validate` builds the browser-only `e2e-checks` aggregate instead:
the same derivations on one machine. It inherits the static lane's Emacs verdict
and does not rerun live ERT.

`end2end/playwright.config.ts` is the one config, loaded verbatim by both the VM
and the host loop ([ADR-0051](adr/0051-single-playwright-config.md)). For each
gated browser its project graph is `*-visual → ordinary → *-admin`: the visual
project selects the four existing `@visual` behavioral tests, disables retries,
and runs first against the combo's fresh database; ordinary excludes those
tests, and admin remains last. Chromium and Firefox each own one Linux baseline
per state under the owning spec's adjacent `*.spec.ts-snapshots/` directory. The
filename carries browser identity but not backend, so SQLite and PostgreSQL
compare the same expected image. WebKit has no visual project or baseline.

Exact comparison is a controlled rendering seam: Nix supplies one DejaVu-only
fontconfig universe to both host and VM browser processes, while
`end2end/tests/visual.css` applies screenshot-only font/animation/caret
stabilization. Comparisons allow zero differing pixels. The public timeline's
timestamp is the sole dynamic mask.

Normal host/VM differences are invocation flags set by the host driver —
`--reporter=html,line`, `PLAYWRIGHT_HTML_OPEN=never`, and
`JAUNDER_E2E_WORKERS=1` (the host serves a debug CSR build; the VM keeps the
config default of 2). The host loop is `cargo xtask e2e-local`
(`xtask/src/steps/e2e_local.rs`), which owns build, spawn on an ephemeral port,
seed, Playwright, stop/reap, and diagnostics verification. Its
`--update-visual-snapshots` mode builds release CSR once, then gives Chromium
and Firefox separate complete server/database/capture lifecycles. Server stderr
is streamed unchanged to the live terminal and a per-run file; stopping the
child closes the pipe so the driver can drain that journal-equivalent input
before invoking the shared zero-panic verifier.

Specs are parallel-safe by construction, via per-test identity fixtures in
`end2end/tests/provisioning.ts`, composed only by `end2end/tests/fixtures.ts`
([ADR-0039](adr/0039-e2e-parallelism-via-per-test-identity-fixtures.md)): `user`
provisions a uniquely-named account out of band, `mailbox` is a recipient-scoped
cursor-tracked mail waiter, `verifiedUser` adds the verification flow. Specs
that mutate the global site-config singleton are quarantined in per-browser
serial `*-admin` projects that run after the main projects — today that is
**two** specs, `admin-site` and `invite`
(`end2end/playwright.config.ts:72-105`).

The config also carries a `webkit` project, but the gate never runs it: both
`flake.nix:963-966` and the CI matrix enumerate chromium and firefox only. The
visual prerequisite runs inside those same four
`{sqlite,postgres}×{chromium,firefox}` derivations and CI jobs; it adds no
workflow lane or backend-specific baseline. Timeout budgets are stated for
Chromium and scaled per browser
([ADR-0012](adr/0012-environment-aware-timeouts.md)) — `slowBrowserTimeoutMs`
for an individual wait and the ambient whole-test budget,
`slowBrowserFirstNavigationTimeoutMs` for the coldest navigation.

Two rules bound what a spec may do to the page. First, authentication is
provisioned by seeding, never by driving the UI
([ADR-0098](adr/0098-e2e-seeded-auth.md)): `test-support seed-user` /
`create-session` mint the account and session, the cookie comes from
`host::auth::session_cookie_header` and the marker from
`common::session_user::encode_marker` — neither artifact is re-spelled in
TypeScript — and the helpers (`signInAsNewUser`, `signInAs`) do not navigate.
Second, each page performs exactly one document load, its entry
([ADR-0111](adr/0111-e2e-one-boot-per-page.md)): the `registeredPage` fixture
takes the entry path from the test and throws on a second call, all later
movement is in-app, and `end2end/tests/bootBudget.ts` enforces the per-`Page`
budget — raising at the next budget-aware call where it can, and sweeping the
rest at teardown (`takeBudgetFailures`).

The suite does not pre-warm ([ADR-0099](adr/0099-e2e-does-not-pre-warm.md)).
There is no warmup navigation, no `JAUNDER_E2E_WARMUP*` setting and no
`e2e.warmup` span, so every test's first navigation is a genuine cold load. A
once-per-worker warmup is not a shortcut around this: Playwright mints a fresh
context per test and the HTTP cache is not shared across `browser.newContext()`.

The suite is a zero-panic gate ([ADR-0032](adr/0032-e2e-zero-panic-gate.md)).
Both e2e surfaces invoke `test-support verify-no-panics`, whose Rust
implementation owns the default-empty allowlist, raw-byte union scan,
location-based de-duplication, and scoped-record preference. Each VM testScript
passes its materialized `jaunder.service` journal; the host loop passes its
drained stderr capture. Both preserve Playwright's status and verify afterward,
so a test failure cannot mask a server panic. A VM panic fails the _derivation_
and can never be cached green, while the copied journal remains an artifact on
every run, fresh or cached.

Diagnostics are captured before the check is allowed to fail
([ADR-0037](adr/0037-e2e-failure-diagnostics-capture.md)):
`trace: "retain-on-failure"` and `screenshot: "only-on-failure"`
(`end2end/playwright.config.ts:62-63`), and the shared `e2eRunAndCapture` helper
(`flake.nix:650`) runs Playwright capturing its exit, streams the line-reporter
output into `build.log`, copies every artifact out of the VM unconditionally,
and only then asserts the exit. On a failed build xtask rescues the bundle from
the `--keep-failed` outPath into `.xtask/diagnostics/<check>/`
(`xtask/src/steps/nix.rs:421`), best-effort so a copy failure can never fail a
gate.

Out-of-process state manipulation — seeding, fixture users, mail reset — goes
through the dedicated `test-support` workspace binary, which links the real
crates and drives the genuine storage code paths, never a production CLI or HTTP
surface and never hand-written per-backend SQL
([ADR-0046](adr/0046-test-support-seed-binary.md)). The shipped binary links
`storage` with only the lightweight `seed-posts` feature; the heavy harness is a
dev-dependency for its own smoke tests (`test-support/Cargo.toml:16,32`).
Capture streams write well-known filenames (`mail.jsonl`, `websub.jsonl`,
`diag.log`) under one `JAUNDER_CAPTURE_DIR`, lifted per combo as a tarball
([ADR-0057](adr/0057-e2e-capture-dir-contract.md) — see observability).

Retries are env-driven and default to 0; the CI/`validate` run sets
`JAUNDER_E2E_RETRIES=1`, so a test that fails then passes is reported `flaky`
rather than failing the check, with the JSON report still recording it
(`end2end/playwright.config.ts:11-17`; decided in #621, not in an ADR).

### Elisp testing

`elisp/` is a first-class, separately-tested subproject
([ADR-0031](adr/0031-elisp-separately-tested-subproject.md)): three elisp steps
— `ert`, `elisp-fmt` and `byte-compile` — run in both `check` (Fix) and
`validate` (Check), each as a `devtool check` step in the `static-checks`
derivation ([ADR-0052](adr/0052-devtool-unifies-static-checks.md),
`xtask/src/steps/static_checks.rs:17-18`); one `emacsForCi` toolchain
(`flake.nix:563`) serves both.

The [Elisp stateless coverage gate](adr/0162-elisp-stateless-coverage-gate.md)
runs the pure and live ERT observations in one hermetic NixOS producer and, for
every controlled outcome, realizes
`$out/elisp-coverage/{lcov.info,summary.txt,status.json}`. Its host consumer
reconciles the pre-test module/form census with current source and LCOV: each
ordinary Edebug point occurs exactly once. It automatically counts as
ignored/exempt, without a marker, only a zero-stop form with exactly its single
synthetic opening-line point that is `require`, `provide`, `declare-function`,
`defgroup`, or `cl-defstruct`; or `defvar`, `defconst`, or `defcustom` with an
absent, `nil`/`t`, number, string, character, keyword, quote/function-quote, or
literal vector initializer. Computed calls, variable references,
backquote/unquote, and all other evaluated or unknown initializers remain
measurable or require a trailing same-line `;; cov:ignore: <reason>` with a
trimmed non-empty reason. An ordinary point or LCOV observation on a structural
candidate fails the guard; malformed markers and markers on covered,
non-executable, or structural lines fail. Controlled producer statuses and
coverage findings fail the consumer; uncontrolled Nix or VM infrastructure
failures remain build failures. The check runs once in `validate --no-e2e` and
the CI static lane; full `validate` inherits its verdict without rerunning the
live suite.

Live client behavior — transport, auth, publish and media round-trips — runs
against a real server through the self-booting harness
([ADR-0035](adr/0035-elisp-live-integration-harness.md)):
`jaunder-test--with-live-server`
(`elisp/test/jaunder-integration-helper.el:222`) spawns the server, discovers
the port from the `runtime.json` file `serve` writes, and provisions credentials
via `jaunder app-password-create`. The suite
(`elisp/test/jaunder-*-integration.el`, driven by
`elisp/scripts/run-integration-tests.el`) remains available host-side via
`JAUNDER_TEST_BINARY` for fast iteration. The coverage gate's combined producer
runs the harness hermetically as the sole authoritative live-suite execution in
the verification ladder. The self-booting harness remains the integration
boundary.

## Verification gates

### The verify ladder & git-enforced gate

The ladder has four local entrypoints, all driven by `xtask`
(`xtask/src/lib.rs`: `run_host_gate`, `run_local_push_gate`, `Command::Check`,
`Command::Precommit`, `Command::Prepush`, and `Command::Validate`):

- **`cargo xtask check`** runs the host static checks in **Fix** mode
  (formatters auto-fix), then every repo-shape and type-safety gate, then the
  host unit tests, and — unless `--no-test` — the host-native `test-local`
  product Rust suite plus the Nix-only `wasm-tests` and `doctests` derivations.
- **`cargo xtask precommit`** is the hook entrypoint. Before gate work it
  classifies the complete pre-run dirty-tree snapshot. The narrow
  `staged-markdown-only` class requires a nonempty tree containing only
  staged-only, regular, case-sensitive `.md` additions or modifications; any
  unstaged, untracked, delete/rename, type-changing, non-Markdown, malformed,
  unparseable, or otherwise unsupported evidence takes the broad route.
  `precommit-routing` is an informational successful result emitted before the
  selected gate: its detail is
  `class=staged-markdown-only reason=isolated-staged-markdown`, or
  `class=broad reason=<stable-kebab-case-reason>`. Broad reasons have stable
  precedence: `uncertain-status`, `empty-state`, `untracked-path`,
  `unstaged-path`, `delete-or-rename`, `unsupported-change`,
  `unsupported-index-mode`, then `non-markdown-path`.

  The narrow route is a fixed ordered filter over the same production
  host/static catalogs: Prettier, sequence/identifier collision checks, the ADR
  bundle, documentation links, flow-document parity, and the error-swallowing
  inventory. It is not further path-routed: Prettier owns global `end2end` plus
  `**/*.md` formatting, ADRs project into `docs/README.md` and this document,
  and document links and flow documents consume repository-wide relationships.
  The isolated-tree predicate makes those reads and formatter writes represent
  the staged Markdown state. All other classifications run the existing broad
  host surface. The after-snapshot and conservative safe-staging reconciliation
  always run, including after failure: only formatter/check mutations to
  already-staged tracked paths with no pre-existing unstaged change are
  restaged; mixed tracked paths, newly-created untracked files, and changed
  delete/rename state fail closed. Pre-existing delete/rename state and
  untracked files stay unstaged and tolerated.

- **`cargo xtask prepush`** is the fast local push-hook entrypoint. It opens
  with the same clean-tree precheck as `validate`, then runs the verify-only
  host/static surface, the auxiliary `xtask`/`tools` non-doc tests, the
  host-native `test-local` product Rust suite, and `workspace-doctests` after
  the product tests. It invokes no Nix derivation.
- **`cargo xtask validate --no-e2e`** runs the verify-only host/static surface,
  then adds `wasm-budget` (kept out of `check` because it costs a
  `nix build .#site`, #836), the Nix static proof, Nix `wasm-tests`, Rust
  `coverage`, Nix `doctests`, and the combined `elisp-coverage-producer`
  followed by its host consumer. Full **`cargo xtask validate`** inherits that
  verdict and — unless `--no-e2e` — adds the browser/backend e2e aggregate; it
  never reruns live ERT.

Both hook entrypoints select orchestration-owned **fail-fast** execution. At
every ordered local boundary — individual static checks, host-gate steps, and
prepush phases — the first newly appended failed, non-skipped step prevents
later work. Unexecuted steps are absent from the command result, not
green-skipped. Apart from precommit's selected routing surface, this does not
alter command order, the clean-tree precondition, or staged-subset
reconciliation authority. `prepush`, explicit **`cargo xtask check`** and
**`cargo xtask validate`**, along with CI's corresponding surfaces, remain
**broad** and **exhaustive**: they retain the unchanged ordered diagnostic graph
and their existing authority.

Enforcement is git-native ([ADR-0029](adr/0029-git-enforced-verify-gate.md)).
`.githooks/pre-commit` calls `cargo xtask precommit`; the `xtask` Cargo alias
uses `cargo run --locked`, so Cargo cannot rewrite `xtask/Cargo.lock` before the
precommit snapshot. Git/index reconciliation lives in Rust rather than the shell
hook. `.githooks/pre-push` calls `cargo xtask prepush`. `prepush` and `validate`
both open with a `clean-tree` precheck that refuses a dirty tree unless
explicitly bypassed (`validate --allow-dirty`, or `SKIP_PRE_PUSH=1` for the
hook) and returns before any expensive step (`xtask/src/lib.rs`:
`clean_tree_precheck`). The hook proves the fast local lane ran on the committed
tip with no uncommitted files hiding; CI remains the non-bypassable hermetic
authority. Every `cargo xtask` run self-heals `core.hooksPath` to the tracked,
relative `.githooks` (`xtask/src/git.rs`: `ensure_hooks_path`).
`SKIP_PRE_COMMIT=1` / `SKIP_PRE_PUSH=1` are deliberate local escapes; CI is the
non-bypassable authority.

#### Prepush parity by failure surface

The fast local lane and `validate --no-e2e` do not run the same tests in
different environments. They have the following explicit authority split
([ADR-0029](adr/0029-git-enforced-verify-gate.md), #1117 supplement):

| `validate --no-e2e` surface                              | `prepush` coverage                                                                                                 | Authority / rationale                                                                                                                                                              |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Verify-only host/static surface                          | Runs the same host surface.                                                                                        | Prepush is the fast host verdict; Nix static proof remains authoritative for the sandboxed offline environment.                                                                    |
| Auxiliary `xtask`/`tools` non-doc tests                  | Runs once.                                                                                                         | The host tests are the applicable verdict; they are outside the application coverage/Nix test gates.                                                                               |
| Host-native product Rust tests                           | Runs `test-local`.                                                                                                 | The host-native product verdict is cheap and cache-friendly.                                                                                                                       |
| Root-workspace doctests and fence reconciliation         | Runs `workspace-doctests`: `cargo test --workspace --doc` plus the same bidirectional root-workspace fence census. | Prepush is authoritative for host-toolchain example execution and reconciliation; Nix `doctests`/`doctests-gate` remains authoritative for the pinned sandbox/offline environment. |
| Nix static proof                                         | Does not run.                                                                                                      | Only the sandboxed offline Cargo environment proves the hermetic static definitions.                                                                                               |
| Rust coverage/CRAP                                       | Does not run.                                                                                                      | Instrumentation and SQLite/PostgreSQL backend parity are part of the verdict.                                                                                                      |
| Wasm browser tests                                       | Does not run.                                                                                                      | The verdict exercises wasm browser primitives unavailable to a cheap host lane.                                                                                                    |
| Elisp coverage                                           | Does not run.                                                                                                      | The coverage VM is the authoritative execution environment.                                                                                                                        |
| Wasm budget                                              | Does not run.                                                                                                      | The verdict is defined by the Nix-built artifact's size semantics.                                                                                                                 |
| Server-function flow verification (full `validate` only) | Does not run.                                                                                                      | This is an e2e/full-validate responsibility outside the `validate --no-e2e` comparator.                                                                                            |

`workspace-doctests` and the Nix doctest producer/gate reconcile the same
root-workspace fence population in both directions; neither path may silently
shrink it. Only the Nix path establishes the pinned sandbox/offline verdict.

The heavy checks are Nix flake check derivations — the hermetic layer. xtask
realizes each via
`nix build -L --keep-failed --accept-flake-config --out-link .xtask/gcroots/<check>`
(`xtask/src/steps/nix.rs:416`): cachix-substituted (an unchanged re-run is a
substitution) and GC-rooted by the out-link, so garbage collection cannot evict
warm gate. `wasm-tests` returns the browser test verdict directly. Rust coverage
and doctests use producer/consumer pairs — `nix-coverage` + `nix-coverage-gate`,
`nix-doctests` + `nix-doctests-gate` — whose producers cannot fail; xtask reads
each verdict from the sandbox's `status.json` (`xtask/src/steps/nix.rs`). The
Elisp producer instead returns the fixed
`elisp-coverage/{lcov.info,summary.txt,status.json}` set for every controlled
outcome; xtask lifts it and its host consumer reconciles current source, census,
LCOV, and strict same-line `;; cov:ignore: <reason>` markers. Uncontrolled Nix
or VM failures remain build failures. xtask itself is host-only; Nix never
invokes it back ([ADR-0034](adr/0034-ci-e2e-matrix-distribution.md)).

### What the ladder actually runs

In order, host `static-checks` runs source consistency (`fmt`, `leptosfmt`,
`prettier`, `elisp-fmt`, `tools-fmt`, `ast-grep-tests`, `no-full-reload`,
`xtask-fmt`), compile/type checks (`byte-compile`, `tsc`, `cargo-deny`,
`clippy`, `web-server-clippy`, `web-no-server-clippy`, `wasm-clippy`,
`tools-clippy`, `xtask-clippy`), then the `ert` runtime check. Both rungs run
the same host steps (`xtask/src/lib.rs:457`-`:479`):

**Two different things are called `static-checks`, and conflating them is
easy.** The host _step_ above runs the listed sub-steps through host-local
lanes. The Nix `static-checks` _derivation_ (`flake.nix:1276`) runs the shared
`devtool check --all --sandbox-cargo` definitions hermetically with
workspace-specific offline Cargo homes, including `ast-grep-tests` for committed
rule fixtures and the `no-full-reload` repository scan
([proposed devtool ast-grep enforcement](adr/0161-devtool-owns-ast-grep-enforcement.md)).
`validate --no-e2e` builds it as `nix-static-checks` before the Nix test checks,
so CI fails if the hermetic static-check surface drifts from the host
definitions.

#### Sandboxed cargo-deny

`cargo-deny` is part of `devtool check --all --sandbox-cargo` under a documented
sandbox policy: host mode keeps full `cargo deny check`, while sandbox mode
skips `advisories` and checks only `bans`, `licenses`, and `sources`
([Sandboxed cargo-deny skips advisories](adr/0145-sandbox-cargo-deny-skips-advisories.md)).

#### Devtool-owned static-check definitions

The project/tool static checks live behind `devtool check` as a shared
command-definition surface, while keeping separate host and sandbox execution
lanes ([ADR-0052](adr/0052-devtool-unifies-static-checks.md),
[devtool owns compiling static-check definitions across host and Nix](adr/0146-devtool-owns-compiling-static-check-definitions.md),
[proposed devtool ast-grep enforcement](adr/0161-devtool-owns-ast-grep-enforcement.md)).
Alongside the compiling definitions, it owns the non-compiling ast-grep rule
fixtures (`ast-grep-tests`) and `no-full-reload` repository scan. The product
clippy commands deliberately cover three distinct surfaces: generic workspace
clippy is broad, feature-unified host coverage; the isolated
`web-no-server-clippy` checks `web`'s no-default-feature host test targets, so
workspace feature unification cannot enable `web/server`; and `wasm-clippy`
checks wasm library targets. The wasm step deliberately omits `--all-targets`,
because `web` test dependencies are host-oriented and cannot compile for
`wasm32-unknown-unknown`. Host xtask lanes execute each definition through their
existing static-check mechanism while retaining host-local Cargo artifacts and
sccache for Rust-compiling checks; sandboxed Nix lanes execute the same
definitions through `devtool check --all --sandbox-cargo` with
workspace-specific offline Cargo homes. `xtask-fmt` and `xtask-clippy` remain
native host checks because `xtask/` is excluded from the flake source.

| Step                                                            | Guards                                                                                                                                                           |
| --------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `identifier-collisions`                                         | duplicate ADR/migration number prefixes, migration parity                                                                                                        |
| `adr-format`, `adr-readme-parity`                               | ADR front-matter shape and the README table                                                                                                                      |
| `adr-view-parity`                                               | every accepted ADR is cited in this document                                                                                                                     |
| `doc-links`                                                     | intra-doc link targets                                                                                                                                           |
| `flow-docs`                                                     | typed CSR route/endpoint/matrix declarations in `docs/flows/`; one flow owner per endpoint; checked snapshot status                                              |
| `test-backend-pattern`                                          | dual-backend storage test shape                                                                                                                                  |
| `server-fn-registrar`                                           | every `web` `#[server]` fn is in the explicit test registrar                                                                                                     |
| `server-fn-tracing`                                             | each server fn's instrumentation                                                                                                                                 |
| `server-fn-coverage`                                            | static lane of the flow-coverage snapshot                                                                                                                        |
| `traced-context`                                                | context propagation                                                                                                                                              |
| `proffered-secret`                                              | inbound-secret directional boundary                                                                                                                              |
| `ast-grep-tests`                                                | committed native ast-grep rule fixtures                                                                                                                          |
| `no-full-reload`                                                | no-allowlist ast-grep repository scan: Rust in `web/src` and `client/src` must not chain `replace`, `assign`, `reload`, or `set_href` from `.location()`         |
| `e2e-goto-wrapper`, `e2e-scaffold`                              | e2e harness shape; no committed `e2eSalt`                                                                                                                        |
| `target-arch-placement`                                         | host/wasm split at module wiring only                                                                                                                            |
| `lint-suppression`                                              | reviewed Rust lint expectation markers; no `#[allow]`                                                                                                            |
| `thin-components`                                               | `#[component]` control-flow budget                                                                                                                               |
| `sqlx-newtype-bind`, `sqlx-newtype-decode`                      | typed SQLx admission and decode boundaries                                                                                                                       |
| `doctest-fences`                                                | the doctest population Nix cannot reach                                                                                                                          |
| `rendered-html-compiler-boundary`, `raw-html-door`, `html-sink` | compiler privacy for trusted HTML plus the two XSS DOM doors                                                                                                     |
| `xlang-literal`                                                 | Rust/TypeScript literal agreement                                                                                                                                |
| `xtask-tests`, `tools-test`                                     | auxiliary workspace unit tests the application coverage/Nix test gates do not execute ([workspace boundaries](adr/0141-cargo-workspace-execution-boundaries.md)) |

### How a gate is built

Three decisions shape every static gate above, and they were each paid for by a
gate that reported green for the wrong reason.

**A gate enumerates; it does not search.**
[ADR-0085](adr/0085-static-type-safety-gates-enumerate.md): a check that hunts
for the spelling its author anticipated can only confirm that hypothesis. So a
gate defines its population **structurally** — from what the AST plainly says —
denies by default, grants no automatic exemption from a pattern, scopes each
exemption to a single site (stating multiplicity where sites are genuinely
indistinguishable), parses rather than scans when the invariant spans lines, and
fails on input it cannot read. It also states, in its own module docs, what it
does not claim to cover.

**Typed storage admission is compiler-first, with a syntactic backstop.** The
sealed `StorageBind` registry makes normal query and builder value admission
fail at compile time unless an exact domain or persistence-role type is
approved. The residual `sqlx-newtype-bind` detector enumerates source-visible
raw SQLx admission doors and fails closed; it follows local aliases and
conservatively rejects unresolved receiver shapes, but is not type resolution or
SQL analysis. SQLx query macros are forbidden in its root because their
generated argument admission is outside that source AST
([typed storage bind admission](adr/0169-typed-storage-bind-admission.md)).

**Membership is structural and fails closed.**
[ADR-0110](adr/0110-gate-population-membership-is-structural.md) separates two
operations that look alike: deciding whether a site is _in_ the population, and
_exempting_ one that is. Only the second needs a human. A gate may therefore
read a path qualifier, a file's `use` bindings, or an enclosing `impl`'s
self-type to identify the door it guards — provided a site it cannot resolve (a
glob import, a generic parameter, a macro body) **stays in the population**.
Obscuring a qualifier buys a gate failure, not an exemption. The three XSS gates
share one traversal implementing this (`xtask/src/steps/ident_gate.rs`), so a
fix cannot land in two copies out of three.

**An exemption is a marker at the site.**
[ADR-0094](adr/0094-gate-exemptions-in-source-markers.md): the form is
`// <gate-step-name>:allow <reason>`, on the line **immediately above** the site
— a position chosen because it was measured, not preferred. Written trailing, 7
of 12 live markers were relocated by `rustfmt`/`leptosfmt`; written above, all
twelve stayed put. A reason is required, block form does not exist, the marked
line must hold exactly one site of that gate, and an orphan marker fails. The
token is derived from the gate's step name, so it cannot drift; it is per-gate,
because one line can belong to two populations.

### Coverage gate

The coverage verdict is **stateless** — a pure function of
`(coverage report, source tree)`, with no committed baseline, manifest, or merge
driver ([ADR-0050](adr/0050-stateless-coverage-gate.md)). It replaced an earlier
stateful ratchet that re-anchored a committed baseline by text identity
([ADR-0030](adr/0030-coverage-reanchor-text-identity.md), superseded). The Nix
`coverage` derivation produces the instrumented report, running the whole suite
under an ephemeral PostgreSQL via `devtool pg` so `storage/src/postgres/*` is
instrumented rather than skipped (`flake.nix:1288-1293`). The host-side gate
(`xtask/src/coverage/`) then applies:

- **One structural exemption**: a literal `unreachable!("msg")` with a non-empty
  message. It needs no marker because it is self-re-flagging — reaching the line
  panics the test, so no report is produced — and recognition is fail-closed
  (`mac.path.is_ident("unreachable")`, so `std::unreachable!` and aliases stay
  measured; `xtask/src/coverage/exempt.rs:57`). The `#[component]` exemption
  this gate once carried was **retired** (#520): components now live in
  wasm-only `component.rs` files
  ([ADR-0070](adr/0070-web-vertical-wasm-only-component-files.md)), so their
  lines never host-compile and never enter the denominator at all.
- **A tripwire**: the gate fails if any _covered_ report line falls inside an
  exempt span. With the `#[component]` arm gone it now protects the
  `unreachable!` exemption only — a covered `unreachable!` means the premise is
  violated.
- **`// cov:ignore`** (line, or a `cov:ignore-start`/`-stop` block) as the sole
  manual acceptance path, reviewable in the diff where it lives.
- **A per-function CRAP threshold of 30**, exclusive, waived only by an
  in-source `crap:allow` within the function's span
  (`xtask/src/coverage/crap.rs:32`).

The coverage source is bounded to cargo sources, enforced by a `drvPath` probe
(`xtask/src/coverage/probe.rs:148-155`,
[ADR-0116](adr/0116-coverage-probe-dirty-tree-workaround.md)): on a clean
worktree nix's flake git-fetcher walks history for `revCount`, which fails on
CI's shallow checkout of a merge commit whose parents are grafted away. A dirty
tree makes nix copy the working directory and read only HEAD. Because the
dirtied file is filter-excluded, the dirtying is constant and never perturbs the
`drvPath` the probe measures — a filter that stopped excluding `README.md` would
fail the probe loudly.

### Doctest gate

Running the doctests is half the gate; the other half is proving the run saw
every fence
([ADR-0095](adr/0095-doctest-gate-enumerates-the-fence-population.md)). The
scanner enumerates fences with `syn` and reconciles them against the run **in
both directions**: an unmatched fence means a proof was never evaluated, an
unmatched run entry means the scanner's own population shrank. The fence
vocabulary is closed to three exact strings — plain, `compile_fail`, `text` —
because `ignore` is collected and reported by rustdoc and would read as a
one-word self-exemption. Every `compile_fail` must carry a `#`-hidden prelude
whose every line appears in a plain fence **in the same doc comment**; the run
is `cargo test --workspace --doc`, never package-scoped. `workspace-doctests`
runs and reconciles the root-workspace population during `prepush`; the Nix
`doctests` producer/gate remains the sandbox/offline authority for that same
population. The `doctest-fences` step covers `xtask/` and `tools`, which the
flake's source filter excludes (`xtask/src/steps/doctest_fences.rs`). Doctests
feed no coverage: `llvm-cov --doctests` is unstable, so `--doc` runs outside
instrumentation.

### Server-fn gates

Host gates and an enumeration-independent runtime suite protect the server-fn
surface. `server-fn-registrar`
([ADR-0066](adr/0066-server-fn-test-registrar-guard.md), amended #848) parses
the `#[macros::server]` inventory and the sole explicit
`register_explicit::<web::<vertical>::<Type>>()` list in
`ensure_server_fns_registered()`. It rejects missing entries, malformed
registrar paths, and duplicate `(vertical, leaf)` type keys; its real-tree test
also proves the non-empty inventory count equals the deduplicated registrar
count. The helper still initializes that list once for integration tests.

The generated-type wire assertions in `server/tests/web/server_fn_wire.rs`
remain an enumeration-independent backstop: they assert each derived
`ServerFn::PATH` and pairwise distinctness, and their table count agrees with
the registrar count.

`server-fn-coverage` ([ADR-0081](adr/0081-empirical-server-fn-flow-coverage.md))
answers a question line coverage cannot: which server entry points a real
browser session drives. The claim is **derived from evidence, not asserted** —
the hit set is extracted from the OTLP traces a passing `sqlite × chromium` e2e
run emits, matched forward from the inventory by `#[tracing::instrument]` span
name plus `code.namespace`. A documentary convention was rejected precisely
because a doc naming a spec that never touches the fn would stay green forever.
The gate has two lanes (`xtask/src/steps/server_fn_coverage_check.rs`): a static
lane in `check`/`validate --no-e2e` that reads the sole committed
`docs/coverage/server-fns.json` snapshot plus the source-derived inventory, and
an e2e lane (`server-fn-coverage-regenerate` / `-verify`) that recomputes that
snapshot from the per-combo `cargo xtask e2e sqlite chromium` traces.

No server-fn host gate carries an **endpoint-drift check**, and that is
deliberate ([ADR-0120](adr/0120-no-endpoint-drift-check.md)). The retired
`server-fn-endpoint` gate compared a hand-written `endpoint = "…"` literal
against the derived `/<vertical>/<ident>`; #714 removed the literal, so the
inventory now computes the endpoint with the very expression such a check would
compare it to — a value against itself, which passes for the wrong reason. What
verifies the computed endpoint instead is `server-fn-coverage`'s seed
cross-check, which matches it against URIs observed in a real captured run
(`xtask/src/steps/server_fn_coverage_check.rs:614`). Endpoint correctness is
therefore asserted only where real traffic exists.

### Component thinness and cross-language literals

`thin-components` ([ADR-0086](adr/0086-enforced-thin-component-budget.md))
enforces the premise the coverage gate rests on. A `#[component]` body fails
above **2** units of raw Rust control flow on either of two surfaces — _setup_
(counted over the AST) and _view_ (counted over the `view!` token stream, since
`syn` cannot see inside a macro). Leptos's declarative `<Show>`, `<For>`, and
child components cost nothing, which makes the cheapest fix the idiomatic one.
There is **no `thin:allow`**: more budget is a design conversation.

`xlang-literal` ([ADR-0109](adr/0109-cross-language-literal-agreement.md))
covers the few constants that cannot be declared once because no import spans
their boundary — the CSR mount marker, the boot-mark prefix. A declared table of
pairs names each site by file, an **anchor** (the syntax introducing the
literal, never its value — a counterpart comment quotes the value, so a
value-anchor would let prose change the verdict), and the opening quote. The
anchor locates; only exact string inequality fails. Zero anchor matches, more
than one, or an unreadable file are all hard failures, because a locator that
has quietly stopped locating is the one thing this gate must never report as
green.

## Development tooling

Development tooling is split across two binary crates plus a shared library,
placed by a single litmus test — _where must this code execute?_
([ADR-0028](adr/0028-devtool-vs-xtask-boundary.md)):

- **`xtask`** — the host-side dev/CI driver
  (`cargo xtask check | validate | e2e | …`). It invokes `nix build` and
  consumes/analyzes exfiltrated build artifacts (coverage gate, CRAP gate,
  reports). It carries the result envelope: every run rewrites the
  `.xtask/last-result.json` sidecar (`xtask/src/result.rs`) and prints an
  `xtask-done: command=… ok=… exit=… duration_ms=…` sentinel on stderr from
  every exit path (`xtask/src/main.rs`), so a truncated log can still prove the
  process finished ([ADR-0028](adr/0028-devtool-vs-xtask-boundary.md)).
- **`tools/devtool`** — the tool that must also run inside Nix build sandboxes,
  where `nix` and `xtask` are unavailable; `devtool coverage emit` produces
  coverage artifacts into `$out` for exfiltration
  ([ADR-0028](adr/0028-devtool-vs-xtask-boundary.md)).
- **`tools/coverage`** — a pure-logic library shared by both sides, with no I/O
  policy of its own ([ADR-0028](adr/0028-devtool-vs-xtask-boundary.md)).

The "devtool = in-sandbox, xtask = host" split is a litmus test about where code
_must_ execute, not a ban on running devtool from a shell. ADR-0028's own
Supplement (#158) extends `devtool run` to the host as the gate-execution
surface for humans and agents, and exposes `devtoolBin` in the devShell; the
host verify ladder's use of `devtool check <name>` is ADR-0052's Decision. Both
host-side subcommands are therefore chartered, not drift.

- **`devtool run -- <cmd>`** is a no-shell single-command runner used both
  in-sandbox and on the host as the gate-execution surface for humans and
  agents. It `exec`s exactly one program (refusing shell re-entry like `bash -c`
  or `nix develop`), parks stdout/stderr under `.xtask/run/`
  (`tools/devtool/src/run.rs`), returns a JSON summary (`exit_code`, `ok`,
  `signal`, `duration_ms`, per-stream `{path, bytes, lines}`), and exits with
  the child's code. `devtoolBin` is exposed in the default devShell for this
  reason ([ADR-0028](adr/0028-devtool-vs-xtask-boundary.md)).
- **`devtool check <name> | --all [--fix] [--sandbox-cargo]`** is the single
  command-definition surface for the migrated static checks (`fmt`, `leptosfmt`,
  `prettier`, `tsc`, `elisp-fmt`, `ert`, `byte-compile`, `cargo-deny`, generic
  product `clippy`, `web-server-clippy`, isolated host-test
  `web-no-server-clippy`, wasm-target `wasm-clippy`, `tools-fmt`, tools
  workspace `tools-clippy`, and ast-grep `ast-grep-tests` plus the
  `no-full-reload` repository scan — `tools/devtool/src/check.rs`). Both gates
  invoke the same definitions: the host verify ladder delegates each through its
  static-check mechanism, preserving host-local Cargo artifacts and sccache for
  Rust-compiling checks; the Nix `static-checks` `runCommand` runs
  `devtool check --all --sandbox-cargo` from the prebuilt `devtoolBin` with
  offline Cargo homes. Cargo-deny keeps a split policy: host mode runs full
  `cargo deny check`, while sandbox mode skips `advisories`
  ([ADR-0052](adr/0052-devtool-unifies-static-checks.md),
  [Sandboxed cargo-deny skips advisories](adr/0145-sandbox-cargo-deny-skips-advisories.md),
  [devtool owns compiling static-check definitions across host and Nix](adr/0146-devtool-owns-compiling-static-check-definitions.md),
  [proposed devtool ast-grep enforcement](adr/0161-devtool-owns-ast-grep-enforcement.md)).

**xtask is host-only — an enforced invariant.** Nix derivations never invoke
xtask; the flow is strictly one-directional (host `cargo xtask` → `nix build`).
The flake's source filters exclude `xtask/` (`!hasInfix "/xtask/" path` in
`flake.nix`), so an accidental `cargo xtask` inside a derivation fails loudly
rather than running a stale copy, and frequently-edited gate logic never busts
the coverage/e2e cache ([ADR-0028](adr/0028-devtool-vs-xtask-boundary.md)).
xtask is also excluded from the root cargo workspace (`exclude = ["xtask"]`,
with its own `[workspace]` in `xtask/Cargo.toml`), and `tools/` is a second
standalone workspace (`coverage`, `devtool`, `doctests` — `tools/Cargo.toml:3`);
explicit `xtask-tests` and `tools-test` steps compensate for the unit suites the
application coverage/Nix test gates do not execute
([Cargo workspace execution boundaries](adr/0141-cargo-workspace-execution-boundaries.md)).
[#1061](https://github.com/jaunder-org/jaunder/issues/1061) tracks the stale
`host_tests` source comment that overstated the related `tools/` Nix-exclusion
claim. All three manifests pin `resolver = "3"` explicitly, because the two
virtual manifests would otherwise default to resolver 1
([ADR-0104](adr/0104-edition-2024-unsafe-env-and-precise-capturing.md)).

**Workspace layering.** The root workspace's shared crates are target-scoped
([ADR-0058](adr/0058-host-crate-layering.md),
[common/host target-reachability closure](adr/0159-common-host-target-closure.md)):
[#847](https://github.com/jaunder-org/jaunder/issues/847) subsumes
[#855](https://github.com/jaunder-org/jaunder/issues/855) at this target
boundary. For items currently in `common`, `common` owns types and operations
reached by CSR or another dual-target consumer, while `host` owns the unconsumed
`common` machinery; this does not make all server- or storage-only code `host`
code. `client` is the browser-infrastructure peer
([ADR-0069](adr/0069-client-crate-wasm-only-home.md)). `host` has no runtime
workspace dependency other than `common`; `macros` is its existing build-time
exception. Two optional `common` capabilities are deliberate host-only
exceptions to its otherwise dual-target dependency purity: `common/sqlx`,
required by orphan-rule trait ownership, and `common/sanitize`, which keeps
`ammonia` behind the host rendering path. External dependencies remain allowed.
A cargo-metadata gate enforces the host invariant and rejects `common/sqlx` in
the exact target/feature-resolved CSR closure. That closure is also expected to
omit `common/sanitize` and `ammonia`; the wasm build and size gate exercise the
resulting browser artifact. The graph gate cannot classify an external
dependency's semantics, so review owns keeping sanitization host-only. The
`macros` proc-macro crate is orthogonal to that runtime trio — build-time
tooling compiled for the compiler host, home to all workspace proc-macros
including the three newtype derives `StrNewtype`, `IdNewtype` and `NumNewtype`
([ADR-0062](adr/0062-macros-crate-proc-macro-home.md),
[ADR-0063](adr/0063-domain-value-newtype-convention.md)). `macros` is itself a
workspace member, so the coverage source filter admits it and its expansion
logic is measured by in-crate `syn::parse_quote!` tests — ADR-0062 records that
correction itself (`:76-83`, #412).

**Dependency patching.** The workspace carries one temporary git
`[patch.crates-io]` entry: `lettre`, routed to a `jaunder-org` fork pinned by
rev until the mailbox-parsing fix lands upstream
([ADR-0119](adr/0119-lettre-fork-pinned-by-rev.md)). `lettre`'s RFC 2822 mailbox
grammar cannot re-parse addresses its `Address` type accepts, so
`MessageBuilder::build` fails for a legal quoted local part or address literal.
The earlier `atom_syndication`/`rss` fork apparatus was removed under
[ADR-0089](adr/0089-upstream-atom-document-io.md): no fork entries in
`[patch.crates-io]`, no `flake = false` fork inputs, and no
`overrideVendorGitCheckout` in `flake.nix` remain.

**A pinned formatter.** The devShell's `leptosfmt` is not a released version:
the flake overrides `pkgs.leptosfmt` to a post-fix upstream rev
(`flake.nix:413-421`, [ADR-0118](adr/0118-leptosfmt-pinned-past-release.md)),
because 0.1.33 mangles a generic component tag that has to wrap and upstream's
fix merged three days after that release, with nothing shipped since. The
override swaps `src` wholesale rather than patching (the fix also moves a
submodule pointer, which a patch cannot do), restates `fetchSubmodules`, and
overrides `cargoDeps` with Crane's `static.crates.io` vendor output. A shared
adapter serves this override and the separately pinned `wasm-bindgen-cli`: it
flattens Crane's registry-hash directory and adds the `Cargo.lock` that
`buildRustPackage` expects. This avoids nixpkgs' crates.io-API vendor path,
which returns 403 in clean CI. The override deliberately keeps nixpkgs'
`version` string, since upstream never bumped it and `versionCheckHook` reads
it. The consequence is that the pinned binary is indistinguishable from the
stock one by `--version`: only behaviour tells them apart, which is one more
reason to invoke the devShell's binary rather than re-resolving one. Remove the
override once a release later than 0.1.33 exists.

**Rust edition and exception-free unsafe code.** Every package in the root,
`tools/`, and `xtask/` workspaces uses edition 2024
([ADR-0104](adr/0104-edition-2024-unsafe-env-and-precise-capturing.md)). Two
consequences are load-bearing for tooling:

- Edition 2024 made `std::env::set_var` / `remove_var` unsafe (RFC 3543). The
  repository performs no in-process environment mutation: executable, command,
  or test-harness composition roots resolve inherited inputs into typed
  configuration, while child environments are configured before spawn through
  `std::process::Command`. Cargo lint configuration forbids unsafe Rust without
  suppression at every package boundary in the root, `xtask`, and `tools`
  workspaces
  ([peripheral process configuration](adr/0158-peripheral-process-configuration.md)).
- Return-position `impl Trait` captures every in-scope lifetime (RFC 3498). View
  helpers that borrow a parameter return `impl IntoView + use<>` — precise
  capturing (RFC 3617) — so the returned opaque type captures nothing (14 sites
  across `web/src/*/component.rs`).

Both the resolver and the formatting style are pinned rather than inferred:
`.rustfmt.toml` sets `edition = "2024"` **and** `style_edition = "2024"`, so a
future edition move changes the language and not the formatting
([ADR-0104](adr/0104-edition-2024-unsafe-env-and-precise-capturing.md)).

**Landing changes: the merge queue and its observer.** `main` is behind a GitHub
merge queue ([ADR-0077](adr/0077-adopt-github-merge-queue.md)): GitHub builds
each PR combined with the current `main` in a temporary `merge_group` branch and
merges only if that build is green, so the up-to-date-before-merge treadmill is
gone while the semantic-conflict guarantee is kept.
`.github/workflows/ci.yml:11` carries the `merge_group:` trigger that makes the
required checks run in that context.

Because green checks are only phase one under a queue — and an ejected PR leaves
the queue silently — xtask owns PR observation. By default,
`cargo xtask pr watch [N]` follows checks until the next caller-actionable
outcome: a green, unarmed PR returns `ready-to-land`, while an already armed or
queued PR continues through the queue. `--until merged` explicitly requests
passive waiting across the ready handoff
([the actionable-handoff decision](adr/0135-pr-watch-actionable-handoff.md)).
`cargo xtask pr land [N]` remains the approval-bearing command: it alone arms
the merge and watches it home
([ADR-0087](adr/0087-xtask-github-pr-observation.md)). The transport is the `gh`
CLI as a subprocess (`xtask/src/pr/gh.rs`), with `snapshot` turning its JSON
into typed values and `decide` holding the pure verdict logic (`xtask/src/pr/`).
Distinguishing outcomes — including `ready-to-land`, `ejected`, `dequeued`,
`timed-out` ("GitHub never finished"), and `watcher-error` ("we could not tell")
— live in `pr.outcome` in the result envelope; success is command-specific
rather than a global synonym for merged.

## Documentation & decision process

The documentation architecture is event-sourced: ADRs in `docs/adr/` are
append-only decision events, and this document — `docs/ARCHITECTURE.md` — is the
materialized view folded from them
([the materialized-view ADR](adr/0127-architecture-view-materialized-from-adrs.md)).
An ADR's Decision text is never edited to track the present; when a decision
changes, a new ADR supersedes it with reciprocal pointers. In-place ADR edits
are limited to metadata and navigation (status lines, moved pointers, short
past-tense annotations), and any new addendum is written in past tense from
birth — "as of <date>, Y held" — never as a present-tense patch. The view is
kept current by two disciplines: committing a draft updates `ARCHITECTURE.md`
(and `CONTEXT.md` when the ubiquitous language changes) in the same feature
change, and a periodic replay audit re-derives the view from the log plus the
code to catch un-ADR'd drift
([the materialized-view ADR](adr/0127-architecture-view-materialized-from-adrs.md)).

The documentation landscape, per [ADR-0000](adr/0000-documentation-strategy.md)
as amended by
[the materialized-view ADR](adr/0127-architecture-view-materialized-from-adrs.md):

- `docs/adr/` — the decision log (MADR-style, the "why"). Each ADR's line-1
  heading is `# ADR-NNNN: <title>` and its status is a single token on a
  canonical `- Status:` line, machine-checked by the `adr-format` gate
  ([ADR-0036](adr/0036-identifier-collision-policy.md)). A **numbered** ADR may
  carry only `{accepted, superseded, deprecated, rejected}`; `proposed` is
  rejected outright, because numbering is itself the acceptance event
  ([ADR-0088](adr/0088-promotion-is-the-acceptance-event.md)). A draft may say
  `proposed` — the drafts pen _is_ that state.
- `docs/ARCHITECTURE.md` — the materialized view; every claim cites its ADR(s),
  and current reality is kept distinct from committed direction.
- `CONTRIBUTING.md` — process (setup, verify, land); it cross-links the view
  rather than restating structure. Root `CONTEXT.md` — the domain glossary. Both
  are projections in the same sense.
- `docs/DESIGN.md` — functional behavior and operational model;
  `docs/ROADMAP.md` — strategic vision and milestones.
- `docs/archive/` — shipped specs, plans, and milestone documents, kept as dated
  `YYYY-MM-DD-<slug>.md` files and kept there rather than deleted
  (`docs/README.md:149-152`).

New ADRs are tracked, numberless `docs/adr/drafts/<slug>.md` files. A draft
carries `# ADR-DRAFT: <title>`, remains `proposed`, and is cited only by its
slug-bearing path; there is no parallel bare draft token. Feature PRs commit the
draft and its architecture projection but neither promote it nor edit the
generated index. ADR-0048 historically governed an out-of-git, ship-time form of
this numberless workflow; the tracked
[successor decision](adr/0152-adr-numbering-happens-after-merge.md) proposes
superseding those two requirements while preserving path-only identity and late
allocation.

After the feature reaches `main`, branch generation derives from fresh `main`,
runs the deterministic ADR promotion mutation, and opens a stable promoter PR.
Main-push and manual generation events share one coalescing concurrency group;
per-PR dequeue recovery is separate so generation cannot replace it. Promotion
stages the tracked-source rename, assigns the next free number, strips one `../`
level from draft-internal relative link targets, rewrites path citations and
`proposed` to `accepted`, and regenerates the index.

The promoter uses a dedicated GitHub App limited to Actions read, Contents
read/write, pull requests read/write, checks read, commit statuses read, and
mandatory Metadata read. Actions read supplies historical merge-group
workflow-run metadata for dequeue correlation; there is no Actions write,
direct-main, or branch-protection bypass. Promotion commits use a deterministic
App-bot author and committer, and only the ordinary merge queue writes the
promoted result to `main`.

There is at most one open promoter for its stable head/base identity. Its head
SHA and generated diff are immutable; drafts merged later wait until it lands
and a subsequent pass handles them. Queue and auto-merge metadata may change as
the PR advances or safely recovers. After arming, exact-head auto-merge request
or queue-membership state verifies success; an immediately queued green PR need
not retain an auto-merge request. The normal pull-request and merge-group check
interval is a healthy proposed-decision lag. Failure to create, check, or merge
the promoter is different: the draft remains proposed until the visible
automation failure is diagnosed and repaired.

Promotion is also the **acceptance event**
([ADR-0088](adr/0088-promotion-is-the-acceptance-event.md)): in the same pass
that replaces the heading token, `promote` rewrites a `- Status: proposed` line
to `accepted`. Any other token a draft carries — `superseded`, `rejected`,
`deprecated` — is a deliberate authorial claim and survives untouched. The
rewrite alone would not hold the property, so `adr-format` enforces the other
half by rejecting `proposed` on any numbered file; rewrite, gate, and table
renderer share one status-line parse, so they cannot disagree about which line
they are reading (`xtask/src/adr.rs:105`, `xtask/src/adr_readme/files.rs:125`,
`xtask/src/adr_readme/files.rs:213`).

Identifier collisions remain loud
([ADR-0036](adr/0036-identifier-collision-policy.md)): the
`identifier-collisions` gate in `cargo xtask check`/`validate` fails on
duplicate numeric prefixes in `docs/adr/` and the migration directories. The
serialized promoter prevents feature branches from allocating ADR numbers at
all; `cargo xtask adr renumber` remains only as deprecated compatibility tooling
pending [#1169](https://github.com/jaunder-org/jaunder/issues/1169), not as the
current recovery path. The gate still runs against the merge-queue tree, where
GitHub stacks the PR on an ephemeral queue branch and runs the required checks
([ADR-0077](adr/0077-adopt-github-merge-queue.md)).

The ADR index table in `docs/README.md` is a generated projection of the ADR
files' headings and Status lines: `cargo xtask adr sync-readme`, invoked by
promotion, regenerates the number, link, and status cells between
`<!-- adr-table:begin/end -->` markers; titles stay hand-curated, and the
`adr-readme-parity` gate keeps table and directory in agreement, naming
`sync-readme` as its recovery
([ADR-0036](adr/0036-identifier-collision-policy.md)). Parity is not
correctness: the check compares two artifacts and stays green when both are
wrong in the same way, which is why the status rule is enforced at the file, not
at the table ([ADR-0088](adr/0088-promotion-is-the-acceptance-event.md)).

Three numbered-ADR gates ignore drafts. `identifier-collisions`, `adr-format`,
and `adr-readme-parity` share one enumeration rule — non-recursive `read_dir`
over `docs/adr/`, then `is_file` → `.md` → leading number — which excludes a
numberless file in a subdirectory twice over. `doc-links` deliberately differs:
it enumerates tracked Markdown, so it checks proposed drafts in feature PRs. The
`doc-links` gate has no ADR of its own; its decision lives in issue
[#682](https://github.com/jaunder-org/jaunder/issues/682), cited from
`xtask/src/steps/doc_links.rs:1`.

A draft path is live because the file is tracked, not merely because it exists
in one working tree. Draft-internal links must use targets that resolve before
and after the one-directory move: a numbered sibling is `../NNNN-slug.md`, and
another draft is `../drafts/<slug>.md`. References elsewhere use the sole
repo-root identity `docs/adr/drafts/<slug>.md`. Promotion rewrites those path
forms to the numbered destination; a bare draft token or a different path form
would survive pointing at nothing.

This document is itself a gated artifact. The `adr-view-parity` step requires
every `accepted` ADR to be cited here, and fails the ladder by name and title
when one is not
([the materialized-view ADR](adr/0127-architecture-view-materialized-from-adrs.md)).
There is no allowlist and no exemption file: when the step names an ADR, the fix
is to describe it here. That closes the loop which otherwise depends entirely on
the replay audit remembering to run.

What the step does **not** see is worth stating plainly, per
[ADR-0085](adr/0085-static-type-safety-gates-enumerate.md). It tests that an ADR
is **cited**, not that the prose around the citation is true, and not that the
citation is in a sensible place — any occurrence of the link or of a bare
`ADR-NNNN` token satisfies it, including one inside a code fence, an HTML
comment, or a "superseded by" aside. It also cannot catch a `superseded` ADR
cited as though current, because the citation still counts. Those blind spots
are why the replay audit and the `jaunder-adr-projection` skill exist rather
than being replaced by the gate.

## Un-ADR'd reality

The former #938 gaps are now covered by accepted decisions: process
configuration ([ADR-0144](adr/0144-process-configuration-cli-contract.md)),
deployment/package outputs
([ADR-0142](adr/0142-declarative-nixos-deployment-package-outputs.md)),
workspace/gate boundaries
([ADR-0141](adr/0141-cargo-workspace-execution-boundaries.md)), and Emacs
credential storage
([ADR-0143](adr/0143-emacs-auth-source-app-password-storage.md)).

The derived `summary_label` persistence policy is deliberately not ADR-backed:
[#754](https://github.com/jaunder-org/jaunder/issues/754) retains the existing
storage boundary rather than establishing a new durable architectural decision.

Two gaps a reader might expect here are absent because the system closed them.
The content-addressed media store is no longer un-ADR'd: ADR-0080 decides the
`<source>/<p1>/<p2>/<sha256>/<filename>` layout, ADR-0084 makes the encoded
filename canonical, and ADR-0090 decides what a media reference is. The
embedded-shell versus on-disk-wasm split no longer exists to document: #239
embedded the SPA shell and #237 embedded the CSR bundle, both closed, so "single
binary" needs no qualification.
