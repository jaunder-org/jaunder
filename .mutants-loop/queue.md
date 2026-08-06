# Mutation-testing queue

Read `PROTOCOL.md` first. States: `todo` → `wip` → `done` | `skipped`.

One item per line:

    - [ ] todo | <pkg> | <file>:<line> | <mutation>

Refill from `out/<pkg>/mutants.out/missed.txt`. Discovery writes that file as it
goes, so it is safe to read while discovery is still running.

## Counts

- todo: 0
- done: 1
- skipped: 0

## Items

- [x] done | common | common/src/backup.rs:48 | replace `BackupMode::label` with
      `"xyzzy"` — pinned the authored UI labels; the old test only asserted
      non-empty.

## Notes for the next wake-up

- Discovery is running package by package. `common` was in progress at the time
  of writing: 31 caught, 17 unviable, 1 missed (now killed).
- Verify a kill with a scoped re-run into a scratch dir, so discovery's own
  output is not clobbered:
  `cargo mutants -p <pkg> --file <file> --output /tmp/mutverify-<name>`
- `cargo mutants` exits non-zero when a mutant survives, so exit 0 on a scoped
  run already means "caught". Read `missed.txt` to be sure.
