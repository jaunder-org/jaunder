# Testing & Coverage Orchestration Redesign

**Date:** 2026-06-18 (revised 2026-06-19 after Phase 0)
**Status:** Design — approved for planning
**Topic:** Replace the piecemeal `verify`/`check-coverage` shell scripts with one `cargo xtask` driver. The **host runs only the fast inner loop** (static checks + clippy); **all test, coverage, and e2e execution happens in the Nix environment that matches CI exactly**. xtask dispatches to Nix and post-processes the results.

## Problem

The testing/coverage tooling grew piecemeal (`verify`, `check-coverage`, `update-coverage-baseline`, `with-ephemeral-postgres`, `e2e-local.sh`, `format`, plus seed/trace helpers). A recent change took ~an hour just to get coverage and testing green. The wasted time, confirmed by the user, had four sources:

- **(A) Redo across environments** — coverage computed on the host (fast, networked) *and* in Nix (slow, hermetic), only the Nix result authoritative, so host work got redone.
- **(B) Late discovery** — coverage only checked at the commit gate, after the developer had moved on.
- **(C) Hand reconciliation** — the gate dumped a per-file DROP list with no indication of which drops were real.
- **(D) Manual orchestration** — no single command; remember which script and flag, in which order.

**Why this revision exists.** An earlier draft tried to make a *host* coverage run congruent with Nix by denying the host network. **Phase 0** (`docs/superpowers/specs/2026-06-18-phase0-congruence-findings.md`) proved that fragile: the divergence is real and network-driven (Nix covers error-path branches only reached when outbound calls fail), but reproducing Nix's exact network *shape* locally — loopback up, external blocked — needs either root (`ip netns`) or an unprivileged restructure (`unshare -rn` test process + PG outside + unix socket) carrying ongoing fragility. So we pivoted: instead of making a second environment match Nix, **run the one environment that already *is* CI — Nix.** That dissolves sources A and the host↔Nix mismatch by construction.

## Goals

1. **One command — `cargo xtask validate` — that, if green, means you may move on.**
2. **Eliminate repeated work:** a single execution environment (Nix) for all tests/coverage so host and CI can never disagree; Nix's GC-rooted store memoizes the expensive coverage build, so an unchanged re-run reuses the cached closure and only the fast host static checks re-run.
3. **Every command falling-down easy to invoke and observe:** typed results, a JSON sidecar, meaningful exit codes.
   - *Secondary benefit — disk footprint.* Today the host `target/` grows to ~1TB over long sessions (instrumented coverage builds + test/e2e profiles compound). Moving all tests/coverage/e2e into Nix leaves only the `check`/clippy build on the host (one profile, slow growth); the heavy builds land in `/nix/store`, which is GC-bounded — Nix keeps only output closures (not every intermediate), the GC root pins just the *current* checks closure, and `nix-collect-garbage` (or removing `.xtask/gcroots/*`) reclaims superseded builds.
4. **Retire the ad-hoc shell scripts** into one library-backed driver.
5. **Nix stays the determinism guarantor and the sole test/coverage environment.**

## Non-goals / YAGNI

- **No host execution of tests/coverage/e2e** — that is the point; it lives in Nix.
- **No network-denial / host↔Nix congruence machinery** — investigated and rejected (see Phase 0).
- **No local cache push-back** — a GC root covers local persistence; CI is the sole pusher.
- **No new runtime** (no Node/gulp); not `just` (it holds no logic).
- Not folding `seed-e2e-fixtures.sh` / trace / wasm helpers in initially.

## Architecture

Two layers, **one-directional — no recursion**:

- **Host xtask** = the developer-facing orchestrator. `check` does the inner loop directly (static + clippy). `validate` runs `check`, then builds the Nix checks, then post-processes their output.
- **Nix derivations** = the actual test/coverage/e2e execution, running the raw tooling (`cargo-llvm-cov` / `nextest` / the existing checks). **They do not call xtask.**

```
cargo xtask validate (host)
  → static + clippy           (host, Mode::Fix)
  → nix build --out-link <gcroot> --accept-flake-config .#checks…   (raw tools run here)
  ← xtask reads the produced coverage manifest, classifies vs the committed
    baseline, auto-heals, emits JSON
```

xtask never calls xtask. Coverage classification/auto-heal need the git diff and the committed baseline — both **host** artifacts — so they belong host-side as post-processing, which is also *why* no recursion is needed: the heavy logic that might tempt a shared in-Nix call has host-only inputs.

## Key decisions

| Decision | Choice | Rationale |
|---|---|---|
| Orchestration vehicle | `cargo xtask` (standalone `./xtask` workspace, isolated `target/`, excluded from the root workspace) + `xshell` | One hermetic binary; host-side only; no new runtime; `xshell` keeps subprocess orchestration terse. |
| Structure | Library crate + thin `main.rs` | Logic returns typed results; CLI only marshals args + serializes. |
| Command names | `check [--no-test]`, `validate [--no-e2e]` | `check` auto-fixes: static+clippy, plus the Nix coverage check unless `--no-test`. `validate` never mutates: static (verify-only) + coverage, plus the e2e VMs unless `--no-e2e`. No `--full`/`--no-fix` flags. |
| Test/coverage environment | **Nix, sole** | Host never runs tests; eliminates host↔Nix divergence by construction; Phase 0 showed host congruence is mechanically fragile. |
| Coverage authority | Nix produces the manifest; **host post-processes** | Classification/auto-heal/JSON run host-side on the Nix-produced manifest; no congruence mechanism needed. |
| Local memo / persistence | `nix build --out-link <stable>` (a GC root) | The out-link pins the closure so `nix-collect-garbage` can't remove it. **This GC-rooted store _is_ the local memo:** an unchanged `nix build` returns the cached output without re-running, so only the fast host static checks re-run. No separate host-side memo layer — a tree-hash memo would double-count this and is unsound (it can skip when the output was GC'd, or false-skip on unstaged edits). |
| Cache pull | Always pass `--accept-flake-config` | The flake declares `jaunder-org.cachix.org` as `extra-substituter`; the box trusts it but the user is untrusted, so the flag is required to honor it. |
| Cache push | CI pushes **build products only**, excludes test-check outputs | Fast CI compile; tests **always actually run** (no cached green checkmarks — protects against impurity poisoning *and* flaky-pass masking). No local push-back. |
| Output | Human summary default + `.xtask/last-result.json` sidecar always + meaningful exit codes | Exit code = gate signal (run bare under `ctx_execute`); sidecar queried separately with `jq`. |
| Formatting | `check` auto-fixes (`Mode::Fix`); `validate` verifies (`Mode::Check`) | The auto-fix-vs-not split is encoded in `check` vs `validate` (no `--no-fix` flag): iterate with `check` (fixes and proceeds), gate with `validate` (fails on unformatted, never mutates) — which is what CI runs. No standalone `format`. |
| Coverage comparison | Line-identity via the git diff (not file %) | A still-existing previously-covered line going uncovered = `regression`; new uncovered lines = `new_uncovered`; both fail. Requires per-line baseline storage. |
| New uncovered lines | Strict ratchet — fail | New code must be covered. |
| Baseline updates | Auto-heal with notification, narrow | Heals only `improvement`/`structural` (and improved/deleted CRAP); never a `regression`/`new_uncovered`. |
| CRAP metric | Kept, gating | Numeric per-file/function comparison; reported under `.coverage.crap`. |
| CI test de-dup | Drop the `nextest` check; the `coverage` check is the test gate | The instrumented coverage run already executes (and gates) every test the plain nextest check does, plus the PG pass. |

## Command surface

```
cargo xtask check --no-test   # host: static checks + clippy only (fast inner loop)
cargo xtask check             # + Nix coverage check (instrumented tests incl. PostgreSQL + coverage); auto-fixes
cargo xtask validate --no-e2e # static (verify-only) + coverage — the pre-push-style gate
cargo xtask validate          # + e2e-sqlite + e2e-postgres — the full CI-faithful gate
# --json available on every command; .xtask/last-result.json always written; xtask-done: sentinel on stderr
```

`check --no-test ⊂ check ⊂ validate --no-e2e ⊂ validate`. The git pre-push hook (if installed) runs `cargo xtask validate --no-e2e`.

## Result envelope (sketch)

Every command returns a typed result serialized to a flat, `jq`-friendly envelope written to `.xtask/last-result.json`:

```json
{
  "command": "validate",
  "ok": false,
  "duration_ms": 48213,
  "steps": [
    { "name": "fmt", "ok": true, "detail": "reformatted 2 files" },
    { "name": "clippy", "ok": true },
    { "name": "nix-coverage", "ok": false }
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

Field shape is illustrative; the plan finalizes it. The contract: top-level `.ok`, `.steps[]` with `name`/`ok`, `.coverage.regressions[]` / `.coverage.healed` queryable without parsing prose.

## Coverage model

1. **One instrumented run, in Nix.** The Nix `coverage` check runs the suite once under instrumentation (whole workspace vs SQLite, then `jaunder` integration vs the ephemeral PostgreSQL it already provisions) and produces the per-line coverage manifest as its output. `validate` copies that manifest out (as the `coverage-update` package already does) and post-processes it. Tests are never run plain-then-instrumented; the coverage check is also the test gate (the redundant CI `nextest` check is dropped).

2. **The verdict classifies coverage deltas at the line-identity level**, mapped through the git diff — *not* by comparing file percentages, because a percentage drop conflates a real regression with dilution. Four outcomes:
   - `regression` — a previously-covered line that **still exists** is now uncovered. **Fails**; never auto-healed.
   - `new_uncovered` — a newly-added line lacking coverage. **Fails** (strict ratchet); never auto-healed.
   - `structural` — the percentage moved *only* because lines were added/deleted, every surviving previously-covered line is still covered, and no new uncovered line was introduced. Eligible for auto-heal.
   - `improvement` — coverage rose. Eligible for auto-heal.

   No `noise` bucket (line coverage is binary). **CRAP** is the exception — a complexity-weighted score, compared numerically: a worse score fails; a better score or a deleted function heals. Reported under `.coverage.crap`.

3. **Auto-heal with notification — narrow.** `validate` rewrites the committed baseline (`.coverage-manifest.json`, `.crap-manifest.json`) **only** for `improvement`/pure `structural` deltas (and improved/deleted CRAP) and reports it loudly (`coverage.healed: true`). It never heals a `regression`/`new_uncovered`. Because the manifest came from Nix, this update is reproducible by construction — and there is no separate manual regeneration step.

   Line-identity comparison requires the baseline to store **per-line coverage**, not just the per-file percentage stored today — a larger, churnier committed manifest, accepted as the cost of a gate that catches stable-line regressions the percentage would hide and stops false-alarming on dilution.

## Phase 0 (rationale, historical)

Phase 0 ran today's `scripts/check-coverage` networked and (attempted) network-denied against the committed `.coverage-manifest.json` (the Nix baseline). Findings: the networked host diverges on **15 server/web handler files, all lower** than the baseline; the cause is **network access** (Nix covers error branches only reached when outbound calls fail). The unprivileged network-denial mechanisms were fragile (`unshare -rn` maps to root and breaks `initdb`; `unshare -cn` can't raise loopback and breaks in-process `127.0.0.1` mock-server tests; `ip netns` needs root). This evidence drove the pivot to **Nix as the sole test/coverage environment.** The congruence/network-denial machinery is **not built.** Full record: `docs/superpowers/specs/2026-06-18-phase0-congruence-findings.md`.

## Cache strategy

**Why caching helps despite ever-changing local code.** The flake is crane-based: `cargoArtifacts = craneLib.buildDepsOnly` (flake.nix:287) is a **deps-only** derivation reused by every consumer via `inherit cargoArtifacts` — `jaunderBin`, `nextest`, `clippy`, `postgresIntegrationTests`, and `coverage` (which inherits the same non-instrumented deps and only recompiles the *workspace* crates with instrumentation). `cargoArtifacts`'s inputs are `Cargo.lock` + dep sources + toolchain, **not** your code, so it is stable across every local edit and is the bulk of the build. Your workspace crates rebuild locally (your code ≠ CI's), but the expensive dependency closure is pulled from cachix. A deps cache *hit* requires `Cargo.lock` **and** `flake.lock` to match what CI built; bumping a dependency rebuilds deps locally until CI publishes that closure. No manual deps-only split is needed — crane already provides it.

- **Local pull:** every Nix invocation xtask makes passes `--accept-flake-config`, so the flake's `jaunder-org.cachix.org` substituter is honored for the (untrusted) local user. *Optional host-level improvement (one-time, out of repo):* promote `jaunder-org.cachix.org` from `trusted-substituters` to active `substituters` in `/etc/nixos/configuration.nix` (a `sudo nixos-rebuild`) to drop the flag dependency. The key is already trusted.
- **Local persistence:** `validate` builds with `--out-link <stable>` so the closure is GC-rooted and survives `nix-collect-garbage`; with memoization, unchanged `validate` is ~free.
- **CI push:** push **build products only** to cachix — crucially **keep `cargoArtifacts`** (the shared deps closure — the single most valuable cached artifact) and the workspace build outputs, while **excluding** the `coverage`/`nextest`/`e2e`/`postgres-integration` *check* result derivations, so CI always re-runs the tests. Prefer an explicit allowlist (push `cargoArtifacts` + build packages) over a name-based `pushFilter` exclusion, so a renamed check can't accidentally leak a cached green checkmark. No local push-back.

## Migration / retirement

| Retired | Replacement |
|---|---|
| `scripts/verify` | `cargo xtask check [--no-test]` / `validate [--no-e2e]` |
| `scripts/check-coverage` | The Nix `coverage` check (unchanged) + host-side xtask post-processing (classification/auto-heal); `--investigate` → JSON `coverage` data |
| `scripts/update-coverage-baseline` | Folded into `validate`'s host-side auto-heal of the Nix-produced manifest |
| `scripts/with-ephemeral-postgres` | **Retained** for the Nix coverage/integration derivations; **not** used host-side |
| `scripts/e2e-local.sh` | Retired; e2e runs only via the Nix e2e checks (`validate`) |
| `scripts/format` | Absorbed into `validate`'s auto-fix |
| `scripts/seed-e2e-fixtures.sh`, trace/wasm helpers | Left as-is initially |

CI: drop the `nextest` check (`coverage` is the test gate); configure cachix to push build-not-test.

## Documentation debt (in-scope, rewritten during implementation)

Rewritten alongside the code: `CONTRIBUTING.md` (Testing + Coverage sections, the verify ladder, the host-vs-Nix paragraphs — now Nix-only); the project `CLAUDE.md` note referencing `scripts/verify`; the beads pre-push / session-close hook text; relevant auto-memory files (verify ladder, coverage regen, manifest merge-conflict handling).

## Risks

- **Nix build latency for local `validate`.** Mitigated by cachix pull (`--accept-flake-config`) and the GC-rooted Nix store (which memoizes the expensive build so an unchanged re-run only re-runs the fast host static checks); coverage is not the inner loop.
- **cachix push filter correctly excluding test products.** A name-based filter is fragile; verify it excludes every check output and an allowlist may be safer.
- **Per-line baseline churn** — larger committed manifest; accepted for correctness.
- **The Nix coverage manifest's format/extraction** must give xtask per-line data to classify; if only per-file percentages are available, the per-line baseline work includes extracting line data from the Nix LCOV output.
