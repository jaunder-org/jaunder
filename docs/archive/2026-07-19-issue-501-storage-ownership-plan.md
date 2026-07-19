# Plan — issue #501: storage owned↔borrowed API shape fixes

Spec:
[`2026-07-18-issue-501-storage-ownership.md`](../specs/2026-07-18-issue-501-storage-ownership.md)
Issue: jaunder-org/jaunder#501

## Review header

**Goal.** Land the three genuine ownership cleanups in `storage/` (AC-1/2/3 of
the spec); the original item #1 is descoped (see spec §Descoped).

**Scope.**

- _In:_ `candidate_slug` returns `Slug` (AC-1); `seed_post_input` takes
  `PostBody` (AC-2); post-summary bind by borrow (AC-3); report the item-#1
  descope on the issue.
- _Out:_ item #1 (`hash_password`/`verify_password` signatures — unchanged); the
  other #498–#507 conversion-audit issues; any newtype/macro change; any trait
  signature change.

**Tasks.**

- [x] 1. AC-3 — `posts.rs:1892` bind `input.summary.as_deref()` instead of
     `.clone()`.
- [x] 2. AC-1 — `candidate_slug -> Result<Slug, InvalidSlug>`; update the
     collision-loop caller and the unit tests.
- [x] 3. AC-2 — `seed_post_input(body: PostBody)`; drop the in-fn `.into()`;
     convert the two callers.
- [ ] 4. Report the item-#1 descope finding on issue #501 (at ship).

**Key decisions / risks.**

- These are **behavior-preserving type-shape refactors**, not new behavior — so
  there is no red-first TDD step. Verification = the existing suite stays green
  plus the spec's observable checks. Tests change only where a signature changed
  (AC-1 asserts on `Slug`; AC-2 callers pass `PostBody`).
- `PostBody` is `#[str_newtype(infallible)]` (`From<String>`), so AC-2 callers
  convert with `.into()` — no validating parse/helper needed.
- `candidate_slug` has one production caller (`post_service.rs:492`) plus two
  sets of tests: storage's own (`post_service.rs:768–791`) **and** two duplicate
  tests in the `web` crate (`web/src/posts/mod.rs:842–852`, test-only — no
  production web usage). AC-1's signature change breaks the web tests too; Task
  2 updates both. There is no non-test cross-crate consumer.

**For agentic workers.** Execute with **jaunder-iterate**; delegate a task via
**jaunder-dispatch** if useful. One commit per task; no `Co-Authored-By`
trailer.

## Global constraints

- Worktree: `.claude/worktrees/issue-501-storage-ownership` (branch
  `worktree-issue-501-storage-ownership`). Run all `cargo`/`git` from here.
- Per-task: run `cargo xtask check` clean before committing (the pre-commit hook
  runs the full check — fmt + clippy + Nix coverage/tests); see
  **jaunder-commit**.
- Follow `CONTRIBUTING.md` (backend parity, coverage policy, dialect-file
  rules).
- Ship gate: `cargo xtask validate --no-e2e` green.

---

## Task 1 — AC-3: bind post summary by borrow

**Files**

- `storage/src/posts.rs` (~`:1892`).

**Change**

- In the INSERT binding block, replace `.bind(input.summary.clone())` with
  `.bind(input.summary.as_deref())`. `input.summary: Option<String>` →
  `Option<&str>`, which sqlx binds identically. No other change.

**Tests**

- No test change: `create_post_persists_summary` (`posts.rs:2379`) already
  round-trips a `Some(summary)` and asserts persistence; it must stay green.
  This is the behavior guard.

**Run**

- `cargo nextest run -p storage create_post_persists_summary` → PASS.

**Observable (spec AC-3)**

- `rg 'summary.clone\(\)' storage/src/posts.rs` → no match; the bind reads
  `input.summary.as_deref()`.

**Commit**

- `cargo xtask check` clean, then commit:
  `storage: bind post summary by borrow (#501)`.

---

## Task 2 — AC-1: `candidate_slug` returns `Slug`

**Files**

- `storage/src/post_service.rs` — `candidate_slug` (`:403`), the collision loop
  caller (`:491–495`), unit tests (`:768–791`).
- `web/src/posts/mod.rs` — the two duplicate tests (`:842–852`) that call
  `candidate_slug` and assert against `&str`.

**Interfaces**

- `pub fn candidate_slug(slug_seed: &Slug, attempt: usize) -> Result<Slug, common::slug::InvalidSlug>`
  - attempt `0`: `return Ok(slug_seed.clone());` (already a valid `Slug`; no
    parse).
  - suffixed path: build the candidate `String` as today, then
    `format!("{}{suffix}", base.trim_end_matches('-')).parse()` and return that
    `Result` (the single `Slug::from_str` chokepoint — no bypass constructor).

**Change — caller (`:491–495`)**

```rust
for attempt in 0..max_attempts {
    let slug = candidate_slug(&slug_seed, attempt).map_err(PerformCreationError::InvalidSlug)?;
    match create_rendered_post(
        storage,
        RenderedPostContent { user_id, /* … */ slug, /* … */ },
```

Drop the intermediate `slug_string` and the caller-side `.parse::<Slug>()`.

**Tests (`:768–791`)**

- Update the three cases to the `Result<Slug, _>` return: `.unwrap()` the result
  and assert on the `Slug` via `AsRef<str>`/equality (e.g.
  `assert_eq!(candidate_slug(&hello, 0).unwrap().as_ref(), "hello")`).
- Remove the now-redundant `c.parse::<Slug>().is_ok()` re-validation asserts at
  `:777`/`:787` (the returned value is already a `Slug`).

**Tests — `web/src/posts/mod.rs:842–852`**

- Update the two duplicate tests to the new return type: `.unwrap()` the result
  and compare via `AsRef<str>` (e.g.
  `assert_eq!(candidate_slug(&base, 0).unwrap().as_ref(), "hello-world")`). Keep
  them as-is otherwise (relocating/removing these duplicates is out of scope).

**Run**

- `cargo nextest run -p storage candidate_slug` → the updated cases PASS.
- `cargo nextest run -p storage post_service` → the slug-collision creation
  paths stay green (behavior guard for the caller change).
- `cargo nextest run -p web candidate_slug` → the updated web tests PASS (this
  compiles the `web` crate, which the storage-only runs above cannot — required
  to catch the cross-crate breakage before the commit gate).

**Observable (spec AC-1)**

- The collision loop (`post_service.rs` ~`:491–495`) contains no
  `.parse::<Slug>()` (read the loop body — a bare `rg` still matches unrelated
  uses at `:326`). Tests assert on a returned `Slug`.

**Commit**

- `cargo xtask check` clean, then commit:
  `storage: candidate_slug returns Slug, not String (#501)`.

---

## Task 3 — AC-2: `seed_post_input` takes `PostBody`

**Files**

- `storage/src/post_service.rs` — `seed_post_input` (`:105–122`).
- Callers: `test-support/src/lib.rs:82`, `storage/src/test_support.rs:625`.

**Interfaces**

- `pub fn seed_post_input(user_id: UserId, slug: Slug, body: PostBody, published: bool) -> CreatePostInput`
  (keep the `#[cfg(any(test, feature = "seed-posts"))]` gate).
  - In the body, pass `body` straight into `RenderedPostContent { body, … }` —
    drop the `body.into()` at `:115`.

**Change — callers**

- `test-support/src/lib.rs:85`: `seed_body(prefix, i)` →
  `seed_body(prefix, i).into()`.
- `storage/src/test_support.rs:628`: `format!("# Post {i}\n\nbody")` →
  `format!("# Post {i}\n\nbody").into()`. (`PostBody: From<String>`,
  infallible.)

**Tests**

- No dedicated new test — both callers are themselves test/seed helpers
  exercised by the existing timeline/seed tests. Compilation under both feature
  sets is the guard.

**Run**

- `cargo nextest run -p storage --features seed-posts` → PASS (storage seed
  path).
- `cargo nextest run -p test-support` (or the suite that drives `seed_posts`) →
  PASS.
- Default build still compiles: `cargo check -p storage`.

**Observable (spec AC-2)**

- `seed_post_input`'s signature names `body: PostBody`; no `String`→`PostBody`
  conversion remains inside the function; compiles under `--features seed-posts`
  and the default test build.

**Commit**

- `cargo xtask check` clean, then commit:
  `storage: seed_post_input takes PostBody (#501)`.

---

## Task 4 — Report the item-#1 descope (ship time)

- Post a comment on issue #501 recording the descope of the original item #1:
  the `spawn_blocking` `'static` requirement means
  `hash_password`/`verify_password` cannot borrow `&Password`, and every
  alternative (`block_in_place` test-runtime churn, owned-threading convention
  break, no-op inner-`String` copy) costs more than the negligible cold-path
  allocation it would remove. Reference the spec §Descoped. This is done as part
  of **jaunder-ship** (with the PR that lands AC-1/2/3), not a code change.
