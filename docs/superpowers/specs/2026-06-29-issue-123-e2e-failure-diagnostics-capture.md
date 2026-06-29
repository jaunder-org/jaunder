# Spec — persist all e2e VM diagnostics on failure (#123 + #49)

**Issues:** [#123](https://github.com/jaunder-org/jaunder/issues/123) — *Playwright failure output not captured in CI logs or validate-diagnostics artifact* (Bug) — and [#49](https://github.com/jaunder-org/jaunder/issues/49) — *VM persists no Playwright/app/serial artifacts on failure (only OTel, only on success)* (Task). Both milestone: E2E test suite. #123 ⊂ #49; this cycle closes both.

## Problem

When a Nix e2e VM check fails, **no recoverable artifacts survive**, so a CI-only red e2e is undiagnosable without local reproduction:

- The in-VM Playwright run is wrapped in `machine.succeed("…playwright test…")` (flake.nix, both `mkE2eSqliteCheck` ~:629 and `mkE2ePostgresCheck` ~:768). On a non-zero exit it **raises immediately**, aborting every step below it.
- All artifact copies — OTel trace, the `playwright-report-<backend>.json` (#152), and the app journal (the `e2ePanicGate`'s `copy_from_vm`, flake.nix ~:542–545) — run **after** that `machine.succeed`, i.e. only on the **success** path. On a Playwright failure they never execute.
- The Playwright config (`nixPlaywrightConfig`, flake.nix ~:430) sets no `trace`/`screenshot`/`outputDir`, so **no trace zip or screenshot is generated inside the VM** in the first place.
- A failed `nixosTest` derivation produces no `$out`, so xtask's `copy_e2e_diagnostics_between` (reads the out-link) gets nothing. `rescue_diagnostics` exists for the no-`$out` case but only pulls the **coverage** check's `emit-out/diagnostics` bundle — not e2e VM artifacts.

Net: on a red e2e you learn only *that* a spec failed, never *why* — no failing-test name guaranteed in any retained log, no trace, no screenshot, no app/system journal.

## Goal

On a failing e2e combo, durably capture into `.xtask/diagnostics/<check>/` (the always-uploaded `validate-diagnostics` CI artifact):

- the Playwright `line`-reporter output (failing test + assertion),
- the Playwright JSON report,
- the Playwright trace + screenshots (failed tests only),
- the jaunder app journal,
- the full system/serial journal,
- the OTel trace.

Both sqlite and postgres variants. A healthy run is unaffected (no new artifacts).

## Design

### 1. Playwright config (`nixPlaywrightConfig`, flake.nix)

Add to `use`:

```js
trace: 'retain-on-failure',
screenshot: 'only-on-failure',
outputDir: '/tmp/e2e/test-results',
```

`retain-on-failure` writes/keeps a trace **only for failed tests** → zero extra artifacts and negligible overhead on a green run (the common case). Video is intentionally omitted (YAGNI — the trace already carries DOM snapshots + screenshots).

### 2. testScript restructure — capture → stream → copy-all → assert

The load-bearing change, applied to **both** backends and factored into one shared Nix helper (extending the existing `e2ePanicGate` pattern) so the two backends cannot drift. Replace the `machine.succeed("…playwright…")` + success-only copies with:

1. `status, out = machine.execute("cd /tmp/e2e && <env> … playwright test --config … --project <browser>")`.
2. `print(out)` — stream the `line`-reporter output to the driver's stdout, which `nix build -L` fans into `build.log` via xtask's `MultiWriter`. **This is recoverable even on failure and even without `--keep-failed`** — the robust core of #123 (the failing test + assertion).
3. **Unconditionally** (no `succeed` that could abort): stop `otel-collector`, then `copy_from_vm` each artifact flat with a `-<backend>` name:
   - OTel trace (now on the failure path too),
   - `playwright-report-<backend>.json`,
   - a tarball of `test-results/` → `playwright-artifacts-<backend>.tar.gz` (trace + screenshots),
   - the full system journal → `system-journal-<backend>.log`.
4. Run the existing `e2ePanicGate <backend>` (app-journal copy + panic assert).
5. **Finally** `assert status == 0, "Playwright failed (exit {status}) …"` — Playwright failure aborts the check only **after** all artifacts are safely copied.

On **success**, the copied files land in `$out` (and the trace tarball is near-empty — `retain-on-failure`). On **failure**, the final assert discards `$out`, but the copies already executed and live in the `--keep-failed` build dir for layer 3 to rescue.

### 3. xtask `rescue_diagnostics` extension (xtask/src/steps/nix.rs)

Extend `rescue_diagnostics` (today: copies only `emit-out/diagnostics` for coverage) to also, for e2e checks, copy the e2e artifacts from the kept `/tmp/nix-build-jaunder-<check>-*` build dir into `.xtask/diagnostics/<check>/`. Reuses the established "failed derivation → no `$out` → pull from kept build dir" pattern already in this file. The path-selection logic is pure and unit-tested (mirrors the existing `copy_e2e_diagnostics_between` test).

**One empirical unknown:** the exact in-build-dir path where `copy_from_vm` output lands on a *failed* build. The verification run (below) resolves it; the rescue glob is then wired to that path. This is the only part not determinable by code reading alone.

## Testing & verification

- **Rust unit test:** the extended `rescue_diagnostics` path-selection over fixture dirs (asserts e2e artifacts are picked up, coverage path still works).
- **End-to-end (one real failing run):** add a throwaway always-failing spec under `end2end/tests/`, run one real e2e combo (~10 min), confirm `playwright-report-*.json`, `playwright-artifacts-*.tar.gz`, both journals, and the OTel trace all land in `.xtask/diagnostics/<check>/`, and that the failing-test name appears in `build.log`. Capture the kept-dir path for the layer-3 glob, then **remove the temp spec before committing**.
- **Gate:** full `cargo xtask validate` green; a healthy run produces no new artifacts (`retain-on-failure`).

## ADR

New short ADR documenting the **e2e failure-diagnostics capture convention** (copy-before-assert in the testScript + `--keep-failed` rescue in xtask) — sibling to ADR-0032 (zero-panic gate) and ADR-0034 (e2e matrix). Future e2e checks follow this convention. Number = next after the current highest; add its row to the `docs/README.md` ADR table.

## Acceptance (merged from #49 + #123)

- [ ] On failure, durable diagnostics reach `.xtask/diagnostics/<check>/`: Playwright trace + screenshots + JSON report, app journal, system/serial journal, OTel trace.
- [ ] The failing test + assertion are recoverable from `build.log` alone (the streamed `line` reporter), independent of `--keep-failed`.
- [ ] The Playwright invocation is wrapped so a non-zero exit triggers the artifact copy **before** the check is failed (capture exit → copy → assert).
- [ ] OTel trace is copied on the failure path, not only on success.
- [ ] Both sqlite and postgres variants covered (via the shared helper).

## Out of scope / follow-ups

- Parallelizing the suite (#61, blocked by #51/#52/#53) — unrelated.
- HTML report rendering — the JSON report + trace zip are sufficient for root-causing; `npx playwright show-trace` opens the zip locally.
