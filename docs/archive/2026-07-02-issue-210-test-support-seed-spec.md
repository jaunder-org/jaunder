# Issue #210 — batch-seed heavy timeline e2e tests via a `test-support` binary

- **Issue:** [#210](https://github.com/jaunder-org/jaunder/issues/210) —
  _perf(e2e): batch-seed heavy timeline tests instead of sequential
  `api.create_post` loops_
- **Milestone:** E2E test suite. Follow-on to #155.
- **Date:** 2026-07-02
- **Status:** proposed (this is a record of the resolved design, not a proposal
  to debate)

## Problem

Three e2e timeline tests are slow because their **fixture setup** is slow, not
the behavior under test:

- `end2end/tests/posts.spec.ts:305` — per-user timeline pagination (seeds 51
  posts: `TIMELINE_PAGE_SIZE(50) + TIMELINE_OVERFLOW_COUNT(1)`).
- `end2end/tests/posts.spec.ts:349` — unauthenticated local timeline (seeds
  `2 × LOCAL_TIMELINE_AUTHOR_COUNT(26)` = 52 posts across two users; the known
  heavy flake).
- `end2end/tests/posts.spec.ts:410` — authenticated home feed pagination.

Each populates its timeline with a **sequential loop of `POST /api/create_post`
round-trips** (`createPublishedPostViaApi`, posts.spec.ts:18–33). Per #155's
trace analysis, `create_post` was 112 calls / ~17s across the suite — pure setup
cost, unrelated to the assertions, and it balloons under CPU contention. #155
band-aided it with a worker-contention timeout scale; #210 removes the cost so
the headroom isn't needed.

## Decision

Introduce a **new workspace binary crate `test-support`** that links `storage`
directly and seeds posts through the real `create_rendered_post` path, then have
the three tests shell out to it **mid-test** (after their runtime registration)
instead of looping HTTP calls.

### Why a separate test binary (not the alternatives)

The design interview weighed and rejected three alternatives; recording them so
the choice isn't reverse-engineered (see ADR `test-support-seed-binary`):

- **A prod CLI subcommand `jaunder seed-posts`** — ships an absurd command in
  the production binary.
- **A prod HTTP endpoint `POST /api/seed_posts`** — even compile-time-gated, it
  means a seed-arbitrary-posts surface living in / near the release artifact,
  and true compile-time gating forces a second build of the _whole server_.
  Undesirable.
- **Raw per-backend SQL (`sqlite3`/`psql`)** — feasible but re-implements post
  creation as two hand-maintained SQL dialects that silently rot against schema
  migrations. A timeline-visible post is **not one row**: it needs a `posts` row
  _and_ a `post_audiences` row (`target_kind_id = 1` = public) or it is private
  and invisible to the timeline (`resolution_where`,
  storage/src/posts.rs:1460–1499); plus a NOT-NULL `rendered_html`. This is
  exactly the backend-parity divergence the storage layer + ADR-0019 dialect
  files + the `test-backend-pattern` guard exist to prevent.

A separate binary that calls the one real storage code path gives us: **zero
production exposure by construction** (no gate needed — nothing in the release
artifact references it), **backend-agnostic + schema-safe** seeding (audiences,
rendered HTML, both dialects for free), and the **same or less** mid-test
plumbing as any alternative.

## Design

### The `test-support` crate

- New workspace **binary** crate at `test-support/` (`src/main.rs`), depending
  on `storage`, `common`, `clap`, `tokio`. **Not** part of the `jaunder` server
  binary's dependency graph — the `jaunder` prod binary and the NixOS module
  never reference it.
- **Not `xtask`.** xtask is host-only and must never run inside a Nix derivation
  (CLAUDE.md invariant); this tool must run _inside_ the e2e VM, so it is a
  normal crane-built workspace binary.
- Naming note: `storage` already has an in-process `storage::test_support`
  **module**. The new **binary crate** `test-support` is the cross-process
  surface for that capability (a live-server e2e drives it over a process
  boundary). It **reuses** `storage::test_support::seed_posts` rather than
  duplicating it. The relationship is made explicit in-code (doc comment on the
  crate).

### The `seed-posts` subcommand (minimal scope for #210)

```
test-support seed-posts --db <STORAGE_ARGS> --username <NAME> --count <N> [--published]
```

- Reuses the server's `StorageArgs` (`--db`, same env/flags `jaunder serve`
  reads) so backend selection (SQLite vs Postgres) is identical to the running
  server — one code path, no per-backend branching in the caller.
- Builds the concrete `PostStorage` + `UserStorage` from the DB arg the same way
  `cmd_serve` constructs `AppState` (reusing the server's `StorageArgs` → pool
  construction; the tool must not fork its own connection logic), resolves
  `--username` → `user_id` via `UserStorage`, then seeds `--count` posts via the
  existing `storage::test_support::seed_posts` (or `create_rendered_post`
  directly).
- `--published` sets `published_at = now` and Public audience (as `seed_posts`
  already does), so seeded posts are visible in local/per-user timelines.
  Absent, posts are drafts.
- Exit non-zero with a clear message on unknown user / persistence failure.
- **Slug uniqueness (load-bearing).** `storage::test_support::seed_posts` uses a
  deterministic `format!("seed-{i}")` slug — safe for the Rust unit tests (fresh
  pool per test) but **not** for the e2e suite, which shares one DB across all
  tests. `:305`, `:349` (two users) and `:410` all seeding `seed-0…N` would
  collide if the slug uniqueness constraint is global (the current HTTP helper
  avoids this because the server auto-dedupes slugs). The tool **must** produce
  slugs unique across every seed invocation against a shared DB — e.g.
  incorporate the `user_id`/username and a per-invocation run token into the
  slug. Implementation may require a slug-prefix parameter on (or a thin wrapper
  around) `seed_posts` rather than calling it verbatim. The plan must confirm
  the slug constraint's scope (global vs per-user) against the migration before
  choosing the scheme.

### Seeded-content shape

`seed_posts` generates `title = None`, `slug = "seed-{i}"`,
`body = "# Post {i}\n\nbody"` (test_support.rs:571–572). The seeded posts must
satisfy the three tests' timeline assertions. The plan's per-test task **must
enumerate** what each of `:305`/`:349`/`:410` asserts against post content today
(e.g. the current helper's `Timeline Post ${i}` titles) and, per test, either
(a) adjust that assertion to the seeded content / structural checks, or (b) give
`seed-posts` a `--title-prefix` / `--body-prefix` arg. Preference: structural
checks where possible; a prefix arg only where an assertion genuinely needs a
stable title. Not left open-ended — resolved per-test in the plan.

### Nix wiring (`flake.nix`)

- Add a crane package building the `test-support` binary (shares
  `cargoArtifacts`; small — no leptos/wasm/web deps; Cachix-cached).
- Put the binary on the **e2e VM PATH** (via `environment.systemPackages` in
  both `mkE2eSqliteCheck` and `mkE2ePostgresCheck`), and expose to the
  Playwright process the values it needs to invoke it: the tool path and the
  same DB connection the server uses (`JAUNDER_DB` / Postgres URL). The prod
  NixOS module (`services.jaunder`) is **not** touched.

### Test changes (`end2end/tests/`)

- Add a Playwright helper (e.g.
  `seedTimelineViaTool(username, count, { published })`) that shells out
  (`child_process`) to `test-support seed-posts …` using the tool path + DB env
  exposed above.
- Replace the `createPublishedPostViaApi` seed loops in `:305`, `:349`, `:410`
  with a single `seedTimelineViaTool` call per user, seeding **mid-test after
  registration**. `:349` calls it once per browser context (two users → two
  invocations), preserving the two-author local timeline.
- **Keep the current post counts** (51; 2×26; 51). They are already near-minimal
  — `PAGE_SIZE + 1` is exactly "one page plus overflow," the minimum that
  exercises the pagination under test; trimming below a page would weaken
  coverage, and batch-seeding makes the count nearly free. (The issue's "trim
  the count" candidate is therefore not pursued — recorded as a decided point.)

## Out of scope / follow-ups

- **Replace `scripts/seed-e2e-fixtures.sh` in its entirety with `test-support`
  subcommands** (fixture users, `site.registration_policy` config — the _"no CLI
  for that yet"_ raw-SQL hack — and mail-capture reset). Filed as a **separate
  issue** (the plan's first task), framed as full replacement, not merely giving
  the script a nicer callee.
- **Migrating `storage::test_support::seed_posts` out of `storage` into
  `test-support`** — likely natural once the binary exists, but **not committed
  now**. Noted as future work only.
- **#155 timeout-headroom reduction + before/after measurement.** Out of scope
  here: (1) only the _seeding_ share of each budget shrinks — the
  hydration/cold-WASM share is untouched; (2) `workerContentionScale` only bites
  at **workers>1**, still blocked by #173, so contention behavior can't be fully
  validated in this cycle. A separate agent re-runs the #152
  `run-e2e-trace-analysis` after this lands and will drive any headroom
  re-tuning + `docs/observability.md` update.

## Acceptance criteria

1. A `test-support` binary crate exists, is **not** in the `jaunder` prod
   binary's dependency graph, and exposes
   `seed-posts --db --username --count [--published]`.
2. `seed-posts` seeds timeline-visible published posts through
   `create_rendered_post` for the named user on **both** SQLite and Postgres (no
   raw SQL, no per-backend branching in the caller), with **collision-free slugs
   across all invocations sharing the e2e DB** (no unique-constraint failure
   when `:305`/`:349`/`:410` all seed in one run).
3. `posts.spec.ts:305`, `:349`, `:410` no longer contain a sequential per-post
   `create_post` round-trip loop for fixture setup; they seed via `test-support`
   mid-test. `:349` still seeds two distinct authors.
4. The three tests pass on all four e2e combos
   (`{sqlite,postgres}×{chromium,firefox}`) under the normal gate
   (`cargo xtask validate` locally; CI e2e matrix), allowing for the documented
   environmental `:349` flake ([[project_csr_e2e_local_heavy_test_flake]]).
5. The `test-support` binary is built and reachable on the e2e VM PATH; the
   production NixOS module and release artifact are unchanged (no seed surface
   in prod).
6. The follow-up "replace `seed-e2e-fixtures.sh` with `test-support`" issue is
   filed.

## Risks

- **Authed home feed (`:410`) materialization — hard plan gate.** If the authed
  home feed reads from a materialized `feed_cache`/`feed_events` (hub sync
  engine) rather than a live `resolution_where` query, storage-level seeding may
  not surface there without also emitting feed events. This must be **resolved
  before the `:410` implementation commit**, not discovered during it: the
  plan's first `:410` task is to trace the `/app` home-feed read path in
  `web/`+`storage/` and confirm whether it hits the same query family the seed
  populates. Evidence to start from: the Rust web tests already seed via
  `storage::test_support::seed_posts` and assert home-feed behavior
  (`server/tests/web/web_posts.rs`, 55/51/26) — read those tests to see which
  surface they exercise. If `:410`'s path is **not** covered by the seed, the
  plan must add a task to emit the required feed rows (or seed via a path that
  does) **before** touching the test. A green `:410` is not proof on its own — a
  feed that silently reads empty could make the assertion vacuous; the plan must
  state how `:410` fails loudly if unseeded.
- **SQLite concurrent write.** The tool opens a second connection to the live DB
  while the server holds it. WAL + busy-timeout should handle it (the storage
  pool already configures this); the e2e run must complete with **no
  `SQLITE_BUSY`/lock errors** — an explicit check, not an assumption.
- **Content-shape assertions** (see above) — resolved per-test during
  implementation.
- **Extra crane build.** A second small binary derivation; acceptable and
  appropriate (a test tool _should_ be its own artifact), shares
  `cargoArtifacts`, Cachix-cached.
