# Mutation-testing queue

Read `PROTOCOL.md` first. **The unit of work is one file**, not one mutant.

## Provenance

Built 2026-08-11 from a workspace-scoped run over five packages, sharded 8 ways,
`--timeout 300`. `reconcile.sh` passes:

    common: 580/580   storage: 569/569   macros: 109/109
    host:    87/87    jaunder: 315/315
    All packages fully examined.

**1660 mutants, every one accounted for, zero timeouts.** This is the first
discovery result in this project that was checked rather than asserted — the
previous one claimed 190 survivors for these same packages, and most of those
were artifacts. Do not trust a future queue that lacks a passing reconcile.

| Package | Caught | Surviving | Unviable |
| ------- | ------ | --------- | -------- |
| common  | 399    | 30        | 151      |
| storage | 295    | 37        | 237      |
| host    | 36     | 23        | 28       |
| jaunder | 232    | 11        | 72       |
| macros  | 28     | 5         | 76       |
| **all** | 990    | **106**   | 564      |

Not scanned, both on purpose: `web` (the user's call — 361 reported survivors
against 157 caught was never credible) and `client` (WASM-only, no host test
reaches it).

## Order of work

Ordered by how much a survivor there tells you, not by count.

**`host` first.** 23 survivors against only 36 caught is by far the worst ratio
in the workspace, and 21 of the 23 sit in one file. That is a genuine hole, not
a rounding artifact.

Then `common` and `jaunder`, which are well covered — a survivor there is
usually a specific missing assertion, the most useful kind of find. `storage`
last of the real work, because 17 of its 37 are in test scaffolding.

## host — 23 surviving in 2 files

- [ ] todo | host/src/metrics.rs | 21
- [ ] todo | host/src/error.rs | 2

`metrics.rs` also holds `login_records_outcome_attribute`, the test that fails
under plain `cargo test` because a global recorder is shared across tests in one
process. Keep new tests process-isolated in the style of the existing ones, and
run the package suite before the gate.

## common — 30 surviving in 7 files

- [ ] todo | common/src/feed/atom.rs | 12
- [ ] todo | common/src/atompub/entry.rs | 10
- [ ] todo | common/src/media.rs | 4
- [ ] todo | common/src/pagination.rs | 1
- [ ] todo | common/src/tag.rs | 1
- [ ] todo | common/src/visibility.rs | 1
- [ ] todo | common/src/test_support/mod.rs | 1 — scaffolding, likely skip

`render.rs` and `backup.rs` are absent from this list. `render.rs` was the file
that reported 27 false survivors package-scoped; it is now fully caught.

## jaunder — 11 surviving in 5 files

- [ ] todo | server/src/feed/worker.rs | 5
- [ ] todo | server/src/commands.rs | 3
- [ ] todo | server/src/atompub/mapping.rs | 1
- [ ] todo | server/src/atompub/mod.rs | 1
- [ ] todo | server/src/cli.rs | 1

## macros — 5 surviving in 4 files

- [ ] todo | macros/src/server_fn.rs | 2
- [ ] todo | macros/src/lib.rs | 1
- [ ] todo | macros/src/num_newtype.rs | 1
- [ ] todo | macros/src/text_enum.rs | 1

Proc-macro crates mutate badly — 76 of macros' 109 mutants are unviable. Judge
each survivor on whether a test could meaningfully assert the expansion.

## storage — 37 surviving in 10 files

- [ ] todo | storage/src/sqlite/backup.rs | 5
- [ ] todo | storage/src/media_manager.rs | 4
- [ ] todo | storage/src/audiences.rs | 2
- [ ] todo | storage/src/feed_events.rs | 2
- [ ] todo | storage/src/posts.rs | 2
- [ ] todo | storage/src/sqlite/feed_events.rs | 2
- [ ] todo | storage/src/db.rs | 1
- [ ] todo | storage/src/media.rs | 1
- [ ] todo | storage/src/users.rs | 1
- [ ] todo | storage/src/test_support.rs | 17 — scaffolding, likely skip
      wholesale. It is test-only code; killing these buys assertions about
      fixtures, not about the product. Decide once, record the decision, move
      on.

If `test_support.rs` is skipped, the real storage work is 20 mutants.

## Done

- [x] done | common/src/backup.rs | 1 — `BackupMode::label` text pinned. The
      only test written so far, and it stands: the old assertion was
      `!label().is_empty()`, which `"xyzzy"` satisfies.

## Counts

- mutants surviving: 106 (89 excluding the two test_support files)
- mutants killed: 1
- files todo: 28
