# Jaunder codebase analysis — simplification, security, robustness, observability

_Date: 2026-06-12. Scope: the Rust workspace (`common/`, `storage/`, `server/`, `web/`), excluding tests except where they bear on a finding._

## Executive summary

The codebase is in good health. Highlights worth preserving:

- **No `unsafe` anywhere** in the workspace.
- **Production code is effectively panic-free.** The ~696 `unwrap()`/`expect()` matches are almost entirely inside in-file `#[cfg(test)]` modules. Spot checks of the worst offenders by raw count show **0** non-test `unwrap`/`expect` in `server/src/lib.rs` and `server/src/cli.rs`, and 1 in `server/src/media_manager.rs`. The raw grep total is misleading.
- **Sound crypto hygiene:** Argon2id for passwords, 256-bit CSPRNG session tokens (`rand::rng().fill_bytes`) stored only as SHA-256 hashes, parameterized SQL throughout, a `Password` newtype with `Display` deliberately unimplemented and `Debug` redacted.
- **A deliberate public/operator error split** (`WebError` vs `InternalError`) with a logging boundary (`server_boundary`).
- **Rich, structured observability** (OpenTelemetry OTLP, pervasive span instrumentation, a custom slow-span layer) per ADR-0011.

The findings below are improvements on top of that baseline, ordered within each section by impact.

---

## 1. Simplification

### 1.1 SQLite and Postgres storage backends are near-duplicates *(high impact)*

`storage/src/sqlite/*` and `storage/src/postgres/*` are paired implementations of the same traits (`UserStorage`, `SessionStorage`, `MediaStorage`, `FeedEventStorage`, `FeedCacheStorage`, `UserConfigStorage`, `SiteConfigStorage`, …). For most methods the two files are **byte-for-byte identical except for the pool type and the span-name prefix**.

Concretely, `SqliteUserStorage::create_user` and `PostgresUserStorage::create_user` share the *same SQL string* (both already use `$1..$N` placeholders), the same `is_unique_violation()` error mapping, the same `Instrument` spans — differing only in `SqlitePool` vs `PgPool` and `storage.sqlite.*` vs `storage.postgres.*` span names. The same is true of `authenticate`, `get_user`, sessions, and the rest.

This is the single largest source of accidental complexity and the highest-leverage change. Every schema or query change must be made twice and kept in sync by hand; the `storage/` tree carries roughly double the query code it needs.

**Options (in rough order of preference):**

1. Push the shared query bodies into `storage::helpers` as functions generic over `E: sqlx::Executor` (or over `sqlx::Database`), and reduce each backend struct to a thin wrapper that supplies its pool. This keeps backend-specific quirks (e.g. `RETURNING` differences, type decoding) overridable while collapsing the identical 90%.
2. A declarative macro per trait method that takes the pool type and span prefix.
3. Carry the backend identity as a span **field** (`db.system = "sqlite" | "postgres"`) rather than baking it into the span *name* — this both enables option 1/2 and makes cross-backend dashboards far easier (see §4.5).

Trade-off to weigh: the project uses runtime `sqlx::query`/`query_as` (not the compile-time `query!` macros), so there is no compile-time-checking cost to abstracting these. The main risk is dialect divergence (datetime handling, `RETURNING`, upserts); a generic helper with per-backend override points contains that.

### 1.2 `init_tracing_impl` has a 6-way branch explosion *(medium)*

`server/src/observability.rs::init_tracing_impl` enumerates the cross product of {OTel present, OTel build failed, no OTel} × {JSON, pretty} as six nearly identical `registry().with(...).try_init()` chains. This is hard to extend (adding a metrics layer would double it again).

Collapse it by building optional, type-erased layers and composing once:

```rust
let fmt_layer = if use_json { fmt::layer().json().boxed() } else { fmt::layer().boxed() };
let otel_layer = tracer.map(|t| tracing_opentelemetry::layer().with_tracer(t));
registry().with(env_filter).with(slow_span_layer).with(fmt_layer).with(otel_layer).try_init();
```

`Option<Layer>` implements `Layer`, and `.boxed()` erases the fmt-format type, so one chain covers all six cases.

### 1.3 Parallel error ladders `WebError` / `InternalError` *(low — but a free win inside it)*

`web/src/error.rs` defines two enums with parallel constructor sets (`not_found`, `validation`, `conflict`, `storage`, `server`, …). It *looks* repetitious, but most of it is **API surface, not duplicated logic** — the deliberate cost of the public/operator split (§2.4), where each failure mode legitimately gets an ergonomic constructor on both types. Separate the two:

**Genuine logic duplication (removable for free).** One body appears verbatim three times — `InternalError::{not_found, validation, conflict}` (lines 98–123):

```rust
let public = WebError::X(...);
let operator_message = public.to_string();
Self { public, operator_message }
```

Extract it into one trait impl and the three constructors collapse to one-line delegates **with no call-site change** (`InternalError::validation("x")` still works), gaining a `?`-ergonomic conversion as a bonus:

```rust
impl From<WebError> for InternalError {
    fn from(public: WebError) -> Self {
        let operator_message = public.to_string();
        Self { public, operator_message }
    }
}
// pub fn validation(m: impl Into<String>) -> Self { WebError::validation(m).into() }  // etc.
```

Two things correctly stay explicit: `unauthorized` (its operator message is the *passed* argument, not `public.to_string()` — really a masking constructor), and the masking variants `storage`/`server`/`server_message` (operator ≠ public by design). `storage`/`server` share a shape and could call a tiny private helper, but they differ in variant *and* literal message, so it saves ~2 lines — optional. **Caveat:** scope `From<WebError>` mentally to the expected/client variants — `WebError::storage(err).into()` would promote the *leaky* public constructor without masking (the §2.4 hazard), so the masking path must remain the explicit `InternalError::storage(err)`.

**The rest is irreducible surface.** `WebError`'s one-line `Self::Variant { message: x.into() }` constructors and the both-types symmetry carry distinct variants/masking decisions — no shared body to extract. A declarative macro *could* table-generate the pairs and would leave call-sites byte-for-byte identical, but it moves the cost to the definition site (go-to-definition lands on a macro; constructors stop being greppable). At ~6 variants that's a bad trade — revisit only if the ladder grows. The `From` impl is strictly better than a macro for the part that's actually duplicated, because it's plain Rust *and* adds a useful conversion.

### 1.4 Large mixed-concern files *(informational)*

`storage/src/backup.rs` (1042 lines) interleaves export, restore, archive/tar handling, Sqlite/Postgres dispatch, manifest validation, and a large test module. `web/src/pages/ui.rs` (1418) and `web/src/pages/posts.rs` (1184) are large UI modules. None are urgent; flagged for future decomposition along the export/restore seam.

### 1.5 Duplicated "empty string → `None`" normalization — masking a semantic inconsistency *(low effort; correctness angle)*

The "blank input means absent" normalization is open-coded ~8 times as `if s.is_empty() { None } else { Some(...) }` (e.g. `web/src/profile/mod.rs:53` & `:58` for `display_name`/`bio`, `web/src/pages/ui.rs:620`, `web/src/pages/posts.rs:531` & `:548`, `web/src/site/mod.rs:38`, `web/src/backup/mod.rs:122`, `storage/src/site_config.rs:33` & `:92`), and the *same idea* is also written idiomatically as `.filter(|s| !s.is_empty())` in another half-dozen places (`web/src/auth/mod.rs:97` & `:153`, `server/src/observability.rs:40`, `storage/src/site_config.rs:106`, `storage/src/post_service.rs:136` & `:216`). The codebase disagrees with itself on how to spell the operation.

A helper already exists but is **mis-scoped**: `web/src/posts/mod.rs:37 fn normalize_summary(s: Option<String>) -> Option<String>` is exactly this function (trim, then empty→`None`), just private and named for one field — itself an instance of the duplication.

The real payoff is **correctness, not line count**: the sites disagree on *whether to trim first*. `profile/mod.rs` checks `is_empty()` without trimming, while `ui`/`posts`/`site`/`post_service` and `normalize_summary` all trim. So a `display_name` of `"   "` becomes `Some("   ")` on the profile path but `None` everywhere else — a latent inconsistency the copy-paste hides. Consolidating forces a single answer.

**Suggestion:** a small documented helper in `common`, e.g. `pub fn non_empty(s: &str) -> Option<&str>` (`{ let t = s.trim(); (!t.is_empty()).then_some(t) }`) for the borrowed shape, folding `normalize_summary` into an `Option<String>` variant; standardize the pure `Option<String>` sites on the std `.filter(...)` one-liner where a helper would be overkill. **Decide the intended semantics first** — do blank display names become `None`? — because consolidation changes behavior on the non-trimming paths; this is a deliberate decision, not a mechanical replace.

### 1.6 `web/src/posts/mod.rs` — coherent theme, three sub-concerns *(low effort; one is a cohesion fix)*

At 975 lines (≈840 before its test module) the file looks oversized, but the trigger for acting isn't the line count — it's that it bundles three distinct concerns behind one theme ("the post HTTP surface"): 7 DTO structs, 16 `#[server]` functions, and the tests. The functions cluster cleanly:

- **Single-post lifecycle** — `create_post`, `get_post`, `get_post_preview`, `update_post`, `publish_post`, `unpublish_post`, `delete_post`, `list_drafts` (shares `normalize_summary`, `PostResponse`, and the `*Result` DTOs).
- **Listing / timelines / feeds** — `list_user_posts`, `list_local_timeline`, `list_home_feed`, `list_posts_by_tag`, `list_user_posts_by_tag` (≈235 lines; shares `TimelinePostSummary`/`TimelinePage` and pagination/windowing shape).
- **Post-format preference** — `get_default_post_format`, `set_default_post_format`.

Two seams, both low-risk because `#[server]` functions register by their `endpoint = "…"` string, not module path — so moving them has no routing impact and `mod.rs` just re-exports:

1. **Relocate the format-preference pair (do regardless — a cohesion fix, not decluttering).** These don't touch post storage; they wrap `UserConfigStorage` (`DEFAULT_POST_FORMAT_KEY`), i.e. a *user preference* that merely concerns posts. They belong with user/profile preferences. Smallest, highest-value move.
2. **Extract the listing/feed cluster into `posts/listing.rs`.** Independently coherent, sizable, and its DTOs cleave along the same seam — leaving the core module as the single-post lifecycle. Result: ~3 files of ~200–350 lines, each navigable.

Don't fragment further: the residual lifecycle cluster is genuinely cohesive (shared helper + `PostResponse`), and chopping finer trades cohesion for module ceremony. **Before extracting, confirm `PostResponse`/`normalize_summary` usage across clusters** (traced here by signature, not by reading every body) so the split doesn't leave a shared helper awkwardly pulled back into `mod.rs`. Related to the §1.4 large-file note, but distinct: this is a cohesion split with a clear seam, not just a big file flagged for someday.

### 1.7 `server/src/lib.rs` — only ~100 of 895 lines are route creation *(low effort; clear seams)*

The crate root bundles three concerns plus their tests; only the router is the file's actual job:

| Concern | Lines (approx) | Belongs in `lib.rs`? |
|---|---|---|
| `create_router` (route table + Leptos/state/context wiring) | 42–141 (~100) | **yes** |
| Backup worker subsystem (`start_backup_worker`, `run_scheduled_backup`, `prune_backups`, `timestamped_backup_name`, `backup_path_for_mode`) | 148–238 (~90) | no — self-contained, zero routing coupling |
| HTTP trace-context plumbing (`ExtractedTraceContext`, `HeaderExtractor`, `extract_trace_context`) | 241–266 (~25) | no — OTel propagation glue |
| Tests (dominated by backup) | 268–895 (~427) | follow their code |

Two moves, both low-risk pure code movement:

1. **Backup subsystem → `server/src/backup.rs`.** A complete scheduling subsystem with no dependency on the router; the ~5 backup tests that dominate the test module travel with it. Only call-site change: `crate::start_backup_worker` → `crate::backup::start_backup_worker` in the serve path. Naming stays consistent with the `common::backup` / `storage` backup / `web::backup` family.
2. **HTTP observability layer → existing `server/src/observability.rs`.** The trace-context helpers are used by the `http_observability` `ServiceBuilder` built inline in `create_router` (lib.rs:50–82) — the `extract_trace_context` middleware plus the `make_span_with` closure that reads `ExtractedTraceContext`. Rather than moving only the helper structs, extract the whole layer into e.g. `observability::http_layer()` returning the configured layer, so `create_router` calls `.layer(crate::observability::http_layer())`. This co-locates all the OTel/tower construction with the rest of the tracing setup.

After both, `lib.rs` is `create_router` + module declarations (~100–120 lines) — genuinely "where we create our routes." **Verify before lifting:** the `make_span_with` closure references nothing local to `create_router` (it reads `ExtractedTraceContext` from request extensions, so it appears self-contained). Same family as §1.6 — a cohesion split along clear seams.

### 1.8 Finish the topic-aggregate config pattern (`FeedsConfig`) *(low effort; make grouped getters the default interface)*

`site_config` keys are namespaced by topic (`backup.*`, `feeds.*`, `site.*`), and the **aggregate-struct-per-topic** interface already exists for three of the four topics — it just was never applied to `feeds`:

| Topic | Aggregate struct | Grouped getter/setter |
|---|---|---|
| `backup.*` | `BackupConfig` | `get_backup_config` / `set_backup_config` ✓ |
| `site.*` | `SiteIdentity` | `get_identity` / `set_identity` ✓ |
| SMTP | `SmtpConfig` | `load_smtp_config` ✓ |
| **`feeds.*`** | **none** | only granular: `get_feeds_min_items`, `get_feeds_min_days`, `get_feeds_websub_hub_url` |

This is the convention, not a new direction: every call site that touches `backup`/`site` already uses the grouped getter (`web/src/site/mod.rs`, `server/src/atompub/mod.rs`, `web/src/backup/mod.rs`, `server/src/lib.rs`). The only places chaining granular getters are the two that touch `feeds.*`.

**The gap and its call sites:**
- `server/src/feed/regenerate.rs:37–55` chains `get_feeds_min_items` + `get_feeds_min_days` + `get_feeds_websub_hub_url` + `get_identity`, each with its own `.await.map_err(storage_err)?`. The three `feeds.*` calls collapse into one `get_feeds_config() -> FeedsConfig { min_items, min_days, websub_hub_url }` (mirroring `get_backup_config`) — which is also exactly what feeds the adjacent `HybridWindow { min_items, min_days }` + hub logic.
- `server/src/feed/worker.rs:47,51` fetches `get_feeds_websub_hub_url` + `get_identity` — same topic, same `FeedsConfig` would serve it.

**Move:** add `FeedsConfig` + `get_feeds_config`/`set_feeds_config` to complete the set, and make the grouped getter the default interface.

**Why it's better than the chain** (beyond fewer `map_err` lines): defaults and invalid-value coalescing live in *one* place — the existing `get_backup_config` already does this (`get_backup_config_ignores_invalid_stored_values`), so each hand-rolled granular caller is a spot for that logic to drift; and a topic struct gives the settings page one type to render a form from and write back (the `get_/set_identity` round-trip).

**Caveats:** keep the granular getters for genuine single-value needs — a grouped getter does N key reads even when the caller wants one (cheap for `site_config`, so clarity wins, but "default interface," not "only interface"). Secondary win worth noting: `get_backup_config` currently issues *four separate* `get()` round-trips; a grouped getter is the natural place to batch them into one `WHERE key IN (...)` read (read-consistency + perf bonus, not required). The `RegenerateError::Storage(String)` flattening at this call site is also a §3.1 stringly-variant — natural to fix together.

### 1.9 `AppState` is an attractive nuisance — dissolve the omnibus via constructor injection *(medium effort; structural)*

**The real problem isn't fan-out — it's that the bundle keeps accreting non-storage junk.** Consumers were already narrowed to take only the `Arc<dyn XStorage>` they need (good interface segregation), so the classic God-Object coupling is mostly solved at the *consumer* boundary. What remains is a drift problem: `AppState` is a single type that is **both** (a) capable of holding any dependency **and** (b) routinely threaded into every server fn context and subsystem — and any type with both properties accretes junk. Evidence: `AppState` carries `websub: Arc<dyn WebSubClient>`, an outbound-HTTP publisher with no storage role, *despite* an in-source comment that the mailer "deliberately lives outside this bundle … not a storage concern." The comment is a **sign-post**; agents (and humans in a hurry) follow topology, not prose — the lowest-friction way to make websub reachable from the feed worker was to add a field, so an agent did. The fix must change the gradient, not add another "don't."

**Diagnosis to record (it's the durable test):** a type is an attractive nuisance when it is *both* a heterogeneous dependency holder *and* passed beyond the composition root. You cannot remove (a) from a storage bundle, so remove (b).

**Target shape — constructor injection, factory only at the root:**
- **Each subsystem declares its needs as constructor parameters.** `FeedWorker::new(posts, feed_cache, feed_events, websub)` — the signature *is* the dependency declaration, and a reviewer sees a weird new parameter immediately (vs. a field quietly appearing on a 14-field bag).
- **A `Backend`/storage factory owns the pool and mints `Arc<dyn *Storage>` handles — used *only* at the composition root**, never injected into subsystems (that would be a service locator and would re-broaden the coupling just fixed). Typed to produce *only* storage, so a non-pool-derived thing like `websub` has no natural slot.
- **Services (mailer, websub) are server-constructed and injected per-consumer — no "services bundle"** (that's just a smaller nuisance with the same disease). The mailer already does this; it is the template the websub change should have followed.
- The composition root assembles: build `Backend`, build each service from env, construct each subsystem with exactly the handles its constructor asks for, and provide the per-trait Leptos contexts for the web layer (already done that way — web server fns `expect_context::<Arc<dyn UserStorage>>()`, so the web *consumers* are already segregated; the remaining whole-`AppState` consumers are the feed worker, media manager, backup worker).

**On the "why construct storage that's never used" worry (invites / email-verification / password-resets / feed-events):** mostly unfounded *for the handles* — `SqliteUserStorage::new(pool)` is an `Arc` clone, no I/O; the one shared expensive resource (the pool) is built once regardless, so lazy handles save microseconds. The worry is *valid* for **resource-holding services** (background workers, mailer, websub client, the pool) — those should be gated on config/feature at the root, and several already are (`start_backup_worker` returns `None` when unconfigured; websub falls back to `NoopWebSubClient`). So: build storage handles freely; gate services. The `Backend` factory gives lazy handle construction as a *free side effect* (you only mint what a constructor asks for) without the locator cost — but it's not the motivation.

**Why this stops the drift without policing:** with no omnibus threaded everywhere, there is no bag to dump into; the lowest-friction way to add a dependency becomes "add a constructor parameter and thread it from the root," which forces the right question. The structural invariant enforces itself once the tempting type is gone — far stronger than the mailer comment.

**Caveats:** breadth doesn't vanish, it *concentrates at the composition root* (which is allowed to know everything — that's its job); the win is demoting the God Object from "a type passed around" to "a few lines of wiring at the edge." Verify no `*Storage` impl holds per-instance state (in-memory cache/memoization) — they look like pure pool-wrappers, but if one isn't, mint-on-demand would split that state and you'd want a single shared instance regardless. **Measure first:** if in practice only `serve` assembles storage and legitimately needs all thirteen, the `Backend` subset-selection value is small and the real wins are just (a) moving the few whole-`AppState` consumers to constructor injection and (b) extracting `websub` + the workers as injected services — a smaller, safer change than a full factory rewrite. **Smallest proving step:** convert the feed worker (it also wants `websub`) to constructor injection — demonstrates the pattern, gives websub its proper home, and shrinks `AppState` without committing to the factory.

**Durable rule worth writing into `CONTRIBUTING.md`/an ADR:** *No type may be both a heterogeneous dependency holder and passed beyond the composition root; dependencies are declared by constructor parameters on the component that uses them.* This is the structural form of the intent the mailer comment only gestured at.

> **P0b verification (2026-06-13): CONFIRMED — mint-on-demand is safe; no `*Storage` impl holds per-instance state.**
> All 22 concrete storage structs (11 traits × {sqlite, postgres}) hold exactly one field — `pool: SqlitePool` / `pool: PgPool` — and nothing else (no cache, memoization, atomic, or counter). They are pure pool-wrappers, so splitting a handle (minting a fresh `Arc<dyn XStorage>` per consumer) only clones an `Arc<Pool>` and cannot diverge state. §1.9's "build storage handles freely; gate services" assumption holds, and a `Backend` factory that mints handles on demand is sound. This *de-risks* the factory but does not by itself justify it — see "Measure first" above; the gating concern remains only for resource-holding *services* (pool, workers, mailer, websub client), which are already config/feature-gated.

> **P0a decision (2026-06-13): see [ADR-0016](decisions/0016-dependency-injection-and-appstate.md).** Direction chosen: constructor injection first (Phase A — extract websub + workers as injected services, drop `websub` from `AppState`), then a storage `Backend` factory at the composition root (Phase B). The "Measure first" question is answered: **non-serve entry points DO over-construct** — `cmd_init`/`cmd_user_create`/`cmd_user_invite`/`cmd_smtp_test` all build the full 13-handle bundle *and* a `reqwest` `HttpWebSubClient` via `default_client_from_env()` while using ≤1 handle (or discarding the bundle), so the factory earns its keep. The `WebSubClient` trait's final home is **`server`** (entire `common/src/websub` module moves there; nothing in `common`/`storage` references it once §1.9 lands). The durable invariant is recorded in CONTRIBUTING.md.

### 1.10 `websub` module: server-only code stranded in `common`, forcing scattered `cfg`s *(low effort; the concrete instance of §1.9 + a `cfg` cleanup)*

The motivating instance of §1.9, plus a self-contained win. `common/src/websub/mod.rs` carries five `#[cfg(not(target_arch = "wasm32"))]` attributes — and it is the **only file in all of `common/` with any `target_arch` cfg**. The cause is *not* a client-bound type intermixed with server code (nothing in `web/` uses websub at all; the sole consumer is `server/src/feed/worker.rs`). It is the inverse: entirely **server-only code stranded in `common`**, which is compiled for both native *and* wasm32 (the hydrate build). The native-only impls (`http.rs` → `reqwest`, `file_capture.rs` → `std::fs`/`std::env`) can't compile for wasm, so they're hand-gated; the pure parts (trait, `WebSubError`, `NoopWebSubClient`) compile on wasm and slip through ungated — which is exactly why the gating looks "almost random." The `cfg`s are a hand-maintained shadow of a crate boundary.

**The split (corrected from an earlier "move it to `storage`" suggestion — websub does *nothing* storage-oriented):** the only storage-layer tie is that `storage::AppState` holds the `Arc<dyn WebSubClient>` and imports just the **trait**. So it splits along exactly the wasm-safe / native-only line the `cfg`s trace by hand:

| Piece | Nature | Home |
|---|---|---|
| `WebSubClient` trait, `WebSubError`, `NoopWebSubClient` | pure, wasm-safe; needed by `AppState` + test helpers | a native-only crate (`storage` today, or wherever the abstraction lands after §1.9) |
| `HttpWebSubClient`, `FileCapturingWebSubClient`, `default_client_from_env` | native-only (`reqwest`, `std::fs`, `std::env`); sole user is the feed worker | `server` |

`common` loses the module entirely → **zero `target_arch` cfgs left in `common`**. Every `cfg` disappears not by gating more carefully but because each piece lands in a crate with a single compilation target.

**Interaction with §1.9:** the cleanest end state isn't "trait in `storage`" — it's that under §1.9, `websub` becomes a server-constructed *service injected into the feed worker*, so the trait travels with its consumer and `AppState` never holds it. Do §1.10's `cfg`-removing move regardless; let §1.9 decide the final resting place of the trait. **Verify before moving:** no other `common/` module imports `websub` (only `web/` and one `server` consumer were checked); and confirm the target crate is never built for wasm (`web`'s dep on `storage` should be `ssr`-gated). Note: the pervasive `#[cfg(feature = "ssr")]` in `web/` is a *different*, legitimate Leptos thing — don't conflate it with this `target_arch` smell.

> **P0b verification (2026-06-13): CONFIRMED, with one correction to the consumer map.**
> - No `common/` module imports the `websub` *module* except the `pub mod websub;` declaration in `common/src/lib.rs`. The hits in `common/src/feed/{rss,json}.rs` are doc comments and `rel="hub"` string literals, not imports.
> - `storage` is never built for wasm: `web/Cargo.toml` gates its storage dep as `dep:storage` under the `ssr` feature, and the `storage` crate contains **zero** `target_arch`/`wasm` cfgs.
> - **Correction:** the native-only constructor `default_client_from_env()` is **not** called only from `server`. It is called from inside `storage` — `storage/src/sqlite/mod.rs:70` and `storage/src/postgres/mod.rs:211` — where the AppState builders mint the `Arc<dyn WebSubClient>`. The feed worker (`server/src/feed/worker.rs`) only *consumes* the trait (`state.websub.send_publish`). **Implication:** §1.10's "move the native impls to `server`" cannot fully land (zero cfgs) without §1.9, because storage's AppState builders currently construct the native client themselves. The trait + `NoopWebSubClient` can move to `storage`; the native `HttpWebSubClient`/`FileCapturingWebSubClient`/`default_client_from_env` must move to `server`, which **forces** AppState to stop holding `websub` (server constructs and injects it into the feed worker) — i.e. §1.10's clean end state *requires* the §1.9 feed-worker constructor-injection step, not merely "lets §1.9 decide." Sequence kq8w.10 after the feed-worker injection in kq8w.76lg.

---

## 2. Security

### 2.1 Username enumeration via authentication timing *(medium)*

In `storage/src/sqlite/users.rs` (and the Postgres twin) `authenticate` returns `InvalidCredentials` **immediately** when the username is not found, but runs a full Argon2id verification (tens of milliseconds) when it *is* found:

```rust
let Some((... hash ...)) = row else {
    return Err(UserAuthError::InvalidCredentials);   // fast path: no hashing
};
let valid = verify_password(password.clone(), hash).await?;  // slow path
```

The error *value* is correctly uniform, but the **response time** is not, giving a remote attacker a reliable oracle to enumerate valid usernames.

**Fix:** when the user is absent, still perform an Argon2 verification against a fixed dummy hash (constant-time-ish equalization), then return `InvalidCredentials`. Keep a single dummy hash constant in `helpers`.

### 2.2 `params.hash[2..]` can panic in the media serve handler *(medium — DoS / robustness)*

`server/src/media.rs::serve_handler` validates the path prefix with:

```rust
if !params.hash.starts_with(&params.p1) || !params.hash[2..].starts_with(&params.p2) {
```

`params.hash` is an attacker-controlled URL path segment. If it is shorter than 2 bytes, or `[2..]` lands off a UTF-8 char boundary, the slice **panics** and the request task aborts (500). `common/src/media.rs::media_path` similarly slices `sha256[..2]` / `[2..4]` unguarded (safe there because the input is a stored hash, but the serve path is not).

**Fix:** validate `hash` up front — require it to be exactly 64 lowercase hex chars (`[0-9a-f]{64}`) and reject otherwise with `NOT_FOUND` *before* any slicing or path joining. This also subsumes 2.3.

### 2.3 Unvalidated path components joined into the filesystem path *(low — defense in depth)*

The serve handler joins `params.p1/p2/hash/filename` directly into `storage_path/media/...`. Axum path segments cannot contain `/`, so classic `../` traversal is blocked, but `filename`/`hash` are otherwise unconstrained (e.g. a literal `..` segment). The `starts_with` check gates `hash`/`p1`/`p2` against each other but `filename` is never validated. Run incoming `filename` through the existing `sanitize_filename`/`validate_filename` (reject `..` and empties) before joining, mirroring the upload path. With 2.2's hex check in place, the residual risk is small, but the asymmetry (upload sanitizes, serve does not) is worth closing.

### 2.4 Public error constructors can leak internal detail *(medium — audit needed)*

`web/src/error.rs` exposes **both** a leaky and a masking ladder:

- `WebError::storage(err)` / `WebError::server(err)` embed the full `error_with_sources(err)` chain — i.e. raw DB/driver text — into a message that is serialized to the client.
- `InternalError::storage/server` correctly mask the public side (`"storage operation failed"`) while retaining the operator detail for logs.

If any server-function boundary uses the `WebError` constructors directly instead of going through `InternalError`/`server_boundary`, internal errors (SQL fragments, file paths) reach the browser. **Recommendation:** audit call sites of `WebError::storage`/`WebError::server`; standardize on `InternalError` at every boundary; consider deleting or `#[doc(hidden)]`-gating the leaky public constructors so they can't be reached by accident.

### 2.5 Content-Disposition header built by unescaped interpolation *(low)*

```rust
format!("attachment; filename=\"{}\"", params.filename)
```

A `"` (or other special char) in `filename` breaks the header value / disposition parsing. CR/LF injection is prevented by axum's segment parsing, but quote-breaking is not. Use RFC 6266 `filename*=UTF-8''…` percent-encoding, or at minimum strip/escape `"` and control chars. (Closing 2.3 narrows the input but doesn't fully encode it.)

### Security positives (keep)

Argon2id with per-hash salt; 256-bit tokens from a CSPRNG, persisted only as SHA-256; `Password` newtype that refuses to `Display` and redacts `Debug`; uniformly parameterized SQL; backup identifier SQL drawn from a fixed allowlist (`TABLES_IN_EXPORT_ORDER`); `CookieSettings.secure`; auth rejections mapped to correct 401/500 without detail leakage.

---

## 3. Robustness — error classification, handling, reporting

### 3.1 Error data fidelity & operational context *(medium — the carrier, not the taxonomy)*

The **outward** taxonomy is good and should stay as-is: the domain enums (`UpdatePostError`, `TaggingError`, `RegisterWithInviteError`, `ConfirmPasswordResetError`, …) model expected failures as discrete variants (`NotFound`, `Unauthorized`, `SlugConflict`, `InviteExpired`, `AlreadyUsed`), and most "unexpected" variants preserve the full `sqlx::Error` via `#[error(transparent)] Internal(#[from] sqlx::Error)` — so the source chain (pool timeout vs constraint vs I/O, plus Postgres SQLSTATE/constraint reachable through `as_database_error()`) survives up to the web boundary.

The problem is **lopsided effort**: the outward enum is rich while the *inward/operator* carrier is a flat string. Information is destroyed at three specific seams, and — more importantly — errors capture *cause* but never *context*, *severity*, or anything a log pipeline can aggregate on. Three moves, increasing in effort; the theme is **stop stringifying, carry structure until the moment of emission, emit fields not prose.**

#### A. Preserve the source everywhere *(cheap, mechanical)*

A handful of variants dissolve a structured error into `Display` text and must be widened to hold the source (`#[from]`/`#[source]`):

| Site | Variant | What's lost |
|---|---|---|
| `storage/src/users.rs:57` | `UserAuthError::Internal(String)` (4 `e.to_string()` sites) | `sqlx::Error` kind + SQLSTATE + source — **on the auth hot path** |
| `server/src/feed/regenerate.rs` | `RegenerateError::Storage(String)`, `BadUrl(String)` | underlying sqlx / URL-parse error |
| `storage/src/post_service.rs` | `PerformCreationError::InvalidSlug(String)` | the slug-validation error type |
| `common/src/mailer.rs` | `MailError::Send(String)` | the `lettre` transport error — connect-fail vs auth-fail vs bad-recipient indistinguishable |

Minor consistency fix alongside: `PerformUpdateError::Storage(sqlx::Error)` / `PerformCreationError::Storage(sqlx::Error)` retain the value but not via `#[from]`/`#[source]`, so it isn't wired into the `source()` chain.

#### B. Make the internal carrier structured, and emit fields at the boundary *(the one structural change — contained to `web/src/error.rs`)*

Replace `InternalError`'s flat `operator_message: String` (and the `error_with_sources` concatenation that feeds it) with discrete, queryable data:

```rust
pub struct InternalError {
    public: WebError,                                          // outward, already good
    kind: ErrorKind,                                           // Storage|Validation|Auth|NotFound|Conflict|External
    class: ErrorClass,                                         // Client|Transient|Bug|External  (see C)
    context: Vec<(&'static str, String)>,                      // ("post_id","42"), ("user_id","7"), ...
    source: Option<Box<dyn std::error::Error + Send + Sync>>,  // preserved, NOT stringified
}
```

and at `server_boundary`, log the chain *and* the structure as separate fields instead of one message:

```rust
tracing::error!(
    server_fn,
    error.kind   = ?err.kind,
    error.class  = ?err.class,
    error.public = ?err.public,
    error.source = err.source.as_deref().map(tracing::field::display),
    // plus each context k/v as its own field
    "server function failed",
);
```

`count by error.kind`, `filter error.class = Bug`, and "every failure touching `post_id=42`" then become trivial queries — and `error.kind`-as-a-field is exactly the hook §4.1 needs to derive an error-rate metric without a separate pipeline.

> **P0b verification (2026-06-13): mostly compiles as sketched, with two concrete gotchas the carrier reshape (kq8w.16) must resolve.**
> Threading today (verified): `boundary!(name, { … })` (`web/src/lib.rs:5`) is a thin wrapper → `server_boundary(name, async move { … }).await`; the block returns `InternalResult<T>` (`Result<T, InternalError>`). **There are no `From` impls for `InternalError`** — every fallible call inside a boundary converts via an explicit constructor (`.map_err(InternalError::storage | ::server | ::validation | ::server_message | …)`). That gives a single, clean chokepoint per constructor to derive `kind`/`class` and to capture (rather than stringify) the source. `InternalError` today is `{ public: WebError, operator_message: String }` and derives only `Debug`.
> 1. **Source capture requires a bound tightening, not just a field add.** The constructors currently take `impl Error + 'static` and immediately flatten via `error_with_sources(&error)`. To hold `source: Option<Box<dyn Error + Send + Sync>>` the constructor bound must become `impl Error + Send + Sync + 'static`. `sqlx::Error` and the `thiserror` domain enums are `Send + Sync`, so this is expected to hold — but it is a signature change with caller-visible effect; audit that no boundary passes a non-`Send` error. `#[derive(Debug)]` still works (the box is `Debug`).
> 2. **"plus each context k/v as its own field" does NOT compile with the static `tracing::error!` macro.** `tracing` field names are compile-time tokens; a runtime `context: Vec<(&'static str, String)>` cannot be expanded into distinct fields in one macro invocation (and `span.record` also needs fields declared up-front). This is the §3.1-B code block directly contradicting the "Carrier choice" guidance below (point 2: *put structured context on the span via `#[instrument(fields(...))]`*). Reconcile in kq8w.16: emit `context` either (a) as **span fields** declared up-front on each server fn's `#[instrument]` (preferred — matches the carrier-choice section and inherits into the boundary log for free), or (b) as a **single serialized field** (`error.context = ?err.context`). Do not promise per-key dynamic fields. The rest of the boundary sketch (`error.kind`, `error.class`, `error.public`, `error.source = …map(tracing::field::display)`) compiles. **Net:** the change stays contained to `web/src/error.rs` + the boundary, as claimed.

#### C. Add an operational `ErrorClass` so triage is mechanical *(small)*

The current taxonomy encodes the outward HTTP-ish mapping but not operational severity: a `sqlx::Error::PoolTimedOut` (infra — alert/retry) and a benign unique-violation surfaced as `Internal` are indistinguishable to a dashboard. Classify on conversion and let the boundary pick log level from the class:

- `Client` — 4xx, expected, never alert (validation, not-found, unauthorized) → `debug`/`info`
- `Transient` — retryable infra (`PoolTimedOut`, connection reset, `Io`) → `warn`, feed retry/alert
- `Bug`/`Invariant` — "can't happen" (`CreatedNotFound`, decode failures) → `error`, page
- `External` — downstream (`MailError`, WebSub) → distinct, so a mail outage doesn't read as a Jaunder bug

sqlx supplies the inputs (`as_database_error()` for SQLSTATE/constraint; `matches!` on `PoolTimedOut`/`Io`). Optionally capture a `std::backtrace::Backtrace` when first creating a `Bug`/`Transient` error — never for `Client` (not worth the cost on a 404).

#### Carrier choice: use `anyhow`, with one deficiency to design around

`anyhow` is the right carrier for the operator side (A/B/C): it already backs the `server/` crate, is widely understood, gives `.context()` and automatic `RUST_BACKTRACE` capture for free, and is server-only — `InternalError` is already `#[cfg(feature = "ssr")]`, so it never needs to cross the wire (the public `WebError` does). Don't roll your own. The split is the idiomatic one: **`thiserror` typed enums in `storage/`/`common/` for matching + outward mapping, `anyhow` as the operator-context carrier inside `InternalError`.**

The one specific deficiency: **`anyhow` erases the type and its `.context()` is a *stringly* chain, not structured key-values.** Two consequences to plan for:

1. *Keep the domain enums typed.* Do **not** collapse `storage/` errors into `anyhow::Error` — you'd lose the ability to map `UpdatePostError::NotFound` → 404 vs `Unauthorized` → 403 without `downcast_ref`. `anyhow` belongs only at the point you've already decided "this is an opaque internal failure, mask as 500, log richly." Derive `ErrorClass`/`kind` from the typed error *before* boxing into `anyhow`, or via `downcast_ref` at the boundary.
2. *Put structured context on the span, not in `anyhow`'s string context.* `.context("updating post 42")` gives prose again — the thing B is trying to escape. Instead carry the vertical cause-chain + backtrace in `anyhow`, and let the `#[instrument(fields(post_id, user_id, …))]` **span** carry the structured horizontal context; the boundary log already inherits span fields. That division (anyhow = why it failed; span fields = what it was doing) gets you both the human-readable chain and the queryable fields without `error-stack`/`snafu` (which are more capable here but far less widely understood — not worth trading away `anyhow`'s ubiquity).

**Net:** keep the outward `WebError` enum simple; invest in the inward carrier. A and C are small and mechanical; B is the single structural change and is contained to `web/src/error.rs` plus the boundary.

### 3.2 `AtomPubError::Malformed(String)` collapses distinct failure classes *(low)*

`common/src/atompub/mod.rs` funnels both `quick_xml::Error` and `std::io::Error` into one stringly variant. For a wire-format boundary this is defensible, but distinguishing "I/O while reading" from "well-formed XML, wrong schema" from "unsupported entity reference" would let the AtomPub endpoints return more precise status codes. Consider at least separating I/O from parse/validation.

### 3.3 Add an allowlist guard to backup identifier SQL *(low — future-proofing)*

`storage/src/backup.rs::order_by_clause` / `quote_identifier` build SQL identifiers by matching on table name. Today every caller passes a constant from `TABLES_IN_EXPORT_ORDER`, so there is no injection. To keep it that way as the code evolves, add an explicit `debug_assert!`/early-return guard that the table is a member of the allowlist before constructing any query string.

### 3.4 Minor TOCTOU in media serve *(informational)*

`serve_handler` checks `file_path.exists()` and then `File::open`. The open error is already mapped to `NOT_FOUND`, so the `exists()` pre-check is redundant and slightly racy — it can be removed, relying solely on the `open` result.

### 3.5 Missing adoption of the validated newtypes *(medium)*

The project already defines validated newtypes — `Username` (`[a-z0-9_-]+`), `Tag` and `Slug` (`[a-z0-9][a-z0-9-]*`), `Password` (min length), `MediaSource` (enum), and uses the `email_address` crate's `EmailAddress` — each constructed only via `FromStr`/parse so interior code works with already-valid values. Adoption is uneven: several sites carry raw `String`/`&str` where a newtype belongs, dropping the guarantee. Note up front that `Username`/`Tag`/`Slug` derive only `Clone, Debug, PartialEq, Eq[, Hash]` — **no `Serialize`/`Deserialize`** — which is *why* the web boundary uses `String` and parses manually.

> **P0b verification (2026-06-13): the decode-time error path is more complicated than "→ `ServerFnErrorErr::Args`"; this materially affects gap-5/§3.5b (kq8w.18).**
> If validating newtypes gain `#[serde(try_from = "String")]` (§3.5b) and are used directly as `#[server]` arguments, a bad value fails at **argument-decode time**, before the fn body — but the resulting `ServerFnErrorErr` variant depends on the input encoding, and jaunder uses **both**:
> - **Default `#[server]` fns** (no `input =`) use leptos's default `PostUrl` input → `server_fn::codec::url`'s `FromReq` → `ServerFnErrorErr::Args`.
> - **`input = Json` fns** (`/create_post` `web/src/posts/mod.rs:144`, `/list_tags` `web/src/tags/mod.rs:36`, and any future ones) → `server_fn::codec::post` → `ServerFnErrorErr::Deserialization`.
>
> Current `WebError::from_server_fn_error` (`web/src/error.rs:80`) collapses **every** variant into `WebError::ServerFunction { message: value.to_string() }` (covered by the existing `server_function_errors_map_to_web_error` test). So a decode-time validation failure surfaces as an opaque `ServerFunction` error, **not** `Validation` (4xx) — for *both* encodings.
> **Inputs for kq8w.18:** to give user-facing validation on bad `Username`/`Tag`/`Slug` args, `from_server_fn_error` must special-case **both `Args` and `Deserialization`** → `WebError::Validation`. Caveat (cannot be fully resolved at this layer): `Deserialization` also fires for a genuinely malformed/truncated request body, which is *not* a user-validation error — both arrive as the same variant carrying only a `String`, so the mapping cannot perfectly distinguish "field failed `try_from`" from "body was garbage." Also weigh that `value.to_string()` echoes the serde/`try_from` message to the client. This is why §3.5 blesses the existing "take `String`, parse on the first line inside the boundary" pattern (`login`/`register`) — it keeps validation failures as typed `Validation` errors and sidesteps the decode-variant ambiguity entirely. Recommend kq8w.18 prefer in-body parsing for new arguments unless the `Args`/`Deserialization` → `Validation` mapping is added deliberately.

**Not holes (validate-at-the-edge, working as intended).** `login`/`register` (`web/src/auth/mod.rs`) take `username: String, password: String` but parse into `Username`/`Password` on the first lines inside the boundary. `ServeParams.source: String` is parsed to `MediaSource` in the handler. These are fine; the `String` is just the wire type.

**Genuine gaps:**

1. **`FeedSurface` (`common/src/feed/feed_path.rs:32-34`) — highest value.** The enum holds `username: String` / `tag: String`, and `parse()` validates them only with *ad-hoc* checks (`is_empty()`, `contains('/')`). Those checks **diverge from the canonical validators**: `Username::from_str` lowercases and enforces `[a-z0-9_-]+`; `Tag::from_str` enforces a leading-alphanumeric `[a-z0-9][a-z0-9-]*`. The feed-path parser therefore accepts strings the real types reject (uppercase, dots, leading hyphen, non-ASCII), creating *two disagreeing notions of "valid username/tag."* Fix: make the enum hold `Username`/`Tag` and parse through their `FromStr` in `parse()` — this removes the divergence and types the struct in one move.

2. **Email downgraded in the verification path.** `UserRecord.email` is properly `Option<EmailAddress>`, but `EmailVerificationStorage::create_email_verification(email: &str, …)` (`storage/src/email.rs`) takes a raw `&str` and `use_email_verification` returns `(i64, String)`. Email is validated on the user record but flows unvalidated through the verification trait. Make these `&EmailAddress` / `EmailAddress`.

3. **Internal storage helpers drop the type.** The feed-window helpers — `window_user_sqlite(username: &str)`, `window_site_tag_sqlite(tag: &str)`, `window_user_tag_sqlite(username: &str, tag: &str)` and their Postgres twins (`storage/src/{sqlite,postgres}/posts.rs`) — take `&str` for values the caller already holds validated. No injection risk (they are `bind`-ed params), but the guarantee is lost across the call. Take `&Username`/`&Tag`. Best folded into the §1.1 dedup, since those signatures get rewritten anyway.

4. **CLI `create-user` (`server/src/cli.rs:143`) `username: String`.** clap can fail fast via `#[arg(value_parser = …)]` wired to `Username::from_str`, rejecting a bad username at parse time instead of deep inside `create_user`. Minor.

5. **`common::tag::parse_and_validate_tags` builds `Tag`s and immediately discards them — `Vec<String>` → `Vec<String>` (with a latent normalization bug).** The purest instance of the anti-pattern: it constructs a validated `Tag` per token, uses it for dedup, then pushes the *raw token* and returns `Vec<String>`:
   ```rust
   let tag = Tag::from_str(trimmed).map_err(...)?;   // validates + normalizes
   if seen.insert(tag.to_string()) { out.push(trimmed.to_string()); }  // discards `tag`
   ```
   It should return `Result<Vec<Tag>, _>`. **This isn't only type hygiene — it's a correctness fix.** `Tag::from_str` lowercases *before* validating, so the dedup key is lowercased (`"rust"`) while the pushed value keeps original case (`"Rust"`). A `"Rust"` input is therefore accepted and persisted un-normalized, and the stored value diverges from the dedup key. Returning `Vec<Tag>` and pushing `tag` (the normalized form) removes the drift for free. Both callers (`web/src/posts/mod.rs:157`, `:327`) feed the result straight into the tagging storage path, so the typed return is fully ergonomic once gap 3's `&[Tag]` work lands; done in isolation it adds one `.iter().map(Tag::as_str)` at the storage boundary — worth it for the bug fix alone. Bonuses: `Tag` derives `Hash`, so the internal `seen: HashSet<String>` can dedup on `Tag` directly; the `Err(String)` return is a §3.1 stringly variant. *(Caveat: dormant if every caller pre-lowercases or the write path re-normalizes — unverified — but the function shouldn't depend on that; normalizing is its job.)* See §3.6 for the correctness angle.

**Enabling refactor.** Add **validating** serde to `Username`/`Tag`/`Slug` so they can be used directly as `#[server]` args and DTO fields, deleting the manual `.parse()` calls (deserialization itself then runs `FromStr`):

```rust
#[derive(Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Username(String);
```

**Correctness caveat — state this loudly:** it must be `#[serde(try_from = "String")]`, *not* a plain `#[derive(Deserialize)]`. A plain derive on a tuple struct constructs the inner `String` directly and **bypasses `FromStr` entirely**, yielding a "validated" type that was never validated — strictly worse than the `String` it replaced. The `try_from` route is what keeps the type honest over the wire.

**Scope caveat — the enabling refactor does *not* extend to secret or auth-validated server-fn arguments.** Two specific limits that a future change (human or agent) must respect:

- **Never type a password argument as `Password`.** `#[server]` arguments are bidirectional — the client *serializes* them and the server *deserializes* them — so any argument type needs **both** `Serialize` and `Deserialize`. `Password` deliberately has no `Display` and a redacted `Debug` precisely so the plaintext can't leak; giving it a `Serialize` impl to make it a server-fn arg reopens exactly that hole (a stray `serde_json::to_string(&pw)` anywhere would then expose it). The correct pattern for a secret is the current one: keep `password: String` at the wire and parse to `Password` on the first line inside the boundary. Typing the argument would be a regression.
- **Typing a non-secret argument (e.g. `username: Username`) is optional, with a real tradeoff.** It moves validation into the framework's argument-*decode* step, where failures surface as a generic `ServerFnErrorErr::Args` → `WebError::ServerFunction` rather than the clean, controlled `WebError::Validation { message }` the hand-written parse produces today. Because there is no way to steer the decode-time error message, the explicit parse in `login`/`register` is a *deliberate* choice and should stay. The strongest payoff for typed values is therefore **internal post-validation plumbing** (gap 3) and **return DTOs** (no secret, no inbound-validation error path) — not the auth wire arguments.

Suggested sequence: (1) `FeedSurface` → `Username`/`Tag`; (2) add `try_from`/`into` serde to the newtypes, then thread them through **internal call sites and return DTOs** (leaving secret/auth-validated `#[server]` *arguments* as `String` + explicit parse, per the scope caveat); (3) email path → `EmailAddress`; (4) storage `&str` → `&Username`/`&Tag`, folded into the dedup.

### 3.6 Tag normalization drift in `parse_and_validate_tags` *(low — but a latent correctness bug, not just typing)*

Flagged here because it is a genuine bug, not only a §3.5 typing nicety. `common::tag::parse_and_validate_tags` dedups on the **normalized** (lowercased) `Tag` but stores the **original-case** token, so an uppercase tag (`"Rust"`) is accepted and persisted un-normalized while its dedup key is `"rust"` — the stored value and the lookup key can diverge. The fix is the same edit as §3.5 gap 5 (return `Vec<Tag>` / push `tag`, not `trimmed`), which is why it's recorded as one item; this entry exists so the correctness angle isn't buried under "type hygiene." Verify whether callers pre-lowercase or the write path re-normalizes before judging severity — if neither, tag lookups can silently miss on mixed-case input.

> **P0b verification (2026-06-13): NOT LIVE — the write path re-normalizes.**
> `tag_post` (`storage/src/{sqlite,postgres}/posts.rs:373`, byte-identical on both backends) re-parses its `tag_display` argument through `Tag::from_str` and stores the **normalized** `tag.as_str()` as the canonical `tags.tag_slug` (the lookup key); the original-case token lands only in the *separate* `post_tags.tag_display` column. Read lookups take an already-normalized `&Tag` (`get_posts_for_tag` et al. bind `tag_slug.as_str()`). So the slug used for both write and lookup is always normalized regardless of what `parse_and_validate_tags` returns — the feared "lookups silently miss on mixed-case input" bug does **not** occur. Neither caller (`web/src/posts/mod.rs:157,327`) pre-lowercases; the safety comes entirely from `tag_post`'s re-parse.
>
> **Consequence for kq8w.20 — the §3.5-gap-5 "fix" is NOT a free correctness win and would *regress* behavior.** `parse_and_validate_tags` is documented (and tested) to preserve the first occurrence's *display casing* in its `Vec<String>` output, which flows to `post_tags.tag_display`. Blindly switching to "return `Vec<Tag>` / push `tag`" (normalized) would discard that display casing. The real cleanup is to dedup/return the typed slug **while still carrying the display token** (e.g. return `Vec<(Tag, String)>` or keep display strings and type only the internal `seen` set). Reframe the bead: this is type hygiene + a stringly-`Err` cleanup (§3.1), *not* a live correctness fix.

---

## 4. Observability

The baseline (ADR-0011) is strong: OTLP export with the W3C TraceContext propagator, a `SlowSpanLayer` that flags operations over `JAUNDER_SLOW_OP_MS`, JSON/pretty switch, `log`→`tracing` bridging, and granular span names (`storage.sqlite.user.create_user.hash_password`). The boundary logger records the server-fn name, masked public error, and full operator message. Gaps:

### 4.1 No metrics, only traces *(medium)*

Everything is spans. Operationally valuable **counters/histograms** are currently only inferable by aggregating traces: failed-login count, upload sizes, quota rejections, backup duration/bytes, session-auth failures. Adding an OTel metrics pipeline (or a Prometheus exporter) for a handful of key signals would make alerting and capacity work far easier than trace aggregation.

### 4.2 Subscriber/exporter init failures are silently swallowed *(low)*

`init_tracing_impl` discards every `try_init()` result with `let _ =`, and OTel exporter-build failure is reported via `eprintln!` (acceptable, since it predates the subscriber) but produces no durable signal. At minimum, capture the `try_init` error into the `eprintln!` path so a misconfigured deployment that double-initializes logging is diagnosable.

### 4.3 Error source chains reach logs but verify no PII *(informational)*

`error_with_sources` walks the full `source()` chain into the operator message — good for debugging. Confirm these chains can't carry user PII (the username *is* recorded as a span field, which is reasonable; passwords are correctly `skip`-ped on the instrument macros). Worth a one-line policy note.

### 4.4 Server-function and feed-worker coverage *(low)*

Instrumentation density is highest in `storage/`. The `web/` server functions rely on the single `server_boundary` log, and the feed worker/HTTP layer are lighter. Consider `#[instrument]` on the server-fn entry points and on `feed::worker` job execution so a failed background job carries a span, not just `tracing::error!(error = %error, ...)`.

### 4.5 Encode backend identity as a field, not a span name *(ties to §1.1)*

Because each storage span name hardcodes `sqlite`/`postgres`, dashboards and the `scripts/analyze-otel-traces` analyzer must special-case both. If §1.1 unifies the backends, attach a `db.system` (or `backend`) **field** on a single span name instead. This is the standard OTel semantic-convention attribute and makes "same operation, either backend" trivially queryable.

### 4.6 Decision-path observability — answering "why did execution reach this line" *(medium — the operator's hardest question)*

The single hardest thing to reconstruct from production telemetry is *why* control flow took the path it did. Two naive approaches both fall short: logging at every branch (verbose to write and read, always-on cost) and dumping all branch-influencing state on every change (machine-reconstructable but noisy). Both emit *lines* — read or replayed sequentially — when the answerable form is a **wide event**: one richly-dimensioned row per unit of work that you *query and slice*, not replay. This is the "canonical log line" / "observability 2.0" model, and in OTel **a span already is that event** — Jaunder just under-fills it.

**Where we are today.** The codebase already encodes branches *structurally* — but in the span **name**: `register` selects `web.auth.register.create_user_open` vs `…create_user_invite`. The path is captured, yet only greppable by name, and the spans carry almost no *determinant fields* (`username`, `user_id`, `create_if_missing` — but not `registration_policy`, `is_operator`, `had_invite`, feed `cache_warmth`). So "of the invite-only registrations, which failed and on what `error.kind`?" is not a query you can run.

Three compounding techniques close this:

1. **Record branch *determinants* as fields, not branches as names.** Capture the value that *decided* the branch plus the outcome label, as span fields. Replace "the span name encodes the branch" with `registration_policy = %policy` on the `register` span — now the field is simultaneously the human explanation *and* a queryable dimension. Mechanically this is the **declare-empty-then-`record`** idiom (`#[instrument(fields(registration_policy = field::Empty, …))]`, then `Span::current().record(...)` once known), which the codebase does not currently use anywhere. Capture *determinants and IDs*, not whole structs (see cost note below). This is the synthesis of the two naive approaches: the "branch + why" of decision-logging, attached to the one structured event of state-dumping.

2. **Add `tracing-error::SpanTrace` — a backtrace made of spans + their fields.** Captured at the moment an error is created, it records the entire stack of active spans with their recorded values: `web.auth.register{registration_policy=invite}` → `…create_user_invite{…}` → `storage.sqlite.user.create_user{username=…}`. That *is* the path-to-the-line with determinants attached — the direct answer to the operator's question. Not currently a dependency (server-only, gate like the rest of the `ssr` observability). It composes with the §3.1 `anyhow` carrier: the internal error gains a `SpanTrace` at creation, and the boundary logs the decision path alongside the cause chain. For this specific pain, the highest-leverage single addition — but its value is proportional to how many determinant fields technique 1 puts on the spans, so the two compound.

3. **Pay for fine detail only on failure (tail decisioning).** The volume objection to branch-level detail is answered by keeping the cheap wide event always, and retaining fine-grained detail *only* for traces that ended badly: either OTel **tail-based sampling** in the collector (a config change, since OTLP is already emitted — keep 100% of errored/slow traces, sample out the rest), or an **in-process dump-on-error** layer that buffers a span's child events and flushes them only when the span closes with an error or exceeds the existing `SlowSpanLayer` threshold (the hook already exists). This is "log every branch, but free unless something breaks."

**Tradeoffs (state them in the durable doc):** high-cardinality fields are what make this answerable but cost backend storage and bump OTel per-span attribute limits — determinants and IDs yes, struct dumps no; the discipline shifts from "log every branch" to "identify each meaningful branch's *determinant*" (a smaller but human judgment — a determinant you forget to record is invisible); and the style biases toward *fewer, wider* spans carrying many fields, with thin spans reserved for things worth timing — a stylistic migration from today's many-thin-phase-spans, not a rewrite.

**Durability & enforcement.** This convention must outlive the analysis. Recommended home: a new ADR extending [ADR-0011](docs/decisions/0011-unified-observability.md) ("decision-path observability"), plus a short imperative subsection in `CONTRIBUTING.md` under *Observability* that future contributors and agents read by default. To make the pattern the path of least resistance rather than a rule to remember, consider macros — sketched in tiers by confidence:

- **Enhance the existing `boundary!` macro (clear win).** It already wraps every server fn, so it is the one place to inject the wide-event discipline invisibly: open a per-server-fn span with a *standard* set of declared-empty determinant fields, and on exit record `outcome`, `error.kind`, and `error.class` (from §3.1) automatically. Centralized, no per-call-site noise.
- **A `record_determinant!(name = value)` helper (mild).** Thin sugar over `Span::current().record(...)` whose value is mostly *intent signalling* and a future lint target ("determinants live on the unit-of-work span") rather than ergonomics.
- **A control-flow-wrapping `decide!`/`branch!` macro (caution).** Tempting — `decide!("registration_policy", policy, { Open => …, InviteOnly => … })` would make recording the determinant unavoidable at the branch point — but wrapping `match`/`if` in a macro obscures control flow and fights readability, the very thing §1 warns against. Recommend *not* building this until 1–2 hand-written examples prove the ergonomics are worth the indirection.

### Observability positives (keep)

OTLP export with W3C TraceContext propagation; the `SlowSpanLayer` (a ready-made hook for technique 3); JSON/pretty switch; granular per-phase spans; and a masked-error logging boundary that already records the server-fn name and operator message.

---

## Suggested priority order

| # | Finding | Area | Impact |
|---|---------|------|--------|
| 1 | §2.1 Auth timing / username enumeration | Security | Medium |
| 2 | §2.2 `hash[2..]` panic in media serve | Security/Robustness | Medium |
| 3 | §2.4 Audit leaky public `WebError` constructors | Security | Medium |
| 4 | §3.1 Error data fidelity & operational context (A/C small; B structural) | Robustness | Medium |
| 5 | §3.5 Adopt validated newtypes (esp. `FeedSurface`) | Robustness | Medium |
| 5a | §3.6 / §3.5-gap-5 `parse_and_validate_tags` → `Vec<Tag>` (fixes tag normalization drift) | Robustness/Correctness | Low effort, latent bug |
| 6 | §1.1 De-duplicate SQLite/Postgres backends | Simplification | High effort, high payoff |
| 7 | §4.1 Add a metrics pipeline | Observability | Medium |
| 8 | §4.6 Decision-path observability (determinant fields + `SpanTrace` + tail sampling) | Observability | Medium |
| 9 | §1.2 Collapse `init_tracing_impl` branches | Simplification | Low effort |
| 10 | §1.5 Consolidate empty-string→`None` normalization (settle trim semantics) | Simplification/Robustness | Low effort |
| 11 | §1.6 Split `web/src/posts/mod.rs` (relocate format prefs; extract listing) | Simplification | Low effort |
| 12 | §1.7 Split `server/src/lib.rs` (backup → `backup.rs`; HTTP obs layer → `observability.rs`) | Simplification | Low effort |
| 13 | §1.8 Finish topic-aggregate config (`FeedsConfig`); grouped getters as default | Simplification | Low effort |
| 14 | §1.9 Dissolve `AppState` omnibus → constructor injection (stops dependency drift) | Simplification/Architecture | Medium |
| 15 | §1.10 Relocate `websub` out of `common` (server-only; removes all `target_arch` cfgs) | Simplification | Low effort |
| 16 | §2.3 / §2.5 Validate & encode media filename | Security | Low |
| 17 | §3.2–3.4, §4.2–4.5 | Robustness/Observability | Low |

Items 1–5 are well-contained, high-value changes and good first PRs — with the caveat that §3.1 splits into mechanical parts (A: widen the stringly variants; C: add `ErrorClass`) and one structural part (B: restructure `InternalError` + boundary), which is itself contained to `web/src/error.rs`. Item 6 is the strategic refactor; doing item 8 and §4.5 alongside it keeps observability coherent after the merge. Note that §3.5's storage-signature cleanup (`&str` → `&Username`/`&Tag`) folds naturally into the §1.1 dedup, and §3.5's `FeedSurface` fix stands alone as the highest-value first step.
