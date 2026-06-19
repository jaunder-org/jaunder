# Testing & Coverage Orchestration Redesign

**Date:** 2026-06-18
**Status:** Design — approved for planning
**Topic:** Consolidate the piecemeal test/coverage shell scripts into a single, memoized, JSON-observable `cargo xtask` driver, and make host coverage congruent with Nix so the baseline stops being a source of repeated work.

## Problem

The testing and coverage tooling grew piecemeal: `scripts/verify`, `scripts/check-coverage`, `scripts/update-coverage-baseline`, `scripts/with-ephemeral-postgres`, `scripts/e2e-local.sh`, `scripts/format`, plus seed/trace helpers. A recent change took roughly an hour just to get coverage and testing green. The wasted time decomposed into four sources, all confirmed by the user:

- **(A) Redo across environments.** Coverage is computed on the host (fast, networked) *and* in Nix (slow, hermetic), and only the Nix result is authoritative. Work done on the host frequently has to be redone in Nix.
- **(B) Late discovery.** Coverage is only checked at the commit gate, so a regression surfaces after the developer has mentally moved on from the code that caused it.
- **(C) Hand reconciliation.** The gate dumps a per-file DROP list with no indication of which drops are real gaps, which are deleted/moved code, and which are baseline staleness. This is sorted out manually every time.
- **(D) Manual orchestration.** No single command to "make this correct"; the developer remembers which script and flag to run in which order.

The single root cause behind A, the host↔Nix mismatch, and most of the slowness is that **coverage is computed in two environments that disagree on exactly two files**. Per `scripts/check-coverage` and `CONTRIBUTING.md`, the *only* documented divergence is network access: the Nix coverage sandbox has no network, so `common/src/websub/http.rs` and `server/src/commands.rs` report lower coverage there than on a networked host. That one fact is the entire reason the baseline can only be regenerated from Nix.

## Goals

1. **One command that, if green, means you may move on.** `cargo xtask validate` is the hub of the workflow, designed so its success is a sufficient proof of correctness.
2. **Eliminate repeated work.** Memoize against tree state; make host coverage congruent with Nix so a host run is never redone; auto-heal the baseline so reconciliation stops being manual.
3. **Make every command falling-down easy to invoke and observe** — typed results, a stable JSON envelope, a queryable sidecar, and meaningful exit codes.
4. **Retire the ad-hoc shell scripts** in favor of one library-backed driver shared by host and CI.
5. **Preserve Nix as the determinism guarantor.** Congruence means the host *conforms to* Nix; it never weakens Nix's hermeticity.

## Non-goals / YAGNI

- Not folding `seed-e2e-fixtures.sh` or the trace/wasm helpers in initially — only if it later pays off.
- Not introducing a new runtime (no Node/gulp). Not adopting `just` (it holds no logic; it would only relocate the scripts).
- Not changing *what* is tested (backend parity, the suites, the metrics) — only how it is orchestrated, computed congruently, observed, and memoized.
- Not per-crate incremental coverage gating (unsound — see Memoization).

## Key decisions

| Decision | Choice | Rationale |
|---|---|---|
| Orchestration vehicle | `cargo xtask` (own `./xtask` workspace, isolated `target/`, excluded from main `default-members`) + `xshell` | One hermetic binary Nix already builds; same logic host and CI; `xshell`'s `cmd!` keeps subprocess orchestration as terse as shell with real errors. No new runtime. |
| Structure | Library crate + thin `main.rs` | All logic returns typed results; CLI only marshals args and serializes. Breaking out granular subcommands later is free. |
| Command names | `check`, `validate`, `validate --full` | Replaces `verify --fast` / `verify` / `verify --full`. `check` = tight loop; `validate` = the hub; `--full` = adds the Nix VM tier. |
| Output | Concise human summary by default + always-write JSON sidecar `.xtask/last-result.json` (gitignored) + meaningful exit codes | Exit code is the gate signal (run bare under `ctx_execute`, `isError` on failure); the sidecar is queried separately with `jq`, so raw output never enters an agent's context. Both render from one typed struct. |
| Formatting | Auto-fix, not gate | `validate` applies formatting (`cargo fmt`, `prettier -w .`) and reports "reformatted N files" rather than failing. Library carries `Mode::{Fix, Check}`; host uses `Fix`, CI uses `Check` (CI must never mutate the tree). No standalone `format` command. |
| Avoid re-running | Memoize `validate` against a whole-tree input hash | Records the input key of the last **green** run; re-invoking on an unchanged tree short-circuits near-instantly, skipping even test execution (which cargo/nextest do not skip). |
| Host↔Nix coverage | Make host congruent by denying network to the host coverage pass | Neutralizes the single divergence variable, so host ≡ Nix by construction. Kills redo (A) and the "baseline only regenerable in Nix" tax. |
| Coverage authority | Nix remains the guarantor; host conforms | Nix CI re-runs the coverage check and that is what truly gates. Congruence means it agrees with the host by construction rather than catching the developer out. |
| CRAP metric | Kept, and gating | Catches under-tested *complex* code that line % misses; recently re-baselined. Reported in the JSON verdict alongside line coverage. |
| Coverage comparison | Line-identity, mapped through the git diff (not file percentages) | A percentage drop conflates a true regression with dilution. A still-existing previously-covered line going uncovered is a `regression`; new uncovered lines `new_uncovered`; both **fail**. Requires the baseline to store per-line coverage. |
| New uncovered lines | Strict ratchet — **fail** | New code must be covered; `new_uncovered` fails `validate` and is never healed. |
| Baseline updates | Auto-heal **with notification**, narrow | Heals **only** `improvement` and pure `structural` deltas (and improved/deleted CRAP); **never** a `regression` or `new_uncovered`. Says loudly when it does. CI's Nix re-check reproduces the numbers by construction. |

## Command surface

```
cargo xtask check          # tight loop: static checks + clippy (was `verify --fast`)
cargo xtask validate       # the hub: check + tests + coverage + e2e, no VM (was `verify`)
cargo xtask validate --full # adds the hermetic Nix VM checks (was `verify --full`)

cargo xtask e2e [--vm]
# coverage is a byproduct of `validate`; no standalone coverage/format/baseline commands
# `--json` available on every command; the sidecar is always written regardless
```

`check ⊂ validate ⊂ validate --full`. The git pre-push hook becomes the one-liner `cargo xtask validate`.

## Result envelope (sketch)

Every command returns a typed result serialized to a flat, `jq`-friendly envelope written to `.xtask/last-result.json`:

```json
{
  "command": "validate",
  "ok": false,
  "duration_ms": 48213,
  "memoized": false,
  "steps": [
    { "name": "static", "ok": true, "skipped": false },
    { "name": "clippy", "ok": true, "skipped": false },
    { "name": "format", "ok": true, "detail": "reformatted 2 files" },
    { "name": "tests", "ok": true, "detail": "691 passed" },
    { "name": "coverage", "ok": false },
    { "name": "e2e", "ok": true }
  ],
  "coverage": {
    "regressions": [ { "file": "server/src/feed/worker.rs", "lines": [142, 147] } ],
    "new_uncovered": [ { "file": "server/src/feed/worker.rs", "lines": [210, 211, 212] } ],
    "structural": [ { "file": "server/src/media.rs", "reason": "covered lines deleted" } ],
    "improvements": [],
    "crap": { "regressions": [] },
    "healed": false
  }
}
```

Field shape is illustrative; the implementation plan finalizes it. The contract is: top-level `.ok`, `.steps[]` with `name`/`ok`, and `.coverage.regressions[]` / `.coverage.healed` queryable without parsing prose.

## Coverage model

The redesign treats coverage as a self-describing byproduct of the one test run, not a separate phase.

1. **One instrumented run yields both signals.** `validate` runs the suite once under instrumentation, in the network-denied (Nix-congruent) environment. That run produces the test pass/fail verdict *and* per-file coverage. Tests are never run plain-then-instrumented. (This is the one good property of today's `check-coverage`, which already doubles as the test gate.) The two-pass accumulation — whole workspace vs SQLite, then `jaunder` integration vs throwaway host Postgres over a unix socket — is preserved so `storage/src/postgres/*` is really instrumented.

2. **The verdict classifies coverage deltas at the line-identity level**, mapped through the git diff — *not* by comparing file percentages, because a percentage drop conflates a true regression with mere dilution. For every line the only question is whether a line that existed before and was covered, and still exists, lost its coverage. Four outcomes:
   - `regression` — a previously-covered line that **still exists** is now uncovered. **Fails** `validate`; **never** auto-healed. One such line fails the gate regardless of the file percentage.
   - `new_uncovered` — a newly-added line that lacks coverage. **Fails** `validate` (strict ratchet: new code must be covered). Never auto-healed (silently accepting untested new code is the erosion we are preventing).
   - `structural` — the percentage moved *only* because lines were added or deleted, **every surviving previously-covered line is still covered**, and no new uncovered line was introduced (e.g. covered code deleted or moved). Safe; eligible for auto-heal.
   - `improvement` — coverage rose. Eligible for auto-heal (ratchets the baseline up).

   There is no `noise` bucket: line coverage is binary, so float wobble does not arise. **CRAP is the exception** — it is a complexity-weighted *score*, not a line-binary, so it keeps a numeric per-file/per-function comparison: a worse CRAP score fails; a better score or a deleted function heals. Reported under `.coverage.crap`.

3. **Auto-heal with notification — narrow by design.** `validate` rewrites the committed baseline (`.coverage-manifest.json`, `.crap-manifest.json`) in place **only** for `improvement` and pure `structural` deltas (and improved/deleted CRAP), and reports it loudly (`coverage.healed: true`). It **never** heals a `regression` or `new_uncovered`. No separate manual Nix regeneration; CI's Nix re-check reproduces the numbers by construction.

   Line-identity comparison requires the baseline to store **per-line coverage** (which lines are covered), not just the per-file percentage stored today. This makes the committed manifest larger and churnier, but it is what makes the gate both correct (catches a stable-line regression the percentage would hide) and quieter (stops false-alarming on pure dilution/deletion).

## Congruence contract & Phase 0

Nix derivations stop calling shell scripts and call `cargo xtask … --mode check` (non-mutating). CI and host run the same library code; the only differences are `Mode::Check` vs `Mode::Fix` and the sandbox wrapper. Nix remains the determinism guarantor.

The load-bearing mechanism is **how the host run denies network** to reach congruence: Nix gets it free (no network in the sandbox); on the host, xtask wraps the coverage pass in a network-denied namespace (candidate: `unshare -rn` with `lo` brought up; ephemeral Postgres over a unix socket so it needs no network). This is the single biggest implementation risk and is proven first:

> **Phase 0 — prove congruence.** Run today's `scripts/check-coverage` on the host (a) normally and (b) network-denied, and diff both against the committed `.coverage-manifest.json` (which *is* the Nix output). **Expected:** the networked run diverges only on `common/src/websub/http.rs` and `server/src/commands.rs`; the network-denied run matches the baseline exactly. If it holds, the "host ≡ Nix" foundation is real and the rest is built on it. If extra files diverge, the true divergence set is learned before committing to the design.

## Migration / retirement

Incremental; an old script is deleted only once its `xtask` subcommand reproduces it. At every step the tree stays green via the existing suite.

| Retired | Replacement |
|---|---|
| `scripts/verify` | `cargo xtask check` / `validate` / `validate --full` |
| `scripts/check-coverage`, `scripts/update-coverage-baseline` | Folded into `validate` (coverage byproduct, baseline auto-heals); `--investigate` becomes the JSON `coverage` data |
| `scripts/with-ephemeral-postgres` | `xtask` library helper, reused by the coverage pass |
| `scripts/e2e-local.sh` | `validate`'s e2e step (library fn) |
| `scripts/format` | Absorbed into `validate`'s auto-fix; no standalone command |
| `scripts/seed-e2e-fixtures.sh`, trace/wasm helpers | Left as-is initially; fold in later only if it pays off |

## Documentation debt (in-scope, rewritten during implementation)

Rewritten alongside the code so docs and behavior land together and stay truthful — **not** in this spec:

- `CONTRIBUTING.md` — Testing and Coverage & dependency policy sections (the verify ladder, `check-coverage` two-pass description, the "baseline only from Nix" paragraph).
- Project `CLAUDE.md` — the context-mode note referencing `scripts/verify`.
- The beads pre-push / session-close hook text that names `scripts/verify` and `scripts/check-coverage`.
- Relevant auto-memory files describing the verify ladder / coverage regen / merge-conflict handling of the manifests.

## Risks

- **Network denial doesn't fully reproduce Nix** (extra divergent files). Mitigated by Phase 0 before any build.
- **`unshare -rn` unavailable/insufficient** in some host configs. Phase 0 validates the exact mechanism; fallback approaches (alternative sandboxing, or per-test network suppression) are evaluated only if needed.
- **Memoization unsoundness** if the input hash misses an outcome-affecting input. Mitigated by hashing the whole tracked tree + lockfile + toolchain id (conservative), never per-crate.
- **xtask compile tax** on first/changed runs. Mitigated by keeping its dependency set light and isolating its workspace/`target/`.
- **Auto-heal mutating a committed file** as a side effect. Mitigated by the loud notification and by it firing only for `improvement`/`structural` deltas — never a `regression` or `new_uncovered`.
- **Per-line baseline is larger and churnier** than today's per-file percentages, producing more diff noise on the committed manifest. Accepted as the cost of correct (regression-catching) and quiet (no dilution false-alarms) gating; the format is chosen to minimize churn where possible.
```
