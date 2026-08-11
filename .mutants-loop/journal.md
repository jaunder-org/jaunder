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
- 2026-08-10 — `--stop` was broken and reported success anyway. setsid puts
  discover.sh in its own process group, so killing the launcher pid left
  cargo-mutants orphaned and still building; it had to be killed by PID by hand.
  `--stop` now signals the process GROUP, then waits and confirms, escalating to
  KILL after 20s. A stop that lies is worse than one that fails loudly. Also
  added `--in <seconds>` for a detached delayed start.
- 2026-08-11 — discovery finished all five packages with ZERO timeouts, which
  confirms the --timeout 300 fix. Results: common 351 caught / 30 missed / 126
  unviable; storage 244 / 35 / 218; host 27 / 23 / 26; jaunder 209 / 10 / 56;
  macros 23 / 4 / 68. 102 survivors, against the 190 the broken run claimed for
  the same five.
- 2026-08-11 — but the totals did not reconcile, and this bug is mine.
  cargo-mutants shards are ZERO-indexed (`--shard k/n` requires k < n). The loop
  ran 1..8, so shard 0 was never run and 8/8 died with "shard k must be less
  than n". Every package is short exactly one shard: common 507 of 580, storage
  497 of 569, macros 95 of 109, host 76 of 87, jaunder 275 of 315. It was
  recorded as success because the exit code cannot distinguish it: cargo-mutants
  uses 2 for "found surviving mutants" (a normal result) and clap also exits 2
  for an invalid argument. shard-8/ held nothing but the .done marker I wrote.
  Fixed the loop to 0..SHARDS-1, and .done is now written only when
  mutants.out/outcomes.json exists — evidence that something was actually
  tested, never the exit code alone. Removed the bogus shard-8 dirs and the
  package .done markers; shards 1-7 keep theirs, so the re-run does only shard 0
  (~1/8 of the work). Sixth silent wrong number in this project, and the first
  one I introduced rather than inherited. Same shape as all the others: a
  plausible summary over an unexamined gap.
- 2026-08-11 — stopped finding these by hand. Added `reconcile.sh`: it diffs the
  mutants `cargo mutants --list` generates for a package against the ones that
  actually got an outcome, and names anything never examined. discover.sh runs
  it automatically at the end, and PROTOCOL forbids refilling the queue until it
  passes. Proved it catches the live gap — "macros: INCOMPLETE — 95/109
  examined, 14 NEVER EXAMINED", naming the exact mutants. Six wrong numbers, all
  the same shape, all found by noticing totals that did not add up. That is now
  a check rather than an act of vigilance.
- 2026-08-11 — DISCOVERY COMPLETE AND VERIFIED. reconcile.sh passes on all five
  packages: common 580/580, storage 569/569, macros 109/109, host 87/87, jaunder
  315/315. 1660 mutants, every one accounted for, zero timeouts. 990 caught, 564
  unviable, **106 surviving** — against the 190 the broken run claimed for the
  same packages. Rebuilt queue.md from these results, ordered by signal rather
  than count: host first (23 survivors against only 36 caught, the worst ratio
  in the workspace, 21 of them in metrics.rs), then common and jaunder where a
  survivor usually means one specific missing assertion, storage last because 17
  of its 37 are in test scaffolding. Excluding the two test_support files leaves
  89 mutants worth arguing about. Deleted HANDOFF.md — it described a world
  three corrections out of date, and a stale handoff is worse than none.
  PROTOCOL, queue.md and this journal carry everything a restart needs.
