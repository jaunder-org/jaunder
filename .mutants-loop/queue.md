# Mutation-testing queue

Read `PROTOCOL.md` first. **The unit of work is one file**, not one mutant.

> **STALE — do not work this queue.** It was built by a discovery run that
> scoped tests to one package at a time. That disables default-off Cargo
> features (`common`'s `sanitize`, its `sqlx`), so feature-gated code was never
> compiled and its mutants were filed as surviving when they are not. 27 of
> `render.rs`'s 27 are false on this basis alone, and any feature-gated code
> elsewhere is equally suspect.
>
> Discovery must be re-run with `--test-workspace true` (already fixed in
> `discover.sh`) before these numbers mean anything. The counts below are an
> upper bound on real survivors, not a work list.

Discovery output is a snapshot. Always re-verify a mutant before working it —
some listed here are already dead.

## Discovery status

| Package | Mutants | Caught | Unviable | Timeout | Surviving               |
| ------- | ------- | ------ | -------- | ------- | ----------------------- |
| common  | 580     | 373    | 139      | 2       | 66                      |
| storage | 569     | 247    | 237      | 0       | 85                      |
| web     | 657     | 157    | 139      | 0       | 361                     |
| host    | 87      | 35     | 28       | 0       | 24                      |
| macros  | 109     | 54     | 46       | 0       | 9                       |
| jaunder | 315     | 231    | 72       | 6       | 6                       |
| client  | 42      | —      | —        | —       | not scanned (WASM-only) |

Discovery is **complete**. 551 surviving mutants in total. `web` holds 361 of
them — nearly two thirds — against only 157 caught, so the honest reading is
that the unit suite hardly reaches it, not that there are 361 good tests to
write there.

## Order of work

Work `common` → `storage` → `host` → `macros` → `jaunder`, then `web` last. The
first four are pure logic with real coverage, so a surviving mutant there
usually means a genuine gap. `web` has 361 survivors against only 157 caught —
mostly server-fn and component code the unit suite barely reaches, so expect to
skip much of it.

## common — 66 surviving in 10 files

- [x] skipped (not compiled) | common/src/render.rs | 27 — all 27 were false.
      Re-run workspace-scoped: 71 mutants, 50 caught, 20 unviable, **0 missed**.
      The `sanitize` feature is default-off, so package-scoped none of this file
      compiled. No test needed; the existing ones were already killing them.
- [ ] todo | common/src/feed/atom.rs | 12
- [ ] todo | common/src/atompub/entry.rs | 10
- [ ] todo | common/src/media.rs | 4
- [ ] todo | common/src/visibility.rs | 4
- [ ] todo | common/src/pagination.rs | 1
- [ ] todo | common/src/tag.rs | 1
- [ ] todo | common/src/atompub/service.rs | 1
- [ ] todo | common/src/test_support/mod.rs | 5 — test scaffolding, not
      production code. Judge whether this is worth anything; skipping the whole
      file is defensible.
- [x] done | common/src/backup.rs | 1 — `BackupMode::label` text pinned

## storage — 85 surviving in 17 files

- [ ] todo | storage/src/posts.rs | 17
- [ ] todo | storage/src/media_manager.rs | 10
- [ ] todo | storage/src/audiences.rs | 9
- [ ] todo | storage/src/sqlite/backup.rs | 6
- [ ] todo | storage/src/subscriptions.rs | 5
- [ ] todo | storage/src/backup.rs | 3
- [ ] todo | storage/src/feed_events.rs | 3
- [ ] todo | storage/src/users.rs | 3
- [ ] todo | storage/src/sqlite/feed_events.rs | 3
- [ ] todo | storage/src/post_service.rs | 2
- [ ] todo | storage/src/sqlite/posts.rs | 2
- [ ] todo | storage/src/test_support.rs | 17 — test scaffolding; same judgement
      call as common's.
- [ ] todo | (5 more files, 1-2 each) — read `missed.txt` for the rest

## host — 24 surviving in 2 files

- [ ] todo | host/src/metrics.rs | 21
- [ ] todo | host/src/error.rs | 3

Note: `metrics.rs` holds the test that fails under plain `cargo test`
(`login_records_outcome_attribute`, a shared global recorder). Tread carefully
and keep new tests process-isolated in the same style as the existing ones.

## macros — 9 surviving in 4 files

- [ ] todo | macros/src/lib.rs | 3
- [ ] todo | macros/src/num_newtype.rs | 3
- [ ] todo | macros/src/server_fn.rs | 2
- [ ] todo | macros/src/text_enum.rs | 1

## jaunder — 6 surviving in 3 files

- [ ] todo | server/src/feed/worker.rs | 3
- [ ] todo | server/src/commands.rs | 2
- [ ] todo | server/src/atompub/mod.rs | 1

This package also had 6 timeouts. A timeout is not a survivor — it usually means
the mutant made something loop forever. Leave them alone.

## web — 361 surviving in 30 files (do last)

Biggest first: `posts/api.rs` 57, `auth/server.rs` 32, `posts/component.rs` 23,
`media/api.rs` 22, `timeline/api.rs` 20, `timeline/server.rs` 19,
`sessions/api.rs` 18, `backup/api.rs` 16, `profile/api.rs` 16, `site/api.rs` 16,
`subscriptions/api.rs` 15, `invites/api.rs` 14, then 18 more files.

Read `out/web/mutants.out/missed.txt` when you get here.

## Counts

- mutants killed: 1
- mutants skipped: 0
- files done: 1
