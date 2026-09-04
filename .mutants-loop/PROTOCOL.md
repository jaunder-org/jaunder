# Mutation testing — how the tool behaves, and how to grind it down

Two halves, and they are now separated.

**Finding** survivors is CI's job: `.github/workflows/mutants.yml` scans all
five packages weekly (incremental) and monthly (full audit), with a real
PostgreSQL cluster, and fails the run on any survivor or timeout. Nothing local
needs to discover anything.

**Killing** them is the grind, one file at a time, driven by the
`jaunder-mutants` skill. This file is the reference behind it: why the flags are
what they are, and how this tool fails.

## The one thing to understand about cargo-mutants

**It reports a plausible number instead of an error.** A dead baseline, an
uncompiled feature, a filter that misses one test, an invalid argument — none of
them look like failures. They look like results.

So when a result surprises you — a whole package with nothing caught, dozens of
survivors in a well-tested file, a file that suddenly went clean — the first
hypothesis is that the measurement is wrong, not that the code is untested.
Every flag below exists because that happened.

## Keep no state

There is no work queue, deliberately.

There was one. It recorded what a single discovery run found, and then had no
way to notice it had stopped being true: `queue.md` listed `host/src/metrics.rs`
as 21 survivors long after commit ee9a34d5 killed all of them. It was not wrong
when written. That is the problem — anything derived from a fixed point in
history rots silently, and a stale work list is indistinguishable from a current
one.

So the work list is re-derived every time:

```bash
devtool run -- .mutants-loop/next-work.sh          # <package> <file> <surviving>
```

It reads the newest **scheduled** run's artifacts — not the newest run, because
a manual dispatch may have scanned a single package and would report an empty
list for the other four. It prints its source run to stderr; read it.

Then **confirm against the working tree before touching anything**, because the
scan is by definition older than your checkout:

```bash
devtool run -- .mutants-loop/verify.sh <package> <file>
```

A file fixed since the scan returns `missed=0` and drops out by itself. That
property is what makes a stale list harmless — do not add a cache to speed it
up.

## Files here

| File           | Holds                                                  |
| -------------- | ------------------------------------------------------ |
| `common.sh`    | the mandatory flags, filter, and TMPDIR — the one copy |
| `next-work.sh` | what is left, ranked, from the last scheduled CI run   |
| `verify.sh`    | `verify.sh <pkg> <file>` — did they die?               |
| `ci-shard.sh`  | one shard of one package, for the CI workflow          |
| `journal.md`   | append-only record, newest last                        |

There is deliberately no local whole-package scan any more. `discover.sh`,
`start-discovery.sh` and `reconcile.sh` are gone: discovery is what the
scheduled workflow does, on better hardware, against every package, with a real
PostgreSQL cluster, and with its own completeness check (the report job requires
`outcomes.json` from every shard and fails on a count mismatch). Running a
second, weaker copy locally only created a second answer that could disagree.

## The unit of work is one FILE

The gate takes minutes whenever source changes, so a commit per mutant would
spend nearly all its time waiting. Take **all surviving mutants in one file** as
a batch: write the tests, verify once, commit once.

If a file holds more than you can handle at once, do a partial batch and commit
it. A partial batch is fine; a broken tree is not.

## Before writing anything: is this mutant real?

A surviving mutant is a _claim_ that no test covers a behavior. The claim can be
false, and a false one costs more than a skipped one — you write a duplicate
test, it passes, the gate goes green, and it looks like progress.

Any "yes" means the mutant is **false**, not a gap:

1. **Behind a `#[cfg(feature = ...)]`?** Grep upward for `cfg(feature`. If the
   feature is off by default, a package-scoped run never compiled it.
2. **Do thorough tests already exist?** Search the test module for the function
   name. A survivor next to ten targeted tests is a smell — suspect the harness.
3. **`#[cfg(test)]`, a doctest, or WASM-only?** Nothing in the host run reaches
   those.

Record it in `journal.md` as `skipped (not compiled)` or
`skipped (already covered)` with the reason. That is a real finding: it says the
measurement was wrong, not the code.

**Never write a test whose only purpose is to kill a mutant you do not
understand.** If you cannot say in one sentence what bug it would catch in
production, skip it.

## How to invoke cargo-mutants here

**Use `verify.sh`.** It shares `common.sh` with `ci-shard.sh`, so a local answer
and a CI answer cannot disagree. This section is why those settings exist, so
you do not "simplify" one away. Every one was learned by getting a wrong answer,
not an error.

- `--test-tool nextest`. Under plain `cargo test` the `host` crate's metrics
  tests share one process and a global recorder, so the **unmutated baseline
  fails** and cargo-mutants skips the entire package. nextest gives each test
  its own process, which is what the repo's own gate uses.

- `-- -E "$MUTANTS_FILTER"`, whose value depends on whether a PostgreSQL cluster
  is running. `MUTANTS_WITH_POSTGRES` is the one switch, and it moves **two**
  things together:

  | `MUTANTS_WITH_POSTGRES`          | test filter                                | `storage/src/postgres` |
  | -------------------------------- | ------------------------------------------ | ---------------------- |
  | unset / `0` (a workstation)      | `not test(/(?i)postgres\|backup_interop/)` | excluded from mutation |
  | `1` (CI, under `devtool pg run`) | `all()`                                    | mutated                |

  They move together because they are one decision. Filtering the postgres tests
  away while still mutating the code they cover reports every one of those
  mutants as MISSED — noise wearing a survivor's name.

  **Consequence for the grind:** survivors in `storage/src/postgres/**` were
  found by CI with a real cluster and **cannot be reproduced by a bare
  `verify.sh`** — it will report `missed=0` and you will "fix" nothing while
  believing you did. Work them under a cluster:

  ```bash
  devtool pg run -- env MUTANTS_WITH_POSTGRES=1 .mutants-loop/verify.sh storage <file>
  ```

  - **Keep the `(?i)`.** A plain `test(postgres)` matches `case_2_postgres` but
    not `backend_2_Backend__Postgres`.
  - **Keep `backup_interop`.** `backup_round_trips_full_cycle_across_backends`
    calls `unique_postgres_url()` directly, so its name never says postgres.

- `--test-workspace true`. **Without this the tool reports mutants as surviving
  in code that was never compiled.** Several crates gate real functionality
  behind a Cargo feature that is off by default — `common`'s `sanitize`, its
  `sqlx`. Nothing in `-p common` alone turns those on; under a workspace-wide
  run, feature unification does (`storage` turns on `sanitize`).

  Package-scoped, the whole `#[cfg(feature = "sanitize")]` module of
  `common/src/render.rs` and its ~40 tests are absent from the build. Mutating
  uncompiled code changes nothing, the tests pass, and the mutant is filed as
  **missed** — 27 false survivors in one file. `nix/checks.nix` already warns
  about this at the doctests derivation: "`--workspace` is load-bearing, not
  incidental".

- **`TMPDIR` on the big disk**, which `common.sh` exports. cargo-mutants copies
  the tree per job and builds it there; the default `/tmp` is a 16 GB tmpfs, and
  a workspace build killed a 51-minute run at mutant 70 of 71 with `ENOSPC`.
  This is the flag most easily lost by typing a command by hand — which is the
  main reason not to.

**A baseline failure is silent-looking.** cargo-mutants prints
`ERROR cargo test failed in an unmutated tree` and exits 4, having tested
nothing. Zero caught _and_ zero unviable means suspect the baseline, not the
tests.

**A timeout is a hole, not a result.** Neither caught nor missed — never
examined, while the summary still reads "0 missed". That is how `storage` looked
fine at 159 timeouts. CI now fails on any timeout for this reason. Locally,
treat more than a few percent as a broken measurement and fix the cap before
reading the numbers.

**Exit codes carry nothing.** 2 means both "found surviving mutants" and
"invalid argument". Judge on artifacts — `outcomes.json` for "did it test
anything", `missed.txt` for the finding.

`client` is deliberately never scanned: WASM-only, no host test reaches it, all
42 mutants survived with nothing caught. `web` likewise, by the user's call —
361 reported survivors against 157 caught was never credible.

## Hard rules

- **Never merge.** A PR at the end of a grind is fine; landing it is the user's
  call.
- **Never leave the tree broken.** If the gate goes red and you cannot fix it in
  one try, undo your change (`git checkout -- <files>`), record the skip, and
  move on. A clean tree matters more than a killed mutant.
- **A red gate is not always your fault — check for contention first.** If the
  gate fails a check you never touched (`tools-test`, `xtask-tests`, anything
  outside the file you edited), see whether a local scan is running:
  `pgrep -af cargo-mutants`. They compete for disk and CPU, and that has already
  produced one false failure. Re-run the gate **before** reverting anything.
  Reverting good work on a spurious failure is the most expensive mistake
  available here.
- **Three strikes.** If a mutant resists three attempts, skip it with a one-line
  reason.
- **Only add tests.** Do not change production code to make a mutant die. If the
  mutant reveals a real bug, file it (`jaunder-issues`) and move on — that is a
  finding, not a chore.
- **Skip is cheap, wrong is not.** A skipped mutant costs nothing; a test that
  asserts the wrong thing costs real time. When unsure, skip.

## Mutants that deserve a skip, not a test

- Trivial or unreachable code (`Display`, `Debug`, builders).
- Anything needing a browser or a network — the unit suite cannot reach it.
- Anything where the "right" behavior is a design question rather than a fact.

Skipping a lot is a normal, healthy outcome. The goal is a clean set of good
tests, not a high score.
