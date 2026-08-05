# Issue #792 — measurement notes

Working notes for the plan
[`2026-08-04-issue-792-e2e-warmup.md`](./2026-08-04-issue-792-e2e-warmup.md).
Raw evidence lives here; the findings that survive go to `docs/observability.md`
(spec AC-6) and the verdict comment (AC-7).

## Task 2 — baseline `drvPath` (captured 2026-08-04, before any `flake.nix` edit)

Tree state at capture: `flake.nix` **unmodified** (`git diff HEAD -- flake.nix`
empty); the only diff-from-HEAD was the staged spec/plan markdown.

**Staged-file caveat, settled empirically.** The plan review verified that
_untracked_ files do not affect `nix eval`, but these planning docs were
**staged**, and staged files _are_ part of a flake's source. Re-evaluating
`e2e-sqlite-chromium` with them staged returned `b4m5d17…` — byte-identical to
the value the reviewer captured before they existed. So staged docs do not reach
the e2e derivations either, and the baseline below is comparable against any
later tree state that differs only in `flake.nix` plus docs/xtask additions.

| Attr                                                 | `drvPath`                                                                                        |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `checks.x86_64-linux.e2e-sqlite-chromium`            | `/nix/store/b4m5d17ziz5ymqjihg72p5mkzmwim673-vm-test-run-jaunder-e2e-sqlite-chromium.drv`        |
| `checks.x86_64-linux.e2e-sqlite-firefox`             | `/nix/store/vv1nicbck2cfiiv98ydgs2yg7jspjcwn-vm-test-run-jaunder-e2e-sqlite-firefox.drv`         |
| `checks.x86_64-linux.e2e-postgres-chromium`          | `/nix/store/nyczll687gcr17shzacpy81jk0faardy-vm-test-run-jaunder-e2e-postgres-chromium.drv`      |
| `checks.x86_64-linux.e2e-postgres-firefox`           | `/nix/store/r7nlys0drfbs6b8fg34w8faybz949v0p-vm-test-run-jaunder-e2e-postgres-firefox.drv`       |
| `packages.x86_64-linux.e2e-sqlite-chromium-cold`     | `/nix/store/5ad04g0yhh0219nb3z0iryjb1x1iy9ml-vm-test-run-jaunder-e2e-sqlite-chromium-cold.drv`   |
| `packages.x86_64-linux.e2e-sqlite-firefox-cold`      | `/nix/store/hdhvm8w6b0g5hzpxkbavryd7gp8birgx-vm-test-run-jaunder-e2e-sqlite-firefox-cold.drv`    |
| `packages.x86_64-linux.e2e-postgres-chromium-cold`   | `/nix/store/x69vznbcs7pwywrrgw2yllm6kj6xw5bx-vm-test-run-jaunder-e2e-postgres-chromium-cold.drv` |
| `packages.x86_64-linux.e2e-postgres-firefox-cold`    | `/nix/store/854x5n6bcg1nnd09vihh0jfaw6nx29f8-vm-test-run-jaunder-e2e-postgres-firefox-cold.drv`  |
| `packages.x86_64-linux.jaunder` (crane-filter probe) | `/nix/store/1ia6nd3g5w9hkysc5hphrrlrj8pwv79k-jaunder-0.1.0.drv`                                  |

Capture command (re-runnable):

```sh
for a in checks.x86_64-linux.e2e-{sqlite,postgres}-{chromium,firefox} \
         packages.x86_64-linux.e2e-{sqlite,postgres}-{chromium,firefox}-cold \
         packages.x86_64-linux.jaunder; do
  echo "$a = $(nix eval --raw ".#$a.drvPath" 2>/dev/null)"
done
```

The last row is the session-length probe (plan task 2c): `flake.nix` is outside
the crane source filter, so `jaunder`'s hash must stay `1ia6nd3g…` across every
salt flip. If it ever moves, a salted run is rebuilding the Rust workspace and
the collection budget is blown.

## Task 3 — the three hash proofs (2026-08-04)

All three pass. Prefixes below; full paths are reproducible with the capture
command above.

| Attr                         | baseline  | 3d: literals in, defaults | 3e: `e2eSalt = "probe"` | 3f: `e2eWarmup = false` |
| ---------------------------- | --------- | ------------------------- | ----------------------- | ----------------------- |
| `e2e-sqlite-chromium`        | `b4m5d17` | `b4m5d17` ✅ same         | `4izlp0q` ✅ differs    | `308p743` ✅ differs    |
| `e2e-sqlite-firefox`         | `vv1nicb` | `vv1nicb` ✅              | `xs0jhrs` ✅            | `qqshv0r` ✅            |
| `e2e-postgres-chromium`      | `nyczll6` | `nyczll6` ✅              | `gci86r0` ✅            | `7zaz7x0` ✅            |
| `e2e-postgres-firefox`       | `r7nlys0` | `r7nlys0` ✅              | `j4r69mq` ✅            | `zff4hb6` ✅            |
| `e2e-sqlite-chromium-cold`   | `5ad04g0` | `5ad04g0` ✅              | `289qb7q` ✅            | `5ad04g0` ✅ unchanged  |
| `e2e-sqlite-firefox-cold`    | `hdhvm8w` | `hdhvm8w` ✅              | `r66d2dy` ✅            | —                       |
| `e2e-postgres-chromium-cold` | `x69vznb` | `x69vznb` ✅              | `ycd94ri` ✅            | —                       |
| `e2e-postgres-firefox-cold`  | `854x5n6` | `854x5n6` ✅              | `hl5y0sq` ✅            | `854x5n6` ✅ unchanged  |
| `jaunder` (crane probe)      | `1ia6nd3` | `1ia6nd3` ✅              | `1ia6nd3` ✅ unchanged  | —                       |

What each column establishes:

- **3d (AC-2, and AC-3's identity half)** — adding the wiring with both literals
  at their defaults changes **nothing**. The cachix pulls and any local e2e
  store paths survive the scaffolding landing.
- **3e (AC-1)** — a non-empty salt moves all eight e2e derivations, so
  `traces run` cannot return a cached suite result. `jaunder` is **unchanged**,
  confirming `flake.nix` sits outside the crane source filter: a salted run
  re-runs the VM suite without rebuilding the Rust workspace. This is what
  bounds the collection session to suite time rather than build time.
- **3f (AC-3)** — `e2eWarmup` is load-bearing on exactly the four warm gate
  checks; the cold packages are untouched, as expected since they never set the
  warmup token.

Independently corroborated: an out-of-band review agent produced `b4m5d17`,
`854x5n6`, `1ia6nd3`, `4izlp0q`, `hl5y0sq` and `308p743` from its own probe of
the same edits, matching every value above.

After 3f the tree was restored (`e2eSalt = ""`, `e2eWarmup = true`) and
`e2e-sqlite-chromium` re-evaluated to `b4m5d17…` — back at baseline.

## Task 7 — collection run log (2026-08-04/05)

Six runs, no discards. Session baseline `/proc/loadavg` **0.75 0.94 0.82**; load
during collection stayed in a narrow band (2.10–2.60), so no run met D6's
load-excursion discard criterion. Every run took ~21 min for its four serial
combos; each carried a distinct salt, so all 24 combo store paths remain
independently readable.

| run | salt | `e2eWarmup` | started (sq-chr) | loadavg after  |
| --- | ---- | ----------- | ---------------- | -------------- |
| A1  | `a1` | `true`      | 22:27:40Z        | 2.18 2.34 1.96 |
| B1  | `b1` | `false`     | 22:49:11Z        | 2.22 2.24 2.08 |
| A2  | `a2` | `true`      | 23:06:28Z        | 2.27 2.41 2.30 |
| B2  | `b2` | `false`     | 23:27:16Z        | 2.47 2.43 2.36 |
| A3  | `a3` | `true`      | 23:44:03Z        | 2.10 2.33 2.35 |
| B3  | `b3` | `false`     | 00:04:40Z        | 2.60 2.38 2.29 |

Per-combo `.stats.duration` (s) / `flaky` / `unexpected`, all runs `expected`
130 except A1 postgres-firefox (129 + 1 flaky):

| run | sq-chr | sq-ff | pg-chr | pg-ff           |
| --- | ------ | ----- | ------ | --------------- |
| A1  | 229.9  | 329.6 | 228.0  | 336.2 (1 flaky) |
| A2  | 224.9  | 323.6 | 227.8  | 327.8           |
| A3  | 226.8  | 323.7 | 226.2  | 323.9           |
| B1  | 174.0  | 254.4 | 171.2  | 257.7           |
| B2  | 174.5  | 256.6 | 171.7  | 258.0           |
| B3  | 172.0  | 256.0 | 171.5  | 257.8           |

**Identifying each run's outputs afterwards.** `traces run` does not print store
paths, and re-deriving them would mean re-setting each salt. Simpler and
salt-independent: enumerate `/nix/store/*-vm-test-run-jaunder-e2e-*` (excluding
`-cold`) and read `.stats.startTime` from each combo's Playwright report — the
24 session runs sort cleanly after every older build, and their sequential start
times also prove nothing was served from cache.

Trace extraction (spec AC-6's re-derivability): `capture/otel-traces.jsonl` out
of each `capture-<backend>.tar.gz`, 342 MB across 24 files. Aggregation was
hand-rolled over the JSONL — `traces analyze` reports maxima and averages, not
medians or percentiles, so it cannot produce the spec's p50 metrics.

Full results, including the span-sum decomposition and the navigation warm/cold
table, are in `docs/observability.md` §"#792 — the per-test warmup A/B".

## Second half — post-removal verification (2026-08-05)

**Functional equivalence: established.** `cargo xtask validate` green end to
end, all four combos `expected = 130`, `unexpected = 0`, `flaky = 0`. A serial
`traces run` on the same tree repeated that. Deleting the code path does what
disabling it via the env flag did.

**Timing equivalence: not cleanly established, and deliberately not claimed.**
Two separate contaminations, both caught rather than reported:

1. `validate` builds the four combos **concurrently** (all four started within 8
   s of each other), so its per-combo durations are contention artefacts:
   sqlite-chromium 436 s vs 191 s serial on the same tree. Comparing that
   against arm B's 174 s would have read as a 2.5× regression from a change that
   is a 23 % improvement.
2. The serial confirmation run was taken with `/proc/loadavg` at **8.64** — four
   other agent sessions and a live `cargo`/`rustdoc` were on the box, against
   2.1–2.6 during the A/B. Its numbers (chromium 191 s vs arm B's 174 s, firefox
   260 s vs 256 s, postgres-firefox 354 s vs 258 s) are consistent with the
   verdict but the spread across combos is what host load looks like, not a
   measurement.

The A/B verdict stands on the six interleaved quiescent runs, not on these.
Anyone wanting a clean post-removal number should re-run `traces run` with a
fresh salt on an idle host.

**Task 6e answered — and the prediction was wrong.** The plan expected the
`server-fn-coverage` snapshot to drift, on the theory that the warmup's `/` load
was the orphan bucket's source. It did not drift: the byte-compare passed
unchanged, and the four app-shell orphans are still there. The bucket's source
is the pre-test window itself, so the mechanism keeps its justification and only
the explanation naming warmup needed fixing.

## Task 4 — guard evidence (2026-08-04)

Unit tests: 6/6 pass —
`cargo test --manifest-path xtask/Cargo.toml e2e_scaffold`. (Not `-p xtask`:
root `Cargo.toml:14` has `exclude = ["xtask"]`, so xtask is a separate workspace
and `-p xtask` matches nothing.)

**AC-4 clause 1** — three `cargo xtask check --no-test` runs:

| `flake.nix` state   | exit | step result                                               |
| ------------------- | ---- | --------------------------------------------------------- |
| `e2eSalt = "run1"`  | 1    | `[FAIL] e2e-scaffold` at `flake.nix:898`, naming the salt |
| `e2eWarmup = false` | 1    | `[FAIL] e2e-scaffold` at `flake.nix:902`, naming warmup   |
| both at defaults    | 0    | `[ ok ] e2e-scaffold`                                     |

The two failure details verbatim:

```
[FAIL] e2e-scaffold — flake.nix:898: `e2eSalt` is set — revert it to `""` before
committing. A non-empty salt changes every e2e derivation hash, so CI rebuilds
all four combos from scratch with no cache hit and nothing fails loudly (#792)

[FAIL] e2e-scaffold — flake.nix:902: `e2eWarmup` is not `true` — restore it
before committing. It disables the per-test warmup on all four gate checks,
silently changing what the gate tests (#792)
```

`check --no-test` rather than `validate --no-e2e`: `Command::Validate` runs
`clean_tree_precheck` and returns early on a dirty tree
(`xtask/src/lib.rs:448-454`, `:712-728`), and a salted `flake.nix` **is** a
dirty tree — so `validate` would fail on `clean-tree` without ever reaching the
guard, producing evidence about the wrong thing. `check` has no such precheck.
In CI the tree is clean by construction, so a _committed_ salt is exactly what
the guard sees there.

**AC-4 clause 2 — the guard cannot run inside an e2e derivation, by
construction.** `flake.nix:266-272` excludes `/xtask/` from the source filter,
with a comment saying exactly this (an accidental `cargo xtask` inside a
derivation fails loudly rather than running stale); `:1112` and `:1161` repeat
it for the other filters. The step lives entirely under `xtask/src/**` and no
`testScript` invokes xtask. Corroborating evidence: with `e2eSalt = "probe"` the
salted attrs still **evaluate** to derivations (task 3e's table), so the salted
build path is intact.

No dedicated `nix build` was spent on this clause at the time: it would purchase
no information over the source-filter argument and would compete with task 7's
quiescence budget.

**It was established anyway, by task 7.** Each of the six collection runs set a
non-empty `e2eSalt` and then built and ran all four e2e combos to completion —
that is 24 successful salted `nix build`s of e2e attrs, which is exactly what
AC-4 clause 2 asks for and rather more than the single build it specified. The
guard demonstrably does not reach inside the derivations, because a guard that
did would have made the entire measurement impossible.
