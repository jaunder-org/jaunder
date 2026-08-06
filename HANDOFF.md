# Handoff — mutation-testing loop

Updated 2026-08-06, after discovery finished. Nothing is running.

## Read this first

**When a session exits it will ask whether to keep or remove this worktree.
Choose KEEP.** The branch is not pushed. Removing the worktree destroys the
work.

## Where you are

- Worktree: `/home/mdorman/src/jaunder/.claude/worktrees/mutation-testing`
- Branch: `worktree-mutation-testing`, level with `main` at `525420d4`
- Working tree: clean. Everything below is committed.

## Why this exists

The user is away for four days and wants `cargo-mutants` work to continue
unattended. All state lives on disk so any wake-up is a cheap restart and no
human is ever needed.

## Commits so far

| Commit     | What                                                    |
| ---------- | ------------------------------------------------------- |
| `d23d0ca4` | groundwork — protocol, discovery script, queue, journal |
| `775f7e7d` | first kill — `BackupMode::label` text pinned            |
| `b23e5817` | one file (not one mutant) is the unit of work           |
| `afb25d09` | discovery under nextest, without the postgres cases     |
| `8048f2e0` | keep discovery off the tmpfs, out of the gate's way     |
| `eff595d2` | exclude `backup_interop`; full queue seeded             |

## State

**Discovery is complete.** It does not need to run again. 551 surviving mutants:

| Package | Mutants | Caught | Unviable | Timeout | Surviving   |
| ------- | ------- | ------ | -------- | ------- | ----------- |
| common  | 580     | 373    | 139      | 2       | 66          |
| storage | 569     | 247    | 237      | 0       | 85          |
| web     | 657     | 157    | 139      | 0       | 361         |
| host    | 87      | 35     | 28       | 0       | 24          |
| macros  | 109     | 54     | 46       | 0       | 9           |
| jaunder | 315     | 231    | 72       | 6       | 6           |
| client  | 42      | —      | —        | —       | not scanned |

Killed so far: 1. `queue.md` holds the work, per package, per file, biggest
first. Work order: `common` → `storage` → `host` → `macros` → `jaunder`, then
`web` last — `web` has 361 survivors against only 157 caught, which says the
unit suite hardly reaches it, not that there are 361 good tests to write.

## To start the loop

Launch with a mode that does not prompt:

    claude --permission-mode acceptEdits

`acceptEdits` alone is not enough — it covers file edits, not the `cargo` and
`git` calls. `.claude/settings.local.json` covers those, and denies push, merge,
rebase, reset, clean and PR creation. Anything unlisted still prompts, and an
unattended prompt ends the run.

Then:

    /loop Follow .mutants-loop/PROTOCOL.md — work the queue one file at a time.

## Traps already hit, all now handled in PROTOCOL.md

Each of these silently produced a wrong or empty result rather than an error:

1. **A failed baseline looks like success.** cargo-mutants exits 4 having tested
   nothing. Three packages — 970 mutants — vanished this way on the first pass.
   Zero caught _and_ zero unviable means suspect the baseline.
2. **`cargo test` is the wrong runner here.** `host`'s metrics tests share a
   global recorder in one process. Only `--test-tool nextest` works.
3. **The postgres filter must be case-insensitive and must name
   `backup_interop`.** `jaunder` failed three times, each on a different test.
   One failing test out of 898 loses all 315 mutants.
4. **Discovery competing with the gate produced a false red.** `tools-test`
   failed during discovery and passed in 25s after. This is the dangerous one:
   the rules say revert-and-skip on a red gate, so an unattended loop would
   throw away good work. TMPDIR now points off the 16 GB tmpfs, jobs are down to
   2, and the loop is told to check `pgrep -af cargo-mutants` first.

## Rules that must survive a restart

- Never push, never open a PR, never merge. The branch waits for the user.
- Never ask a question — skip the item and write down why.
- Only add test code. If a mutant reveals a real bug, record it and move on.
- Never leave the tree broken. A clean tree beats a killed mutant.
- The gate costs ~8 min on a source change, ~30s on docs. Batch by file.
