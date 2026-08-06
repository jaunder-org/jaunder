# Mutation-testing queue

Read `PROTOCOL.md` first. **The unit of work is one file**, not one mutant.

Refill from `out/<pkg>/mutants.out/missed.txt`. Discovery writes that file as it
goes, so it is safe to read while discovery is still running.

## Progress

| Package | Discovery | Caught | Unviable | Timeout | Surviving |
| ------- | --------- | ------ | -------- | ------- | --------- |
| common  | complete  | 373    | 139      | 2       | 66        |
| storage | running   | —      | —        | —       | —         |
| others  | queued    | —      | —        | —       | —         |

## Work queue — `common`, by file, biggest first

- [ ] todo | common/src/render.rs | 27 mutants
- [ ] todo | common/src/feed/atom.rs | 12 mutants
- [ ] todo | common/src/atompub/entry.rs | 10 mutants
- [ ] todo | common/src/test_support/mod.rs | 5 mutants
- [ ] todo | common/src/media.rs | 4 mutants
- [ ] todo | common/src/visibility.rs | 4 mutants
- [ ] todo | common/src/pagination.rs | 1 mutant
- [ ] todo | common/src/tag.rs | 1 mutant
- [ ] todo | common/src/atompub/service.rs | 1 mutant
- [x] done | common/src/backup.rs | 1 mutant — `BackupMode::label` text pinned

`common/src/test_support/mod.rs` is test scaffolding, not production code. Judge
whether killing those five is worth anything before spending time on it;
skipping the file wholesale is a defensible call.

The `backup.rs` line still appears in `missed.txt` — that file was written
before the fix landed. Discovery output is a snapshot, so always re-verify a
mutant before working it.

## Counts

- files todo: 9
- files done: 1
- mutants killed: 1
- mutants skipped: 0
