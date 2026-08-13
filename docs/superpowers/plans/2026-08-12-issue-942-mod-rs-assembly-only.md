# Plan — issue #942: `mod.rs` assembles the module surface

**Spec:**
[`docs/superpowers/specs/2026-08-12-issue-942-mod-rs-assembly-only.md`](../specs/2026-08-12-issue-942-mod-rs-assembly-only.md)
**Issue:** [#942](https://github.com/jaunder-org/jaunder/issues/942) **Branch:**
`issue-942-mod-rs-assembly-only` · fork-point tag
`issue-942-mod-rs-assembly-only-base` **For agentic workers:** drive with
**`jaunder-iterate`**; delegate an individual task with **`jaunder-dispatch`**.
Tick checkboxes in real time.

---

## Review header

**Goal.** Empty all 16 in-scope `mod.rs` files of item definitions so each
states only its module surface, without changing a single public path or any
behaviour. Record the rule as an ADR and in `CONTRIBUTING.md`.

**Scope — in:** the 16 files in the spec's Scope table; the three prep items;
the ADR + `CONTRIBUTING.md` + `docs/ARCHITECTURE.md`; filing the deferred issue.
**Scope — out:** `server/tests/storage/mod.rs` (task 1 files it); any new xtask
check; any behaviour change; the `jaunder-review` overlay (lands in
`~/src/agent-configuration`, see the spec's "Related work").

**Tasks.**

1. File the deferred `server/tests/storage/mod.rs` issue.
2. Records: ADR draft finalised, `CONTRIBUTING.md`, ADR projected into
   `ARCHITECTURE.md`.
3. `common/src/feed/` → `config.rs`.
4. `common/src/atompub/` → `ns.rs`, `error.rs`.
5. `common/src/test_support/` → 5 siblings, narrowing the `expect` suppression
   per-file in the same commit.
6. `client/src/perf/` → `names.rs`.
7. `web/src/error/` → `wire.rs`.
8. `web/src/reactive/` → `invalidator.rs`.
9. `server/src/mailer/` → `factory.rs`.
10. `server/src/websub/` → `contract.rs`, `factory.rs`.
11. `server/src/projector/` → `shell.rs`, `document.rs`, `handlers.rs`.
12. `server/src/atompub/` → `router.rs`, `error.rs`, `guards.rs`.
13. `storage/src/postgres/` → `atomic.rs`, `open.rs`.
14. **Prep** — widen sqlite's two `pub(super)` fns to `pub(crate)`.
15. `storage/src/sqlite/` → `atomic.rs`, `open.rs`.
16. `xtask/src/coverage/` → `model.rs`, `run.rs`.
17. `xtask/src/pr/` → `types.rs`, `invocation.rs`, `execute.rs`.
18. `server/tests/projector/` → 5 siblings.
19. `server/tests/helpers/` → `registrar.rs`, `session.rs`, `atompub.rs`,
    `http.rs`, re-pointing the registrar gate's `REGISTRAR` constant in the same
    commit.
20. Fix the 19 `mod.rs` citations in `docs/ARCHITECTURE.md`.
21. Full gate + self-review against the spec's acceptance criteria.

**Key risks / decisions.**

- **Two prep items cannot be separate commits, and are merged.** The registrar
  constant (`server_fn_registrar_check`, hardcoded path) must change in the
  _same_ commit as the helpers move: the `.githooks/pre-commit` hook runs
  `cargo xtask check`, which runs both the registrar check and `host_tests` —
  and xtask's own test suite `expect()`s the registrar file to be readable. A
  standalone constant change therefore cannot be committed at all without
  `SKIP_PRE_COMMIT=1`, and history here stays green commit-by-commit. Likewise
  the `expect`-suppression narrowing is meaningless before its siblings exist.
- **Task 14 must precede task 15.** `pub(super)` at `mod.rs` means "crate root";
  in a sibling it means "this directory". Moving without widening breaks
  `storage/src/db.rs`. This one _is_ a real standalone prep — it compiles and
  gates green on its own.
- **Task 12 hazard:** `server/src/atompub`'s `From<anyhow::Error>` calls
  `crate::media::map_error` — the _server crate's_ `media`, not the sibling
  `atompub::media`. Keep the path crate-absolute.
- **Ordering is simplest-first** so the mechanical pattern is established on
  1-item files before the 42-item one.
- **Every commit gates green.** The pre-commit hook enforces it, so no task may
  plan a deliberately-red intermediate commit.

---

## Global constraints

Apply to every task below.

- **Pure-move discipline.** A move commit's diff shows only: lines leaving
  `mod.rs`, the same lines arriving in a sibling verbatim, that sibling's `use`
  header, and new `mod`/re-export lines in `mod.rs`. Anything else belongs in a
  prep commit first.
- **Paths never change.** Re-export everything that was reachable before.
  `pub(crate)` items re-export as `pub(crate) use`. Explicit lists, not globs —
  a glob only above 25 items, with the count in the commit message.
- **Docs follow their subject.** `///` moves with its item. `//!` stays in
  `mod.rs` unless a paragraph describes only relocated code. Don't invent a
  `//!` where none exists.
- **Gated wiring stays.** Any `#[cfg(target_arch = …)]` on a `mod`/`use` line
  stays in `mod.rs` — `target-arch-placement` permits it nowhere else.
- **No new suppressions.** No `#[allow]`/`#[expect]` beyond task 5's narrowing.
- **Gate before commit.** `devtool run -- cargo xtask check` (the pre-commit
  hook runs the same and fails-and-restages if it reformats). See
  **`jaunder-commit`**. **No `Co-Authored-By` trailer.**
- **Audit.** After each move task, confirm the file has dropped off this list.
  It starts at 17 files / 393 items and must end at exactly 1
  (`server/tests/storage/mod.rs`, deferred). The check, reconstructible from
  this plan alone — do not depend on a script in `/tmp`, which does not survive
  the session:

  ```
  rg -c '^(pub(\([^)]+\))? )?(default )?(async )?(unsafe )?(fn|struct|enum|union|trait|impl|type|const|static|macro_rules!) ' -g 'mod.rs'
  ```

  Add a second pass for inline test modules, which the rule also forbids:

  ```
  rg -c '^#\[cfg\(test\)\]' -g 'mod.rs'
  ```

  A convenience script wrapping both sits at `/tmp/mod-rs-audit.sh` for this
  session; the two commands above are the durable form.

---

## Task 1 — File the deferred issue

**Why first:** separable work, so it can be picked up concurrently rather than
blocked behind this branch (`jaunder-start` step 5).

- [x] Create the issue via **`jaunder-issues`**, milestone "Correctness & data
      integrity", label `ready-for-agent`, body carrying the six constraints
      from the spec's **Deferred** section: ~16 clusters (banners are a start,
      not the answer — tags alone is ~35 tests); ~15 private helpers shared
      across clusters, each needing `pub(super)` + `use super::fixtures::…`; the
      `storage::{…30 items}` preamble partitioned per file or `unused_imports`
      fires; the three ADR-0124 imports per file; no sibling named `storage.rs`
      (`clippy::module_inception`, ADR-0067); ADR-0053 homing — concern-named
      siblings, no `sqlite/`/`postgres/` subdirs unless a test is dialect-only.
- [x] Reference #942 as the parent context.
- **Verify:** issue exists and is visible in the Jaunder Backlog project. →
  **Filed as [#950](https://github.com/jaunder-org/jaunder/issues/950)**, type
  `Task`, milestone "Correctness & data integrity", label `ready-for-agent`.
- **Commit:** none (tracker-only).

## Task 2 — Records

**Files:** `docs/adr/drafts/mod-rs-assembles-module-surface.md` (already
drafted, gitignored) · `CONTRIBUTING.md` · `docs/ARCHITECTURE.md`

- [x] Re-read the ADR draft; confirm line 1 is exactly `# ADR-DRAFT: <Title>`,
      `- Status: proposed`, and that links to sibling ADRs are bare
      (`0070-….md`) per `docs/adr/drafts/README.md`.
- [x] `CONTRIBUTING.md`: add the rule to the repository-layout section — what a
      `mod.rs` may contain, workspace-wide, enforced at review not by a gate,
      citing `docs/adr/drafts/mod-rs-assembles-module-surface.md`.
- [x] `docs/ARCHITECTURE.md`: project the decision per
      **`jaunder-adr-projection`**, citing the draft **by path** (promote
      rewrites it at ship). Update line 916's prose so the rule reads
      workspace-wide rather than web-vertical-only. → projected into
      **§Workspace** (the workspace-wide home, not the web section), with
      descriptive link text rather than an `ADR-DRAFT` label, which `promote`
      would otherwise leave dangling. The per-vertical line (now 931) points at
      §Workspace.
- **Verify:** `devtool run -- cargo xtask check` — `adr-format` and `doc-links`
  green. (`adr-view-parity` only binds numbered ADRs, so it stays green now and
  is satisfied at promote.)
- **Commit:**
  `docs(adr): mod.rs assembles the module surface, nothing more (#942)`

## Task 3 — `common/src/feed/`

**Files:** `common/src/feed/mod.rs` → new `common/src/feed/config.rs`

- [x] Move `pub struct FeedsConfig` (3 pub fields) to `config.rs`.
- [x] Header: `use crate::tagged_url::HubUrl;` plus
      `use crate::feed::{FeedMinDays, FeedMinItems};` — those two arrive via
      `mod.rs`'s `pub use settings::…`, so import them by their re-exported
      path.
- [x] `mod.rs`: add `mod config;` and `pub use config::FeedsConfig;` alongside
      the existing 8 `pub mod` + `pub use` pairs. → used `pub mod config;` to
      match the sibling pairs' existing style; the module was already public in
      effect and `pub mod` keeps the file's shape uniform.
- **Verify:** `devtool run -- cargo nextest run -p common` — PASS.
- **Commit:** `refactor(common): move FeedsConfig out of feed/mod.rs (#942)`

## Task 4 — `common/src/atompub/`

**Files:** `common/src/atompub/mod.rs` → new `ns.rs`, `error.rs`

- [x] `ns.rs`: the three `pub const` (`ATOM_NS`, `APP_NS`, `J_NS`) with their
      docs.
- [x] `error.rs`: `pub struct AtomPubError`, its `#[derive(Debug, Error)]` /
      `#[error(...)]` attributes, the inherent
      `impl AtomPubError { pub fn new }`, the `#[cfg(test)] mod tests` (1 test),
      and `use thiserror::Error;`. → **one deviation from verbatim**: the doc
      comment links `[`AtomError`]`, which resolved in `mod.rs` because the
      `atom_syndication` re-export is there. In `error.rs` it does not, and
      `doc-links` is a gate, so the doc gained an explicit
      `[`AtomError`]: crate::atompub::AtomError` target.
- [x] `mod.rs`: keep the `//!` doc, `mod xml;`, the four `pub mod` + `pub use`
      pairs, and the `pub use atom_syndication::{…}` block. Add
      `mod ns; pub use ns::{APP_NS, ATOM_NS, J_NS};` and
      `mod error; pub use error::AtomPubError;`.
- [x] Check `xml.rs`/`entry.rs` for `use super::{ATOM_NS, …}` — the re-export
      keeps those resolving; confirm rather than assume. → confirmed by a green
      gate; no consumer needed editing.
- **Verify:** `devtool run -- cargo nextest run -p common` — PASS.
- **Commit:**
  `refactor(common): move atompub constants and error out of mod.rs (#942)`

## Task 5 — `common/src/test_support/`

**Files:** `common/src/test_support/mod.rs` → new `identity.rs`, `content.rs`,
`media.rs`, `urls_time.rs`, `numbers.rs`

- [x] Partition the ~40 `parse_*` fns and `MEDIA_TEST_SHA256` (**38** in
      fact): - `identity.rs` — username, display*name, bio, email, password,
      session_label, token, token_hash, smtp*\* - `content.rs` — post_title,
      post_body, post_summary, slug, tag, tag_label, site_title, rendered\_\* -
      `media.rs` — content_hash, filename, content_type, max_file_size,
      user_quota, byte_size, `MEDIA_TEST_SHA256` - `urls_time.rs` — parse_url,
      parse_root_relative_url, etag, utc_instant, permalink_date - `numbers.rs`
      — page_size, page_offset, row_limit, retention_count, invite_ttl_hours,
      destination_path, feed_min\_\*
- [x] Partition the long `use crate::…` preamble per file; do not copy it
      wholesale (`unused_imports` is denied).
- [x] **Narrow the suppression in this same commit.** Delete the subtree-wide
      inner `#![expect(clippy::expect_used)]` from `mod.rs:9`, and add a
      per-file inner `#![expect(clippy::expect_used)]` to each new sibling that
      calls `expect()` — and **only** those. `mod.rs` retains 39 `expect(` calls
      today, so all of them leave with the code and `mod.rs` needs no attribute.

  > **Documented carve-out to the pure-move rule.** These attribute lines are
  > the one exception to Global Constraint 1 in this branch: a per-file
  > suppression cannot be written before the per-file split exists, so it cannot
  > be prep. The commit message must say so. Everything else in this commit is a
  > verbatim move.

- [x] `mod.rs` keeps its `//!` doc and `mod env; pub use env::{Env, with_env};`,
      and gains the 5 `mod` + re-export pairs. Explicit lists; if any one
      sibling exceeds 25 exports, a glob is permitted and the commit message
      states the count. → no sibling reached 25, so all five lists are explicit
      and no glob was needed.
- [x] Confirm `storage::test_support`'s re-export of `MEDIA_TEST_SHA256` still
      resolves.
- **Verify:** `devtool run -- cargo nextest run -p common` and
  `devtool run -- cargo nextest run -p storage` — PASS.
- **Commit:**
  `refactor(common): split test_support parsers out of mod.rs (#942)`

## Task 6 — `client/src/perf/`

**Files:** `client/src/perf/mod.rs` → new `names.rs` ·
`xtask/src/steps/xlang_literal_check.rs`

- [x] `names.rs`: the 5 `pub const` mark names, plus the
      `#[cfg(test)] mod tests`. The tests call `mark`, which is _not_ defined
      here — the moved test module needs `use crate::perf::mark;`. → confirmed
      by the compiler: `use super::*` does not carry `mark`, because the
      re-export is target-gated. The explicit import is the only edit.
- [x] `mod.rs` keeps the `//!` doc **and both `#[cfg(target_arch = "wasm32")]` /
      `#[cfg(not(…))]` `mod`+`pub use` pairs** — `target-arch-placement` permits
      that gate only on a `mod`/`use` in a `mod.rs`. Add
      `mod names; pub use names::{…};`. The `//!` doc's intra-doc links to
      `MARK_PREFIX` and `mark` still resolve through the re-exports.
- [x] `MARK_PREFIX`'s doc mentions the `xlang-literal` gate; confirm
      `xtask/src/steps/xlang_literal_check.rs` matches on the literal, not a
      file path. → **the plan's assumption was wrong.** The gate's `PAIRS` table
      pins the site by **path**, and one of its own tests asserts that path in
      the failure text. Both are re-pointed at `names.rs` in this same commit —
      a third folded prep, for the reason task 19 records: a lone constant
      change names a file that does not exist yet, and a lone move leaves the
      gate pointed at a file that no longer holds the literal, so neither half
      can be a green commit on its own.
- **Verify:** `devtool run -- cargo xtask check` — `target-arch-placement` and
  `xlang-literal` green.
- **Commit:** `refactor(client): move perf mark names out of mod.rs (#942)`

## Task 7 — `web/src/error/`

**Files:** `web/src/error/mod.rs` → new `wire.rs`

- [x] `wire.rs`: `pub type WebResult<T>`, `pub enum WebError` with its derives
      and `#[serde(rename_all = "snake_case")]`, the inherent `impl` (5
      constructors), `impl FromServerFnError for WebError`, and the
      `#[cfg(test)] mod tests`.
- [x] The `FromServerFnError` impl calls `server::emit_arg_decode_failure`
      behind a statement-level `#[cfg(feature = "server")]`, and `mod server;`
      is private — so `wire.rs` must be a sibling **inside** `web/src/error/`
      and reach it as `super::server::emit_arg_decode_failure`.
- [x] `mod.rs` keeps `#[cfg(feature = "server")] mod server;`, the
      `#[cfg(all(test, feature = "server"))] pub(crate) use server::project;`,
      the `#[cfg(feature = "server")] pub use server::{…}` list, and the
      explanatory comment block. Add
      `mod wire; pub use wire::{WebError, WebResult};`. → the comment block
      stays adjacent to the `server` re-exports it describes, so the two `mod`
      declarations now sit together above it.
- **Verify:** → `cargo xtask check` green;
  `cargo nextest run -p web --features server` — **265 passed**.
  `devtool run -- cargo nextest run -p web --features server` — PASS. The plain
  `-p web` run is **not sufficient**: `web`'s `default = []`, so the
  `#[cfg(feature = "server")]` call to `emit_arg_decode_failure` never compiles
  without the feature, and the sibling-placement hazard above would go untested.
  Follow with `devtool run -- cargo xtask check`.
- **Commit:** `refactor(web): move WebError out of error/mod.rs (#942)`

## Task 8 — `web/src/reactive/`

**Files:** `web/src/reactive/mod.rs` → new `invalidator.rs`

- [x] `invalidator.rs`: `pub struct Invalidator`, its inherent `impl`
      (`new`/`notify`/`track`), `impl Default`, and the
      `#[cfg(test)] mod tests`.
- [x] The tests use the `invalidator_scope!` macro, reached through `mod.rs`'s
      gated `pub(crate) use`. In the moved file import it as
      `use crate::reactive::invalidator_scope;` — the comment at `mod.rs:18-21`
      says this consumption chain is what keeps the host build free of a denied
      `unused_imports`, so verify that comment still describes reality and
      update it if the chain now runs through `invalidator.rs`. → the crate path
      is **required**, not merely preferred: inside `invalidator.rs`, `super` is
      the `invalidator` module, so the old `use super::{…}` would no longer
      reach the macro. Both the `mod.rs` comment and the in-test comment that
      names the chain are re-pointed at `invalidator.rs`; the chain itself is
      unbroken.
- [x] `mod.rs` keeps its `//!` doc, **both**
      `#[cfg(any(target_arch = "wasm32", test))]` lines (`mod scope;` and
      `pub(crate) use scope::invalidator_scope;`) and their explanatory
      comments. Add `mod invalidator; pub use invalidator::Invalidator;`.
- **Verify:** `devtool run -- cargo xtask check` — `target-arch-placement`
  green; `devtool run -- cargo nextest run -p web --features server` — PASS
  (same default-features caveat as task 7).
- **Commit:** `refactor(web): move Invalidator out of reactive/mod.rs (#942)`

## Task 9 — `server/src/mailer/`

**Files:** `server/src/mailer/mod.rs` → new `factory.rs`

> Named `factory.rs`, not `build.rs`: the latter reads as a Cargo build script
> even though it is harmless in a subdirectory.

- [x] `factory.rs`: `pub async fn build_mailer` with its
      `#[tracing::instrument]`, and the `#[cfg(test)] mod tests` (3 tests, one
      `#[apply(backends)]` — carry its `// guard:no-backend` markers verbatim).
- [x] Header: `use super::{FileMailSender, LettreMailSender};` — both arrive via
      `mod.rs`'s re-exports. The rest of the old `mod.rs` preamble (`Arc`,
      `MailSender`/`NoopMailSender`, `SiteConfigStorage`/`load_smtp_config`)
      travels with `build_mailer`, and the tests' `use super::*` still reaches
      all of it — a child module sees its parent's private imports.
- [x] `mod.rs` keeps its `//!` doc, `mod file; mod smtp;` and the two `pub use`
      lines; add `mod factory; pub use factory::build_mailer;`.
- **Verify:** `devtool run -- cargo nextest run -p jaunder mailer` — PASS.
- **Commit:** `refactor(server): move build_mailer out of mailer/mod.rs (#942)`

## Task 10 — `server/src/websub/`

**Files:** `server/src/websub/mod.rs` → new `contract.rs`, `factory.rs`

- [ ] `contract.rs`: `pub enum WebSubError` and
      `#[async_trait] pub trait WebSubClient` **with its doc comment intact** —
      it carries a `compile_fail` doctest and a positive one naming
      `jaunder::websub::{NoopWebSubClient, WebSubClient}`, and `doctest-fences`
      reconciles the fence population, so a dropped or unreachable fence is a
      gate failure.
- [ ] `factory.rs`: `pub fn default_client` plus the `#[cfg(test)] mod tests`;
      header `use super::{FileCapturingWebSubClient, HttpWebSubClient};`.
- [ ] `mod.rs`: this file has no `//!` doc today — **do not add one** (Global
      Constraints: docs follow their subject; inventing module docs is separate
      work). Keep the three `pub mod` + `pub use` lines **exactly where they
      are** — the existing wiring currently sits below the trait, and relocating
      those lines would be a reorder, which the spec's Method assigns to a prep
      commit. Append
      `mod contract; pub use contract::{WebSubClient, WebSubError};` and
      `mod factory; pub use factory::default_client;`.
- [ ] Confirm `file_capture.rs`/`http.rs`/`noop.rs` still resolve
      `use super::{WebSubClient, WebSubError}` through the re-export.
- **Verify:** `devtool run -- cargo xtask check` — `doctest-fences` green;
  `devtool run -- cargo nextest run -p jaunder websub` — PASS.
- **Commit:**
  `refactor(server): split websub contract and factory out of mod.rs (#942)`

## Task 11 — `server/src/projector/`

**Files:** `server/src/projector/mod.rs` → new `shell.rs`, `document.rs`,
`handlers.rs`

> This `mod.rs` has **no** `mod` declarations today — it is 100% items. After
> this task it is pure assembly, which is the intended outcome.

- [ ] `shell.rs`: `pub struct Shell(pub Arc<str>)` with `#[derive(Clone)]`.
      `Shell` is named externally as `jaunder::projector::Shell`
      (`server/tests/projector/mod.rs:30`), so it must stay re-exported.
- [ ] `document.rs`: `pub fn document`, `fn cacheable`, `fn shell_response`,
      `fn permalink_response`, `fn timeline_response`, `fn tag_response`, and
      the `#[cfg(test)] mod tests` (6 tests) — the tests reach these via
      `super::`, so they travel together and the private fns need no widening.
      The tests use `include_str!("../../../csr/index.html")`, resolved relative
      to the containing file; a same-directory sibling keeps it valid — **do
      not** nest deeper.
- [ ] `handlers.rs`: `pub fn register<S>`, `type PermalinkPath`, and the five
      private `async fn` handlers. `register` is their only consumer, so keeping
      them together avoids any `pub(super)` widening (cohesion rule 2).
- [ ] `mod.rs`: keep the `//!` doc; add the three `mod` lines and
      `pub use shell::Shell; pub use document::document; pub use handlers::register;`.
- **Verify:** `devtool run -- cargo nextest run -p jaunder projector` — PASS.
- **Commit:**
  `refactor(server): split projector/mod.rs into shell, document, handlers (#942)`

## Task 12 — `server/src/atompub/`

**Files:** `server/src/atompub/mod.rs` → new `router.rs`, `error.rs`,
`guards.rs`

- [ ] `error.rs`: `pub enum HandlerError`, `impl IntoResponse for HandlerError`,
      the **11** `From<…> for HandlerError` impls (`sqlx`, `StatusCode`,
      `AtomError`, `AtomPubError`, `TaggingError`, `TagValidationError`,
      `InvalidPostBody`, `PerformCreationError`, `PerformUpdateError`,
      `DeleteMediaError`, `anyhow`), and `fn log_internal`. **Hazard:** the
      `From<anyhow::Error>` impl reaches `crate::media::map_error` — the _server
      crate's_ `media`, not the sibling `atompub::media`. Keep it written
      `crate::media::map_error` so it cannot re-resolve.
- [ ] `guards.rs`: `pub(crate) fn require_user_match`,
      `pub(crate) async fn base_url`, `pub(crate) async fn required_base_url`.
      These stay `pub(crate)` and `mod.rs` re-exports them
      **`pub(crate) use guards::{…};`** — a bare `pub use` of a `pub(crate)`
      item does not compile. `posts.rs`, `media.rs`, `service.rs` and `rsd.rs`
      call them as `super::require_user_match` etc. (6 call sites), which the
      re-export preserves.
- [ ] `router.rs`: `pub fn router<S>`, `async fn record_atompub_request`,
      `fn atompub_op`, `fn atompub_result`. Header needs
      `use super::{media, posts, rsd, service};`.
- [ ] **Split the existing 10-test `mod tests` between the two files** rather
      than sending it whole to `router.rs`: 8 of the tests exercise
      `HandlerError` and the `From` impls and belong in `error.rs`; the
      remainder, which cover `atompub_op`/`atompub_result`, belong in
      `router.rs`. Sending them all to `router.rs` would leave `error.rs`
      testless and force `use super::{HandlerError, atompub_op, atompub_result}`
      across the seam — the exact separation partition rule 2 exists to prevent.
      Read each test and home it with its subject.
- [ ] `mod.rs`: keep the `//!` doc and the five `pub mod` lines; add the three
      new `mod` lines, `pub use error::HandlerError;`,
      `pub use router::router;`, and the `pub(crate) use guards::{…};` line.
- [ ] `mapping.rs` and `posts.rs` have `use super::*;` inside their own test
      modules — confirm they still see what they need.
- **Verify:** `devtool run -- cargo nextest run -p jaunder atompub` — PASS.
- **Commit:**
  `refactor(server): split atompub/mod.rs into router, error, guards (#942)`

## Task 13 — `storage/src/postgres/`

**Files:** `storage/src/postgres/mod.rs` → new `atomic.rs`, `open.rs`

> Split along the file's existing `// ---` banners (partition rule 1). Sibling
> names carry no dialect prefix — the directory does, matching `users.rs`,
> `posts.rs`, `pool.rs` (spec, Scope).

- [ ] `atomic.rs`: `pub struct PostgresAtomicOps`, its inherent `impl` (`new`),
      and `#[async_trait] impl AtomicOps for PostgresAtomicOps`.
- [ ] `open.rs`: `fn make_postgres_app_state`, `fn postgres_password_from_env`,
      `pub fn resolved_postgres_options`,
      `pub(crate) async fn open_postgres_database_with_pool` (with its
      `#[tracing::instrument]`), `open_postgres_database`, `database_is_empty`,
      and the `#[cfg(test)] mod tests` — the tests call the private
      `postgres_password_from_env` via `super::*`, so they must travel here. One
      is `#[apply(postgres_only)]`; it stays under `postgres/`, so ADR-0053
      homing holds.
- [ ] `make_postgres_app_state` names all 14 `Postgres*Storage` types, which
      arrive via `mod.rs`'s re-exports — `use super::{…}` them.
- [ ] `mod.rs`: keep the 14 `mod`+`pub use` pairs, `pub(crate) mod backup;`, and
      the three `#[cfg(test)] mod` decls. Add
      `mod atomic; pub use atomic::PostgresAtomicOps;` and `mod open;` with
      `pub use open::resolved_postgres_options;` plus
      **`pub(crate) use open::{database_is_empty, open_postgres_database, open_postgres_database_with_pool};`**.
- [ ] Confirm `storage/src/lib.rs:63`'s `pub use postgres::{…}` list still
      resolves.
- **Verify:** `devtool run -- cargo nextest run -p storage` — PASS.
- **Commit:**
  `refactor(storage): split postgres/mod.rs into atomic and open (#942)`

## Task 14 — **Prep**: widen sqlite's `pub(super)` functions

**Files:** `storage/src/sqlite/mod.rs`

**Why prep:** at `mod.rs`, `pub(super)` means "visible at the crate root", which
is why `storage/src/db.rs:15,289` can call these. In `sqlite/open.rs` the same
keyword would mean "visible in `crate::sqlite`" and those callers break.
Widening first keeps task 15 a pure move.

- [ ] Change `pub(super) async fn open_sqlite_database` and
      `pub(super) async fn database_is_empty` to `pub(crate)`.
- [ ] Leave every call site alone — `pub(crate)` is strictly wider, so
      `crate::sqlite::open_sqlite_database` still resolves.
- **Verify:** `devtool run -- cargo xtask check --no-test` — green.
- **Commit:** `refactor(storage): widen sqlite open fns to pub(crate) (#942)`

## Task 15 — `storage/src/sqlite/`

**Files:** `storage/src/sqlite/mod.rs` → new `atomic.rs`, `open.rs`

> Mirror task 13's split exactly — the symmetry is free and worth keeping.

- [ ] `atomic.rs`: `pub struct SqliteAtomicOps`, inherent `impl`, and
      `#[async_trait] impl AtomicOps`.
- [ ] `open.rs`: `fn make_sqlite_app_state`,
      `pub(crate) async fn open_sqlite_database_with_pool` (with
      `#[tracing::instrument]`), and the two functions widened in task 14. This
      file has no `#[cfg(test)] mod tests` to carry.
- [ ] `mod.rs`: keep the **13** `mod`+`pub use` pairs (sqlite has 13, not
      postgres's 14 — there is no `bootstrap`), `pub(crate) mod backup;`, and
      `#[cfg(test)] mod pool;`. Add
      `mod atomic; pub use atomic::SqliteAtomicOps;` and
      `mod open; pub(crate) use open::{database_is_empty, open_sqlite_database, open_sqlite_database_with_pool};`.
- [ ] Confirm `storage/src/lib.rs:75`'s `pub use sqlite::{…}` still resolves.
- **Verify:** `devtool run -- cargo nextest run -p storage` — PASS.
- **Commit:**
  `refactor(storage): split sqlite/mod.rs into atomic and open (#942)`

## Task 16 — `xtask/src/coverage/`

**Files:** `xtask/src/coverage/mod.rs` → new `model.rs`, `run.rs`

- [ ] `model.rs`: `pub struct LineCov`, `FileCoverage`, `CoverageReport` with
      their `#[derive(Serialize)]`.
- [ ] `run.rs`: `pub fn run`, `fn run_inner`, `fn write_failures_dump`,
      `fn failure_report`, and the `#[cfg(test)] mod tests`. Header needs
      `use super::{crap, gate, report};`.
- [ ] `mod.rs` keeps its `//!` doc — **including the intra-doc links**
      `[`exempt`]`, `[`report`]`, `[`crap`]`, which stay valid — and the five
      `pub mod` lines. Add the two `mod` lines and explicit re-exports.
- [ ] `target_arch_placement_check.rs` and `thin_components.rs` cite
      `[`crate::coverage::exempt`]` in their own docs; that path is unchanged.
- **Verify:** `devtool run -- cargo xtask check` — `doc-links` and
  `doctest-fences` green.
- **Commit:** `refactor(xtask): split coverage/mod.rs into model and run (#942)`

## Task 17 — `xtask/src/pr/`

**Files:** `xtask/src/pr/mod.rs` → new `types.rs`, `invocation.rs`, `execute.rs`

- [ ] `types.rs`: `PrNumber` + `impl Display`, `Subject`, `Outcome` + inherent
      `impl` + **the hand-written `impl Serialize`** (its delegation to `as_str`
      is load-bearing against drift — keep them adjacent) + `impl Display`,
      `EventKind` + `as_str`, `Event`, `PrReport`.
      `crate::result::CommandResult.pr` embeds `crate::pr::PrReport`, so that
      path must survive.
- [ ] `invocation.rs`: `pub struct GitFacts` + its private `impl … read`,
      `pub struct Invocation<'a>`.
- [ ] `execute.rs`: `pub fn execute`, `pub fn execute_with<S,A,C>`,
      `pub fn into_result`, and the `#[cfg(test)] mod tests` (11 tests + the
      `SpyArmer` struct). The tests use `use crate::pr::test_support::*;` — that
      path is unchanged because `#[cfg(test)] pub(crate) mod test_support;` is
      assembly and stays in `mod.rs`. Header needs
      `use super::{gh, land, snapshot, watch};`.
- [ ] `mod.rs`: keep the `//!` doc, the five `pub mod` lines and the
      `#[cfg(test)] pub(crate) mod test_support;`. Add the three `mod` lines and
      explicit re-exports.
- [ ] Confirm `decide.rs`/`land.rs`/`snapshot.rs`/`watch.rs` still resolve their
      `use super::{…}` imports.
- **Verify:** `devtool run -- cargo xtask check` — green; xtask's own tests
  PASS.
- **Commit:**
  `refactor(xtask): split pr/mod.rs into types, invocation, execute (#942)`

## Task 18 — `server/tests/projector/`

**Files:** `server/tests/projector/mod.rs` → new `fixtures.rs`, `permalink.rs`,
`listing.rs`, `tags.rs`, `caching.rs`

> **No sibling named `projector.rs`** — ADR-0067 promoted that file _to_
> `mod.rs` to avoid `clippy::module_inception`.

- [ ] `fixtures.rs`: `const TEST_SHELL`, `fn projector_app`,
      `async fn seed_tagged_post`, `async fn seed_published_post`, `fn get` —
      all currently private, so widen to `pub(super)` (they now have consumers
      in four siblings, which is exactly the case the spec permits).
- [ ] Partition the 17 tests by route: `permalink.rs`, `listing.rs` (profile +
      site_timeline), `tags.rs`, `caching.rs`.
- [ ] **Every** new file repeats the three ADR-0124 imports: `use rstest::*;`,
      `use rstest_reuse::*;`, and the template by bare name
      (`use storage::test_support::backends;`). Omit the third and
      `#[apply(backends)]` is unresolved.
- [ ] `mod.rs` becomes pure assembly: five `mod` lines. It has no `//!` doc and
      no re-exports today and needs none — nothing outside the directory names
      these tests.
- **Verify:** `devtool run -- cargo xtask check` — `test-backend-pattern` green;
  `devtool run -- cargo nextest run -p jaunder projector` — all 17 tests PASS.
- **Commit:**
  `test(server): split projector integration tests out of mod.rs (#942)`

## Task 19 — `server/tests/helpers/` and the registrar gate

**Files:** `server/tests/helpers/mod.rs` → new `registrar.rs`, `session.rs`,
`atompub.rs`, `http.rs` · `xtask/src/steps/server_fn_registrar_check.rs`

> **One commit, deliberately.** `server_fn_registrar_check` parses
> `server/tests/helpers/mod.rs` by a hardcoded constant, so the constant and the
> move must change together. Splitting them either way produces a commit that
> cannot be made: `.githooks/pre-commit` runs `cargo xtask check`, which runs
> both the registrar check **and** `host_tests` — and xtask's own
> `enumeration_of_web_src_matches_the_registrar` test does
> `read_to_string(REGISTRAR).expect("registrar reads")`. A lone constant change
> fails the hook twice over. The gate is already fail-loud; do **not** add
> hardening the spec does not ask for.

- [ ] Change `const REGISTRAR: &str = "server/tests/helpers/mod.rs";`
      (`server_fn_registrar_check.rs:62`) to
      `"server/tests/helpers/registrar.rs"`.

- [ ] `registrar.rs`: `pub fn ensure_server_fns_registered` (the 55
      `register_explicit::<…>()` calls) and
      `pub const REGISTERED_SERVER_FN_COUNT`, with the doc block explaining the
      count.
- [ ] `session.rs`: `pub struct SeededSession` + its inherent `impl` (`cookie`,
      `seed_post`), `async fn issue_session`, `seed_and_session`,
      `create_session_for`, `create_user_and_session`,
      `create_operator_and_session`, `fn tmp_storage_path`, `session_cookie`,
      `token_from_set_cookie`, `basic_header`, `seed_base_url`,
      `setup_with_base_url`, `assert_one_absolute_link_email`,
      `assert_no_email`. The private `issue_session`/`seed_and_session` stay
      with their only consumers (cohesion rule 2), so no widening.
- [ ] `atompub.rs`: the whole builder family — `atompub_authed`, `atompub_xml`,
      `atompub_uri`, `atompub`, `atompub_at`, `atompub_get`, `atompub_send_xml`,
      `atompub_post_xml`, `atompub_put_xml`, `atompub_upload`. Takes
      `&SeededSession`, so `use super::session::SeededSession;` — a one-way
      dependency on `session.rs`.
- [ ] `http.rs`: `enum Auth<'a>`, `enum PostBody` + inherent `impl`,
      `async fn post_inner`, `post_form`, `post_form_with_mailer`,
      `post_form_with_secure_flag`, `post_form_with_ua`,
      `post_form_with_bearer`, `post_json`, `pub struct MultipartFile<'a>`,
      `post_multipart`, `get_asset`, `body_string`, `make_app`. The private
      `Auth`/`PostBody`/`post_inner` stay with their wrappers — no widening.
- [ ] `mod.rs`: keep the header comment and
      `mod websub_capturing; pub use websub_capturing::CapturingWebSubClient;`.
      Add the four `mod` lines and explicit `pub use` lists. ADR-0067 makes
      `crate::helpers::…` the documented import path for every subsystem, so the
      re-exports must cover everything currently reachable — check each
      subsystem still compiles rather than eyeballing the list.
- [ ] No `#[cfg(test)]` anywhere here (the whole tree is a test target), and
      `server/tests/main.rs`'s crate-level
      `#![expect(clippy::unwrap_used, clippy::expect_used)]` covers new siblings
      automatically — add no per-file suppression.
- **Verify:** `devtool run -- cargo xtask check` — `server-fn-registrar` green
  and `host_tests` green; `devtool run -- cargo nextest run -p jaunder` — the
  whole integration binary PASSES.
- **Commit:** `test(server): split test helpers out of mod.rs (#942)`

## Task 20 — Fix `docs/ARCHITECTURE.md` citations

**Files:** `docs/ARCHITECTURE.md`

> `rg -c 'mod\.rs' docs/ARCHITECTURE.md` reports **22 occurrences**, but 3 of
> those (lines 916, 926, 935) are prose about the layout rule, not citations.
> The real citation count is **19**.

- [ ] Re-point the **19** citations: 17 carry a line number and need file
      **and** line corrected; 2 name a file only (lines 834
      `server/src/projector/mod.rs`, 2083 `server/tests/helpers/mod.rs`) and
      need the path re-pointed only if the cited code moved.
- [ ] **Line 705 is the awkward one** — it cites `mod.rs:28` with no path at
      all, relying on the surrounding prose for context. Resolve which file it
      means before correcting it, and give it an explicit path while you are
      there.
- [ ] Confirm line 916's prose (updated in task 2) still reads correctly against
      the finished tree, and that the counts at 926 remain true.
- **Verify:** `devtool run -- cargo xtask check` — `doc-links` and
  `adr-view-parity` green. Spot-check each citation resolves to the named item.
- **Commit:**
  `docs(architecture): re-point mod.rs citations after the split (#942)`

## Task 21 — Full gate and self-review

- [ ] Run both audit commands from Global Constraints — exactly **1** offending
      file remains (`server/tests/storage/mod.rs`, deferred).
- [ ] `git diff issue-942-mod-rs-assembly-only-base..HEAD --stat` — confirm no
      file outside the 16 module directories, the four docs files, and the
      registrar check is touched (acceptance criterion 2).
- [ ] `rg -n 'use .*::\*;' <the 16 dirs>` — every glob re-export is justified by
      a >25-item export list, with the count in its commit message (criterion
      3).
- [ ] `git log --oneline` — every prep commit precedes its move commit
      (criterion 4).
- [ ] `devtool run -- cargo xtask validate` — green (criterion 10).
- [ ] Walk the spec's 16 acceptance criteria and confirm each.
- **Commit:** none — this is verification. Then hand to **`jaunder-ship`**.

---

## Self-review

- **Every acceptance criterion is covered.** 1 → tasks 3–19; 2 → task 21; 3 →
  tasks 5, 21; 4 → task 14 plus the folded preps in tasks 5 and 19, checked at
  21; 5 → the pure-move constraint on every move task; 6, 7 → task 19; 8 → task
  5; 9 → task 18 (the only task naming a directory where `module_inception`
  binds — the `storage.rs` half is unreachable here because that file is
  deferred to #950); 10 → task 21; 11 → task 2; 12 → task 2; 13 → task 20; 14 →
  task 2; 15 → the plan adds no xtask check; 16 → task 1.
- **Ordering constraints are explicit** where they bind: 14→15 is the only
  remaining inter-task dependency. Everything else is independent and could be
  reordered or parallelised.
- **Two prep items are folded into their move commits** because neither can
  stand as a green commit alone — the reasoning is recorded at the merged tasks
  and in the spec's Method section. Task 14 is the only genuine standalone prep.
- **No placeholders.** Every task names real files, real items, and a real
  verify command.
