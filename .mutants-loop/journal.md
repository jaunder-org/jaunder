# Journal

Append-only. Newest last. One line per event.

- 2026-08-06 — groundwork laid: worktree, protocol, queue, discovery script.
  2339 candidate mutants across 9 packages. Discovery not yet started.
- 2026-08-06 — branch fast-forwarded to main (525420d4). Groundwork committed
  (d23d0ca4). Discovery restarted from the new base.
- 2026-08-06 — killed common/src/backup.rs:48 `BackupMode::label -> "xyzzy"`.
  The existing test asserted only `!label().is_empty()`; pinned the authored
  labels instead. backup.rs now 0 missed / 3 caught.
- 2026-08-06 — measured the real cost: the pre-commit gate takes ~8 min on a
  source change (Nix cache miss), ~30s on a docs-only change. Changed the unit
  of work from one mutant to one file so the gate is not the bottleneck.
- 2026-08-06 — common discovery complete: 580 mutants, 373 caught, 139 unviable,
  2 timeout, 66 surviving across 10 files. storage now running.
- 2026-08-06 — first full discovery pass finished, and three packages produced
  nothing: storage, host, jaunder all exited 4 (baseline failed). Two causes.
  storage/jaunder: `case_2_postgres` tests need a live PostgreSQL. host:
  `metrics::tests::login_records_outcome_attribute` fails under `cargo test`
  ("jaunder.auth.logins not exported") because a global recorder is shared
  across tests in one process. Both fixed by `--test-tool nextest` plus
  `-- -E 'not test(postgres)'`. Proved on host: 35 caught, 24 missed, 28
  unviable, baseline green. Re-running the three.
- 2026-08-06 — dropped `client` from discovery. All 42 mutants survived with 0
  caught: WASM-only crate, no host test reaches it. Noise, like postgres.
- 2026-08-06 — the gate failed `tools-test` while discovery was running, then
  passed the same check in 25s once discovery stopped. Cause: cargo-mutants
  copies the tree per job into /tmp, a 16 GB tmpfs, ~2.4 GB each, beside a Nix
  build. Fixed by pointing TMPDIR at the big disk and dropping to 2 jobs. Added
  a rule: check `pgrep -af cargo-mutants` before believing a red gate.
- 2026-08-06 — storage and host now yield results (storage 247 caught / 85
  missed / 237 unviable; host 35 / 24 / 28). jaunder still failed baseline: the
  filter `test(postgres)` is case-sensitive and missed
  `backend_2_Backend__Postgres`. One test out of 898 lost the whole package.
  Fixed with `test(/(?i)postgres/)`, verified by `nextest list` — 0 postgres
  tests remain. Re-running jaunder.
- 2026-08-06 — jaunder failed a third time, on one different test:
  `misc::backup_interop::backup_round_trips_full_cycle_across_backends` calls
  `unique_postgres_url()` directly, so its name never says postgres and no
  name-based postgres filter can catch it. Filter extended to
  `not test(/(?i)postgres|backup_interop/)`; `cargo nextest run -p jaunder`
  under it is green. The filter now lives in one variable (`$FILTER` in
  discover.sh) so discovery and verification cannot drift apart.
- 2026-08-06 — DISCOVERY COMPLETE for all six scanned packages. jaunder came in
  at 231 caught / 6 missed / 72 unviable / 6 timeout. 551 surviving mutants in
  total: web 361, storage 85, common 66, host 24, macros 9, jaunder 6. Discovery
  no longer needs to run, so the loop has the machine to itself.
- 2026-08-10 — started the queue at common/src/render.rs and found the whole
  discovery is measuring the wrong thing. `sanitize` is a default-OFF feature;
  `cargo mutants --package common` never enables it, so the entire
  `#[cfg(feature = "sanitize")]` module and its ~40 tests are not compiled.
  Mutating uncompiled code changes nothing, tests pass, mutant filed as MISSED.
  All 27 render.rs survivors are false on that basis. flake.nix already warned
  ("--workspace is load-bearing, not incidental") for the same reason at its
  doctests derivation. Fix is `--test-workspace true`, now in discover.sh and
  PROTOCOL.md. Queue marked STALE; discovery must be re-run before any more
  test-writing. Added an "is this mutant real?" gate to PROTOCOL — a false
  survivor costs more than a skipped one, because writing the duplicate test
  passes the gate and looks like progress.
- 2026-08-10 — hypothesis CONFIRMED on render.rs. Workspace-scoped it yields 71
  mutants (not 27 — more code compiles): 50 caught, 20 unviable, 0 missed. All
  27 reported survivors were false.
- 2026-08-10 — that verify run died at 51 min with a real ENOSPC: I ran
  cargo-mutants by hand and the TMPDIR fix lived only in discover.sh, so it used
  the 16 GB tmpfs. A workspace build is much bigger than a package one. Root
  cause is the shape of the setup, not the disk: four things must be right
  (runner, workspace scope, filter, TMPDIR) and they were spread across a script
  and a doc. Moved all of them into `.mutants-loop/common.sh`, sourced by
  discover.sh and a new verify.sh. PROTOCOL now says to call the scripts and
  never hand-roll the command. discover.sh also no longer marks a package done
  when the baseline failed (exit 4).
- 2026-08-10 — re-ran discovery workspace-scoped, web and client excluded (the
  user's call on web). Baseline green on common; 0 missed through the first 154
  of 580 mutants, consistent with the render.rs finding.
- 2026-08-10 — the harness killed that run mid-package ("ERROR interrupted").
  Two gaps it exposed, both now fixed:
  - discover.sh only resumed BETWEEN packages, so 154 mutants of work were
    thrown away. It now scans each package in 8 shards (`--shard i/8`), each
    with its own `.done`. An interruption costs one shard. The price is one
    extra baseline build per shard (~30s), against losing an hour.
  - a run launched as a child of the agent's shell is a tracked background task,
    and those get killed. `start-discovery.sh` now launches it setsid-detached
    with a pidfile, so it survives the session. `--status` sums shard progress
    live; `--stop` kills it by PID (never pattern-kill). Also noting 12 timeouts
    in the first 154 mutants, against 2 in the entire package-scoped run:
    workspace-wide test runs are slower, so cargo-mutants' auto timeout bites
    more often. Timeouts are not survivors, but they are unexamined. If the rate
    holds, the timeout needs raising.
- 2026-08-10 — the rate did not hold, it got worse, and it is a real defect in
  the measurement. common finished 298 caught / 126 unviable / **83 timeout**,
  storage 120 / 218 / **159 timeout** — a third of storage unexamined. Every
  timeout line reads exactly "20s test": they hit cargo-mutants' auto cap,
  derived from a 2s baseline. Under two parallel jobs the same builds take
  49-91s where they take 18-28s clean, and the test phase stretches with them.
  The mutants timing out are mundane (`replace > with ==` in a FromStr,
  `NoopMailSender::send_email -> Ok(())`) and cannot hang. Set `--timeout 300`
  in common.sh. Verified on common/src/mailer.rs, which had timeouts before: now
  3 caught, 1 unviable, 0 timeout, 0 missed. This is the fifth way this tool has
  reported a confident wrong number, and the nastiest: the summary still says "0
  missed" while a third of the package was never actually examined. Re-running
  discovery from scratch — caught results would survive, but a partial re-run
  leaves exactly the kind of mixed state that has caused trouble here already.
