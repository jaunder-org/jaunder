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

| File                   | Holds                                          |
| ---------------------- | ---------------------------------------------- |
| `common.sh`            | the mandatory flags, filter, and TMPDIR        |
| `discover.sh`          | the discovery run (never edits code)           |
| `start-discovery.sh`   | starts discovery detached; `--status`/`--stop` |
| `verify.sh`            | `verify.sh <pkg> <file>` — did they die?       |
| `reconcile.sh`         | did discovery examine every mutant? run this   |
| `discover.log`         | discovery progress                             |
| `out/<pkg>/missed.txt` | surviving mutants, merged once a package ends  |
| `queue.md`             | the work queue and its state                   |
| `journal.md`           | append-only record, newest last                |

`out/<pkg>/missed.txt` is written only when every shard of that package has
finished. Mid-run, the per-shard files under `out/<pkg>/shard-N/mutants.out/`
are the live view — `start-discovery.sh --status` sums them for you.

## Each wake-up

1. Read `queue.md`. Do not read anything else first. Check whether its results
   are marked stale — a queue built by a discovery run with the wrong flags
   lists mutants that were never really alive.
2. If the queue has no `todo` files left, refill it — but **run
   `.mutants-loop/reconcile.sh` first and refuse to refill unless it passes**.
   It counts the mutants cargo-mutants generates against the ones that actually
   got an outcome. A discovery run can finish tidily having never examined a
   large slice of the work; that has happened more than once, and it always
   looked like a clean result. Only then group the mutants by file, biggest file
   first — one `todo` line per file.
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
4.  Confirm the kills:

        devtool run -- .mutants-loop/verify.sh <package> <file>

    It prints the counts and lists anything still alive. `missed` must be 0, or
    hold only what you deliberately skipped.

    **Do not hand-roll a `cargo mutants` command.** The flags are not optional,
    there are four things to get right (runner, workspace scope, test filter,
    TMPDIR), and each one fails by producing a plausible wrong number rather
    than an error. `verify.sh` and `discover.sh` share `common.sh` precisely so
    the two cannot disagree.

5.  Commit. One file per commit. `test(<pkg>): kill N mutants in <file>`
6.  Mark the items `done` or `skipped` in `queue.md`.

## Before writing anything: is this mutant real?

A "surviving mutant" is a claim that no test covers a behavior. The claim can be
false, and a false one costs more than a skipped one — you write a duplicate
test, it passes, the gate goes green, and it looks like progress.

Ask these first. Any "yes" means the mutant is **false**, not a gap:

1. **Is the code behind a `#[cfg(feature = ...)]`?** Grep upward from the mutant
   line for a `cfg(feature`. If the feature is not on by default, a
   package-scoped run never compiled it. Re-verify with `--test-workspace true`
   before believing anything.
2. **Do tests for this function already exist?** Search the file's test module
   for the function name. If there is thorough coverage and the mutant still
   "survived", suspect the harness, not the tests. The existing tests in this
   repo are good; a survivor next to ten targeted tests is a smell.
3. **Is it `#[cfg(test)]`, a doctest, or WASM-only?** Nothing in the host unit
   run reaches those.

Record a false survivor as `skipped (not compiled)` or
`skipped (already covered)` with the reason. That is a real finding and worth
writing down — it says the measurement was wrong, not the code.

**Never write a test whose only purpose is to kill a mutant you do not
understand.** If you cannot say in one sentence what bug the test would catch in
production, skip it.

## How to invoke cargo-mutants here

**Use `verify.sh` / `discover.sh`.** They share `common.sh`, which carries
everything below. This section is why those settings exist, so you do not
"simplify" one away. Every one of them was learned by getting a wrong answer,
not an error.

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
  - The one copy of this expression is `$MUTANTS_FILTER` in `common.sh`, which
    both scripts source. There is deliberately nowhere else to change it.

  One surviving postgres test out of 898 is enough to fail the baseline and lose
  a whole 315-mutant package. That has now happened twice.

- `--test-workspace true`. **Without this the tool reports mutants as surviving
  in code that was never compiled.** Several crates gate real functionality
  behind a Cargo feature that is off by default — `common`'s `sanitize`, its
  `sqlx`. Nothing in `-p common` alone turns those on. Under a workspace-wide
  test run, feature unification enables them (`storage` turns on `sanitize`).

  Package-scoped, the whole `#[cfg(feature = "sanitize")]` module of
  `common/src/render.rs` and its ~40 tests are absent from the build. Mutating
  code that is not compiled changes nothing, the tests pass, and cargo-mutants
  files the mutant as **missed**. That produced 27 false survivors in one file.

  `flake.nix` already warns about this at the doctests derivation:
  "`--workspace` is load-bearing, not incidental". The repo knew; the first
  discovery run did not.

- **`TMPDIR` on the big disk**, which `common.sh` exports. cargo-mutants copies
  the tree per job and builds it there. The default `/tmp` is a 16 GB tmpfs, and
  a workspace-wide build is far bigger than a package one — it killed a
  51-minute run at mutant 70 of 71 with `ENOSPC`. This is the flag most easily
  lost by typing a `cargo mutants` command by hand, which is the main reason not
  to.

**A baseline failure is silent-looking.** cargo-mutants prints
`ERROR cargo test failed in an unmutated tree` and exits 4, having tested
nothing. If a package reports zero caught and zero unviable, suspect the
baseline before believing the result.

**A timeout is not a result — it is a hole.** A timed-out mutant was neither
caught nor missed; it was never examined. The summary still reads "0 missed"
while a third of the package went unmeasured, which is how `storage` looked fine
at 159 timeouts. Treat a timeout rate above a few percent as a broken
measurement and fix the cap before reading anything into the numbers.

**The pattern behind every failure so far: this tool reports a plausible number
instead of an error.** A dead baseline, an uncompiled feature, a filter that
misses one test — none of them look like failures. They look like results. So
when a result surprises you (a whole package with nothing caught, dozens of
survivors in a well-tested file), the first hypothesis is that the measurement
is wrong, not that the code is untested.

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
