# Handoff — mutation-testing loop

Written 2026-08-06, before a host restart. Nothing is running.

## Read this first

**When the old session exits it will ask whether to keep or remove this
worktree. Choose KEEP.** Nothing here is committed yet. Removing the worktree
destroys all of it.

## Where you are

- Worktree: `/home/mdorman/src/jaunder/.claude/worktrees/mutation-testing`
- Branch: `worktree-mutation-testing`, branched from `origin/main`
- Working tree: clean apart from the new, untracked files listed below
- Nothing has been committed, pushed, or run

## Why this exists

The user is leaving town for four days and wants `cargo-mutants` work to
continue unattended. The agreed shape: keep all state on disk so any wake-up is
a cheap restart, and never need a human.

## What is on disk

| Path                          | What it is                                           |
| ----------------------------- | ---------------------------------------------------- |
| `.mutants-loop/PROTOCOL.md`   | the standing orders — **read before doing any work** |
| `.mutants-loop/discover.sh`   | discovery run, one package at a time, resumable      |
| `.mutants-loop/queue.md`      | work queue, currently empty                          |
| `.mutants-loop/journal.md`    | append-only record                                   |
| `.claude/settings.local.json` | allow/deny lists so the loop never hits a prompt     |
| `HANDOFF.md`                  | this file                                            |

## Facts already established

- `cargo mutants` is on PATH through the devShell. `cargo mutants --list` works
  and reports **2339 candidate mutants**.
- Per package: `web` 614, `common` 580, `storage` 569, `server` (package name
  `jaunder`) 314, `macros` 109, `host` 87, `client` 42, `test-support` 22,
  `csr` 2.
- Workspace package names are not all directory names. `server/` is the
  `jaunder` package.
- `.cargo/mutants.toml` already excludes `main.rs`, the WASM hydrate entry,
  observability, assets, and all of `storage/src/postgres/**`.
- Host has 16 cores. `discover.sh` uses `--jobs 4`.

## The next step, and the open risk

**Discovery has never been run.** It is the next action:

    .mutants-loop/discover.sh        # start in background

`cargo-mutants` does a baseline test run before mutating. **If that baseline
fails, the whole four days produce nothing.** So after starting it, wait a few
minutes and check `.mutants-loop/discover.log` for a green baseline on `common`
before trusting the run. That check was never done.

## To start the loop

Launch with a mode that does not prompt:

    claude --permission-mode acceptEdits

`acceptEdits` alone is not enough — it covers file edits, not the `cargo` and
`git` calls. `.claude/settings.local.json` covers those. Anything unlisted still
prompts, and an unattended prompt ends the run.

Then point the session at `.mutants-loop/PROTOCOL.md` and let it work.

## Rules that must survive the restart

- Never push, never open a PR, never merge. The branch waits for the user.
- Never ask a question — skip the item and write down why.
- Only add test code. If a mutant reveals a real bug, record it and move on.
- Never leave the tree broken. A clean tree beats a killed mutant.

## Decisions the user already made

- Groundwork only, for now — the user stopped the discovery run deliberately.
- Allowlist over `--dangerously-skip-permissions`, accepting that an unpredicted
  command will stall the run.
