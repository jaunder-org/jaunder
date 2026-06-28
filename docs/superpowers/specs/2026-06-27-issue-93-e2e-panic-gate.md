# Issue #93 — e2e zero-panic gate + visible-by-default journal

* Status: proposed
* Issue: #93 (e2e gate: capture VM logs on success too + fail on any SSR panic)
* Date: 2026-06-27

## Problem

A Rust panic inside the running server during an e2e run is currently **invisible**:

1. SSR/server panics are isolated to spawned tasks — the HTTP response still returns 200, Playwright assertions still pass, so the **e2e check passes despite the panic** (this is exactly how #89 hid: a per-render `expect_context` panic that never failed a test).
2. On a CI **cache-hit**, `nix build -L` emits no machine output, so `.xtask/diagnostics/<check>/build.log` is empty — a panic that occurred when the derivation *first* built (and got cached green) leaves no trace on subsequent hits.
3. There is no durable, well-known artifact carrying the server's log for a *passing* run.

Net: a panic can be cached as "green" forever and never surface. The fix must (a) make a panic **fail** the e2e check, and (b) make the server log **discoverable by default** without a rebuild, including on cache-hits.

## Decision

### Part 1 — In-sandbox zero-panic assertion (the gate)

In each e2e nixos `testScript` (`mkE2eSqliteCheck` / `mkE2ePostgresCheck` in `flake.nix`), after the existing test steps and alongside the existing otel-trace copy, run one **shared snippet**:

```python
# Zero-panic gate (#93): the jaunder service log must contain no Rust panic.
machine.succeed("journalctl -u jaunder.service --no-pager -o cat > /tmp/jaunder-journal.log")
machine.copy_from_vm("/tmp/jaunder-journal.log", "jaunder-journal-<backend>.log")  # → $out, before the assert
journal = machine.succeed("cat /tmp/jaunder-journal.log")
ALLOWED_PANICS = []  # default-deny; add a proven-benign substring + a comment here if one ever appears
panics = [l for l in journal.splitlines() if "panicked at" in l and not any(a in l for a in ALLOWED_PANICS)]
assert not panics, "e2e zero-panic gate: jaunder.service logged Rust panic(s):\n" + "\n".join(panics)
```

- **Marker:** `panicked at` — the Rust panic-message prefix; covers the reactive_graph/context panics and every other panic. Not ERROR-level logs (a panic is the bar).
- **Source:** the `jaunder.service` journal (where server panics land).
- **Copy before assert:** the journal reaches `$out` even when the assert fails, so a failing run is diagnosable.
- **Allowlist:** default-deny. An empty, commented `ALLOWED_PANICS` is the documented seam for an explicit, justified exception — the bar never silently moves.
- **DRY:** the snippet is defined once (a Nix string) and interpolated into both backend `testScript`s.

Because a panic now makes the derivation **fail**, it cannot be cached green and cannot silently reappear.

### Part 2 — Visible-by-default journal (the discoverability)

The journal now lives in the e2e check's `$out` (via `copy_from_vm`), so it survives cache-hits. To make it **discoverable in one fixed place** regardless of cache state, the xtask `e2e` step (`xtask/src/steps/nix.rs`), after the `nix build`, copies the result's `jaunder-journal-*.log` from the realized output (`.xtask/gcroots/e2e/…`) into `.xtask/diagnostics/e2e/`.

CI already uploads `.xtask/diagnostics/` with `if: always()` (`.github/workflows/ci.yml`), so the journal is then an artifact on **every** run — fresh or cached, pass or fail. No ci.yml change required.

## Consequences

- **The gate may surface pre-existing hidden panics.** Once it runs, any *other* isolated panic that was previously cached-green will fail the e2e check. Each must be addressed at landing — **fixed** by preference, `ALLOWED_PANICS`-listed only with a written, proven-benign justification. (#89's SSR panics are already fixed, so they won't trip it.)
- `build.log` (the `-L` stream) remains the fresh-run capture; the `$out`→diagnostics journal is the cache-durable one. Both end up under `.xtask/diagnostics/e2e/`.
- Per-backend journals (`jaunder-journal-sqlite.log`, `jaunder-journal-postgres.log`) keep the two backends' logs distinct.

## Alternatives considered

- **Host-side scan of `build.log`** (xtask greps the `-L` log, fails the step). Rejected: a cache-hit produces no `-L` log, so it misses the detection entirely on cached runs, and it doesn't make the derivation itself panic-aware (the panic stays cacheable-green).
- **Upload-on-success only / rely on existing `if: always()`** without the `$out` journal. Rejected: doesn't cover cache-hits (empty `build.log`), which is the actual #89 blind spot.

## Scope of change

- `flake.nix` — a shared panic-gate snippet; interpolate into `mkE2eSqliteCheck` and `mkE2ePostgresCheck` `testScript`s (journal dump + `copy_from_vm` + assert).
- `xtask/src/steps/nix.rs` — the `e2e` step copies `jaunder-journal-*.log` from the realized `e2e` output into `.xtask/diagnostics/e2e/`; a pure unit test for the copy/path logic where feasible.
- `docs/adr/0032-e2e-zero-panic-gate.md` (+ README ADR-table row) — records the "e2e checks must be panic-free; logs visible-by-default" policy and the `ALLOWED_PANICS` escape hatch. (Written this cycle.)

## Testing

- The nixos `testScript` assertion is exercised by the e2e check itself: a clean run passes the gate; a run with a panic fails it. (No separate unit test for the Python snippet — it's validated end-to-end by `cargo xtask validate`'s e2e.)
- The xtask copy logic (resolving the journal path under the realized output, copying into diagnostics) gets a host-side unit test where the path handling can be isolated.
- **Acceptance:** full `cargo xtask validate` green on `main` (post-#89, the e2e journal is panic-free → gate passes), and the `jaunder-journal-*.log` files present under `.xtask/diagnostics/e2e/` after a run.

## Separable concerns

None anticipated beyond any pre-existing panic the gate surfaces at landing (handled in-cycle, not deferred).
