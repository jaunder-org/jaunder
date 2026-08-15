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

This file is updated in the same change that ships any new ADR (the convention
is stated in `docs/adr/template.md`; mechanical enforcement at
`cargo xtask adr promote` is committed follow-up work), and periodically
re-derived from the full log to catch drift. Process — how to build, verify, and
land work — lives in [CONTRIBUTING.md](../CONTRIBUTING.md); the domain glossary
lives in [CONTEXT.md](../CONTEXT.md).

## Workspace

Jaunder is a full-stack Rust application: an [Axum] server with a
client-side-rendered [Leptos] frontend
([ADR-0002](adr/0002-frontend-framework.md),
[ADR-0040](adr/0040-web-rendering-leptos-csr.md)), deployed as a single binary
([ADR-0008](adr/0008-deployment-model.md)) over a pluggable SQLite/PostgreSQL
storage layer ([ADR-0001](adr/0001-storage-backends.md)), with an Emacs blogging
client and an AtomPub API as first-class publishing surfaces.

Shared code is split by compile target, not by convenience: `common` is the
target-agnostic domain crate, `host` is its host-only sibling, and `client` is
the symmetric browser-infrastructure peer. `host` never compiles to wasm, so it
uses `std::fs`/`std::env` without the `#[cfg]` gating `common` would demand
([ADR-0058](adr/0058-host-crate-layering.md)). `client` holds raw browser glue
(`web_sys`/`js_sys`/`wasm_bindgen` and wasm-side Leptos plumbing), plus two
host-testable browser contracts; it contains no application domain types. `web`
and `csr` depend on `client`, never the reverse
([ADR-0069](adr/0069-client-crate-wasm-only-home.md)). Proc-macros live apart
from all three, in `macros`
([ADR-0062](adr/0062-macros-crate-proc-macro-home.md)).

| Crate          | Target      | Responsibility                                                                                                                                                                                           |
| -------------- | ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `common`       | host + wasm | Shared domain logic and types: validated newtypes, rendering, visibility, feed/AtomPub serialization.                                                                                                    |
| `storage`      | host        | Storage traits, record types, SQL migrations, and the SQLite/PostgreSQL backends ([ADR-0019](adr/0019-generic-storage-backend-via-dialect.md)).                                                          |
| `server`       | host        | The `jaunder` binary: Axum router, CLI, background workers, integration tests.                                                                                                                           |
| `web`          | host + wasm | Leptos components and `#[server]` functions — the UI and its server halves, split host/wasm at the file level ([ADR-0070](adr/0070-web-vertical-wasm-only-component-files.md)).                          |
| `csr`          | wasm        | The client-side-rendering entry point: mounts `web` in the browser ([ADR-0041](adr/0041-public-projector-and-csr-client.md)).                                                                            |
| `host`         | host        | Strictly-host-focused shared code: error carrier, capture dir, auth/token parsing, invites, metrics, and SMTP relay configuration ([ADR-0058](adr/0058-host-crate-layering.md)).                         |
| `client`       | host + wasm | Browser infrastructure: `localStorage`, dialogs, DOM/file-upload glue, reactive revalidation, CSR performance marks, and bounded client telemetry ([ADR-0069](adr/0069-client-crate-wasm-only-home.md)). |
| `macros`       | build-time  | The workspace's proc-macro home: newtype, `text_enum`, sqlx-bridge and server-fn derives ([ADR-0062](adr/0062-macros-crate-proc-macro-home.md)).                                                         |
| `test-support` | host        | A seed binary linking `storage` for out-of-process e2e seeding ([ADR-0046](adr/0046-test-support-seed-binary.md)).                                                                                       |

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
`exclude = ["xtask"]`) and `tools/` (members `devtool`, `coverage`, `doctests` —
the in-sandbox tools that run where `xtask` is unavailable). Both are covered
under [Development tooling](#development-tooling). `elisp/` (the Emacs client,
[ADR-0031](adr/0031-elisp-separately-tested-subproject.md)) and `end2end/`
(Playwright) are covered in their sections.

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
The trait bodies are implemented once by a generic `XStore<DB>` bounded on a
`Backend: sqlx::Database` marker trait (`storage/src/backend.rs`, implemented
for `Sqlite` and `Postgres`, carrying only the `db.system` span constant);
backend-specific SQL is isolated in per-trait `XDialect` impls under
`storage/src/{sqlite,postgres}/*.rs`. Traits with no divergence need no dialect
at all, and `Backend` deliberately carries no sqlx bind/executor bounds — each
store impl restates exactly the subset it uses
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

`storage::AppState` (`storage/src/app_state.rs`) is a bundle of fourteen trait
handles — thirteen `Arc<dyn *Storage>` plus `Arc<dyn AtomicOps>` — built by
`open_database` at the composition root. It holds storage only; services
(mailer, WebSub client, background workers) are constructed in `server` and
injected per-consumer as constructor parameters, and there is no services
bundle. The durable invariant: no type may be both a heterogeneous dependency
holder and passed beyond the composition root
([ADR-0016](adr/0016-dependency-injection-and-appstate.md)).

The web layer takes its dependencies per-trait via Leptos context.
`server::provide_app_state_contexts` (`server/src/context.rs:25`) publishes
thirteen of the handles (all but `feed_cache`, which no `#[server]` fn needs),
and each server fn fetches exactly what it uses —
`expect_context::<Arc<dyn UserStorage>>()`. The helper lives in `server`, not
`storage`, because using Leptos context as the DI mechanism is an
application-wiring decision
([ADR-0016](adr/0016-dependency-injection-and-appstate.md)). Nothing in the
codebase now pins reactive-owner lifetime for this: `server_boundary`
(`web/src/error/server.rs:99`) is a thin error-projection wrapper that awaits
the body and maps `InternalError → WebError`. The owner-pinning machinery was
dismantled in two steps: `server_resource` went in #515, then
`owner_ancestry_strong` and the `owner_lifetime` tests in #594. Dropping
component SSR left only one server-fn invocation path —
`leptos_axum::handle_server_fns_with_context` on `POST /api/…`, which holds a
parentless root owner strong for the whole future by itself. The ADR-0016
#89/#124/#138 addenda that described that pinning are explicitly marked
superseded-and-historical inside the ADR
([ADR-0016](adr/0016-dependency-injection-and-appstate.md)).

Operations that must span multiple traits atomically (`create_user_with_invite`,
`confirm_password_reset`) live on the `AtomicOps` trait
(`storage/src/atomic.rs`) and run as single transactions in the concrete backend
([ADR-0001](adr/0001-storage-backends.md),
[ADR-0016](adr/0016-dependency-injection-and-appstate.md)).

### Query and transaction discipline

- **Cursor pagination.** Timeline and collection listings paginate by keyset
  cursor, never offset: `PostCursor`/`CollectionCursor` (`storage/src/posts.rs`)
  round-trip through an opaque wire pair, giving fixed-cost queries that are
  stable under concurrent inserts ([ADR-0004](adr/0004-pagination-strategy.md)).
- **SQLite transactions.** SQLite dialect code avoids read-then-write deferred
  transactions (the shared→reserved lock upgrade that yields unretryable
  `SQLITE_BUSY` under WAL concurrency): "read to validate, then write" is
  expressed as a single autocommit `UPDATE/INSERT/DELETE … WHERE … RETURNING`,
  and genuinely multi-statement transactions open with `BEGIN IMMEDIATE`
  (`storage/src/sqlite/posts.rs:53`, `storage/src/sqlite/atomic.rs:48`). Where
  the same operation is generic, Postgres reaches the same serialization with
  `SELECT … FOR UPDATE` instead
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

`storage::export_backup`/`restore_backup` (`storage/src/backup.rs`) implement a
portable dump: a `manifest.json` plus one NDJSON file per table under `db/`,
together with the media tree, written either as a directory or as a gzipped tar
archive built in-process with the `tar` and `flate2` crates. The backed-up table
set is auto-derived from the live schema — every table minus the explicit
`TABLES_EXCLUDED_FROM_BACKUP` denylist (`_sqlx_migrations`, `feed_cache`;
`storage/src/backup.rs:26`) and SQLite-internal tables, sorted for a
reproducible manifest — so a migration that adds a table needs no backup code
change; a golden guardrail test pins the exact set (`storage/src/backup.rs:677`)
([ADR-0064](adr/0064-backup-target-auto-derivation.md)).

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

Post creation accepts an optional client-supplied idempotency key, so a retried
AtomPub POST does not create a duplicate post — the mechanism decided in issue
[#79](https://github.com/jaunder-org/jaunder/issues/79) as a follow-on to
ADR-0047, not in an ADR of its own. The `idempotency_keys` table (migration
`0023_create_idempotency_keys`, `UNIQUE(user_id, key)`) is written in the same
transaction as the post (`storage/src/posts.rs:2274`); a duplicate key surfaces
as `CreatePostError::IdempotencyConflict` and is deliberately _not_ retried as a
slug collision (`storage/src/post_service.rs:466`), and
`PostStorage::post_id_for_idempotency_key` maps a replayed key back to the post
it originally created. AtomPub is its only caller
(`server/src/atompub/posts.rs:366`); the web composer passes
`idempotency_key: None` (`web/src/posts/api.rs:188`), so the mechanism is a
machine-client contract, not a browser one.

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

## Content model

A post stores its **source**: a `PostBody` in an author-chosen `PostFormat`
(`Markdown` | `Org` | `Html`, `common/src/render.rs:35`), from which
`common::render::render` derives the stored `rendered_html`. The two forms feed
two deliberately separate serialization surfaces — syndication feeds emit HTML,
the AtomPub Collection the native source — detailed in the Protocols section
([ADR-0015](adr/0015-atompub-serialization-surfaces.md)).
`storage/src/posts.rs:42::PostRecord` carries both plus title, `Slug`, summary,
tags, and `created_at`/`updated_at`/`published_at`/`deleted_at`.

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

Every write path converges on **one canonical stored body**
([ADR-0024](adr/0024-server-side-org-canonicalization.md)): `canonicalize_body`
additionally strips the Org title source, so headers the server stores
structurally (today `#+TITLE:`) do not survive in the body while unrecognized
`#+FOO:` lines round-trip verbatim; clients synthesize their own header block on
the way out. `perform_post_update` (`storage/src/post_service.rs:236`, naming
block `:251-267`) and `perform_post_creation` (`:401`, block `:417-424`) derive
naming from the _original_ body via `derive_post_naming`
(`common/src/render.rs:618`) before canonicalizing, because canonicalization
removes the Org title line.

**`RenderedHtml` guarantees "contains no active markup", through two named
doors** ([ADR-0079](adr/0079-rendered-html-sanitization.md)).
`RenderedHtml::sanitize` **establishes** the invariant by scrubbing through a
single module-level `ammonia` `SANITIZER` (`common/src/render.rs:274,311`);
`RenderedHtml::from_trusted` (`:112`) only **inherits** it from an earlier
sanitize, and the `rendered-html-from-trusted` static check fails the build on
any new use — its allowlist is down to one production call site, the seed-DTO
wire rebuild. The field is private, so nothing outside the module can mint one:
there is no `Deserialize` (seed DTOs go through `deserialize_with`), no
`From<String>` (compile-fail-pinned at `:90`), no `Default`, no `pub(crate)`
constructor. The `sqlx::Decode` from the derived `#[derive(SqlxBridge)]` bridge
(`common/src/render.rs:93`) is the one in-module path that fills the field
without passing a door: a `rendered_html` column decodes straight into the type,
like every other domain column, and deliberately does **not** sanitize on read
([ADR-0123](adr/0123-rendered-html-storage-decode.md)). That is a decision, not
an oversight — this is not new outside data, and routing it through
`from_trusted` would put a gate-policed door on a path the gate cannot inspect.
Its **blessing risk is real and accepted**: decoding some other text column into
this type would bless it too, and the `rendered-html-from-trusted` check does
not catch that (its population is `from_trusted` uses, and a `FromRow` field
typed `RenderedHtml` names no door). The decision rests on one argument — typing
a column as `RenderedHtml` is a deliberate, reviewable act — so every new
`RenderedHtml`-typed row field is a security review point until
[#701](https://github.com/jaunder-org/jaunder/issues/701) widens the gate
([ADR-0123](adr/0123-rendered-html-storage-decode.md)). `ammonia` sits behind a
`sanitize` feature on `common`, off for wasm, enabled by `storage`, and `render`
itself does not _exist_ without it (`common/src/render.rs:241,605`) — absence
rather than a weaker guarantee. The allowlist is ammonia's audited default
widened only to keep a `language-*` `class` on `<pre>`/`<code>`.

**A post's media references are derived from that sanitized HTML, never
supplied** ([ADR-0090](adr/0090-media-references-extracted-at-render.md)).
`RenderOutput` (`common/src/render.rs:554`) holds the HTML and a `Vec<MediaRef>`
with **both fields private** and `RenderOutput::render` as its only constructor,
so a value whose reference set disagrees with its HTML is unrepresentable;
`into_html` consumes the pair. `CreatePostInput`/`UpdatePostInput` carry that
type in place of a bare HTML field (`storage/src/posts.rs:212-215`), and the
rows land in the post's own transaction (`post_media`, migration
`0025_create_post_media.sql` — keyed `(post_id, source, sha256, filename)`, no
FK to `media`). Publication therefore gets its **own** storage operation,
`publish_post` (`storage/src/posts.rs:1241`, called from
`web/src/posts/api.rs:463`), which sets the publication timestamp and touches
nothing else — that is what lets rendering stay the sole constructor.

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
restart-durable for cached feeds, and publish is an explicit
`PublishUpdate { Unpublish, Publish { at } }` so scheduling, backdating, and
pullback to draft round-trip
([ADR-0027](adr/0027-scheduled-publishing-time-gated-visibility.md)).
_Audience_: posts target audiences
(`AudienceTarget::{Public, Private, Subscribers, Named}`) stored as
`post_audiences` rows; a viewer is a `ViewerIdentity` (channel identity or
anonymous, `common/src/visibility.rs`) and sees a post iff they are the author
or any targeted audience admits them — OR semantics, failing closed (`Private`
is zero rows); subscriptions route through the admission seam
(`SubscriptionPolicy`, wired to the auto-approving `OpenSubscriptionPolicy`)
([ADR-0020](adr/0020-content-visibility-and-subscription-model.md)).

**Accepted local lifecycle target.** A local Post is durable canonical identity
and latest state. Before every meaningful content, tag, audience, media,
publication, or deletion change, storage is to append a full prior-state **Post
Revision**, including immutable creation/prior modification and
publication/deletion timestamps; no-op writes create none. Revisions are
immutable through the storage API, owner-readable (including after deletion),
and not revert operations. Soft deletion is to retain the Post and its history
indefinitely while removing it from active web, Syndication Feed, and AtomPub
Collection surfaces. Permalink and syndicated-item identity is active-only, so a
later Post may reuse the URL and feed readers may conflate the two. Media
referenced by a retained current Post or revision is to participate in the
ordinary reference guard; explicit forced deletion remains an administrative
override. A future purge remains deliberately undecided and requires its own
decision ([local Post lifecycle decision](adr/drafts/local-post-lifecycle.md)).

**Current lifecycle behavior.** Updates snapshot only title, slug, body, format,
and rendered HTML; no-op updates still snapshot; dedicated state changes create
no revision; history has no read surface; and deleted Posts/revisions release
media protection.
[Revision/history/media corrections](https://github.com/jaunder-org/jaunder/issues/1055)
and [Deleted Post replay](https://github.com/jaunder-org/jaunder/issues/1056)
remain implementation debt, not alternate lifecycle policy.
[ADR-0009](adr/0009-edit-delete-policy.md) governs consumed content and does not
reach these local rows.

Cross-cutting values are validated newtypes whose `FromStr` is the single
chokepoint: `Username`, `Slug`, `Tag`, `Password`
(`common/src/{username,slug,tag,password}.rs`). Tagging is keyed on the `Tag`
slug (`PostTag`, `post_tag_diff` in `storage/src/posts.rs`).

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
  ([ADR-0079](adr/0079-rendered-html-sanitization.md)): `RenderedHtml::sanitize`
  is already the door any future inbound producer must use, and the static check
  enforces that, but no inbound producer exists yet.
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
`PermalinkUrl`, … , each a `TaggedUrl<Role>` alias in `common/src/tagged_url.rs`
(roles at `tagged_url.rs:212-287`;
[ADR-0112](adr/0112-role-tagged-site-urls.md)). This is what stops two adjacent
same-typed URLs being transposed: `send_publish(hub, feed)`,
`render_rsd_document(service, homepage)`, and the `FeedMetadata`
`canonical_url`/`self_url`/`hub_url` fields
(`common/src/feed/metadata.rs:38-40`) were all live hazards before the roles
existed.

### Syndication feeds

Public read-only feeds serve arbitrary feed readers, so every item carries the
post's `rendered_html` — Atom `type="html"` and the RSS/JSON Feed equivalents
([ADR-0015](adr/0015-atompub-serialization-surfaces.md)). Rendering lives in
`common/src/feed/`: `render_atom` (`atom.rs:6`), `render_rss` (`rss.rs:16`), and
`render_json` (`json.rs:6`). The URL grammar is `common/src/feed/feed_path.rs` —
`FeedSurface::{Site, SiteTag, User, UserTag}` (`feed_path.rs:102-107`)
canonicalized against three `FeedFormat`s. `server/src/feed/handlers.rs` serves
the cached bytes; `regenerate_feed` (`server/src/feed/regenerate.rs:35`)
rebuilds them. Scheduled posts reach feeds via `FeedWorker::go_live_pass`
(`server/src/feed/worker.rs:84`), which enqueues regeneration for feeds whose
posts crossed their publish time
([ADR-0027](adr/0027-scheduled-publishing-time-gated-visibility.md)).

**Accepted membership target.** Cached membership is to apply anonymous/Public
eligibility before ranking, then select the union of the first `feeds.min_items`
Posts and all Posts at or newer than the inclusive `feeds.min_days` cutoff,
ordered by `published_at DESC, post_id DESC`. Defaults are 20 Posts and 30 fixed
24-hour UTC days. The window is exact when regenerated, not continuously as time
passes. A successful setting mutation is to durably invalidate all cached feeds
before returning; checked overlarge ages mean all history, while corrupt stored
values are errors
([Syndication Feed hybrid-window decision](adr/drafts/syndication-feed-hybrid-window.md)).
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
([Syndication Feed HTTP-validation decision](adr/drafts/syndication-feed-http-validation.md)).
[Current tuple completeness, item-derived timestamp, 304 metadata, and conditional parsing](https://github.com/jaunder-org/jaunder/issues/1054)
remain implementation debt.

The Atom feed document is built by upstream `atom_syndication` — `render_atom`
assembles an `atom_syndication::Feed` and lets the crate emit the XML
([ADR-0089](adr/0089-upstream-atom-document-io.md)). RSS goes through the `rss`
crate the same way.

### AtomPub editing interface

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
whole policy is the `format_wire` seam: the two private pure functions
`format_to_wire` (`server/src/atompub/mapping.rs:41`) and `wire_to_format`
(`mapping.rs:51`), revertible in one line. The seam has always lived in the
server crate; only the namespace and `j:slug` definitions it works against live
in `common/src/atompub`.

Two Jaunder wire extensions ride the namespace `https://jaunder.org/ns/atompub`
(`J_NS`, `common/src/atompub/ns.rs:6`;
[ADR-0023](adr/0023-atompub-jaunder-wire-extensions.md)): a read-only `j:slug`
on every entry — drafts and scheduled included, incoming values ignored
(`mapping.rs:123`) — and
`<j:extension version="1" features="format-media-type slug"/>` in the service
document (`common/src/atompub/service.rs:68-70`), so clients feature-detect once
and degrade gracefully.

Atom document I/O is upstream's, not ours
([ADR-0089](adr/0089-upstream-atom-document-io.md)). `atom_syndication` 0.12.10
made bare-`<entry>` I/O public, so the hand-rolled reader and writers are gone:
parsing is `Entry::from_str` at the call site and serialization is
`Entry::write_to` / `Feed::write_to` (`common/src/atompub/entry.rs:334`, `:406`,
`:483`). Jaunder's own foreign markup stays Jaunder's — `app:control/app:draft`
and `j:slug` live in the entry's extension map behind `is_draft`/`set_draft` and
`j_slug`/`set_j_slug`, each helper owning its `xmlns:` prefix so an entry
declares a namespace only when it actually carries the marker. `quick-xml`
remains a direct dependency, but only for the non-Atom documents Jaunder still
writes itself: the service document, RSD, the shared XML helpers, and the
categories renderer (`common/src/atompub/{service,rsd,xml,categories}.rs`).

`common/src/atompub/categories.rs` is the exception that proves the rule.
`render_categories_document` (`:20`) is written and re-exported, but no route
and no server caller reaches it — the AtomPub categories document is **not
served**. Whether the module survives is
[#928](https://github.com/jaunder-org/jaunder/issues/928).

The crates come from the registry — `atom_syndication` 0.12.10 and `rss` 2.1
(`common/Cargo.toml:25-27`). Earlier,
[ADR-0043](adr/0043-quick-xml-fork-patch.md) (now **superseded**) had cleared
two RUSTSEC advisories by forking both crates onto `quick-xml` 0.41 and wiring
the forks in through `[patch.crates-io]`, flake inputs, and a crane vendor
override. **That apparatus was deleted outright** — the only surviving
`[patch.crates-io]` entry is an unrelated `lettre` fork (`Cargo.toml:134-135`),
`flake.nix` has no syndication inputs and no vendor override, and `deny.toml`
keeps `allow-git = []`. The advisories stay cleared by the same single mechanism
as before: `quick-xml` ≥ 0.41.

### WebSub publishing

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
([publisher-side WebSub decision](adr/drafts/publisher-side-websub.md)).

**Current publisher behavior.** Production pings through
`WebSubClient::send_publish(&HubUrl, &FeedUrl)`
(`server/src/websub/contract.rs:45`) and reports `Success`, `Exhausted`,
`Failed`, or `NoHub`.
[AtomPub does not enqueue, web enqueue is not atomic, and triggers are coarse](https://github.com/jaunder-org/jaunder/issues/1051).
[Configuration changes do not invalidate caches, worker/regenerator snapshots can differ, configuration access errors can collapse to `NoHub`, HTTP failures retry alike, `Retry-After` is ignored, budgets are shared, and terminal rows lack redrive](https://github.com/jaunder-org/jaunder/issues/1052).
`HttpWebSubClient` runs in production; noop and file-capture implementations
back tests.

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
the `AuthUser` axum extractor (`web/src/auth/server.rs`), which resolves
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
the `AuthUser` extractor; failures map to unauthorized/internal errors through
`AuthRejection` ([ADR-0007](adr/0007-auth-mechanisms.md)). The operator-only
variant `require_operator()` layers the `is_operator` check on top.

`viewer_identity()` (`web/src/viewer.rs`) applies the optional-auth half of the
same rule for visibility-filtered reads: it returns a local viewer on successful
authentication, anonymous for absent or stale cookie-only credentials, and an
error for every failure attributable to a Bearer or Basic credential.

**Session establishment for the web client is cookie-only**
([ADR-0107](adr/0107-web-session-establishment-is-cookie-only.md)): a
`#[server]` fn on the auth path sets the `HttpOnly` `session` cookie and returns
**no session-token material** in its body. Concretely `register` returns `()`
(`web/src/registration/api.rs:59`) and `login` returns a `LoginResponse`
carrying only `is_operator` (`web/src/auth/api.rs:38`). The one deliberate
exception is `create_app_password` (`web/src/sessions/api.rs:62`), which returns
the raw token because showing it once at creation is the whole point of an app
password — that endpoint establishes no browser session. **No web endpoint hands
the browser a bearer token** — though endpoints will still _accept_ one, as
logout does. No machine gate enforces the rule: it is held by
`assert_body_carries_no_token` (`server/tests/web/web_auth.rs:34`), which checks
the success body against the token recovered from `Set-Cookie`, called for
register (`:83`) and login (`:332`)
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
  (`storage/src/sessions.rs:185`, `password.rs:116`, `email.rs:161`,
  `sqlite/atomic.rs:131`, `postgres/atomic.rs:98`). The neighbouring
  `host::token::generate` (`:28`) mints invite codes, not session tokens
  (`host/src/invite.rs:59` is its only caller). The two are distinct newtypes,
  `common::token::{RawToken, TokenHash}`
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
  `(token_hash, user_id, label, created_at, last_used_at)`
  (`storage/src/sessions.rs:164`), and `label` is a mandatory validated newtype,
  `common::session_label::SessionLabel` — browser logins auto-generate a
  User-Agent/host label, app passwords carry a user-supplied name. Revocation is
  deleting the session in the Sessions UI
  ([ADR-0014](adr/0014-atompub-authentication.md)).
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
marker set by `AuthUser` only after Bearer/Basic token and username checks
succeed. Outer router middleware appends the expiry header, preserving any
`Set-Cookie` values already emitted by the handler
([explicit Authorization replaces ambient session state](adr/0132-explicit-authorization-replaces-session-cookie.md)).

### Password hashing

Passwords are hashed with **Argon2id** at the crate-default parameters (m=19456,
t=2) via `common::password::Password::hash` (`common/src/password.rs`)
([ADR-0018](adr/0018-constant-time-authentication.md)). Test builds may enable
the `cheap-kdf` feature, which swaps in `Params::MIN_M_COST` with t=1 so the
suite is not dominated by KDF time; `verify()` derives cost from the stored PHC
hash, so it needs no branch. The feature fails closed twice, at different times:

- **Compile time** —
  `#[cfg(all(feature = "cheap-kdf", not(debug_assertions)))] compile_error!`
  (`common/src/lib.rs`). The guard keys on `debug_assertions`, so an _optimized_
  build carrying the feature fails to build rather than producing a weak-hashing
  artifact; ordinary test builds are unaffected.
- **Startup** — `server/src/main.rs` reads `common::CHEAP_KDF_ENABLED` and, if
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
  `Password::hash` path so it carries production parameters, with a hardcoded
  valid-hash fallback so initialization is infallible) on the absent-user path
  before returning `InvalidCredentials` (`storage/src/users.rs:350`), closing
  the username-enumeration timing oracle. **Durable invariant: the absent-user
  path MUST keep this equalizing verification** — do not remove it as a "fast
  path" and preserve it through any refactor. The backend dedup is already done:
  `authenticate` is a single generic `UserStore<DB: Backend>` impl
  (`storage/src/users.rs:212`), so SQLite and Postgres cannot drift apart here
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
- **High-entropy secret (invite code, reset token): cheap-reject first.** Both
  operations live on the `AtomicOps` trait (`storage/src/atomic.rs:101`), which
  each backend implements separately (`storage/src/sqlite/atomic.rs:30`, `:125`;
  `storage/src/postgres/atomic.rs:26`, `:92`). `create_user_with_invite`
  validates the invite with a cheap lookup before hashing (the SQLite backend
  takes its write lock up front per ADR-0021, so the hash runs inside the
  immediate transaction on the success path only), and `confirm_password_reset`
  atomically claims the reset token before hashing the new password — it
  originally hashed first, which ADR-0022 recorded as a violation and
  [#60](https://github.com/jaunder-org/jaunder/issues/60) fixed. A ~256-bit
  secret admits no useful timing oracle, and hashing first would turn
  bogus-secret requests into a CPU-exhaustion amplifier while destroying invite
  issuance as a throttle
  ([ADR-0022](adr/0022-validate-before-expensive-work.md)).

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
`sidebar/component.rs:60-70`, `posts/component.rs:185-208`), so the CSR mount
causes no reflow: flash-free by coincidence, not markup twins.

Markup is built with **maud's `html!`**
([ADR-0093](adr/0093-web-render-html-macro.md)), and the trusted-HTML invariant
is carried by one crate-local newtype, `web::html::Markup` (`web/src/html.rs`),
which shadows `maud::Markup` inside `web`. The single raw door is
`Markup::from_rendered_html`, which takes a `&RenderedHtml`
(`web/src/html.rs:59`) so the sanitization invariant is what opens it. Three
xtask gates gate the area — `html_sink_check`, `raw_html_door_check`, and
`rendered_html_from_trusted_check` — and the first two read inside macro bodies,
so a hand-built `String` cannot reach a sink unescaped.

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
- `api.rs` — the vertical's wire types and **every** `#[server]` fn,
  dual-compiled;
- `server.rs` — `#[cfg(feature = "server")]` host-only helpers;
- `component.rs` — the `#[component]` UI, declared
  `#[cfg(target_arch = "wasm32")]`;
- plus ungated, host-tested, coverage-measured state/logic files
  (`compose_state.rs`, `input_state.rs`, `state.rs`, `render.rs`, …).

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
`crate::error::server_boundary` (`macros/src/server_fn.rs:166`); there is no
hand-written `boundary!` call. ADR-0016's SSR-era owner-pinning addenda have
been retired: the sole server-fn invocation path, `leptos_axum`'s `/api`
handler, holds the owner strong for the whole call, so no `ScopedFuture` wrapper
and no sanctioned `Resource` constructor exist — components call `Resource::new`
directly (13 files across `web/src`), and no clippy `disallowed-methods` entry
bans it — `clippy.toml` has no `disallowed-methods` entry at all; it only
_relaxes_ `unwrap`/`expect` for tests, which the workspace otherwise denies
(`Cargo.toml:141`).

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
(`web/src/forms/field.rs:22`), rendered by `<ValidatedInput<T>>` /
`<ValidatedTextarea<T>>` (`web/src/forms/component.rs:110,185`) or bound
directly for a bespoke layout. The chrome both shells wrap themselves in,
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
(`web/src/audiences/api.rs`,
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
`leptos_router` (`use_navigate()` / `<a>`), and raw `window.location` navigation
is forbidden in `web/src` and `client/src`, enforced by the `no-full-reload`
xtask source scan (`xtask/src/steps/no_full_reload_check.rs`) because those call
sites are wasm-gated and invisible to the default clippy pass. The SPA user
namespace is `~`-prefixed: the permalink route's leading segment is a custom
`TildeUsername` route match (`web/src/route_segments.rs:13`, wired at
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

The backend emits spans via `tracing` + `tracing-opentelemetry` in the `server`
crate ([ADR-0011](adr/0011-unified-observability.md)). `init_tracing`
(`server/src/observability.rs`) installs the OTLP tracer only when
`JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT` (fallback `OTEL_EXPORTER_OTLP_ENDPOINT`)
is set; with no endpoint every emit is a no-op, and exporter-setup failure is
non-fatal. `with_http_observability` (same file) layers the per-request tracing
span onto the router, together with a `tower-http` `x-request-id` that it mints
when absent and propagates onto the response. Inbound W3C `traceparent` headers
are extracted onto the per-request span, so backend spans parent into the
caller's trace. Span fields and metric attributes are exported, so they MUST NOT
carry user PII or secrets — stable identifiers (`user_id`, `error.kind`) only.

### Server-fn span names are macro-derived

Every `#[server]` fn in `web/src` is written as `#[macros::server]`, which emits
the `#[tracing::instrument]` attribute itself with the name
`web.<vertical>.<ident>` computed from the file path and identifier
(`macros/src/server_fn.rs`) — so no `#[server]` fn's span name is written in the
source, and none can drift ([ADR-0011](adr/0011-unified-observability.md),
amended 2026-07-30). Hand-written `instrument` names do still exist outside that
set — `require_auth` carries one (`web/src/auth/server.rs:112`) — because they
are not server fns. The macro rejects `fields(…)`, `level`, `err`, and `ret`
outright. What the author still writes is `skip(…)` / `skip_all`, and the
`server-fn-tracing` gate holds a default-deny `RECORDABLE_TYPES` allowlist over
every unskipped parameter type: an unlisted type fails the gate until someone
classifies it. A type is admissible only if it is bounded by its own type, is
operator configuration, is already published in a permalink, or is `Username`.

The earlier arrangement — the gate writing the `name = "…"` literal into
`web/src`, and a `server_fn` field on the boundary log event — was retired with
that addendum. Nothing under `web/src` is rewritten by `cargo xtask check` for
span naming any more.

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
time is measured by an envelope nested around it (`end2end/tests/fixtures.ts`):

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

### Measurement frames are not mixed

The browser measurements arrive in two clocks, and a decomposition is computed
entirely within one of them
([ADR-0100](adr/0100-measurement-frames-are-not-mixed.md)). The **document
frame** — `performance.mark` and `PerformanceResourceTiming`, relative to that
document's `timeOrigin` — is what `capture-trace.ts`'s `harvestDocument` returns
and the only frame boot analysis decomposes: `bootTotalMs` is `timeOrigin` to
`jaunder.boot.mount_done`, and its parts sum to it by construction. The **Node
frame** — `Date.now()` stamps taken in the Playwright driver — carries
`committedMs`, `mountedMs`, and `commitToMountMs`. `commitToMountMs` is still
reported as the bridge to suite wall-clock but is never decomposed; the
difference `commitToMountMs - bootTotalMs` is reported separately as **frame
skew** and charged to the harness (`frame_skew_ms` in
`xtask/src/traces/boot_phases.rs`). The two frames differ by cross-process,
plausibly engine-asymmetric lag, so mixing them would charge harness IPC to the
app's boot phases.

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

### Errors at the boundary

The carrier owns its boundary observability:
`InternalError::emit_boundary_failure` (`host/src/error.rs`) logs five discrete
tracing fields — `error.kind`, `error.class`, `error.public`, `error.source`
(the preserved typed chain, rendered once), `error.context` — at the level
derived from the error class, and emits the `jaunder.errors` metric. The metric
is an error-event counter whose bounded attributes include kind/class plus
`error.disposition = boundary | swallowed` and
`telemetry.origin = server | client`; it is not a unique-root-cause count.
`server_boundary` (`web/src/error/server.rs`) calls it with `boundary/server`,
then performs only the outward wire projection and returns the masked public
error. Which fn failed is not a field: the event is raised inside the enclosing
`web.<vertical>.<ident>` span, and both configured sinks render span context
([ADR-0011](adr/0011-unified-observability.md),
[ADR-0017](adr/0017-error-handling-and-the-public-boundary.md)).

An unexpected native failure that deliberately preserves the primary result goes
through one `host::error` reporting interface, which couples the fixed warning
and `swallowed/server` metric so a caller cannot forget either side. Diagnostic
self-failure uses only its console/stderr fallback; routing it through the same
reporter would recurse.

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

The single env var `JAUNDER_CAPTURE_DIR` enables app-driven capture
([ADR-0049](adr/0049-app-driven-scoped-server-diagnostics.md),
[ADR-0057](adr/0057-e2e-capture-dir-contract.md)); production leaves it unset
(fully inert). Each stream writes a filename defined once in `host::capture`
(`mail.jsonl`, `websub.jsonl`, `diag.log`), resolved at a composition root. The
diag stream is a WARN+-filtered JSON `tracing` layer plus a panic hook appending
`kind: "panic"` JSONL records through its own `O_APPEND` handle (bypassing
`tracing` to avoid deadlock). The shared `test_support::panic_gate` verifier
([ADR-0032](adr/0032-e2e-zero-panic-gate.md)) resolves the diagnostic filename
through `host::capture`, scans raw bytes from the union of that stream and a
required server log, and de-duplicates by panic location with the scoped record
winning. Per combo the e2e harness tars the directory out as
`capture-<backend>.tar.gz` — those three files plus `otel-traces.jsonl` — into
the [ADR-0037](adr/0037-e2e-failure-diagnostics-capture.md) artifact set.

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
request from the storage path (`server/src/media.rs:116,147`) — and the process
may also write a runtime-info JSON file and read a PostgreSQL password file; see
"Outside the binary" below.

That runtime file is not only informational: it is a **startup mutex**.
`check_startup_mutex` (`server/src/runtime_file.rs:104`, called from
`server/src/commands.rs:487`) makes `serve` **refuse to start** when the file
names a live writer process, and an unreadable file refuses too rather than
serving with a broken guard (`runtime_file.rs:61`). A `RuntimeFileGuard` removes
it on graceful exit and on SIGTERM/SIGINT (`commands.rs:1227-1234`). The e2e
harness reads the same file for port discovery.

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

`site-config` is not a free-form door. Its `key` argument is the `SiteConfigKey`
enum, so clap rejects an unknown key at parse time, and each key carries the
validator that `set` runs before any row is written
([ADR-0102](adr/0102-config-key-closed-registry.md); the registry macro is
`common/src/config_key.rs:85,158`). `list` is the deliberate exception: it dumps
every stored row, flagging keys outside the registry as `UNKNOWN KEY` and
recognised keys holding unparseable values as `INVALID`, so legacy rows stay
visible.

Deployment is configured by environment variables, each a clap `env` fallback
for a flag (`server/src/cli.rs`). The process-shape ones are `JAUNDER_BIND`
(listen address, `:267`), `JAUNDER_DB` (database URL, default
`sqlite:./data/jaunder.db`, `:41`), `JAUNDER_STORAGE_PATH` (the data directory,
default `./data`, `:33`), `JAUNDER_ENV` (`dev` | `prod`, `:271`),
`JAUNDER_RUNTIME_FILE` (`:276`) and `JAUNDER_VERBOSE` (`:25`). PostgreSQL takes
its secret by either `JAUNDER_DB_PASSWORD` or `JAUNDER_DB_PASSWORD_FILE`
(`:39-40`, read at `storage/src/postgres/open.rs:40,44`). The observability
variables are covered under [Observability](#observability). `prod` is
load-bearing in two places: it sets the `secure_cookies` flag passed to
`create_router` (`server/src/commands.rs:546`, `server/src/lib.rs:32`), and it
disables the dev-only auto-initialization of a missing database on `serve`
(`server/src/commands.rs:501-512`).

**What the flake ships.** `flake.nix` exports `packages.jaunder` (the server
binary), `packages.site`, and `nixosModules.jaunder` (`flake.nix:247-249`,
`1059-1062`). `packages.site` is **no longer a deployment artifact** — the
binary embeds the bundle — and is retained only so `cargo xtask audit-wasm` can
build `.#site` and inspect the bundle for size analysis (`flake.nix:464-473`,
[ADR-0028](adr/0028-devtool-vs-xtask-boundary.md)). The `services.jaunder`
module (`flake.nix:44-118`) creates a dedicated `jaunder` user/group, runs under
systemd from `StateDirectory=jaunder` with `WorkingDirectory=%S/jaunder`, passes
`bind` and `db` through unconditionally and `JAUNDER_ENV=prod` only when `prod`
is set (`flake.nix:95-101`), runs
`jaunder init --db "$JAUNDER_DB" --skip-if-exists` in `preStart`
(`flake.nix:105`), and starts `jaunder serve`. There is no site symlink; the
module comment names #237 as the reason. Two `nixosConfigurations` test VMs
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
([ADR-0042](adr/0042-emacs-org-atom-mapping-struct-seam.md)).

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
(port excluded) and username, and errors when no entry matches.
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
response reader — a metadata harvest, not a full entry parse — returning
`content-src`, `content-type`, `slug`, and `published` from a response entry via
`libxml-parse-xml-region`. Media URLs come from that harvested `<content src>`
and are never reconstructed client-side, so the server stays authoritative about
URL layout ([ADR-0045](adr/0045-emacs-media-content-src.md)). Only local image
links (`png`/`jpg`/`jpeg`/`gif`/`webp`/`svg`, `file:` or `attachment:`) qualify
for upload; the extension table is the qualification predicate shared by
detection and substitution (`jaunder--media-link-p`,
`elisp/jaunder-media.el:48`, over the extension table at `:31`).

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
publish. Commands bind the private `jaunder--active-blog` special via the
`jaunder--with-blog` macro; the transport reads it only through
`jaunder--active-base-url` / `jaunder--active-username`, which error when no
blog is active ([ADR-0047](adr/0047-emacs-publish-orchestration.md)).

The user-facing commands are `jaunder-new-post`, `jaunder-publish`, and
`jaunder-save-draft` (publish forced to `app:draft`). Publish performs all
network mutation before any destructive local change
(`elisp/jaunder-publish.el:178`): map → validate (non-empty body; a `scheduled`
post needs a future `#+DATE:`) → record the machine zone → media upload
(sha256-deduped server-side, pre-flighted so a missing file uploads nothing) →
entry send → write-back → rename to `<slug>.org`. The send is a `POST` create,
or a `PUT` when `JAUNDER_ID` is present, carrying `If-Match` only when the
buffer also records a `JAUNDER_SYNCED` ETag. Write-back persists `JAUNDER_ID`
first, from the `Location` header, before `JAUNDER_SLUG`, `JAUNDER_SYNCED`,
`JAUNDER_SYNCED_AT`, the resolved publish time, and the rename — so any failure,
including a `412` stale-ETag, is recoverable by a plain re-publish
([ADR-0047](adr/0047-emacs-publish-orchestration.md)). Media substitution
applies to the sent body only; the authoring buffer is never modified.

Creates go through `jaunder--create-with-retry`
(`elisp/jaunder-publish.el:151`), which retries a 5xx or **any** signalled error
— the handler is a bare `(error …)` (`:165`), so a non-transport failure such as
a missing auth-source entry is also retried twice before re-signalling — up to
three attempts (≈1s then ≈2s backoff) under **one** `Idempotency-Key`, so the
server dedups the replay. The key is ephemeral, not stable across invocations:
it is a fresh md5 of local entropy per call, so a later re-publish gets a new
key and an edit is never mistaken for a retry. The server side of that contract
was decided in issue [#79](https://github.com/jaunder-org/jaunder/issues/79) as
a follow-on to ADR-0047 — see the Storage section.

### Committed direction

Unit D — the reverse atom→org mapping and per-directory reconcile — is designed
around the same seams (the shared response harvester, the directory-keyed blog
config) but unbuilt: `jaunder--atom->org` (`elisp/jaunder-publish.el:238`) is a
stub that signals "not yet implemented"
([ADR-0042](adr/0042-emacs-org-atom-mapping-struct-seam.md),
[ADR-0047](adr/0047-emacs-publish-orchestration.md)).

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
surface to a redacting `Debug` plus `AsRef<str>` (`Password`,
`common/src/password.rs:19`), `secret, serde` re-opens only the validating serde
bridge for an inbound twin (`common/src/password.rs:64`,
`common/src/invite.rs:26`), `secret, sqlx` re-adds storability
(`host/src/invite.rs:27`), and `no_sqlx, no_ord` gives the bearer-token
`RawToken` the full ergonomic trailer minus storability and ordering
(`common/src/token.rs:109`). Numeric IDs take the fixed `IdNewtype` trailer
(eight of them, `common/src/ids.rs:15-44`); bounded numeric values take the
parameterized `NumNewtype` one, whose bound is declarative and re-run by
`FromStr`, serde, and the column (`PageSize`, `PageOffset`, `RowLimit` —
`common/src/pagination.rs:29,62,81`), with an opt-in `clamp` flag
(`macros/src/num_newtype.rs:430`) for a public bound that should coerce rather
than reject.

There are two kinds of newtype, and the choice is **invariant-first**
([ADR-0101](adr/0101-infallible-kind-is-invariant-first.md)). The reviewer's
question is "is there a string this type should refuse?", not "does the
constructor reject?" — the latter is a property of code already written, and
reading it as evidence about the value is what mislabelled `PostTitle` and
`PostBody`. Both now hand-write a validating `FromStr`
(`common/src/post_title.rs:34`, `common/src/post_body.rs:70`), and no production
type takes `#[str_newtype(infallible)]` today. The diagnostic ADR-0063 §3 draws
from this: a type declared infallible that needs a downstream gate to reject
some of its values was mis-declared — the gate is the invariant, displaced.

ADR-0101 also replaces the trusted door with a typed proof wherever a caller can
supply one. `PostSummary::truncated` takes a `SummarySeed`
(`common/src/post_summary.rs:63,114`) whose three constructors — from a `Slug`,
a `PostTitle`, or the first non-blank line of a `PostBody` — are each infallible
because their source is already non-blank. What remains is a plain length-cap,
the one half of the invariant the door genuinely coerces. The trusted door
survives only where no caller can supply a proof.

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
below it. Twelve enums adopt it, seven of them with `sqlx`: `PostFormat`
(`common/src/render.rs:26`), `TargetKind` (`common/src/visibility.rs:43`),
`MediaSource` (`common/src/media.rs:601`), `SmtpTlsMode`
(`common/src/smtp_tls_mode.rs:18`), the two config-key enums
(`common/src/config_key.rs:103,221`), and `FeedEventStatus`
(`common/src/feed/event_status.rs:17`).

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
([ADR-0071](adr/0071-sqlx-string-newtype-bridge.md)). `.bind(newtype)` binds
directly and `query_as` decodes straight into the newtype, so
`query_as::<_, (PostId, TagId, …)>` makes a swapped destructuring a compile
error where two adjacent bare `i64`s made it invisible. `Decode` re-validates
for a string newtype and re-runs the bound for a `NumNewtype`; it is an
infallible wrap for an `IdNewtype`, which has no value invariant. `Encode` is a
storability capability, not a conversion — which is why `secret` drops the
bridge by default and `no_sqlx` exists. Feature isolation keeps `sqlx` out of
the wasm build, guarded by a `compile_error!` in `common`. Two xtask gates keep
the bridge from being bypassed — `xtask/src/steps/sqlx_newtype_bind_check.rs` on
the write side and `xtask/src/steps/sqlx_newtype_decode_check.rs` (syn-parsed,
allowlist-with-reason) on the read side. Since ADR-0091 there is exactly one
bridge implementation, `macros/src/sqlx_bridge.rs:67`, driven by a `BridgeSpec`;
the three newtype derives, `#[derive(SqlxBridge)]`, and `#[text_enum(sqlx)]` all
call it.

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

**Time.** A timestamp crossing the web boundary is `UtcInstant`
(`common/src/time.rs:26`), a third instant-backed flavor of the convention
wrapping `chrono::DateTime<Utc>`
([ADR-0072](adr/0072-timestamps-cross-boundary-as-utcinstant.md)). Its trailer
is hand-written: the wire form is RFC 3339 via chrono's own serde, and `FromStr`
canonicalizes any offset to UTC, making it the single validation chokepoint and
the hook for the client-side `Field<T>` path. The premise that unblocked it is
that `chrono` is already in the wasm bundle through the unconditional
`web → common → chrono` chain — the `web`-level server-only gate never kept the
crate out. Storage and `common`/`host` internals still carry raw
`DateTime<Utc>`; the newtype is a boundary type.

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
rstest templates — lives _inside_ `storage`, as the single module file
`storage/src/test_support.rs` gated
`#[cfg(any(test, feature = "test-support"))]` (`storage/src/lib.rs:42`)
([ADR-0033](adr/0033-shared-db-test-harness-crate.md)). `storage`'s own tests
reach it through `cfg(test)`; external test crates enable the `test-support`
feature. A separate crate is impossible: it must return `storage::AppState`, so
`storage`'s tests would dev-depend on a crate that depends on `storage`, and
`storage`'s own test target then links two distinct instances of itself
(`E0308: multiple different versions of crate storage`).

There are four templates, not three: `backends` and `backends_matrix` (both
dual; the second is the `#[values]`-based variant), plus `sqlite_only` and
`postgres_only` (`storage/src/test_support.rs:418-448`). The backend axis of
`backends_matrix` is `#[values]`-based because a `#[case]`-based axis cannot
coexist with a test's own named `#[case]` rows; it composes as rows × backends,
and the attribute order is `#[apply(backends_matrix)]`, then the
`#[case::name(..)]` rows, then `#[tokio::test]`
([ADR-0124](adr/0124-rstest-reuse-cross-module-templates.md)). Each template is
`#[export]`ed, so it expands to a name-mangled `macro_rules!` that a plain
`use storage::test_support::backends;` brings into scope and
`#[apply(backends)]` then resolves **by bare name** — no
`#[apply(path::to::template)]` and no `pub use` re-export (ADR-0124;
`storage/src/test_support.rs:430-434`). That is why the templates stay in
`storage::test_support` rather than moving to a consumer.

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
target unmodified (`storage/src/backup.rs:98`; Postgres maps its SQLSTATE class
at `storage/src/postgres/backup.rs:28`).

### The e2e suite

Each e2e check is a NixOS-test VM running Playwright against a real served
instance, one derivation per `{backend}×{browser}` combo (`mkE2eCombo`,
`flake.nix:969`). CI runs `cargo xtask validate --no-e2e` in one job plus a
`{sqlite,postgres}×{chromium,firefox}` matrix — each job
`cargo xtask e2e <backend> <browser>` — aggregated by an `e2e-gate` job, so
branch protection needs two stable names
([ADR-0034](adr/0034-ci-e2e-matrix-distribution.md)). `e2e-gate` also requires
the separate `elisp-integration` job (`.github/workflows/ci.yml:162`). Local
`cargo xtask validate` builds the `e2e-checks` aggregate instead: the same
derivations on one machine.

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
`end2end/tests/fixtures.ts`
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
(`flake.nix:563`) serves both. Elisp is exempt from the Rust coverage gate,
which cannot instrument it — so ADR-0031 rests correctness on ERT discipline, "a
test per pure function" (`:83`). Nothing enforces that: it is a stated
expectation, not a gate.

Live client behavior — transport, auth, publish and media round-trips — runs
against a real server through the self-booting harness
([ADR-0035](adr/0035-elisp-live-integration-harness.md)):
`jaunder-test--with-live-server`
(`elisp/test/jaunder-integration-helper.el:222`) spawns the server, discovers
the port from the `runtime.json` file `serve` writes, and provisions credentials
via `jaunder app-password-create`. The suite
(`elisp/test/jaunder-*-integration.el`, driven by
`elisp/scripts/run-integration-tests.el`) runs hermetically as the
`e2e-elisp-integration` nixosTest — which joins the `e2e-checks` aggregate and
is also its own CI job — and host-side via `JAUNDER_TEST_BINARY` for fast
iteration.

## Verification gates

### The verify ladder & git-enforced gate

The ladder has two rungs, both driven by `xtask` (`xtask/src/lib.rs:452`,
`:487`):

- **`cargo xtask check`** runs the host static checks in **Fix** mode
  (formatters auto-fix), then every repo-shape and type-safety gate, then the
  host unit tests, and — unless `--no-test` — the Nix `wasm-tests`, `coverage`,
  and `doctests` derivations.
- **`cargo xtask validate`** runs the same set **verify-only**, adds
  `wasm-budget` (kept out of `check` because it costs a `nix build .#site`,
  #836), and — unless `--no-e2e` — the e2e aggregate.

Enforcement is git-native ([ADR-0029](adr/0029-git-enforced-verify-gate.md)).
`.githooks/pre-commit` runs a single `cargo xtask check`, compares
`git status --porcelain` before and after, and fails when the run changed the
tree, so the author restages consciously rather than having the fix folded in
silently; since the coverage gate went stateless this fires only on formatting
fixes ([ADR-0050](adr/0050-stateless-coverage-gate.md)). `.githooks/pre-push`
runs `cargo xtask validate --no-e2e`. `validate` opens with a `clean-tree`
precheck that refuses a dirty tree unless `--allow-dirty` and returns before any
expensive step (`xtask/src/lib.rs:801`), making pre-push the one point proving
_what was measured == the committed tip == what CI sees_. Every `cargo xtask`
run self-heals `core.hooksPath` to the tracked, relative `.githooks`
(`xtask/src/git.rs:97`). `SKIP_PRE_COMMIT=1` / `SKIP_PRE_PUSH=1` are deliberate
local escapes; CI is the non-bypassable authority.

The heavy checks are Nix flake check derivations — the hermetic layer. xtask
realizes each via
`nix build -L --keep-failed --accept-flake-config --out-link .xtask/gcroots/<check>`
(`xtask/src/steps/nix.rs:416`): cachix-substituted (an unchanged re-run is a
substitution) and GC-rooted by the out-link, so garbage collection cannot evict
a warm gate. `wasm-tests` returns the browser test verdict directly. Coverage
and doctests use producer/consumer pairs — `nix-coverage` + `nix-coverage-gate`,
`nix-doctests` + `nix-doctests-gate` — whose producers cannot fail; xtask reads
each verdict from the sandbox's `status.json` (`xtask/src/steps/nix.rs`). xtask
itself is host-only; Nix never invokes it back
([ADR-0034](adr/0034-ci-e2e-matrix-distribution.md)).

### What the ladder actually runs

In order, after the host `static-checks` step (`fmt`, `leptosfmt`, `prettier`,
`tsc`, `elisp-fmt`, `ert`, `byte-compile`, `cargo-deny`, `clippy`,
`wasm-clippy`, `tools-fmt`/`tools-clippy`, `xtask-fmt`/`xtask-clippy`), both
rungs run the same host steps (`xtask/src/lib.rs:457`-`:479`):

**Two different things are called `static-checks`, and conflating them is
easy.** The host _step_ above runs fourteen sub-steps, compiling and
non-compiling alike. The Nix `static-checks` _derivation_ (`flake.nix:1146`) is
narrower: only the eight non-compiling checks, behind one `devtool check --all`,
because none of them compiles and so none needs vendored dependencies. The
compiling checks — `clippy`, `wasm-clippy`, `deny` and the two workspace-local
clippy runs — keep their own crane derivations, which is what makes the cheap
`runCommand` over a broad source tree viable for the rest.

| Step                                                       | Guards                                                    |
| ---------------------------------------------------------- | --------------------------------------------------------- |
| `identifier-collisions`                                    | duplicate ADR/migration number prefixes, migration parity |
| `adr-format`, `adr-readme-parity`                          | ADR front-matter shape and the README table               |
| `adr-view-parity`                                          | every accepted ADR is cited in this document              |
| `doc-links`                                                | intra-doc link targets                                    |
| `test-backend-pattern`                                     | dual-backend storage test shape                           |
| `server-fn-registrar`                                      | every `web` `#[server]` fn is in the test registrar       |
| `server-fn-tracing`                                        | each server fn's instrumentation                          |
| `server-fn-coverage`                                       | static lane of the flow-coverage snapshot                 |
| `traced-context`                                           | context propagation                                       |
| `proffered-secret`, `proffered-filename-position`          | untrusted-input handling                                  |
| `no-full-reload`                                           | SPA navigation                                            |
| `e2e-goto-wrapper`, `e2e-scaffold`                         | e2e harness shape; no committed `e2eSalt`                 |
| `target-arch-placement`                                    | host/wasm split at module wiring only                     |
| `thin-components`                                          | `#[component]` control-flow budget                        |
| `sqlx-newtype-bind`, `sqlx-newtype-decode`                 | newtypes at the SQL boundary                              |
| `doctest-fences`                                           | the doctest population Nix cannot reach                   |
| `rendered-html-from-trusted`, `raw-html-door`, `html-sink` | the three XSS doors                                       |
| `xlang-literal`                                            | Rust/TypeScript literal agreement                         |
| `xtask-tests`, `tools-test`                                | xtask's and `tools/`'s own unit tests                     |

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
in both directions — an excluded file must not change it, an instrumented `.rs`
must (`xtask/src/coverage/probe.rs`) — run in CI as
`cargo xtask coverage probe-source` (`.github/workflows/ci.yml:52`). Before
every eval the probe **dirties an excluded tracked file**, `README.md`
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
is `cargo test --workspace --doc`, never package-scoped. The population splits
across two steps: the Nix `doctests` derivation covers the workspace, and the
`doctest-fences` step covers `xtask/` and `tools/`, which the flake's source
filter excludes (`xtask/src/steps/doctest_fences.rs`). Doctests feed no
coverage: `llvm-cov --doctests` is unstable, so `--doc` runs outside
instrumentation.

### Server-fn gates

Two gates guard the `#[server]` surface from opposite ends, both drawing their
inventory from one `syn` enumerator.

`server-fn-registrar` ([ADR-0066](adr/0066-server-fn-test-registrar-guard.md))
exists because test binaries link `web` as an rlib, and dead-code elimination
drops each `#[server]` macro's `inventory`-based auto-registration. One
hand-maintained registrar (`server/tests/helpers/registrar.rs`) is therefore the
sole list, registration is **mandatory** with no per-fn opt-out, and the gate
fails on any `web` `#[server]` fn missing from it, matching on
`(vertical, leaf)`. It checks only the missing direction — a stale entry already
fails to compile.

`server-fn-coverage` ([ADR-0081](adr/0081-empirical-server-fn-flow-coverage.md))
answers a question line coverage cannot: which server entry points a real
browser session drives. The claim is **derived from evidence, not asserted** —
the hit set is extracted from the OTLP traces a passing `sqlite × chromium` e2e
run emits, matched forward from the inventory by `#[tracing::instrument]` span
name plus `code.namespace`. A documentary convention was rejected precisely
because a doc naming a spec that never touches the fn would stay green forever.
The gate has two lanes (`xtask/src/steps/server_fn_coverage_check.rs`): a static
lane in `check`/`validate --no-e2e` that reads only committed files, and an e2e
lane (`server-fn-coverage-regenerate` / `-verify`) that runs on the per-combo
`cargo xtask e2e sqlite chromium` path only.

Neither gate carries an **endpoint-drift check**, and that is deliberate
([ADR-0120](adr/0120-no-endpoint-drift-check.md)). The retired
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
- **`devtool check <name> | --all [--fix]`** is the single implementation of the
  non-compiling static checks (`fmt`, `leptosfmt`, `prettier`, `tsc`,
  `elisp-fmt`, `ert`, `byte-compile`, `tools-fmt` —
  `tools/devtool/src/check.rs`). Both gates invoke the same code: the host
  verify ladder runs `cargo run -p devtool -- check <name>` per step
  (`xtask/src/steps/static_checks.rs`), and the Nix `static-checks` `runCommand`
  runs `devtool check --all` from the prebuilt `devtoolBin` — so each check's
  tool + args live exactly once and host/Nix drift is structurally impossible
  ([ADR-0052](adr/0052-devtool-unifies-static-checks.md)). Compiling checks
  (`clippy`, `cargo-deny`) stay in crane derivations plus host StepSpecs
  ([ADR-0052](adr/0052-devtool-unifies-static-checks.md)).

**xtask is host-only — an enforced invariant.** Nix derivations never invoke
xtask; the flow is strictly one-directional (host `cargo xtask` → `nix build`).
The flake's source filters exclude `xtask/` (`!hasInfix "/xtask/" path` in
`flake.nix`), so an accidental `cargo xtask` inside a derivation fails loudly
rather than running a stale copy, and frequently-edited gate logic never busts
the coverage/e2e cache ([ADR-0028](adr/0028-devtool-vs-xtask-boundary.md)).
xtask is also excluded from the root cargo workspace (`exclude = ["xtask"]`,
with its own `[workspace]` in `xtask/Cargo.toml`), and `tools/` is a second
standalone workspace (`coverage`, `devtool`, `doctests` — `tools/Cargo.toml:3`).
All three manifests pin `resolver = "3"` explicitly, because the two virtual
manifests would otherwise default to resolver 1
([ADR-0104](adr/0104-edition-2024-unsafe-env-and-precise-capturing.md)).

**Workspace layering.** The root workspace's shared crates are target-scoped
([ADR-0058](adr/0058-host-crate-layering.md)): `common` is target-agnostic
(host + wasm, zero host-only cfg carve-outs); `host` holds strictly-host-focused
shared code; and `client` is the wasm-only peer
([ADR-0069](adr/0069-client-crate-wasm-only-home.md)). `host`'s load-bearing
invariant is that it depends on no workspace crate _above_ it — `common` and the
build-time `macros` are permitted — and it may take external infrastructure deps
(`anyhow`, `base64`, `http`, `opentelemetry`, `rand`, `sha2`, `sqlx`, `tracing`)
but never our domain or storage abstractions
([ADR-0058](adr/0058-host-crate-layering.md)). The `macros` proc-macro crate is
orthogonal to that runtime trio — build-time tooling compiled for the compiler
host, home to all workspace proc-macros including the three newtype derives
`StrNewtype`, `IdNewtype` and `NumNewtype`
([ADR-0062](adr/0062-macros-crate-proc-macro-home.md),
[ADR-0063](adr/0063-domain-value-newtype-convention.md)). `macros` is itself a
workspace member, so the coverage source filter admits it and its expansion
logic is measured by in-crate `syn::parse_quote!` tests — ADR-0062 records that
correction itself (`:76-83`, #412).

**Dependency patching.** The workspace carries one temporary git
`[patch.crates-io]` entry: `lettre`, routed to a `jaunder-org` fork pinned by
rev (`Cargo.toml:124-125`) until the mailbox-parsing fix lands upstream
([ADR-0119](adr/0119-lettre-fork-pinned-by-rev.md)) — lettre's own RFC 2822
mailbox grammar cannot re-parse addresses its `Address` type accepts, so
`MessageBuilder::build` fails for a legal quoted local part or address literal
(#297). It is pinned by rev rather than branch so a build input changes only
when someone changes it here; moving it means pushing the fork, updating the
rev, and `cargo update -p lettre`, since the lockfile pins the commit either
way. It is paired with a `deny.toml` `[sources.allow-org]` entry for
`jaunder-org` (`deny.toml:248-253`): `unknown-git` is only "warn", so that entry
is not what makes the gate pass — it is what makes the exception deliberate, and
its removal the signal the patch is gone. Drop both together.

The repository formerly ran a much larger patching apparatus: `atom_syndication`
and `rss` were routed to forks raising their `quick-xml` requirement, with the
hermetic Nix build re-resolving the same revs through crane's vendor override
([ADR-0043](adr/0043-quick-xml-fork-patch.md), now **superseded**). That era is
over. Upstream releases of both crates now depend on `quick-xml` ≥ 0.41, so the
fork bridge was retired and AtomPub Atom document I/O was delegated to
`atom_syndication` ([ADR-0089](adr/0089-upstream-atom-document-io.md)). Nothing
of the apparatus survives: no fork entries in `[patch.crates-io]`, no
`flake = false` fork inputs, and no `overrideVendorGitCheckout` in `flake.nix`.

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

**Rust edition and the one `unsafe` seam.** All workspace crates are on edition
2024 ([ADR-0104](adr/0104-edition-2024-unsafe-env-and-precise-capturing.md)).
Two consequences are load-bearing for tooling:

- Edition 2024 made `std::env::set_var` / `remove_var` `unsafe` (RFC 3543). The
  workspace answer is a single audited seam: `common::test_support::with_env`
  (`common/src/test_support/env.rs`) is the **only** place that names either
  function, and the only env-related `unsafe` block — verified by grep across
  all `.rs` sources. It takes one process-global lock for the whole closure,
  restores prior values on exit including on panic, and ignores lock poisoning.
  It is deliberately not reentrant.
- Return-position `impl Trait` now captures every in-scope lifetime (RFC 3498).
  View helpers that borrow a parameter return `impl IntoView + use<>` — precise
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
kept current by two disciplines: shipping an ADR updates `ARCHITECTURE.md` (and
`CONTEXT.md` when the ubiquitous language changes) in the same change, and a
periodic replay audit re-derives the view from the log plus the code to catch
un-ADR'd drift
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

New ADRs are drafted **out of git** in `docs/adr/drafts/` — the directory is
gitignored except its `README.md`, so a premature number cannot be committed
([ADR-0048](adr/0048-adr-out-of-git-draft-workflow.md)). A draft carries the
heading `# ADR-DRAFT: <title>` and is referenced only by its
`docs/adr/drafts/<slug>.md` path — there is no bare `ADR-DRAFT` token, because
the path is what `promote` can rewrite. At ship, after the final rebase,
`cargo xtask adr promote` assigns each draft the next free number, moves it to
`docs/adr/NNNN-<slug>.md`, strips one `../` level from its link targets,
rewrites its path-form references repo-wide, syncs the README table, and stages
the result — the ADR's first appearance in history is already correctly numbered
([ADR-0048](adr/0048-adr-out-of-git-draft-workflow.md)).

Promotion is also the **acceptance event**
([ADR-0088](adr/0088-promotion-is-the-acceptance-event.md)): in the same pass
that replaces the heading token, `promote` rewrites a `- Status: proposed` line
to `accepted`. Any other token a draft carries — `superseded`, `rejected`,
`deprecated` — is a deliberate authorial claim and survives untouched. The
rewrite alone would not hold the property, so `adr-format` enforces the other
half by rejecting `proposed` on any numbered file; rewrite, gate, and table
renderer share one status-line parse, so they cannot disagree about which line
they are reading (`xtask/src/adr.rs:105`, `xtask/src/adr_readme.rs:152`,
`xtask/src/adr_readme.rs:391`).

Identifier collisions are made loud, not silent
([ADR-0036](adr/0036-identifier-collision-policy.md)): the
`identifier-collisions` gate in `cargo xtask check`/`validate` fails on
duplicate numeric prefixes in `docs/adr/` and the migration directories, and
`cargo xtask adr renumber` resolves an already-committed collision in one
command. The gate must see the _merged_ tree to be worth anything. ADR-0036's
addendum obtained that with a strict up-to-date-before-merge ruleset; the merge
queue supersedes that mechanism while keeping the guarantee — GitHub stacks the
PR on an ephemeral queue branch and runs the required checks there
([ADR-0077](adr/0077-adopt-github-merge-queue.md)).

The ADR index table in `docs/README.md` is a generated projection of the ADR
files' headings and Status lines: `cargo xtask adr sync-readme` (folded into
`renumber` and `promote`) regenerates the number, link, and status cells between
`<!-- adr-table:begin/end -->` markers, titles stay hand-curated, and the
`adr-readme-parity` gate keeps table and directory in agreement, naming
`sync-readme` as its recovery
([ADR-0036](adr/0036-identifier-collision-policy.md)). Parity is not
correctness: the check compares two artifacts and stays green when both are
wrong in the same way, which is why the status rule is enforced at the file, not
at the table ([ADR-0088](adr/0088-promotion-is-the-acceptance-event.md)).

Four gates guard the log, and a draft is invisible to all of them **as a gated
file**. `identifier-collisions`, `adr-format`, and `adr-readme-parity` share one
enumeration rule — non-recursive `read_dir` over `docs/adr/`, then `is_file` →
`.md` → leading number — which excludes a numberless file in a subdirectory
twice over. `doc-links` enumerates tracked files instead, and an uncommitted
draft is not tracked ([ADR-0048](adr/0048-adr-out-of-git-draft-workflow.md)).
The `doc-links` gate has no ADR of its own; its decision lives in issue
[#682](https://github.com/jaunder-org/jaunder/issues/682), cited from
`xtask/src/steps/doc_links.rs:1`.

A draft is **not** invisible as a link _target_, and the asymmetry has teeth.
`doc-links` resolves a target with `.exists()` against the working tree
(`xtask/src/doc_links.rs:207-210`), not against the tracked set — so a tracked
document linking a gitignored draft passes locally, where the pen is populated,
and fails in a fresh clone, where it is not. `adr promote` is what closes the
window: it rewrites every `drafts/<slug>` path-form reference repo-wide to the
assigned number before the branch is pushed
([ADR-0048](adr/0048-adr-out-of-git-draft-workflow.md)). Referencing a draft by
path is therefore not a convention but a prerequisite — a link written any other
way survives promotion pointing at nothing.

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

The claims below are true of the system, but no ADR establishes them. Each is
either a decision worth recording or detail not worth an ADR; the tracking issue
decides which. Two issues hold the current set:
[#937](https://github.com/jaunder-org/jaunder/issues/937) (protocol and content
lifecycle) and [#938](https://github.com/jaunder-org/jaunder/issues/938)
(infrastructure and process).

- **The CLI subcommand surface and the `JAUNDER_*` process configuration.**
  ADR-0102 governs `site_config` database keys, a different surface.
  [#938](https://github.com/jaunder-org/jaunder/issues/938)
- **The NixOS module and the package outputs.** Load-bearing deployment reality
  with no ADR beyond ADR-0008's single-binary framing.
  [#938](https://github.com/jaunder-org/jaunder/issues/938)
- **The cargo-workspace exclusions.** The root `exclude = ["xtask"]` and the
  separate `tools/` workspace are stated in `flake.nix` comments and ADR asides
  only; #276 assumes the tools workspace rather than deciding it.
  [#938](https://github.com/jaunder-org/jaunder/issues/938)
- **The `host_tests` gate steps** (`xtask-tests`, `tools-test`) are recorded in
  no ADR. [#938](https://github.com/jaunder-org/jaunder/issues/938)
- **`auth-source` as the Emacs client credential store.** ADR-0014 decides the
  app-password scheme and the wire header but not where a client keeps the
  secret; ADR-0035 and issue #76 both build on the rule as settled.
  [#938](https://github.com/jaunder-org/jaunder/issues/938)

Two gaps a reader might expect here are absent because the system closed them.
The content-addressed media store is no longer un-ADR'd: ADR-0080 decides the
`<source>/<p1>/<p2>/<sha256>/<filename>` layout, ADR-0084 makes the encoded
filename canonical, and ADR-0090 decides what a media reference is. The
embedded-shell versus on-disk-wasm split no longer exists to document: #239
embedded the SPA shell and #237 embedded the CSR bundle, both closed, so "single
binary" needs no qualification.
