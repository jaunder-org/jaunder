# Mutation-testing loop — standing orders

You are running unattended. The user is away. **No one can answer a question and
no one can approve anything.** If you find yourself wanting to ask, the answer
is always: skip the item, write down why, and go to the next one.

## The job

`cargo-mutants` changes the source code in small ways (a "mutant"). If the test
suite still passes, the mutant "survived" — that means a real bug of that shape
would also pass. Your job is to write a test that kills it.

## State on disk

Everything lives in `.mutants-loop/`. Nothing lives in your head.

| File                               | Holds                                |
| ---------------------------------- | ------------------------------------ |
| `discover.sh`                      | the discovery run (never edits code) |
| `discover.log`                     | discovery progress                   |
| `out/<pkg>/mutants.out/missed.txt` | surviving mutants found by discovery |
| `queue.md`                         | the work queue and its state         |
| `journal.md`                       | append-only record, newest last      |

## Each wake-up

1. Read `queue.md`. Do not read anything else first.
2. If the queue has fewer than 10 `todo` items, refill it from the newest
   `missed.txt` files. Add each mutant as one `todo` line.
3. Take the first `todo` item. Mark it `wip`.
4. Do the work (below).
5. Update `queue.md` and append one line to `journal.md`.
6. Go to step 3 until you are low on context, then stop. The next wake-up starts
   again at step 1.

## Working one mutant

1. Read the code around the mutant. Understand what behavior it breaks.
2. Write a test that fails with the mutant and passes without it. Put it with
   the other tests for that module.
3. Run the gate: `devtool run -- cargo xtask check --no-test`, then the
   package's tests. Both must pass.
4. Confirm the kill:
   `devtool run -- cargo mutants --package <pkg> --file <file> --line <line>`
   (or re-run the single mutant however is cheapest). It must report caught.
5. Commit. One mutant per commit.
   `test(<pkg>): kill mutant <file>:<line> — <what it was>`
6. Mark the item `done` in `queue.md`.

## Hard rules

- **Never push. Never open a pull request. Never merge.** The branch waits for
  the user.
- **Never ask a question.** Skip instead.
- **Never leave the tree broken.** If the gate goes red and you cannot fix it in
  one try, undo your change (`git checkout -- <files>`), mark the item
  `skipped`, and move on. A clean tree matters more than a killed mutant.
- **Three strikes.** If a mutant resists three attempts, mark it `skipped` with
  a one-line reason. Do not keep going.
- **Only add tests.** Do not change production code to make a mutant die. If the
  mutant reveals a real bug, mark the item `skipped (real bug)` with a short
  note and move on — the user will decide.
- **Skip is cheap, wrong is not.** A skipped mutant costs nothing. A test that
  asserts the wrong thing costs the user real time. When unsure, skip.
- **Do not touch anything outside `.mutants-loop/` and test code.**

## Mutants that deserve a skip, not a test

- Trivial or unreachable code (`Display` impls, `Debug`, builders).
- Anything needing a browser, a network, or PostgreSQL — the unit suite cannot
  reach it. Note it and move on.
- Anything where the "right" behavior is a design question rather than a fact.

Skipping a lot is a normal, healthy outcome. The user wants a clean set of good
tests, not a high score.
