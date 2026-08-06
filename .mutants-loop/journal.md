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
