# Spec — lower the e2e VM driver timeout (#130)

**Issue:** [#130](https://github.com/jaunder-org/jaunder/issues/130) — *test(e2e): lower the e2e VM driver timeout so boot/infra flakes fail fast* (milestone: E2E test suite)

## Problem

Each e2e NixOS VM test runs under the test-driver's **default `globalTimeout` of
3600 s (60 min)**. There is no literal `3600` in the repo — neither
`pkgs.testers.nixosTest` call sets `globalTimeout`, so the framework default
applies. Healthy runs peak at **~10.6 min** (the slowest single-browser combo —
Firefox, per #152's measurement of ~578 s Playwright + ~22 s boot + seed/warmup),
giving a ~6× margin.

When an **infra/boot flake** occurs (e.g. CI run `28309366884`: the sqlite guest
kernel oopsed during early boot, the VM never produced a root shell), the driver
waits the *entire* 60 min before terminating — converting a fast-detectable flake
into an hour-long run. This bounds the *cost* of such flakes; it does not fix the
underlying flake (a separate infra concern).

## Change

Add `globalTimeout = 1200;` (20 min) to both NixOS-test attrsets in `flake.nix`:

- `mkE2eSqliteCheck` — `pkgs.testers.nixosTest { … }` (~line 560)
- `mkE2ePostgresCheck` — `pkgs.testers.nixosTest { … }` (~line 666)

Each gets a short comment justifying the value against the measured ~10.6 min
healthy max and referencing #130.

### Why this knob

`globalTimeout` is the NixOS test-driver's **total wall budget** — exactly the
"per-VM e2e driver timeout" the issue describes. Currently unset → defaults to
3600 s; a boot hang burns the full hour. Setting it to 1200 s makes the driver
terminate near the 20-min bound instead.

### Why 1200 s (20 min)

~1.9× the measured ~10.6 min healthy max. Conservative headroom for CI-runner
variance (CI is typically slower than local), while still cutting a boot-flake
from 60 → 20 min (**3× faster fail**). Justified against the measured max rather
than a round number, per the issue's acceptance note.

### What is *not* changed

The per-step `wait_for_unit` (60 s) and `wait_for_open_port` (30 s) timeouts stay
as-is — they already fail fast on *service-level* hangs. This issue targets the
*driver-total* budget, which is what governs whole-VM / early-boot hangs that
never reach those steps.

One uniform value across both backends, keyed to the slowest combo (Firefox);
no per-backend differentiation.

## Verification

- `cargo xtask validate` runs all four `{sqlite,postgres}×{chromium,firefox}`
  combos. A healthy pass completing well under 1200 s proves the new budget is
  adequate (no false timeouts on a healthy run).
- The fast-fail-on-hang behavior is `globalTimeout`'s documented nixpkgs
  semantics. It cannot be deterministically simulated locally without injecting a
  boot hang, and a nix driver-timeout value is not unit-testable, so we rely on
  the framework contract plus the justifying comment rather than an automated
  regression test.

## Out of scope / follow-ups

- Fixing the underlying boot/kernel-oops flake (separate infra concern — file
  separately if it recurs).
- Tightening the timeout further once per-combo times are re-measured post-#129
  fan-out (the issue notes this as a future option).

## Acceptance (from the issue)

- [x] e2e VM driver timeout reduced from 3600 s to 20 min (1200 s), justified
  against the ~10.6 min healthy max (~1.9×).
- [x] A boot hang / non-responsive VM terminates near the new bound rather than at
  60 min (by `globalTimeout` semantics).

## ADR

None — a tuning change, not a novel architectural decision.
