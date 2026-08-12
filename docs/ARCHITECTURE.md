# Architecture

This document is the **materialized view** of the repository's architectural
decision log: the single authoritative statement of the architecture as it is
_now_, folded from the ADRs in [docs/adr/](adr/) (see
[ADR-DRAFT](adr/drafts/architecture-view-materialized-from-adrs.md)). The ADRs
are the immutable events — each records why a decision was made, pinned to its
moment; this view records what is currently true, and every claim cites the
decision(s) that established it. Read this to learn the system; open a cited ADR
only when you need the _why_.

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
the symmetric wasm-only peer. `host` never compiles to wasm, so it uses
`std::fs`/`std::env` without the `#[cfg]` gating `common` would demand
([ADR-0058](adr/0058-host-crate-layering.md)). `client` holds only raw browser
glue (`web_sys`/`js_sys`/`wasm_bindgen` and wasm-side Leptos plumbing) and no
domain types; `web` and `csr` depend on `client`, never the reverse
([ADR-0069](adr/0069-client-crate-wasm-only-home.md)). Proc-macros live apart
from all three, in `macros`
([ADR-0062](adr/0062-macros-crate-proc-macro-home.md)).

| Crate          | Target      | Responsibility                                                                                                                                                                                                   |
| -------------- | ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `common`       | host + wasm | Shared domain logic and types: validated newtypes, rendering, visibility, feed/AtomPub serialization.                                                                                                            |
| `storage`      | host        | Storage traits, record types, SQL migrations, and the SQLite/PostgreSQL backends ([ADR-0019](adr/0019-generic-storage-backend-via-dialect.md)).                                                                  |
| `server`       | host        | The `jaunder` binary: Axum router, CLI, background workers, integration tests.                                                                                                                                   |
| `web`          | host + wasm | Leptos components and `#[server]` functions — the UI and its server halves, split host/wasm at the file level ([ADR-0070](adr/0070-web-vertical-wasm-only-component-files.md)).                                  |
| `csr`          | wasm        | The client-side-rendering entry point: mounts `web` in the browser ([ADR-0041](adr/0041-public-projector-and-csr-client.md)).                                                                                    |
| `host`         | host        | Strictly-host-focused shared code: error carrier, capture dir, auth/token parsing, invites, metrics ([ADR-0058](adr/0058-host-crate-layering.md)).                                                               |
| `client`       | wasm        | Strictly-browser shared infrastructure: `localStorage`, confirm dialog, DOM primitives, file upload, reactive revalidation, and the CSR performance marks ([ADR-0069](adr/0069-client-crate-wasm-only-home.md)). |
| `macros`       | build-time  | The workspace's proc-macro home: newtype, `text_enum`, sqlx-bridge and server-fn derives ([ADR-0062](adr/0062-macros-crate-proc-macro-home.md)).                                                                 |
| `test-support` | host        | A seed binary linking `storage` for out-of-process e2e seeding ([ADR-0046](adr/0046-test-support-seed-binary.md)).                                                                                               |

Every `client` module that touches the browser carries
`#[cfg(target_arch = "wasm32")]`, so a host build of the crate is an
all-but-empty rlib with no coverage-measured browser glue. The one exception is
`client::perf`, whose mark names are plain `&str` data and are therefore pinned
by host tests ([ADR-0069](adr/0069-client-crate-wasm-only-home.md)).

Two sibling trees are outside the root workspace, each its own cargo workspace:
`xtask/` (the host-only dev/CI driver, also named in the root
`exclude = ["xtask"]`) and `tools/` (members `devtool`, `coverage`, `doctests` —
the in-sandbox tools that run where `xtask` is unavailable). Both are covered
under [Development tooling](#development-tooling). `elisp/` (the Emacs client,
[ADR-0031](adr/0031-elisp-separately-tested-subproject.md)) and `end2end/`
(Playwright) are covered in their sections.

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
  (`storage/src/sqlite/posts.rs:53`, `storage/src/sqlite/mod.rs:203`). Where the
  same operation is generic, Postgres reaches the same serialization with
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
before `COMMIT`. The clear-then-load split keeps the two restore shapes
identical and CASCADE-safe
([ADR-0064](adr/0064-backup-target-auto-derivation.md)). Restore refuses any
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

<!-- un-ADR'd: local soft delete (soft_delete_post stamps deleted_at, every public read filters `deleted_at IS NULL`). -->

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
constructor. The derived `sqlx::Decode` is the one in-module path that fills the
field without passing a door, and that is a **deliberately accepted residual
risk**, argued in place (`:190`): typing a column as `RenderedHtml` is itself a
reviewable act, and the static check does not catch it. `ammonia` sits behind a
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
an unusable title falls back to a synthesized slug. Once `published_at` is set
the storage layer freezes the slug (`storage/src/post_service.rs:230`,
[ADR-0027](adr/0027-scheduled-publishing-time-gated-visibility.md)).

**Visibility is two orthogonal predicates on the same reads.** _Time_: a post is
draft (`published_at` NULL), scheduled (future), or live (past); every public
read gates `published_at <= now` with `now` an explicit parameter, the feed
worker's `go_live_pass` (`server/src/feed/worker.rs`) makes future-dated go-live
restart-durable for cached feeds, and publish is an explicit
`PublishUpdate { Unpublish, Publish { at } }` so scheduling and backdating
round-trip ([ADR-0027](adr/0027-scheduled-publishing-time-gated-visibility.md)).
_Audience_: posts target audiences
(`AudienceTarget::{Public, Private, Subscribers, Named}`) stored as
`post_audiences` rows; a viewer is a `ViewerIdentity` (channel identity or
anonymous, `common/src/visibility.rs`) and sees a post iff they are the author
or any targeted audience admits them — OR semantics, failing closed (`Private`
is zero rows); subscriptions route through the admission seam
(`SubscriptionPolicy`, wired to the auto-approving `OpenSubscriptionPolicy`)
([ADR-0020](adr/0020-content-visibility-and-subscription-model.md)).

**Local edits are never destructive**: every update snapshots the pre-edit row
into an immutable `post_revisions` row — title, slug, body, format, rendered
HTML at that moment (`storage/src/sqlite/posts.rs:73`,
`storage/src/postgres/posts.rs:75`; table created in migration
`0008_create_posts.sql`). The rows are write-only today: `PostRevisionRecord`
(`storage/src/posts.rs:119`) has no read query, so no surface exposes edit
history yet.

<!-- un-ADR'd: local revision snapshots. No ADR decides them. ADR-0009 is about
CONSUMED content only ("for followed sources", "when an update is received"),
so it does not cover this despite the resemblance. Same class as soft delete. -->

**Local deletion is soft and un-ADR'd**: `soft_delete_post`
(`storage/src/posts.rs:689,1308`) stamps `deleted_at`, and public reads filter
`deleted_at IS NULL` (26 sites in `storage/src/posts.rs`). Whether a hard delete
ever happens is undecided.

<!-- un-ADR'd: local soft delete. No ADR mentions deletion policy at all, and no
GitHub issue decides it either (searched open and closed). -->

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

The Atom feed document is built by upstream `atom_syndication` — `render_atom`
assembles an `atom_syndication::Feed` and lets the crate emit the XML
([ADR-0089](adr/0089-upstream-atom-document-io.md)). RSS goes through the `rss`
crate the same way.

<!-- un-ADR'd (GAP): `HybridWindow` item selection (`feeds.min_items` /
`feeds.min_days`, `common/src/feed/window.rs`) and `feed_etag` conditional GET
(`common/src/feed/metadata.rs:66`, consumed at
`server/src/feed/handlers.rs:61-81`) are load-bearing feed policy with no ADR. -->

### AtomPub editing interface

The authenticated Collection (`server/src/atompub/mod.rs:28-44`:
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
(`J_NS`, `common/src/atompub/mod.rs:51`;
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
writes itself: the service document, RSD, and the shared XML helpers
(`common/src/atompub/{service,rsd,xml}.rs`).

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

WebSub is publisher-side only. After regenerating a feed, the feed worker pings
the configured hub (the `feeds.websub_hub_url` setting,
`common/src/config_key.rs:165`) through
`WebSubClient::send_publish(&HubUrl, &FeedUrl)` (`server/src/websub/mod.rs:45`)
with bounded retries and a `websub_ping` metric whose outcome distinguishes
`Success`, `Exhausted`, `Failed`, and `NoHub`
(`server/src/feed/worker.rs:239-266`) — an unconfigured hub is a recorded no-op,
not a silent one. `HttpWebSubClient` runs in production; noop and file-capture
implementations back the tests.

<!-- un-ADR'd (GAP): publisher-side WebSub is built and exercised by e2e specs,
but no ADR decides it. ADR-0010 names WebSub only as a future *ingestion*
channel (that leg is issue #921). -->

### Committed direction — inbound federation

[ADR-0010](adr/0010-protocol-integration.md) commits Jaunder to becoming a
unified reader across ActivityPub, AT Protocol, and web feeds: push-first
delivery (ActivityPub inbox, WebSub subscriptions, AT Jetstream), adaptive
polling as the fallback, everything normalized into the unified content model
([ADR-0005](adr/0005-unified-content-model.md)).

**None of it is built.** There is no ActivityPub inbox, no Jetstream consumer,
no polling scheduler, and no fetcher; all 25 migrations under
`storage/migrations/` are publishing-side, with no table for fetched content.
Inbound RSS/Atom ingestion is the first slice (issue #282), with WebSub
subscription (#921), ActivityPub (#287), and adaptive polling (#920) sequenced
behind it. Read every sentence in this subsection as intent, not as description.

## Authentication

Jaunder authenticates over three transports that all resolve to one credential
system: **session cookies** for the web frontend and **Bearer tokens** for API
clients ([ADR-0007](adr/0007-auth-mechanisms.md)), plus **HTTP Basic** carrying
app-specific passwords for AtomPub clients such as MarsEdit
([ADR-0014](adr/0014-atompub-authentication.md)). All three paths converge on
the `AuthUser` axum extractor (`web/src/auth/server.rs`), which resolves
identity through the `SessionStorage` trait
([ADR-0007](adr/0007-auth-mechanisms.md)). Header parsing itself lives in the
target-agnostic `host::auth::resolve_credential`, with precedence: `session=`
cookie, then `Authorization: Bearer`, then `Authorization: Basic`.

<!-- un-ADR'd: the cookie > Bearer > Basic precedence order and the host-crate homing of resolve_credential are implementation reality, not decided in an ADR -->

Leptos server functions obtain the same identity via `require_auth()`
(`web/src/auth/server.rs`), which pulls the request `Parts` from context and
runs the `AuthUser` extractor; failures map to unauthorized/internal errors
through `AuthRejection` ([ADR-0007](adr/0007-auth-mechanisms.md)).

### Credentials and sessions

- A session token is 32 cryptographically random bytes, base64url-encoded
  (`storage::auth::generate_token`); only its SHA-256 digest is persisted
  (`storage::auth::hash_token`) — the raw token is never stored.
  <!-- un-ADR'd: hashed-at-rest token storage is load-bearing but not recorded in an ADR -->
- An **app password** is just a labelled session: minting calls
  `SessionStorage::create_session(user_id, label)` — no separate table, no
  `kind` marker, so tokens are interchangeable across transports (accepted for
  the self-hosted single-user trust model). Sessions never expire;
  `sessions.label` is mandatory (browser logins auto-generate a User-Agent/host
  label, app passwords carry a user-supplied name), and revocation is deleting
  the session in the Sessions UI
  ([ADR-0014](adr/0014-atompub-authentication.md)).
- On the Basic path the supplied username must match the resolved session's user
  (`verify_basic_username` → 401 on mismatch); combined with per-user collection
  URIs this scopes a token for user X to `/atompub/X/*`
  ([ADR-0014](adr/0014-atompub-authentication.md)). Basic sends the token on
  every request, so the TLS-terminating reverse proxy is load-bearing for
  AtomPub ([ADR-0014](adr/0014-atompub-authentication.md)).

Cookie management is layered:
`web::auth::{set_session_cookie, clear_session_cookie}` are leptos adapters over
the pure header builders
`host::auth::{session_cookie_header, clear_session_cookie_header}`, which emit
`session=<token>; HttpOnly; SameSite=Lax; Path=/` (plus `Secure` when the
deployment's `CookieSettings` say HTTPS); clearing sets `Max-Age=0`
([ADR-0007](adr/0007-auth-mechanisms.md)).

<!-- un-ADR'd: the concrete cookie attributes are implementation detail under ADR-0007 -->

### Password hashing

Passwords are hashed with **Argon2id** at the crate-default parameters (m=19456,
t=2) via `common::password::Password::hash`
([ADR-0018](adr/0018-constant-time-authentication.md)). Test builds may enable
the `cheap-kdf` feature for fast hashing, and this fails closed twice: a
`compile_error!` rejects `cheap-kdf` in any release/optimized build, and the
server binary aborts at startup if `common::CHEAP_KDF_ENABLED` is set
(`server/src/main.rs`).

<!-- un-ADR'd: the cheap-kdf feature and its dual fail-closed guard are load-bearing but not ADR'd -->

### Timing discipline: the entropy dividing line

Two deliberate, opposite orderings govern when the expensive Argon2 work runs,
split by the **entropy of the value being validated**:

- **Enumerable identifier (username): equalize timing.**
  `UserStorage::authenticate` performs an Argon2 verification against a fixed
  dummy hash (`storage::helpers::dummy_password_hash()`, computed once via
  `OnceLock` through the real `Password::hash` path so it carries production
  parameters, with a hardcoded valid-hash fallback so initialization is
  infallible) on the absent-user path before returning `InvalidCredentials`,
  closing the username-enumeration timing oracle. **Durable invariant: the
  absent-user path MUST keep this equalizing verification** — do not remove it
  as a "fast path" and preserve it through any refactor or backend dedup.
  Applies identically to both SQLite and Postgres backends
  ([ADR-0018](adr/0018-constant-time-authentication.md)).
- **High-entropy secret (invite code, reset token): cheap-reject first.**
  `create_user_with_invite` validates the invite with a cheap lookup before
  hashing (the SQLite backend takes its write lock up front per ADR-0021, so the
  hash runs inside the immediate transaction on the success path only), and
  `confirm_password_reset` atomically claims the reset token before hashing the
  new password. A ~256-bit secret admits no useful timing oracle, and hashing
  first would turn bogus-secret requests into a CPU-exhaustion amplifier while
  destroying invite issuance as a throttle
  ([ADR-0022](adr/0022-validate-before-expensive-work.md)).

Do not apply the equalizing-dummy-hash rule to high-entropy-secret paths, or
cheap-reject to enumerable identifiers — each ADR carries the scope boundary to
the other ([ADR-0018](adr/0018-constant-time-authentication.md),
[ADR-0022](adr/0022-validate-before-expensive-work.md)).

### Username boundary

Usernames are a validated domain newtype, `common::username::Username` (an
existing exemplar of the proposed
[ADR-0063](adr/0063-domain-value-newtype-convention.md) convention): `FromStr`
lowercases the input and rejects anything not matching `[a-z0-9_-]+`, and the
serde bridge routes wire (de)serialization through the same validation, so
interior code only ever sees canonical lowercase usernames. Web entry points
(login, registration, password reset) lowercase before parsing.

<!-- un-ADR'd: the lowercase-canonical username rule itself predates/escapes the ADR log; only the newtype convention is ADR'd -->

## Web frontend

The web UI is Leptos ([ADR-0002](adr/0002-frontend-framework.md)), rendered
**client-side only** ([ADR-0040](adr/0040-web-rendering-leptos-csr.md)): no SSR,
no hydration, a UI-free server — no reactive page render in the request path,
which structurally eliminates the concurrent-SSR disposal class; server
rendering a reactive component to string is the prohibited trap door back.

### Rendering model: projector + CSR client

The mechanism is "SSR the data, not the components"
([ADR-0041](adr/0041-public-projector-and-csr-client.md)): a thin non-reactive
**public projector** (`server/src/projector/`) renders the anonymous shell for
public routes via the pure render fns in `web/src/render/`, fetching through
explicit-viewer `fetch_*` seams as `ViewerIdentity::Anonymous`, so its output is
byte-identical per URL (CDN-cacheable). It embeds a `PageSeed` JSON blob
(`id="jaunder-seed"`) the CSR client reads on boot to seed first paint;
client-side navigation falls back to the `#[server]` fns, still the data API on
`/api`. Reactive components render their anonymous DOM via `inner_html` of the
_same_ pure fns the projector uses, so the CSR mount causes no reflow:
flash-free by coincidence, not markup twins. The authenticated owner stays
flash-free by _enhancement_
([ADR-0044](adr/0044-authenticated-owner-flash-free-enhancement.md)): an
advisory localStorage auth marker, read by an inline blocking `<head>` script
(`web::render::PREPAINT_SCRIPT`, identical on both HTML surfaces), sets
`<html class="authed">` before first paint; `current_user()` is only a
background reconcile; owner affordances are additive decoration in CSS-reserved
slots on the untouched DOM, never a branch switch. The personalized cockpit is
its own route, `/app`; `/` stays public.

### Crates, features, and the build

`web` is one crate compiling two ways by cargo feature — `csr` (the wasm client)
and `server` (the server-side data-API build; renamed from `ssr`)
([ADR-0041](adr/0041-public-projector-and-csr-client.md),
[ADR-0056](adr/0056-web-canonical-colocated-leptos.md)). The `csr` crate is a
thin wasm entry point calling `web::mount_csr()`. `cargo xtask build-csr` builds
the wasm bundle without cargo-leptos (via `devtool csr-bundle` + wasm-bindgen)
into `target/site/pkg/`, which `jaunder serve` serves alongside the compiled-in
SPA-shell fallback.

<!-- un-ADR'd: the cargo-leptos-free wasm bundling pipeline (xtask build-csr → devtool csr-bundle → target/site/pkg) -->

### Module layout — in-flight migration to co-location

The target layout is the **canonical co-located Leptos CSR shape**
([ADR-0056](adr/0056-web-canonical-colocated-leptos.md)): each feature's
`#[component]` UI, `#[server]` fns, and wire types live together in one module,
split by cargo feature — never `target_arch` — with `#[component]` UI ungated
(dead but coverage-exempt on the host). This migration is **in flight**, one
per-vertical cleanup issue at a time: `web/src/pages/` still holds 17 files
behind the module-level `#[cfg(target_arch = "wasm32")] pub mod pages` gate from
[ADR-0055](adr/0055-web-host-wasm-boundary-module-level.md) (superseded by
ADR-0056, but live until the verticals migrate; `audiences/` is the converted
reference). ADR-0055's surviving rules hold: pure logic keeps a host-tested,
coverage-measured home, and no fake-value host stubs, ever.

### Server-fn surface, DI, and errors

Feature modules follow the server-submodule pattern
([ADR-0013](adr/0013-server-submodule-pattern.md)): shared DTOs and `#[server]`
fns in `mod.rs`, server-only helpers in a feature-gated `server.rs`, every
`#[server]` body wrapped in `boundary!("name", { … })`. Server fns get their
dependencies via per-trait Leptos context, never a bundle —
`expect_context::<Arc<dyn FooStorage>>()`
([ADR-0016](adr/0016-dependency-injection-and-appstate.md)). The owner-pinning
from ADR-0016's SSR-era addenda remains in force: `server_boundary` runs each
body in a `ScopedFuture` holding the full owner ancestry strong, and
`web::server_resource` (raw `Resource::new` is clippy-banned) is the only
sanctioned `Resource` constructor — `expect_context` stays reliable regardless
of await ordering, though the SSR races that motivated the addenda cannot occur
post-CSR.

`web/` is a **thin shell**
([ADR-0059](adr/0059-thin-web-shell-error-layering.md)): it keeps only the
leptos UI, the `#[server]` surface, and the wire types. Errors flow through the
one-way T1→T2→T3 pipeline — typed domain errors (`storage`/`common`) → the
operator carrier `host::error::InternalError` → the wire type `WebError`, via
the lossy projection `web/src/error.rs::project`; the masked public boundary
([ADR-0017](adr/0017-error-handling-and-the-public-boundary.md)) means internal
detail structurally cannot reach a client.

### Reactive idioms

Revalidation goes through one primitive, `web::reactive::Invalidator`
([ADR-0060](adr/0060-web-invalidator-revalidation-idiom.md)): committed
mutations `notify()`, resources `track()`; `action::<A>()` is success-gated;
cross-component scopes are per-vertical `invalidator_scope!` newtypes. Keyed
lists whose rows mutate in place or hold nested state render from a
`reactive_stores::Store` (`#[store(key: …)]`) fed by `Invalidator::patched` (→
`Signal<ListState>`) plus a keyed `<For>` mounted unconditionally; flat lists
stay plain `map`/`collect`
([ADR-0061](adr/0061-web-keyed-list-reactive-store.md)). The style companion is
`docs/web-style-guide.md`.

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
non-fatal. Inbound W3C `traceparent` headers are extracted onto the per-request
span, so backend spans parent into the caller's trace. Span fields and metric
attributes are exported, so they MUST NOT carry user PII or secrets — stable
identifiers (`user_id`, `error.kind`) only.

<!-- un-ADR'd: `with_http_observability` also sets/propagates a request id. -->

E2E tracing is layered ([ADR-0011](adr/0011-unified-observability.md)): an
automatic `e2e.test` span per test (`end2end/tests/fixtures.ts` — request,
navigation, resource, and timed-action summaries) plus opt-in `e2e.flow.*`
semantic-phase spans (`end2end/tests/perf.ts`). Trace context flows via
`JAUNDER_E2E_TRACEPARENT` (`flake.nix` → `end2end/tests/otel.ts`), so
browser-side and backend spans share one trace. In the e2e VMs an otel-collector
writes `otel-traces.jsonl` into the capture dir
([ADR-0057](adr/0057-e2e-capture-dir-contract.md), #332);
`cargo xtask traces analyze` consumes those files offline (see the tooling
section).

<!-- un-ADR'd: e2e VMs also copy out `playwright-report-<backend>.json`. -->

### Metrics

An OTLP `MeterProvider` is installed next to the tracer (`build_otel_meter` in
`server::observability`), behind the same endpoint gate
([ADR-0011](adr/0011-unified-observability.md)). Emits go through the
`host::metrics` facade (`host` is native-only, keeping `opentelemetry` out of
wasm — [ADR-0058](adr/0058-host-crate-layering.md)); every helper takes a
bounded enum, so no call site can attach an unbounded label. `init_tracing`
returns a `#[must_use]` `TelemetryGuard` whose `Drop` force-flushes both
providers on every exit path, so one-shot CLI commands export buffered telemetry
instead of silently dropping it; one binding at the `run()` dispatch boundary
covers every command, and export failures are logged, never propagated
([ADR-0011](adr/0011-unified-observability.md)).

### Errors at the boundary

`server_boundary` (`web/src/error.rs`) logs operator detail as discrete tracing
fields — `error.kind` / `error.class` plus the preserved typed source chain — at
class-appropriate levels, then returns only the masked public error
([ADR-0017](adr/0017-error-handling-and-the-public-boundary.md)). The carrier
(`host::error::InternalError`: `ErrorKind`, `ErrorClass`, `anyhow` source chain)
is the T1 layer of [ADR-0059](adr/0059-thin-web-shell-error-layering.md).

### Scoped server diagnostics (e2e capture)

The single env var `JAUNDER_CAPTURE_DIR` enables app-driven capture
([ADR-0049](adr/0049-app-driven-scoped-server-diagnostics.md),
[ADR-0057](adr/0057-e2e-capture-dir-contract.md)); production leaves it unset
(fully inert). Each stream writes a filename defined once in `host::capture`
(`mail.jsonl`, `websub.jsonl`, `diag.log`), resolved at a composition root. The
diag stream is a WARN+-filtered JSON `tracing` layer plus a panic hook appending
`kind: "panic"` JSONL records through its own `O_APPEND` handle (bypassing
`tracing` to avoid deadlock); the zero-panic gate
([ADR-0032](adr/0032-e2e-zero-panic-gate.md)) sources panics from the union of
`diag.log` and the journal. Per combo the e2e harness tars the directory out as
`capture-<backend>.tar.gz` — those three files plus `otel-traces.jsonl` — into
the [ADR-0037](adr/0037-e2e-failure-diagnostics-capture.md) artifact set.

### Committed direction

- Saturation gauges via async observable callbacks — deferred in
  [ADR-0011](adr/0011-unified-observability.md).
- A configurable diag level and an analyzer over the diag JSONL — left open in
  [ADR-0049](adr/0049-app-driven-scoped-server-diagnostics.md).

## Deployment

Jaunder deploys as a **single self-contained server binary behind an external
reverse proxy** ([ADR-0008](adr/0008-deployment-model.md)). The binary bundles
the application, the default SQLite storage, and its static assets; it never
terminates TLS itself — HTTPS is the reverse proxy's job (nginx, Caddy, …), so
Jaunder binds plain HTTP and production exposure is a proxy-configuration
concern, not an application feature.

**Embedded assets** ([ADR-0003](adr/0003-asset-management.md)). The base
stylesheets `jaunder.css` and `jaunder-themes.css` (in `server/assets/`) are
compiled into the binary via `rust-embed` (`StaticAssets` in
`server/src/assets.rs`) and served at `/style` by `axum_embed::ServeEmbed`, with
ETag/conditional-request support. Theme switching is a `data-theme` attribute on
the HTML shell. User-uploaded stylesheets are deliberately _not_ embedded — they
live in and are served from the storage layer.

**The CSR client's place in the served binary.** The SPA shell (`index.html`) is
embedded in the server as a compile-time constant and served as the fallback for
app routes, so no shell file exists on disk. The wasm bundle itself
(`pkg/jaunder.wasm` + public assets) is served from the on-disk site root via
`ServeDir` — it is a sibling artifact staged next to the binary, the one part of
the deployment that is not inside the executable. Rendering architecture —
leptos-CSR client plus the server-side public projector — is owned by the web
section ([ADR-0040](adr/0040-web-rendering-leptos-csr.md),
[ADR-0041](adr/0041-public-projector-and-csr-client.md)).

<!-- un-ADR'd: the embedded-SPA-shell / on-disk-wasm-bundle split (#239) qualifies ADR-0003/0008's "single binary" and is recorded only in code comments and flake.nix. -->

**CLI surface.** The `jaunder` binary is also the operations tool
(`server/src/cli.rs`): `serve` runs the server; `init` prepares the storage
directory and database; `create-pg-db` bootstraps a PostgreSQL database;
`user-create`, `user-invite`, and `app-password-create` manage accounts;
`smtp-test` verifies mail configuration; `backup` (directory or archive mode)
and `restore` round-trip the data, with the backup target auto-derived from the
storage configuration ([ADR-0064](adr/0064-backup-target-auto-derivation.md),
[ADR-0054](adr/0054-backup-test-homing-and-uniform-restore-failure.md)).

<!-- un-ADR'd: the CLI subcommand surface and the JAUNDER_BIND / JAUNDER_DB / JAUNDER_ENV environment variables have no ADR. -->

**What the flake ships.** `flake.nix` exports `packages.jaunder` (the server
binary), `packages.site` (the wasm bundle + public assets the binary serves from
disk), and `nixosModules.jaunder` — a `services.jaunder` NixOS module that
creates a dedicated `jaunder` user/group, runs from
`StateDirectory=/var/lib/jaunder`, symlinks the site package into place, runs
`jaunder init --skip-if-exists`, and starts `jaunder serve` under systemd. Two
`nixosConfigurations` test VMs (interactive, PostgreSQL) exist for development
only.

<!-- un-ADR'd: the NixOS module and package outputs are load-bearing deployment reality with no ADR beyond ADR-0008's single-binary framing. -->

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

`elisp/jaunder.el` is the umbrella entry point over the feature modules:
transport (`jaunder-transport.el`, plus the `jaunder-service.el`
service-document capability probe

<!-- un-ADR'd: service-document probe module -->), atom (`jaunder-entry.el`,

`jaunder-atom.el`), org mapping (`jaunder-org.el`, `jaunder-datetime.el`), and
commands (`jaunder-publish.el`, `jaunder-media.el`, `jaunder-config.el`,
`jaunder-warn.el`).

### Transport and auth

`jaunder--http-request` is built on `plz`, which drives the `curl` binary;
`url.el` is not used anywhere in the client — headers ride in the curl argv, so
the Basic auth header is sent deterministically, and 4xx/5xx return as a
`(:status :headers :body)` plist, unsignalled
([ADR-0038](adr/0038-emacs-http-transport-plz-not-url-el.md)). Authentication is
the server's app-password Basic scheme
([ADR-0014](adr/0014-atompub-authentication.md); the auth section owns details);
`jaunder--auth-secret` retrieves the app password from Emacs `auth-source`,
keyed on the active blog's host and username.

<!-- un-ADR'd: auth-source as the client-side credential store -->

### Org → Atom mapping

`jaunder--org->atom` returns an abstract `cl-defstruct jaunder-entry`; a
separate `jaunder--atom-entry->xml` renders the wire `<entry>` via built-in
`dom.el`/`dom-print` — the struct seam keeps the mapping pure-data-testable,
catches field-name typos at byte-compile time, and confines all wire knowledge
to one serializer ([ADR-0042](adr/0042-emacs-org-atom-mapping-struct-seam.md)).
The shared `jaunder--harvest-response-fields` parses response entries (XML →
alist); media URLs are harvested from the response `<content src>`, never
reconstructed client-side — the server is authoritative about URL layout
([ADR-0045](adr/0045-emacs-media-content-src.md)).

### Publish orchestration

The `jaunder-blogs` defcustom is the sole configuration: it maps directories to
`(:base-url :username …)` plists, resolved by longest-prefix match on the
buffer's directory and validated loudly (absolute base URL, non-empty username;
an unmatched directory errors)
([ADR-0047](adr/0047-emacs-publish-orchestration.md)). Commands bind the private
`jaunder--active-blog` special via `jaunder--with-blog`; the transport reads it
only through `jaunder--active-base-url` / `jaunder--active-username`, which
error when no blog is active
([ADR-0047](adr/0047-emacs-publish-orchestration.md)).

The user-facing commands are `jaunder-new-post`, `jaunder-publish`, and
`jaunder-save-draft` (publish forced to draft). Publish performs all network
mutation before any destructive local change: validate → media upload
(sha256-deduped, idempotent) → entry send (`POST` create, or `PUT`+`If-Match`
when `JAUNDER_ID` is present) → write-back, persisting `JAUNDER_ID` first (from
the `Location` header) before slug/synced/rename — so any failure, including a
`412` stale-ETag, is recoverable by a plain re-publish
([ADR-0047](adr/0047-emacs-publish-orchestration.md)). Creates carry a stable
`Idempotency-Key` header so the server dedups a retried `POST`.

<!-- un-ADR'd: client Idempotency-Key on create (#79 follow-on in ADR-0047, since built) -->

### Committed direction

Unit D — the reverse atom→org mapping and per-directory reconcile — is designed
around the same seams (the shared response parser, the directory-keyed blog
config) but unbuilt; `jaunder--atom->org` is a stub
([ADR-0042](adr/0042-emacs-org-atom-mapping-struct-seam.md),
[ADR-0047](adr/0047-emacs-publish-orchestration.md)).

## Testing & verification gates

`CONTRIBUTING.md` remains the how-to; this section records the architecture of
the gates.

### The verify ladder & git-enforced gate

The ladder has two rungs, both driven by `xtask` (`xtask/src/lib.rs`):
`cargo xtask check` runs the host static checks in Fix mode (auto-fixing
formatters), the repo-shape guards, the host tests, and — unless `--no-test` —
the Nix coverage check; `cargo xtask validate` runs the same set verify-only,
plus — unless `--no-e2e` — the full e2e aggregate. Enforcement is git-native
([ADR-0029](adr/0029-git-enforced-verify-gate.md)): `.githooks/pre-commit` runs
a single `cargo xtask check` and, if the run changed the tree, fails and asks
the author to restage — since the coverage gate went stateless this fires only
on formatting fixes ([ADR-0050](adr/0050-stateless-coverage-gate.md));
`.githooks/pre-push` runs `cargo xtask validate --no-e2e`. `validate` refuses a
dirty working tree unless `--allow-dirty`, making pre-push the one point proving
_what was measured == the committed tip == what CI sees_. Every `cargo xtask`
run self-heals `core.hooksPath` to the tracked, relative `.githooks`
(`xtask/src/git.rs`). `SKIP_PRE_COMMIT=1` / `SKIP_PRE_PUSH=1` are deliberate
local escapes; CI is the non-bypassable authority.

The heavy checks are Nix flake check derivations — the hermetic layer. xtask
realizes each via `nix build -L --keep-failed --out-link .xtask/gcroots/<check>`
(`xtask/src/steps/nix.rs`): cachix-cached (an unchanged re-run is a
substitution) and GC-rooted by the out-link, so garbage collection cannot evict
a warm gate. xtask itself is host-only; Nix never invokes it back
([ADR-0034](adr/0034-ci-e2e-matrix-distribution.md)).

<!-- un-ADR'd: the ladder also carries sequence_check and host_tests steps (xtask/src/steps/),
recorded in no ADR. -->

### Coverage gate

The coverage verdict is **stateless** — a pure function of
`(coverage report, source tree)`, with no committed baseline, manifest, or merge
driver ([ADR-0050](adr/0050-stateless-coverage-gate.md), superseding the
baseline/re-anchor lineage of
[ADR-0030](adr/0030-coverage-reanchor-text-identity.md)). One workspace-wide
instrumented nextest pass runs in the Nix `coverage` derivation with an
ephemeral PostgreSQL live for the whole run; the gate (`xtask/src/coverage/`)
then applies: structural exemptions for `#[component]` bodies (CSR UI is
validated by the e2e matrix, never host-side) and for `unreachable!("msg")` with
a non-empty message (self-re-flagging: reaching it panics the test) — both
fail-closed; `// cov:ignore` (line or `cov:ignore-start`/`-stop` block) as the
sole manual acceptance path, reviewable in the diff where it lives; and a
per-function CRAP threshold T = 30, overridable only by
`// crap:allow: <reason>`. A tripwire fails the gate if any _covered_ line falls
inside an exempt span, enforcing the "native tests never render components"
assumption. The coverage source is bounded to cargo sources, enforced by a
`drvPath` probe (`cargo xtask coverage probe-source`, run in CI).

### Backend-parity & test homing

The dual-backend harness — `Backend`, `TestEnv`, per-test DB provisioning, the
`backends`/`sqlite_only`/`postgres_only` rstest templates — lives _inside_
`storage` as the feature-gated `storage::test_support` module
([ADR-0033](adr/0033-shared-db-test-harness-crate.md)); a separate crate is
impossible (dev-dependency cycle), and the feature gate keeps it out of release
builds. A storage test is homed by what it proves, and placement is
coverage-neutral under the single PG-live instrumented pass
([ADR-0053](adr/0053-storage-test-homing-and-dual-backend.md)): backend-common
contracts are written `#[apply(backends)]` in the generic home module; a
single-backend test is _presumed_ a Postgres coverage gap unless it has a
decisive backend-exclusive reason. The `test-backend-pattern` guard
(`xtask/src/steps/test_pattern_check.rs`) verifies that every `#[tokio::test]`
under `storage/src/**` carries a backend template or a
`// guard:no-backend — <reason>` marker. Fault-injection hooks are gated on
`#[cfg(any(test, feature = "test-utils"))]`, not bare `#[cfg(test)]`, so
cross-crate integration tests can drive them dual-backend
([ADR-0026](adr/0026-test-fault-injection-hooks-feature.md)). Backup is a
cross-backend _contract_ (a portable dump): its fidelity and negative tests live
in `server/tests/misc/`, including a four-hop `postgres→sqlite→postgres→sqlite`
cycle, and a constraint-violating restore fails uniformly —
`BackupError::ConstraintViolation`, target unmodified — on both backends
([ADR-0054](adr/0054-backup-test-homing-and-uniform-restore-failure.md)).

### E2E architecture

Each e2e check is a NixOS-test VM running Playwright against a real served
instance, one derivation per `{backend}×{browser}` combo (`mkE2eCombo` in
`flake.nix`). CI runs `cargo xtask validate --no-e2e` in one job plus a
`{sqlite,postgres}×{chromium,firefox}` matrix, each job
`cargo xtask e2e <backend> <browser>`, aggregated by an `e2e-gate` job so branch
protection needs two stable names
([ADR-0034](adr/0034-ci-e2e-matrix-distribution.md)); local
`cargo xtask validate` builds the `e2e-checks` aggregate — same derivations,
distributed vs. one machine.

<!-- un-ADR'd: e2e-gate also `needs:` the CI elisp-integration job
(.github/workflows/ci.yml). --> One Playwright config,

`end2end/playwright.config.ts`, is loaded verbatim by both the VM and the host
loop; host/VM differences are invocation flags only
([ADR-0051](adr/0051-single-playwright-config.md)). The host loop is
`cargo xtask e2e-local`, which owns the whole lifecycle — build, spawn
`jaunder serve` on an ephemeral port, seed, run Playwright, tear down. Specs are
parallel-safe via per-test identity fixtures (`user`/`mailbox`/`verifiedUser` in
`end2end/tests/fixtures.ts`); the suite runs at `workers=2`, with the lone
global-singleton spec `admin-site` quarantined in per-browser serial projects
([ADR-0039](adr/0039-e2e-parallelism-via-per-test-identity-fixtures.md)).
Timeout budgets are stated for Chromium and scaled per browser
([ADR-0012](adr/0012-environment-aware-timeouts.md); `slowBrowserTimeoutMs`).

The suite is a zero-panic gate ([ADR-0032](adr/0032-e2e-zero-panic-gate.md)):
each VM testScript asserts the server journal contains no `panicked at` line,
default-deny via an explicit `allowed_panics` list (empty today), so a panic
fails the _derivation_ and can never be cached green. Diagnostics are captured
before the check may fail
([ADR-0037](adr/0037-e2e-failure-diagnostics-capture.md)):
`trace: 'retain-on-failure'`, `screenshot: 'only-on-failure'`, and the shared
`e2eRunAndCapture` helper streams the line-reporter output to `build.log`,
copies all artifacts unconditionally, then asserts the Playwright exit; on a
failed build xtask rescues them from the `--keep-failed` outPath into
`.xtask/diagnostics/<check>/`. Out-of-process state manipulation (seeding,
fixture users, mail reset) goes through the dedicated `test-support` workspace
binary, which links the real `storage` code paths — never a production CLI/HTTP
seed surface or hand-written per-backend SQL
([ADR-0046](adr/0046-test-support-seed-binary.md)). Capture streams write
well-known filenames under one `JAUNDER_CAPTURE_DIR`, lifted per combo as a
tarball ([ADR-0057](adr/0057-e2e-capture-dir-contract.md) — see observability).

### Elisp testing

`elisp/` is a first-class, separately-tested subproject
([ADR-0031](adr/0031-elisp-separately-tested-subproject.md)): host `ert` and
`elisp-fmt` steps run in both `check` (Fix) and `validate` (Check), with the
hermetic mirror now the `static-checks` derivation via `devtool check`
([ADR-0052](adr/0052-devtool-unifies-static-checks.md)); one `emacsForCi`
toolchain serves both. Elisp is exempt from the Rust coverage gate
(cargo-llvm-cov cannot instrument it; the stated expectation is a unit test per
pure function). Live client behavior — transport, auth, publish/reconcile
round-trips — runs against a real server via the self-booting harness
([ADR-0035](adr/0035-elisp-live-integration-harness.md)):
`jaunder-test--with-live-server` spawns `jaunder serve --bind 127.0.0.1:0`,
discovers the port race-free from the `runtime.json` file `serve` always writes
(which doubles as a startup mutex), and provisions credentials via
`jaunder app-password-create`. The suite (`elisp/test/*-integration.el`, via
`elisp/scripts/run-integration-tests.el`) runs hermetically as the
`e2e-elisp-integration` nixosTest in the `validate` tier and as its own CI job,
and host-side via `JAUNDER_TEST_BINARY` for fast iteration.

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

ADR-0028's original "devtool = in-sandbox, xtask = host" charter has been
deliberately softened: devtool is now the **shared host/sandbox tool**, and two
of its subcommands run host-side by design.

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
  <!-- un-ADR'd: ADR-0052 chartered 7 non-compiling checks; the set is now 8 — `byte-compile` was added and the former tsc-deps step folded into `devtool check tsc`. -->

**xtask is host-only — an enforced invariant.** Nix derivations never invoke
xtask; the flow is strictly one-directional (host `cargo xtask` → `nix build`).
The flake's source filters exclude `xtask/` (`!hasInfix "/xtask/" path` in
`flake.nix`), so an accidental `cargo xtask` inside a derivation fails loudly
rather than running a stale copy, and frequently-edited gate logic never busts
the coverage/e2e cache ([ADR-0052](adr/0052-devtool-unifies-static-checks.md)).
xtask is also excluded from the root cargo workspace (`exclude = ["xtask"]`,
with its own `[workspace]` in `xtask/Cargo.toml`), and `tools/` is a second
standalone workspace (`coverage`, `devtool`).

<!-- un-ADR'd: the cargo-workspace exclusions themselves (root `exclude = ["xtask"]`, the separate `tools/` workspace) are stated only in flake.nix comments and ADR asides, never decided in their own ADR. -->

**Workspace layering.** The root workspace's shared crates are target-scoped
([ADR-0058](adr/0058-host-crate-layering.md)): `common` is target-agnostic
(host + wasm, zero host-only cfg carve-outs); `host` holds strictly-host-focused
shared code; a `client` crate is the reserved future peer for wasm-only shared
code. `host`'s load-bearing invariant is that it depends on no workspace crate
except `common` — it may take external infrastructure deps (today `anyhow`,
`http`, `opentelemetry`, `sqlx`, `tracing` in `host/Cargo.toml`) but never our
domain/storage abstractions ([ADR-0058](adr/0058-host-crate-layering.md)). The
`macros` proc-macro crate is orthogonal to that runtime trio — build-time
tooling compiled for the compiler host, home to all workspace proc-macros: the
`#[client_only]` identity attribute that `xtask/src/coverage/exempt.rs`
recognizes and exempts alongside `#[component]`
([ADR-0062](adr/0062-macros-crate-proc-macro-home.md)), and the
`StrNewtype`/`IdNewtype` derives
([ADR-0063](adr/0063-domain-value-newtype-convention.md)). ADR-0062's claim that
the crate "contributes no gate-measured lines" is out of date: only the expanded
output in consumer crates escapes measurement — `macros` itself is a workspace
member, so the coverage source filter auto-admits it and its expansion logic
(including derive error paths) is measured via in-crate `syn::parse_quote!` unit
tests in `macros/src/lib.rs`.

**Dependency patching.** The workspace carries one temporary git
`[patch.crates-io]`: `atom_syndication` and `rss` are routed to `jaunder-org`
forks at pinned revs (root `Cargo.toml`) that raise their quick-xml requirement
to ≥ 0.41, clearing RUSTSEC-2026-0194/0195 without an advisory ignore. The
hermetic Nix build resolves the same revs via `flake = false` inputs fed to
crane's `overrideVendorGitCheckout` (`flake.nix`). The apparatus is deleted once
upstream releases depend on quick-xml ≥ 0.41
([ADR-0043](adr/0043-quick-xml-fork-patch.md)).

## Documentation & decision process

The documentation architecture is event-sourced: ADRs in `docs/adr/` are
append-only decision events, and this document — `docs/ARCHITECTURE.md` — is the
materialized view folded from them
([ADR-DRAFT](adr/drafts/architecture-view-materialized-from-adrs.md)). An ADR's
Decision text is never edited to track the present; when a decision changes, a
new ADR supersedes it with reciprocal pointers. In-place ADR edits are limited
to metadata and navigation (status lines, moved pointers, short past-tense
annotations), and any new addendum is written in past tense from birth — "as of
<date>, Y held" — never as a present-tense patch. The view is kept current by
two disciplines: shipping an ADR updates `ARCHITECTURE.md` (and `CONTEXT.md`
when the ubiquitous language changes) in the same change, and a periodic replay
audit re-derives the view from the log plus the code to catch un-ADR'd drift
([ADR-DRAFT](adr/drafts/architecture-view-materialized-from-adrs.md)).

The documentation landscape, per [ADR-0000](adr/0000-documentation-strategy.md)
as amended by
[ADR-DRAFT](adr/drafts/architecture-view-materialized-from-adrs.md):

- `docs/adr/` — the decision log (MADR-style, the "why"). Each ADR's line-1
  heading is `# ADR-NNNN: <title>` and its status is a single token from
  `{proposed, accepted, superseded, deprecated, rejected}` on a `- Status:`
  line, machine-checked by the `adr-format` gate
  ([ADR-0036](adr/0036-identifier-collision-policy.md)).
- `docs/ARCHITECTURE.md` — the materialized view; every claim cites its ADR(s),
  and current reality is kept distinct from committed direction.
- `CONTRIBUTING.md` — process (setup, verify, land); it cross-links the view
  rather than restating structure. Root `CONTEXT.md` — the domain glossary. Both
  are projections in the same sense.
- `docs/DESIGN.md` — functional behavior and operational model;
  `docs/ROADMAP.md` — strategic vision and milestones.
- `docs/archive/` — shipped, dated specs/plans and milestone documents.
  <!-- un-ADR'd: ADR-0000 says transient docs are *deleted* once captured;
  current practice (issue #39) archives them as dated files in docs/archive/
  at ship instead. -->

New ADRs are drafted **out of git** in `docs/adr/drafts/` — the directory is
gitignored except its `README.md`, so a premature number cannot be committed
([ADR-0048](adr/0048-adr-out-of-git-draft-workflow.md)). A draft carries the
heading `# ADR-DRAFT: <title>` and is referenced only by its
`docs/adr/drafts/<slug>.md` path. At ship, after the final rebase,
`cargo xtask adr promote` assigns each draft the next free number, moves it to
`docs/adr/NNNN-<slug>.md`, rewrites its path-form references repo-wide, syncs
the README table, and stages the result — the ADR's first appearance in history
is already correctly numbered
([ADR-0048](adr/0048-adr-out-of-git-draft-workflow.md)).

Identifier collisions are made loud, not silent
([ADR-0036](adr/0036-identifier-collision-policy.md)): the
`identifier-collisions` gate in `cargo xtask check`/`validate` fails on
duplicate numeric prefixes in `docs/adr/` and the migration directories, the
branch-protection ruleset requires PRs to be up to date with `main` so the gate
runs against the merged tree, and `cargo xtask adr renumber` resolves an
already-committed collision in one command. The ADR index table in
`docs/README.md` is a generated projection of the ADR files' headings and Status
lines: `cargo xtask adr sync-readme` (folded into `renumber` and `promote`)
regenerates the number, link, and status cells between
`<!-- adr-table:begin/end -->` markers, titles stay hand-curated, and the
`adr-readme-parity` gate keeps table and directory in agreement
([ADR-0036](adr/0036-identifier-collision-policy.md)).
