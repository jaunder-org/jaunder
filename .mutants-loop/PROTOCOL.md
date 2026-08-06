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
2. If the queue has no `todo` files left, refill it from the newest `missed.txt`
   files. Group the mutants by file, biggest file first — one `todo` line per
   file.
3. Take the first `todo` file. Mark it `wip`.
4. Do the work (below).
5. Update `queue.md` and append one line to `journal.md`.
6. Go to step 3 until you are low on context, then stop. The next wake-up starts
   again at step 1.

## The unit of work is one FILE, not one mutant

The pre-commit gate takes about **8 minutes** whenever source changes (a Nix
cache miss), and about 30 seconds when only docs change. So one commit per
mutant would spend nearly all four days waiting on the gate.

Take **all surviving mutants in one file** as a batch. Write all the tests, run
the gate once, verify once, commit once. Ten files cost ten gate runs, not
sixty-six.

If a file holds more mutants than you can handle in one wake-up, do a partial
batch and commit it. A partial batch is fine; a broken tree is not.

## Working one file

1.  Read the code around each mutant. Understand what behavior it breaks.
2.  Write tests that fail with the mutants and pass without them. Put them with
    the other tests for that module.
3.  Run the package's tests first — they are fast, and they catch your mistakes
    before the 8-minute gate does. Only then let the commit run the full gate.
4.  Confirm the kills, into a scratch dir so discovery's own output survives.
    **Use these exact flags** — see "How to invoke cargo-mutants here" below:

        devtool run -- cargo mutants -p <pkg> --file <file> \
          --test-tool nextest --output /tmp/mutverify-<name> \
          -- -E 'not test(/(?i)postgres|backup_interop/)'

    Then read `/tmp/mutverify-<name>/mutants.out/missed.txt`. It must be empty,
    or hold only the ones you deliberately skipped.

5.  Commit. One file per commit. `test(<pkg>): kill N mutants in <file>`
6.  Mark the items `done` or `skipped` in `queue.md`.

## How to invoke cargo-mutants here

Two flags are mandatory. Both were learned the hard way — the first discovery
run lost three whole packages to them.

- `--test-tool nextest`. Under plain `cargo test` the `host` crate's metrics
  tests share one process and a global recorder, so the **unmutated baseline
  fails** and cargo-mutants skips the entire package. nextest runs each test in
  its own process, which is what the repo's own gate uses.
- `-- -E 'not test(/(?i)postgres|backup_interop/)'`. Everything it excludes
  needs a live PostgreSQL that is not running. Every excluded test has a sqlite
  twin covering the same code, so no mutant goes unexamined.
  - **Keep the `(?i)`.** A plain `test(postgres)` matches `case_2_postgres` but
    not `backend_2_Backend__Postgres`.
  - **Keep `backup_interop`.** `backup_round_trips_full_cycle_across_backends`
    calls `unique_postgres_url()` directly, so its name never says postgres.
  - The canonical copy of this expression is `$FILTER` at the top of
    `discover.sh`. If you change one, change both — a filter that drifts between
    discovery and verification gives two different answers.

  One surviving postgres test out of 898 is enough to fail the baseline and lose
  a whole 315-mutant package. That has now happened twice.

**A baseline failure is silent-looking.** cargo-mutants prints
`ERROR cargo test failed in an unmutated tree` and exits 4, having tested
nothing. If a package reports zero caught and zero unviable, suspect the
baseline before believing the result.

`client` is deliberately not scanned: it is WASM-only, no host test reaches it,
and all 42 of its mutants survived with nothing caught. Pure noise.

## Hard rules

- **Never push. Never open a pull request. Never merge.** The branch waits for
  the user.
- **Never ask a question.** Skip instead.
- **Never leave the tree broken.** If the gate goes red and you cannot fix it in
  one try, undo your change (`git checkout -- <files>`), mark the item
  `skipped`, and move on. A clean tree matters more than a killed mutant.
- **A red gate is not always your fault — check for contention first.** If the
  gate fails a check you never touched (`tools-test`, `xtask-tests`, anything
  outside the file you edited), see whether discovery is still running:
  `pgrep -af cargo-mutants`. Discovery and the gate compete for disk and CPU,
  and that has already produced one false failure. Wait for discovery to finish
  and run the gate again **before** reverting anything. Reverting good work on a
  spurious failure is the most expensive mistake available to you.
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
