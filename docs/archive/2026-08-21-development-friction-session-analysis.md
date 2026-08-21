# Development Friction From OMP Session Transcripts

## Scope

Analyzed OMP session artifacts matching:

```text
/home/mdorman/.omp/agent/sessions/-src-jaunder*
```

Corpus inventory from the filesystem:

- 9 matching session directories.
- 1,848 files.
- 521 JSONL transcripts.
- 781 `*.log` files, including bash, shake, GitHub, and tool-output logs.
- 443 bash logs.
- About 1.05 GB total session artifacts.

Method:

- Parsed session JSONL for bash tool calls, command strings, exit status, wall time, and output snippets.
- Parsed all `*.log` files for `xtask-done:` sentinels and `[ ok ]` / `[FAIL]` step lines.
- Used the sentinels as the reliable source for completed `xtask` gates because they carry command, success, exit code, and duration.
- Read representative source logs for failure interpretation.
- The companion script
  `docs/archive/2026-08-21-development-friction-session-analysis.mjs`
  reproduces the corpus inventory and `xtask-done:` gate summaries from the live
  session logs.

Caveats:

- Some commands ran through background wrappers, CI logs, or copied GitHub logs; not every command has a normalized wall-time field.
- Some logs contain aggregate CI output for several jobs; those are counted per `xtask-done:` marker when present.
- The analysis captures what agents actually ran, not every possible local command.

## Executive summary

Most measured development time is spent on **green gates**, not on finding defects.

Across 125 `xtask-done:` gate records in logs:

- Total measured gate time: about **3.65 hours**.
- Green gate time: about **2.44 hours** (**66.8%**).
- Failed gate time: about **1.21 hours**.
- Failures were sparse: **12 failed gate records**.

The dominant friction pattern is repeated `cargo xtask check` / `cargo xtask validate --no-e2e` / hook runs that usually pass. The failures that do occur are often cheap/static problems surfaced after an expensive aggregate command has already done a lot of work.

## Gate-level data

`xtask-done:` records found in all matching logs:

| Command | Runs | Failures | Median | P90 | Max | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `check` | 70 | 1 | 41.870s | 120.120s | 366.075s | 71.005m |
| `validate --no-e2e` | 39 | 3 | 92.678s | 226.191s | 1,017.614s | 95.734m |
| `precommit` | 7 | 0 | 74.607s | 123.927s | 123.927s | 9.497m |
| `validate` | 1 | 1 | 0.008s | 0.008s | 0.008s | 0.008s |
| `e2e-local` | 2 | 2 | 286.309s | 286.309s | 286.309s | 5.687m |
| `e2e-postgres-firefox` | 2 | 2 | 672.218s | 672.218s | 672.218s | 17.769m |
| `e2e-sqlite-firefox` | 1 | 1 | 381.572s | 381.572s | 381.572s | 6.359m |
| `e2e-postgres-chromium` | 1 | 1 | 382.845s | 382.845s | 382.845s | 6.381m |
| `e2e-sqlite-chromium` | 1 | 1 | 379.434s | 379.434s | 379.434s | 6.324m |
| `coverage-probe-source` | 1 | 0 | 18.314s | 18.314s | 18.314s | 18.314s |

Interpretation:

- `check` is high-frequency and usually green: 69/70 successful in the parsed gate logs.
- `validate --no-e2e` is lower-frequency but more expensive: median about 93s, max about 17m.
- E2E failures are few in count but large in wall-clock cost.

## What failures actually found

Failed `xtask` gate records with source evidence:

| Category | Evidence | Cost |
| --- | --- | ---: |
| Formatting | `prettier` failed on `.agent-shell/transcripts/2026-08-14-08-36-55.md`; source: `/home/mdorman/.omp/agent/sessions/-src-jaunder-jaunder/2026-08-14T12-36-59-977Z_01a00046-91c9-7000-b5b5-216f66cc7cee/108.bash-original.log:9-12,70-72`. | 68.046s |
| Doc-link/static | `doc-links` failed on generated skill links; source: `/home/mdorman/.omp/agent/sessions/-src-jaunder/2026-08-03T20-17-42-661Z_019fc946-6905-7000-9097-c5cfcac4be20/456.bash.log:2895-2936`. | 1,017.614s |
| Dirty tree | `clean-tree` failed immediately with modified/untracked docs/spec files; source: `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-1/2026-08-13T01-17-37-590Z_019ff8b2-39b6-7000-9170-110f257151ed/372.shake.log`. | 0.008s |
| Nix/test/doctest | `check` failed on `nix-coverage`, `coverage`, `nix-doctests`, and `doctests`; failure named duplicate SQLite media test and fixed-output derivation hash mismatch; source: `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-1/2026-08-13T01-17-37-590Z_019ff8b2-39b6-7000-9170-110f257151ed/285.shake.log`. | 46.181s |
| Clippy + coverage/CRAP | CI `validate --no-e2e` failed with `[FAIL] clippy` and `[FAIL] coverage — 15014 uncovered line(s), 126 CRAP over threshold`; source: `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-4/2026-08-13T12-50-30-107Z_019ffb2c-925b-7000-a90f-02724506b825/1282.github.log`. | 677.395s |
| Local e2e | `e2e-local-playwright` failed; sources: `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-4/2026-08-13T12-50-30-107Z_019ffb2c-925b-7000-a90f-02724506b825/1121.shake.log` and `1123.shake.log`. | 286.309s, 54.926s |
| CI e2e matrix | Nix e2e jobs failed for postgres/firefox, sqlite/firefox, postgres/chromium, sqlite/chromium; source: `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-4/2026-08-13T12-50-30-107Z_019ffb2c-925b-7000-a90f-02724506b825/1334.github.log`. | 379-394s per leg |
| CI e2e single leg | `nix-e2e-postgres-firefox` failed; source: `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-6/2026-08-13T14-51-23-296Z_019ffb9b-3f20-7000-b3a6-06f0ab058079/285.read.log`. | 672.218s |

Step-level result counts from 114-125 gate logs are striking: every recurring `xtask` step except `prettier` and `doc-links` was green in the parsed sentinel-bearing logs. Examples:

- `clippy`: 114 ok, 0 failed in local `xtask` step logs.
- `wasm-clippy`: 114 ok, 0 failed in local `xtask` step logs.
- `nix-coverage`: 107 ok, 0 failed in the step-count sample, with one separate `check` failure in a `shake.log` outside the bash-only subset.
- `nix-doctests` / `nix-doctests-gate`: 107 ok, 0 failed in the step-count sample, with one separate `check` failure in the `shake.log` cited above.
- `prettier`: 113 ok, 1 failed.
- `doc-links`: 113 ok, 1 failed.

This does **not** mean clippy/test failures never happen. Direct targeted command parsing found compile/clippy/static failures and many targeted `nextest` failures. It means the expensive full gates usually did not discover them; they were more often found during focused work or CI.

## Where time is going

### 1. Repeated green `check` runs

`cargo xtask check` dominates count: 70 completed sentinels, only one failure. It consumed about 71 minutes of observed gate time.

Representative green `check` durations:

- 24.535s: `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-1/2026-08-13T01-17-37-590Z_019ff8b2-39b6-7000-9170-110f257151ed/179.bash-original.log`.
- 128.017s: `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-2/2026-08-20T11-07-30-891Z_01a01eda-cccb-7000-98bb-9bde71097251/281.bash-original.log`.
- 135.625s: `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-2/2026-08-20T11-07-30-891Z_01a01eda-cccb-7000-98bb-9bde71097251/299.bash-original.log`.
- 366.075s: `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-3/2026-08-14T20-41-26-380Z_01a00202-166c-7000-a43c-0d08f71379e0/392.bash-original.log`.

Friction: many `check` runs were pre-commit gate reruns. That means every commit paid the full check surface, even for low-risk changes.

### 2. `validate --no-e2e` as pre-push/CI gate in the older session corpus

`validate --no-e2e` consumed about 96 minutes. Median was about 93s; P90 about 226s. Failures were rare but expensive when they happened.

Representative examples:

- Green local pre-push: 113.142s at `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-1/2026-08-13T01-17-37-590Z_019ff8b2-39b6-7000-9170-110f257151ed/184.bash-original.log`.
- Formatting failure after 68.046s at `/home/mdorman/.omp/agent/sessions/-src-jaunder-jaunder/2026-08-14T12-36-59-977Z_01a00046-91c9-7000-b5b5-216f66cc7cee/108.bash-original.log`.
- CI doc-link failure after 1,017.614s at `/home/mdorman/.omp/agent/sessions/-src-jaunder/2026-08-03T20-17-42-661Z_019fc946-6905-7000-9097-c5cfcac4be20/456.bash.log`.

Friction: pre-push repeats many checks that were just run at commit time, then adds Nix/coverage/doctest work. The expensive run sometimes reports a cheap issue.

### 3. E2E failures are high-cost and late

E2E has fewer records but large failure costs:

- Local Playwright failures: 54.926s and 286.309s.
- CI matrix failures: about 379-394s per leg in one matrix log.
- One postgres/firefox Nix e2e leg: 672.218s.

Friction: e2e is valuable for browser/user-flow behavior, but it is too expensive to run speculatively or broadly during routine implementation.

### 4. Targeted tests find more failures than full gates

JSONL parsing found many direct `cargo nextest` failures: 144 failed nextest-like command results among 498 direct nextest-like runs. These are likely a mix of real red/green TDD failures, wrong filters, and focused debugging.

Friction interpretation:

- Targeted test failures are expected and useful during implementation.
- Full green gates after targeted work are mostly confirmation, not discovery.
- The process should preserve targeted failures while reducing repeated full green gate cost.

### 5. Hook staging and dirty-tree checks are useful when first

The dirty-tree `validate` failure took 8ms. That is the best kind of failure: cheap, deterministic, before expensive work.

By contrast, the pre-push prettier failure showed a cheap formatting problem while still paying a 68s command. The log shows `[FAIL] prettier` at lines 9-12 and the final failure only at lines 70-72 in `/home/mdorman/.omp/agent/sessions/-src-jaunder-jaunder/2026-08-14T12-36-59-977Z_01a00046-91c9-7000-b5b5-216f66cc7cee/108.bash-original.log`.

Friction: cheap failure checks should be explicit preflights or should short-circuit local hook commands.

## Likely optimization targets

### A. Make local hooks fast and explicit

Current evidence supports the direction of splitting hook policy from full validation:

- Commit hook should stay fast and local.
- Push hook should run clean-tree + fast local verification, not hermetic Nix validation, if the goal is low-friction push.
- `validate --no-e2e` should remain an explicit CI/static confidence command, not an automatic every-push tax.

Risk: a fast `prepush` that only runs host verify + local product tests does not test the same things as `validate --no-e2e`; it skips hermetic coverage, wasm browser tests, and doctests unless local equivalents are added.

### B. Add local equivalents before claiming parity

If the policy goal is "same tests, different environment", then local lanes are still missing for:

- doctests;
- wasm browser tests;
- coverage/CRAP instrumentation, if coverage is still expected before PR.

Without those, `prepush` is a faster but weaker gate. That may be acceptable for hooks, but it should be named and documented as such.

### C. Short-circuit local aggregate gates on cheap failures

Local commands should fail fast on:

- clean-tree;
- formatters;
- doc-links / ADR formatting;
- clippy/static compile failures.

The observed 68s prettier pre-push failure is the canonical waste case.

### D. Stop running full gates before likely review churn

Earlier session content described agents running expensive gates before review results were available. The transcript corpus backs the cost side: full `check`/`validate` is frequently green but expensive. Gate sequencing should be:

1. targeted tests during implementation;
2. cheap static/format preflight;
3. review if review is expected to change code;
4. full gate once after review churn is incorporated.

### E. Cache or receipt exact-tree gate results

Many commit/push cycles run full checks on nearly identical trees. A durable receipt keyed by tree hash could let a hook skip a previously completed gate for the exact tree.

Useful receipts:

- host static/check receipt;
- product test receipt;
- validate/no-e2e receipt if still used locally.

The receipt must include command version/config inputs, not just `HEAD`, or it can certify stale policy.

### F. Make change-class-aware gates boring

Several long checks appear around docs, plans, generated skills, and process-doc changes. Change-class routing could reduce cost:

- docs-only: markdown formatting, doc-links, ADR gates if relevant;
- xtask/tools-only: xtask/tools tests + formatting/clippy for those workspaces;
- product Rust: targeted nextest + static gate;
- web/e2e-affecting: targeted Playwright/e2e-local.

This should be a small policy table inside `xtask`, not ad hoc agent judgement.

### G. Preserve targeted tests; reduce broad confirmation loops

Direct targeted nextest runs found many failures. That is productive friction. Broad green gates are the larger waste.

Process target:

- Encourage failing fast with targeted `cargo nextest run ...` during edits.
- Run broad gate once at the boundary where it has authority.
- Avoid running broad gate again in the hook when the exact tree has already been checked.

## Recommended next experiments

1. **Instrument xtask step durations.** `xtask-done` gives total duration; step-level durations would show whether the tail cost is Nix, tests, clippy, or packaging for each run.
2. **Add `cargo xtask prepush` as a fast hook lane, but document it as weaker than `validate --no-e2e` until local doctest/wasm/coverage replacements exist.**
3. **Add fail-fast mode for local hook commands.** Stop after first failed cheap step; keep CI exhaustive if exhaustive logs are needed.
4. **Prototype exact-tree receipts for precommit/prepush.** A hook should not rerun the same successful command on the same tree unless command policy changed.
5. **Create local doctest lane if doctests are required in the fast push hook.** Otherwise pushing will intentionally skip them locally and rely on CI/full validation.
6. **Classify changes before gates.** Start with conservative docs-only and xtask/tools-only routing; fall back to full gate on uncertainty.

## Reproduction

Run:

```bash
node docs/archive/2026-08-21-development-friction-session-analysis.mjs
```

By default it reads `/home/mdorman/.omp/agent/sessions` and includes only
session directories whose names start with `-src-jaunder`. Pass a different
session root as the first argument to analyze another exported corpus.

## Source appendix

Representative source files read or parsed:

- `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-1/2026-08-13T01-17-37-590Z_019ff8b2-39b6-7000-9170-110f257151ed/179.bash-original.log`
- `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-1/2026-08-13T01-17-37-590Z_019ff8b2-39b6-7000-9170-110f257151ed/184.bash-original.log`
- `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-1/2026-08-13T01-17-37-590Z_019ff8b2-39b6-7000-9170-110f257151ed/285.shake.log`
- `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-1/2026-08-13T01-17-37-590Z_019ff8b2-39b6-7000-9170-110f257151ed/372.shake.log`
- `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-2/2026-08-20T11-07-30-891Z_01a01eda-cccb-7000-98bb-9bde71097251/*.bash-original.log`
- `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-3/2026-08-14T20-41-26-380Z_01a00202-166c-7000-a43c-0d08f71379e0/392.bash-original.log`
- `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-4/2026-08-13T12-50-30-107Z_019ffb2c-925b-7000-a90f-02724506b825/1121.shake.log`
- `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-4/2026-08-13T12-50-30-107Z_019ffb2c-925b-7000-a90f-02724506b825/1123.shake.log`
- `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-4/2026-08-13T12-50-30-107Z_019ffb2c-925b-7000-a90f-02724506b825/1282.github.log`
- `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-4/2026-08-13T12-50-30-107Z_019ffb2c-925b-7000-a90f-02724506b825/1334.github.log`
- `/home/mdorman/.omp/agent/sessions/-src-jaunder-agent-6/2026-08-13T14-51-23-296Z_019ffb9b-3f20-7000-b3a6-06f0ab058079/285.read.log`
- `/home/mdorman/.omp/agent/sessions/-src-jaunder-jaunder/2026-08-14T12-36-59-977Z_01a00046-91c9-7000-b5b5-216f66cc7cee/108.bash-original.log`
- `/home/mdorman/.omp/agent/sessions/-src-jaunder/2026-08-03T20-17-42-661Z_019fc946-6905-7000-9097-c5cfcac4be20/456.bash.log`
