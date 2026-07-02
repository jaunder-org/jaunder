# ADR-0037: e2e VM diagnostics are captured before the check is failed, and recovered from the kept outPath

- Status: accepted
- Deciders: mdorman, Claude Opus
- Date: 2026-06-29

## Context and Problem Statement

When an e2e nixos VM check failed, **no artifacts survived**, so a CI-only red
e2e was undiagnosable without local reproduction (#123). Each Playwright run was
wrapped in `machine.succeed("…playwright…")`, which raises on a non-zero exit
and aborts every step below it — and _all_ artifact copies (OTel trace, the
Playwright report, the app journal) ran after that call, i.e. only on the
success path (#49). The config generated no trace or screenshot at all. A failed
`nixosTest` derivation also produces no registered `$out`, so the success-path
copier (which reads the realized out-link) had nothing to read.

The result: on a red e2e you learned only _that_ a spec failed, never _why_ — no
failing test name guaranteed in any retained log, no trace, no screenshot, no
app/system journal. This is the diagnostic sibling of the gap ADR-0032 closed
for server panics.

## Decision

**Capture every diagnostic before the check is allowed to fail, and recover it
from the kept output path on a failed build.**

1. **Generate forensics for failed tests.** The VM Playwright config sets
   `trace: 'retain-on-failure'`, `screenshot: 'only-on-failure'`, and an
   artifact `outputDir` — so a green run (the common case) writes nothing extra,
   and a failure leaves a trace zip + screenshots.

2. **Capture → stream → copy-all → assert.** A shared `e2eRunAndCapture` helper
   (one definition, both backends, so they cannot drift) runs Playwright via
   `machine.execute` (capturing its exit, not aborting), `print`s its
   `line`-reporter output so the failing test + assertion reach `build.log` via
   the `nix build -L` stream — recoverable even on failure and without
   `--keep-failed` — then copies **all** diagnostics out of the VM
   unconditionally (OTel trace, the JSON report, a tarball of the
   trace/screenshot `outputDir`, the system journal, and the app journal), and
   only **then** asserts the Playwright exit. Artifacts are therefore safe
   before any failure aborts the run.

3. **Recover from the kept outPath.** `nix build --keep-failed` leaves the
   failed derivation's output store path on disk, world-readable, at the
   deterministic path `nix eval --raw .#checks.<system>.<check>.outPath`. On a
   failed build, `xtask` reads that path and reuses the existing success-path
   copier to lift the diagnostics into the always-uploaded
   `.xtask/diagnostics/<check>/`. (The `/tmp` build dir, by contrast, is
   nixbld-owned and unreadable, and its location is configurable — the outPath
   is both simpler and more robust.)

## Consequences

- Good: a CI-only e2e failure is diagnosable from the `validate-diagnostics`
  artifact alone — failing test + assertion in `build.log`, plus the trace zip,
  screenshots, JSON report, and both journals. Closes #123 and #49.
- Good: a healthy run is unaffected — `retain-on-failure` writes no trace, and
  the tarball step is a no-op when there are no failures.
- Good: one shared helper and one shared copier serve both backends and both the
  success and failure paths, so the capture set cannot silently diverge.
- Neutral: the failure recovery relies on `--keep-failed` leaving the outPath on
  disk; a subsequent `nix store gc` would remove it, but rescue runs immediately
  after the failed build, before any GC.
- Cost: a larger `testScript` tail (the shared helper) and one `nix eval` on the
  failure path.
