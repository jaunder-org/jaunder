# Plan: batch the feed-event fan-outs (issue #766)

Spec: `docs/superpowers/specs/2026-07-31-issue-766-sqlite-busy-e2e.md` (rev 2 —
trace-demonstrated mechanism; read "Demonstrated mechanism" before touching
code).

**For agentic workers:** execute with `jaunder-iterate`, delegating individual
tasks via `jaunder-dispatch` where useful. Tick checkboxes as you go.

## Review header

**Goal.** Stop feed-event fan-outs from issuing one autocommit write per (post ×
feed surface) — the write-lock churn that starved two requests past SQLite's 5s
busy_timeout (spec §Demonstrated mechanism) — by batching each fan-out into one
write-first transaction via a new `FeedEventStorage::enqueue_many`.

**Scope.** In: `enqueue_many` (generic impl, dual-backend tests); its three
production fan-out callers (worker steady-state + catch-up,
`web::feed_events::enqueue_feed_events` on the request path); mock-counted
regression tests; ADR draft; diagnosis comment on #766; four follow-up issues;
prototype cleanup. Out: `busy_timeout` changes, ADR-0022/Argon2 placement,
per-tag write loops (follow-up issue), N+1 regeneration reads (follow-up issue),
session-touch debounce (follow-up issue), seed-tool tracing (follow-up issue).

**Tasks.**

1. Hygiene + durable record: unstage/delete prototype, diagnosis comment on
   #766, file four follow-up issues.
2. `enqueue_many` in storage (TDD, dual-backend).
3. Worker `go_live_pass` batches, both branches (TDD, mock-counted).
4. Request-path `enqueue_feed_events` batches (TDD, mock-counted).
5. ADR draft: bounded write-lock occupancy.
6. Final gate (full `cargo xtask validate`).

**Key risks / decisions.**

- `enqueue_many` is implemented **once in the generic store** as a write-first
  deferred tx looping the existing single-row INSERT — measured 5–6× faster than
  the autocommit loop, one lock acquisition, no per-dialect SQL (spec
  Deliverable 2). ADR-0021-safe: first statement is a write. New impl-block
  bound `for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>`
  (exact precedent: `storage/src/posts.rs:935`).
- No dedupe in `enqueue_many`: the old loops never deduped and the drain groups
  by `feed_path` (`worker.rs:125-128`); a test pins duplicates passing through.
- `enqueue` (single) **stays** — for fixtures and one-off enqueues — but after
  this plan no production fan-out loops it.
- Worker mock tests do NOT currently stub `enqueue` (they stub
  `feed_urls_needing_catchup → Ok(vec![])` and never reach it). The breakage
  after the refactor is different: `go_live_pass` will call `enqueue_many` even
  with an empty list — wait: it must NOT (empty early-return lives in the
  storage impl, but avoiding the mock-expectation churn AND the pointless call
  belongs in the caller too). Decision: the worker skips the call when
  `urls.is_empty()`, so idle ticks issue zero enqueue calls and the six existing
  mock tests keep passing unmodified.
- `MockFeedEventStorage` gains `expect_enqueue_many` automatically
  (`mockall::automock` on the trait, slice-arg precedent `mark_regenerated`).
  `web` dev-deps already carry `storage` `test-utils` (web/Cargo.toml:41), so
  the request-path test can mock-count too.

## Global constraints

- Backend parity (CONTRIBUTING): storage behavior tests use the dual-backend
  template (`#[apply(backends)]` + `#[case] backend: Backend`).
- No `BEGIN` before a read in SQLite-reachable paths (ADR-0021).
- Coverage gate: new code paths need tests in the same change.
- Commits: run `cargo xtask check` before each commit (`jaunder-commit`); no
  `Co-Authored-By` trailer.

---

## Task 1 — hygiene + durable record

- [x] Unstage and delete the prototype **before any commit** (it is currently
      staged; a plain `git commit` would sweep it into Task 2's commit — spec
      §Cleanup forbids it landing at all, including transiently):
      `git restore --staged storage/tests/lock_prototype.rs` then
      `rm storage/tests/lock_prototype.rs`. Its numbers live in the spec.
- [x] Post the diagnosis comment on #766 (`gh issue comment 766` or GitHub MCP).
      Content: condensed spec §Demonstrated mechanism — victim spans, enqueue
      completion rates (21–117/s), no >700ms completion gap, the per-row fan-out
      loops as source (worker + request path), seed as trigger; link the
      artifact (`e2e-diagnostics-sqlite-firefox`, run 30674367840) and note rev
      1's refuted seed-tx hypothesis so nobody re-walks that path.
- [x] File via **`jaunder-issues`** — filed as #769 (A), #770 (B), #771 (C),
      #772 (D), all typed/labeled/milestoned, in Backlog project #1 at P3:
  - **A — seed tool is untraced**: no tracing/OTLP init in `test-support`/CLI
    storage writes; invisible during #766 diagnosis. Ref #766.
  - **B — session-touch write amplification**: every authed request writes
    `last_used_at` (`storage/src/sessions.rs:115`); debounce to shrink the
    victim surface. Hardening, not a bug. Ref #766.
  - **C — per-tag write loops** (same class, smaller cardinality):
    `web/src/posts/api.rs:221-223`, `storage/src/posts.rs:426-431`
    (`apply_post_tag_diff`), `server/src/atompub/posts.rs:296-301` (inlined
    duplicate); test-tooling: `SeedRawPost` per-tag loop
    (`storage/src/test_support.rs:1066`), `server/tests/web/web_tags.rs:108` (60
    writes). Ref #766 + the ADR draft.
  - **D — N+1 regeneration reads**: `build_feed_items` calls `get_tags_for_post`
    per post per regeneration (`server/src/feed/regenerate.rs:132-158`); 15,594
    spans in the failing run. Read-only; hot background path. Ref #766.

## Task 2 — `FeedEventStorage::enqueue_many` (storage crate)

**Files**

- `storage/src/feed_events.rs` — trait method, generic impl, in-file
  dual-backend tests (generic file, not an ADR-0019 dialect file; test mod
  already imports `use crate::test_support::{backends, fp, Backend};` at :260
  and file-level `chrono::{DateTime, Duration, Utc}` at :6).

**Interface**

```rust
/// Insert `pending` rows for every path in `feed_paths`, in ONE write-first
/// transaction — a single write-lock acquisition for the whole batch.
/// Production fan-outs MUST use this, not per-row `enqueue`: per-row
/// autocommit loops are the SQLite lock-churn failure mode diagnosed in #766.
/// Duplicates are inserted as-is; the drain dedupes by grouping on feed_path.
async fn enqueue_many(&self, feed_paths: &[FeedPath]) -> Result<(), FeedEventError>;
```

**Steps (TDD)**

- [x] RED: dual-backend tests in the existing `#[cfg(test)]` mod. `FeedPath` has
      no `Ord` (deliberate — `common/src/feed/feed_path.rs:16-19`), so compare
      as `HashSet<FeedPath>` (`Hash` derived); the record field is `feed_path`
      (`feed_events.rs:37`):

```rust
#[apply(backends)]
#[tokio::test]
async fn enqueue_many_creates_pending_rows_in_one_batch(#[case] backend: Backend) {
    let env = backend.setup().await;
    let paths = [fp("/feed.rss"), fp("/~alice/feed.rss"), fp("/tags/t/feed.rss")];
    env.state.feed_events.enqueue_many(&paths).await.unwrap();

    let claimed = env
        .state
        .feed_events
        .claim_pending_batch(10, Duration::seconds(60))
        .await
        .unwrap();
    let urls: std::collections::HashSet<_> =
        claimed.iter().map(|r| r.feed_path.clone()).collect();
    let expected: std::collections::HashSet<_> = paths.iter().cloned().collect();
    assert_eq!(urls, expected);
}

#[apply(backends)]
#[tokio::test]
async fn enqueue_many_inserts_duplicates_as_is(#[case] backend: Backend) {
    let env = backend.setup().await;
    let paths = [fp("/feed.rss"), fp("/feed.rss")];
    env.state.feed_events.enqueue_many(&paths).await.unwrap();
    let claimed = env
        .state
        .feed_events
        .claim_pending_batch(10, Duration::seconds(60))
        .await
        .unwrap();
    assert_eq!(claimed.len(), 2);
}

#[apply(backends)]
#[tokio::test]
async fn enqueue_many_empty_input_is_a_no_op(#[case] backend: Backend) {
    let env = backend.setup().await;
    env.state.feed_events.enqueue_many(&[]).await.unwrap();
    let claimed = env
        .state
        .feed_events
        .claim_pending_batch(10, Duration::seconds(60))
        .await
        .unwrap();
    assert!(claimed.is_empty());
}
```

- [x] `devtool run -- cargo nextest run -p storage enqueue_many` → FAIL (compile
      error until the trait method exists; smallest honest RED).
- [x] GREEN: trait method (doc above) + generic impl:

```rust
#[tracing::instrument(
    name = "storage.feed_events.enqueue_many",
    skip(self, feed_paths),
    fields(db.system = DB::DB_SYSTEM, count = feed_paths.len())
)]
async fn enqueue_many(&self, feed_paths: &[FeedPath]) -> Result<(), FeedEventError> {
    if feed_paths.is_empty() {
        return Ok(());
    }
    // One write-first transaction: a single write-lock acquisition (and one
    // WAL sync) for the whole batch, instead of one per row (#766). First
    // statement is a write, so no deferred-upgrade hazard (ADR-0021).
    let mut tx = self.pool.begin().await?;
    for feed_path in feed_paths {
        sqlx::query("INSERT INTO feed_events (feed_url) VALUES ($1)")
            .bind(feed_path)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
```

Add impl-block bound
`for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,`. Update the
`FeedEventStore` doc comment (`feed_events.rs:142-144`) — `enqueue` is no longer
the only shared method.

- [x] `devtool run -- cargo nextest run -p storage enqueue_many` → PASS (sqlite
      locally; postgres halves via the Nix gate — no local Postgres).
- [x] `devtool run -- cargo xtask check` → green; committed as `7e79d8d3`
      (`feat(storage): add FeedEventStorage::enqueue_many batched enqueue (#766)`).

## Task 3 — worker `go_live_pass` batches (server crate)

**Files**

- `server/src/feed/worker.rs` — both branches (:76-94) + in-file mock tests.

**Steps (TDD)**

- [x] RED: two in-file tests, one per branch (the existing tests' posts-mock
      arrangement is the template):

```rust
#[tokio::test]
async fn go_live_catchup_enqueues_all_surfaces_in_one_batch() {
    // posts mock: expect_feed_urls_needing_catchup → Ok(vec![fp1, fp2, fp3])
    // events mock:
    //   expect_enqueue_many().times(1)
    //     .withf(|paths| paths.len() == 3 /* and contains fp1..fp3 */)
    //     .returning(|_| Ok(()));
    //   expect_enqueue().times(0);
    // Drive: worker.go_live_pass(now) once (last_tick == None → catch-up).
}

#[tokio::test]
async fn go_live_window_enqueues_all_surfaces_in_one_batch() {
    // Prime last_tick (one catch-up pass with empty urls — no enqueue call),
    // then posts mock: list_posts_gone_live_between → 2 posts with tags.
    // events mock: expect_enqueue_many().times(1)
    //   .withf(|paths| /* every affected_feed_urls surface of both posts */)
    //   .returning(|_| Ok(()));
    // expect_enqueue().times(0);
}
```

- [x] `devtool run -- cargo nextest run -p jaunder go_live` → FAIL (per-row;
      note the server crate's package name is `jaunder`, not `server`) `enqueue`
      unexpected-call panic).
- [x] GREEN — collect-then-batch, **skipping the call when empty** so idle ticks
      issue zero enqueue calls (keeps the six existing mock tests —
      `tick_logs_and_returns_when_claim_fails`,
      `tick_returns_when_batch_is_empty`,
      `tick_regenerates_and_completes_without_hub`,
      `tick_marks_exhausted_when_regen_fails_past_backoff_table`,
      `tick_reschedules_on_regen_failure_within_backoff`,
      `spawn_tick_drives_one_tick` — passing unmodified):

```rust
None => {
    let urls = self.posts.feed_urls_needing_catchup(now).await?;
    if !urls.is_empty() {
        self.feed_events.enqueue_many(&urls).await?;
    }
}
Some(last) => {
    let mut urls = Vec::new();
    for post in self.posts.list_posts_gone_live_between(last, now).await? {
        urls.extend(affected_feed_urls(&post.username, &post.tag_slugs));
    }
    if !urls.is_empty() {
        self.feed_events.enqueue_many(&urls).await?;
    }
}
```

(`feed_urls_needing_catchup` already returns `Vec<FeedPath>` —
`storage/src/posts.rs:757`; `affected_feed_urls` returns `Vec<FeedPath>` —
`common/src/feed/feed_path.rs:106`.) Update the `go_live_pass` doc comment:
fan-out is batched into one write per pass (#766).
`tick_logs_when_go_live_pass_fails_but_still_drains` (:403) injects its failure
via `feed_urls_needing_catchup` → **unchanged**.

- [x] `devtool run -- cargo nextest run -p jaunder feed` → PASS (new tests + all
      existing; `server/tests/feed/feed_worker.rs` real-backend tests exercise
      `enqueue_many` end-to-end unchanged).
- [x] `devtool run -- cargo xtask check` → green; committed as `9cd768c3`
      (`fix(feed): batch the go-live fan-out into one enqueue_many per tick (#766)`).

## Task 4 — request-path `enqueue_feed_events` batches (web crate)

**Files**

- `web/src/feed_events.rs` — the fn (:27-36) + its in-file test mod.

**Steps (TDD)**

- [x] RED: in-file mock-counted test (`storage` `test-utils` is in web's
      dev-deps — web/Cargo.toml:41; note the fn takes `&dyn FeedEventStorage`):

```rust
#[tokio::test]
async fn enqueue_feed_events_issues_one_batched_call() {
    let mut events = storage::MockFeedEventStorage::new();
    events
        .expect_enqueue_many()
        .times(1)
        .withf(|paths| !paths.is_empty())
        .returning(|_| Ok(()));
    events.expect_enqueue().times(0);
    let tags: BTreeSet<Tag> = BTreeSet::new();
    enqueue_feed_events(&events, &"alice".parse().unwrap(), &tags)
        .await
        .unwrap();
}
```

(Adjust the `Username` construction to the crate's idiom; existing tests in the
mod show the imports. The test mod may need
`#[cfg(all(test, feature =   "server"))]`-style gating consistent with the
file's `#![cfg(feature =   "server")]`.)

- [x] `devtool run -- cargo nextest run enqueue_feed_events` → FAIL (workspace
      run, not `-p web`: the bare crate builds without the `server` feature and
      the file compiles away; workspace feature-unification enables it).
- [x] GREEN:

```rust
pub async fn enqueue_feed_events(
    events: &dyn FeedEventStorage,
    username: &Username,
    tag_slugs: &BTreeSet<Tag>,
) -> Result<(), FeedEventError> {
    // One batched write per mutation, not one per surface (#766): this runs
    // synchronously inside every post-mutation server fn.
    events.enqueue_many(&affected_feed_urls(username, tag_slugs)).await
}
```

(`affected_feed_urls` always returns at least the site+author surfaces, so no
empty-skip is needed here; the storage impl's empty early-return covers the
degenerate case anyway.)

- [x] `devtool run -- cargo nextest run enqueue_feed_events` → PASS; the five
      call sites in `web/src/posts/api.rs` (:228, :363, :478, :512, :540) are
      unchanged — behavior identical, verified end-to-end by the existing server
      integration tests.
- [x] `devtool run -- cargo xtask check` → green; committed as `733b9bf0`
      (`fix(web): batch enqueue_feed_events into one enqueue_many call (#766)`).

## Task 5 — ADR draft

- [x] Via **`jaunder-adr`** (draft-out-of-git flow): author numberless draft
      `docs/adr/drafts/sqlite-bounded-write-lock-occupancy.md`. Decision: on the
      SQLite path, write-lock occupancy must be bounded in **both** dimensions —
      duration (no slow compute/IO inside a write tx; ADR-0021's holds) and
      **acquisition count** (no per-row autocommit write loops in fan-outs;
      batch into one write-first tx). Context: #766's demonstrated starvation
      (unfair busy-wait under churn); consequences: `enqueue_many` is the
      template; the per-tag loops and N+1 reads are tracked in follow-ups
      (issues C, D); note the measured 5–6× total-time win. Builds on ADR-0021.
- [x] Commit — n/a: the drafts pen is gitignored (ADR-0048); the draft enters
      git at ship via `cargo xtask adr promote`, which numbers and stages it.

## Task 6 — final gate

- [x] `devtool run --cwd <worktree> -- cargo xtask validate` (Bash background
      mode; all four e2e combos). Outcome: static + coverage green; 3/4 combos
      green concurrently — **including sqlite/firefox, the combo this issue
      reddened** — while postgres/firefox hit the 15-min Playwright timeout
      (exit 124) with the VM starved to ~36s CPU over 15min wall (four
      concurrent QEMU VMs oversubscribed the host; ADR-0034 distributes CI for
      this exact reason). Re-run in isolation
      (`cargo xtask e2e postgres     firefox`) → green. All four combos have
      passed on this tree.

## Self-review notes

- Task 2 tests live in the generic `feed_events.rs` test mod (dual-backend
  template), not dialect files; `storage/tests/` is outside the
  `test-backend-pattern` guard's roots, but the prototype is deleted in Task 1
  anyway — before any commit, because it is already staged.
- The `times(1)` + `expect_enqueue().times(0)` pair at each fan-out is the
  deterministic regression gate; no timing assertions anywhere.
- `enqueue_many` returns `()` — no caller needs ids; the single-row `enqueue`
  (which returns an id) remains for fixtures/one-offs.
- Worker empty-skip keeps six existing mock tests green without stubs and avoids
  a pointless storage call on idle ticks.
