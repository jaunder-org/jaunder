# Issue #792 — measure the per-test e2e warmup, then decide its fate

Status: draft (first half of a two-half cycle — see "Cycle shape") Issue:
https://github.com/jaunder-org/jaunder/issues/792 Parent: #788 (e2e wall-clock
investigation), lever 2

## Goal

Speed up the e2e suite under real-world conditions without sacrificing its
utility. This cycle's contribution is a **trustworthy measurement** of what the
per-test warmup costs and what removing it buys, plus the tooling that makes
such a measurement repeatable — not the removal itself.

## Background

`warmupPageContext` (`end2end/tests/fixtures.ts:234-261`) runs once per test
from the `_autoPerfSpan` auto fixture (`:590`): it navigates the default page's
context to `${BASE_URL}/` and waits for `body[data-mounted]`, so the test body's
first navigation hits a warm HTTP cache. It is gated on `JAUNDER_E2E_WARMUP`,
set by the build in exactly one place — `flake.nix:949`, on all four gate check
derivations. (`CONTRIBUTING.md:282` and `docs/observability.md:605` also
document setting it by hand, and `fixtures.ts:263-268` exports a
`maybeWarmupPage` wrapper with zero callers — dead since `b6451579`. Both matter
to the second half's deletion footprint, not to this one.)

#788's investigation put per-test time _outside_ the `e2e.test` span at 28–31 %
of the suite, and observed that `navigation.commit_to_mount` is **no faster warm
than cold** (chromium 993 ms warm vs 876 ms cold; firefox 1 618 vs 1 355), which
puts the warmup's premise in question.

### Why #788's numbers cannot decide this

Three independent reasons, all discovered during this cycle's design interview:

1. **Provenance.** They come from CI run 30714621799 — a shared runner under
   unknown load. The effect under test is of the same order as plausible host
   noise.
2. **Staleness.** #791 has since landed: registration is seeded via API rather
   than driven through the UI, removing what was ~35 % of in-span time. Every
   proportion in the #788 write-up was computed against a suite that no longer
   exists.
3. **The proposed experiment is confounded.** #792's text proposes comparing
   `cargo xtask traces run` against `traces run --cold`. But the cold packages
   set `JAUNDER_E2E_WORKERS=1` (`flake.nix:976`) while the warm checks run at 2
   workers (`:949`) in a larger VM (`vmMemory = 3072`). Warm-vs-cold therefore
   conflates warmup with worker count and VM contention, and cannot isolate the
   warmup.

The cold family is not incoherent — `flake.nix:957-965` states its purpose
(per-navigation cold cost, where worker contention would corrupt attribution).
It is simply the wrong instrument for a suite-speed question.

### What #794 already gives us

The lifecycle envelope landed: `_autoPerfSpan` brackets the warmup and emits an
`e2e.warmup` span with its own request/navigation counts (`fixtures.ts:590-593`,
`:939-960`), and `e2e.context_mint` covers context creation. Warmup's direct
cost is therefore readable from a single warm run — no A/B needed for that half.
The A/B is needed only for the counterfactual: what the suite costs _without_
it.

## Cycle shape

The work after the measurement is conditional on its result, so this cycle halts
mid-way:

- **First half (this spec):** the cache-busting salt, its guard, the two-arm
  collection, and a recorded verdict.
- **Checkpoint:** re-plan against observed numbers.
- **Second half (planned later):** act on the verdict — delete, keep, or make
  warmup browser-conditional, with the docs/flake/fixture footprint that
  implies.

`#792` stays open across the checkpoint.

## Decisions

### D1 — Arms differ in exactly one token, at gate-identical settings

- **Arm A (control):** today's gate configuration — warmup on, `RETRIES=1`,
  `WORKERS=2`, `vmMemory = 3072`, `vmCores = 2`.
- **Arm B (treatment):** identical but for the `JAUNDER_E2E_WARMUP=1` token.

Neither arm uses the cold packages, whose worker count differs (see Background).

### D2 — A salt literal in `flake.nix` busts the Nix cache

Nix caches check derivations, so a repeated `traces run` would return a _cached_
result rather than executing the suite — silently handing back traces from
whenever that derivation was last built, possibly on a CI runner. That defeats
the entire point of re-collecting on a quiescent host, and it does so invisibly.

The salt is a **string literal in `flake.nix`**, edited to force a fresh build
and reverted afterwards. Chosen over a CLI/env mechanism because a literal is
visible in `git diff` and `git status`, where an exported env var is invisible
to everyone but the person who set it. It also means **no xtask change at all**
— attr paths are unchanged, so `e2e_attr()` and `traces run` keep working
untouched.

Salting the gate's own check derivations is deliberate: it makes arm A the
gate's actual recipe rather than a clone of it.

### D3 — An empty salt must be a byte-exact no-op

Spliced via `lib.optionalString (e2eSalt != "")`, never as an always-present
empty variable. Otherwise merely adding the wiring rehashes all eight e2e
derivations once, forcing a full local rebuild and discarding the cachix pulls
for no benefit.

### D4 — A gate-side guard blocks committed scaffolding — both literals

A non-empty committed salt fails nothing — CI just misses cache on every e2e job
and rebuilds all four combos from scratch, for as long as it stays in. The
symptom is "CI got slow", which is not a symptom anyone diagnoses promptly.

The same argument applies **more strongly** to D5's `e2eWarmup` literal: a
committed `e2eWarmup = false` would silently disable warmup on all four gate
checks — a real behaviour change to the gate, equally undiagnosable and worse
than a cache miss. The guard therefore covers **both** literals: `e2eSalt` must
be empty and `e2eWarmup` must be `true`.

(The `e2eWarmup` clause is scaffolding for this half only. If the second half
lands a browser-conditional warmup, that clause is what must change —
deliberately and visibly — rather than a knob drifting untracked.)

The guard lives on the **static/lint side**, not in the e2e checks: a local
salted e2e run must still work (it builds the e2e attrs directly), while
`cargo xtask validate --no-e2e` and CI fail loudly on committed scaffolding.

It plugs into the existing grep-style guard-step list —
`xtask/src/lib.rs:455-475` under `validate`, the same list at `:411-430` under
`check`, alongside `steps::proffered_secret_check` and
`steps::no_full_reload_check`. CI reaches it via `cargo xtask validate --no-e2e`
(`.github/workflows/ci.yml:46`).

### D5 — A warmup literal makes the arms symmetrical

Arm B is expressed as a second literal (`e2eWarmup`) rather than by hand-editing
the `warmupEnv` string each run. Six alternating hand-edits across a multi-hour
session is avoidable operator error; two literals flipped together are auditable
in one `git diff`. `e2eWarmup = true` must reproduce today's `warmupEnv` string
byte-exactly (D3's no-op requirement applies to it too).

### D6 — Collection protocol

- **Runs:** 6, interleaved `A1, B1, A2, B2, A3, B3` (n=3 per arm). Interleaving
  catches host drift across the session; n=3 gives a median rather than a point.
- **Distinct salt per run.** Without this, runs 2 and 3 return cached results
  instantly and identically — the salt is what makes n>1 possible at all.
- **Host quiescent**, operationally: no other interactive or batch workload
  started by us for the duration, and `/proc/loadavg` sampled immediately before
  and after each run and recorded in the results table. A run whose before/after
  load materially exceeds the session's baseline is **discarded and re-run**,
  not silently kept. Without this, "quiescent" is an unfalsifiable claim and the
  whole justification for re-measuring evaporates.
- **Runs executed in background** so the session does not contend with them.
- **Invalid runs.** A run that aborts (hard test failure, build error, or the
  load-excursion rule above) is discarded, recorded in the results table with
  its reason, and re-run with a fresh salt. Discards are not silently
  renumbered. If an arm cannot produce 3 valid runs, that inability is itself a
  finding for the verdict comment — not a reason to lower n unannounced.
- **Combos:** `traces run` always builds both backends (`traces/run.rs:72-73`;
  `--browser` restricts only the browser axis), and builds them **serially**, so
  combos do not contend with each other.
- **Deciding data: sqlite only**, both browsers. Postgres traces are collected
  free and retained as evidence for a follow-up, not as a measured arm.

### D7 — Metrics

- **Primary:** per-combo suite duration from the Playwright JSON report's
  `.stats.duration` — not nix build time, which folds in VM boot and store
  extraction. Median of 3 per (arm, browser).
- **Guardrail:** `.stats.flaky` and `.stats.unexpected` from the same object, so
  the primary metric and its guardrail come from one read and cannot disagree
  about which run they describe.
- **How both are obtained.** `traces run` extracts only
  `capture/otel-traces.jsonl` into a `TempDir` it then deletes
  (`traces/run.rs:64-112`); the report stays in the built check's store path
  (`run.rs:125-127`) and nothing in xtask reads a suite-level duration
  (`traces/report.rs` parses per-attempt `results[].duration` only). So
  extraction is a separate operator step per combo:
  `nix build --print-out-paths <attr>`, then jq `.stats` on
  `<out>/playwright-report-<backend>.json`. Because each run carries a distinct
  salt, each run's store path is distinct and **all six runs' reports remain
  readable after the fact** — the extraction is re-derivable, not a one-shot
  capture. The exact commands are recorded with the results (AC-6) so a later
  reviewer can reproduce the medians rather than take them on trust.
- **Secondary:** `e2e.warmup` span p50 and per-combo total (arm A — the cost we
  would stop paying); the lifecycle envelope decomposition (`e2e.context_mint`,
  warmup, fixture setup, teardown/export) in both arms; first-navigation
  `navigation.request` p50 in both arms (the warm-cache benefit the warmup buys,
  predicted ~100–150 ms against a warmup cost of a full mount).

### D8 — Decision rule: per-browser, flakiness vetoes

Applied **per browser**, since `warmupEnv` is already set per combo
(`flake.nix:938-955`) — a browser-conditional warmup is a conditional in that
map, costing ~2 lines of Nix and no fixture complexity.

1. **Flakiness veto.** Aggregation is **the sum of `.stats.flaky` +
   `.stats.unexpected` across that browser's 3 valid runs**, compared arm B vs
   arm A. If arm B's sum exceeds arm A's, warmup is **kept** for that browser
   regardless of speed. A run that is faster because it retried more is not
   faster. Summed rather than medianed deliberately: these are small integers
   usually at 0, where a median would absorb a single new flake in one of three
   runs — the exact signal the veto exists to catch. The rule is conservative on
   purpose, and pinned here so it cannot be chosen at verdict time to suit the
   numbers.
2. **Otherwise speed decides.** Faster median suite duration wins for that
   browser.
3. A split verdict is a legitimate outcome: warmup may end up on for one browser
   and off for the other.

Firefox carries the risk — it is both the slow browser (#788 measured 658 s vs
chromium's 420 s per combo, on the run cited above) and the flaky one
(`flake.nix:944-948`, specifically `:946`, names Firefox 5 s `expect` races as
the reason `RETRIES=1` exists), and `RETRIES=1` _hides_ fail-then-pass in the
exit code. The guardrail reads `results.json`, which records it.

### D9 — No ADR this half

The salt is a mechanism documented inline and in `docs/observability.md`, not an
architectural decision. The ADR candidate is the verdict itself (whether the e2e
suite warms up, and why), which belongs to the second half.

## Non-goals

- Deleting, keeping, or conditionalising the warmup — that is the second half.
- Any change to the cold package family. (Note for the second half: if warmup is
  deleted, the cold family's identity collapses to "workers=1", making its name,
  comment, and `traces run --cold` misleading. Out of scope here.)
- App-side mount cost (#801), worker/VM sizing, or the `RETRIES=1` policy.
- Backend performance comparison, or explaining why Firefox is slower — both are
  answerable from data this cycle collects, and are filed as follow-ups.

## Acceptance criteria

**AC-1 — Salt exists and is threaded.** `flake.nix` declares a top-level
`e2eSalt` string literal defaulting to `""`, reaching every derivation produced
by `mkE2eCombo` — the four warm checks
(`checks.x86_64-linux.e2e-<backend>-<browser>`) and the four cold packages
(`packages.x86_64-linux.e2e-<backend>-<browser>-cold`). There is no
`checks.…-cold`; the eight attrs span both namespaces. _Observable:_ with
`e2eSalt = "probe"`, `nix eval --raw` of `.drvPath` for all eight attrs differs
from its empty-salt value.

**AC-2 — Empty salt is a byte-exact no-op.** With `e2eSalt = ""`, the `drvPath`
of all eight e2e attrs equals the value at `wt-base-issue-792`. _Observable:_
recorded before/after `nix eval --raw … .drvPath` comparison, all eight equal,
**taken with no other tracked change in flight** — `nix eval` hashes
tracked-and-modified files, so an unrelated staged edit invalidates the
comparison. (Untracked files do not affect it.)

**AC-3 — `e2eWarmup` literal reproduces today's config.** With
`e2eWarmup = true`, the `warmupEnv` string on each warm check is byte-identical
to today's, and AC-2's hash equality holds with both literals at their defaults.

**AC-4 — Guard fails committed scaffolding.** The gate exits non-zero with a
message naming the offending literal when `e2eSalt` is non-empty **or**
`e2eWarmup` is not `true`, and passes when both are at their defaults.

_Command caveat:_ plain `cargo xtask validate --no-e2e` cannot demonstrate this
— `Command::Validate` runs `clean_tree_precheck` and returns early on a dirty
tree (`xtask/src/lib.rs:448-454`, `:712-728`), and a salted `flake.nix` _is_ a
dirty tree, so it would fail on `clean-tree` without ever reaching the guard.
Use `cargo xtask validate --no-e2e --allow-dirty` or
`cargo xtask check --no-test` (no clean-tree precheck), and record which. The
criterion is about the guard firing, not about which entry point reaches it —
and in CI, where the tree is clean by construction, a committed salt is exactly
what the guard catches. _Observable, clause 1:_ three runs — salted,
warmup-flipped, and clean — with the stated exit codes and the offending literal
named in each failure message. _Observable, clause 2 (the guard must not reach
inside the e2e derivations):_ with a non-empty salt, `nix build` of one e2e attr
completes successfully. This clause is the one most easily got wrong and needs
its own evidence — a guard wired into the e2e checks would pass clause 1 while
making every salted measurement impossible.

**AC-5 — Salt is documented.** `CONTRIBUTING.md` states what the salt is for,
that it must be reverted before committing, and that the guard enforces this.
(`CONTRIBUTING.md` alone: the salt is build mechanics, and
`docs/observability.md` is about tracing. That file gets the _findings_, per
AC-6 — not the mechanism.)

**AC-6 — Collection executed as specified, and archived where findings live.**
Six valid runs in the order `A1, B1, A2, B2, A3, B3`, each with a distinct salt,
on a host quiescent per D6's operational definition. _Observable:_ a findings
section in `docs/observability.md`, following the existing convention there
(e.g. `:414` "#155 — post-CSR Firefox e2e tax (findings, 2026-07-02)", `:461`,
`:506`), carrying a per-run table with: run label, arm, salt, the two literals'
values, `/proc/loadavg` before and after, `.stats` (duration, flaky, unexpected)
per combo, and the exact extraction commands used. Discarded runs appear in the
table with their reason (D6), so the record shows what was thrown away rather
than only what was kept.

**AC-7 — Verdict recorded on #792.** A comment carrying: median per-combo suite
duration per (arm, browser) for sqlite; retry/flaky counts per combo; arm A's
`e2e.warmup` cost; the envelope decomposition for both arms; first-navigation
`navigation.request` p50 for both arms; and an explicit per-browser
delete/keep/conditional recommendation derived from D8's rule.

**AC-8 — Follow-ups filed** (from the plan's first task, so they can be picked
up concurrently):

- sqlite-vs-postgres e2e performance comparison, citing the postgres traces this
  cycle collects;
- why Firefox is slower, sized from #794's per-phase boot breakdown across the
  four collected populations (noting Firefox reports no long-tasks);
- the remainder of the per-test envelope — `e2e.context_mint`, fixture
  setup/teardown, span export — re-baselined post-#791.

**AC-9 — Rule applied, not improvised.** The verdict comment states D8's rule
and shows the numbers it was applied to, including the flakiness veto check per
browser.

## Risks

- **A stale-cached arm A** — the failure this spec exists to prevent; AC-1/AC-6
  are its mitigation. If the salt were ineffective, arm A could silently be a CI
  runner's numbers compared against a fresh local arm B.
- **Session length.** 6 runs × 4 serial combos is likely multiple hours of
  quiescence. Mitigation: background execution, and the interleaved order means
  a truncated session still yields complete matched pairs. Bounded by a
  favourable fact, verified during the spec review: `flake.nix` is **outside the
  crane source filter**, so editing the salt leaves
  `packages.x86_64-linux.jaunder.drvPath` unchanged. A salted run re-runs the VM
  suite only — it does not rebuild the Rust workspace.
- **The measurement may say "keep".** That is a legitimate outcome; the second
  half then documents why the warmup earns its place, and #792 closes without a
  speedup.
