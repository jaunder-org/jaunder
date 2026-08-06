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
