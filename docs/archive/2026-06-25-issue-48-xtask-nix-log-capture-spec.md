# Spec — #48: capture Nix build logs on failure (verify-gate diagnostics)

**Issue:** jaunder-org/jaunder#48 (milestone 1, "Verify-gate hardening"; P1; blocks #18)
**Date:** 2026-06-25
**Related:** #49 (e2e VM artifact persistence — sibling diagnostic gap), #18 (the sqlite lock flake whose verification depends on this).

## Problem

`xtask/src/steps/nix.rs::build_check` runs every Nix check (`nix-coverage`,
`nix-coverage-gate`, `nix-e2e`) via `nix build` and, on failure, surfaces only a
one-line summary:

- it runs `Command::new("nix").args(["build", "--keep-failed", "--accept-flake-config",
  "--out-link", …, &installable]).status()` — `.status()` inherits stdio, so **no**
  output is captured to any file or variable;
- it does **not** pass `-L` / `--print-build-logs`, so the full per-derivation build
  log is never printed (nix shows a progress bar + on failure only "last 10 log
  lines" and a `nix log …` pointer);
- on failure it records `StepResult::fail(step).detail("nix build .#… exited with
  <status>")` — the only thing that reaches `.xtask/last-result.json`;
- it calls `rescue_diagnostics(check)`, whose `/tmp/nix-build-jaunder-<check>/emit-out/diagnostics`
  glob does not match the e2e VM derivations (the dispatched check is the `e2e`
  aggregate), so it copies nothing.

**Why `nix log` alone is insufficient.** The failing derivation is typically a
*transitive dependency* (e.g. `vm-test-run-jaunder-e2e-sqlite`), not the top-level
`installable` (the `e2e` aggregate is a trivial `symlinkJoin`). `nix log .#checks…e2e`
returns the aggregate's empty log, not the VM failure. Only `nix build -L` streams
**every dependency's** build log, so that is what we capture.

**CI exfiltration already exists but is starved.** `.github/workflows/ci.yml:59-70`
already uploads `.xtask/diagnostics/` (and `.xtask/last-result.json`) as the
`validate-diagnostics` artifact with `if: always()` (runs on failure) and 14-day
retention. But `.xtask/diagnostics/` is populated only by the (ineffective)
`rescue_diagnostics`, so for an e2e failure the uploaded artifact is effectively
empty. Writing the captured log under `.xtask/diagnostics/` plugs directly into this
existing upload — **no workflow change is required**, and the failing build log
becomes retrievable from the CI run's artifacts.

## Goal

When any Nix check dispatched by xtask fails, the full build log (including the
failing transitive derivation) is captured to a durable, gitignored path,
referenced from the failure `detail`, and — because of the path — automatically
included in CI's existing `validate-diagnostics` artifact. Diagnosable without a
rebuild, locally and from CI.

## Design

### Capture mechanism: `-L` + a fan-out writer

`build_check` changes from a fire-and-forget `.status()` to a spawn that fans the
child's **stderr** (where `nix -L` writes all build logs) out to both our stderr (so
a human running the gate sees live output) and a per-check log file. The fan-out is a
tiny `Write` adapter driven by `std::io::copy` — not a hand-rolled read loop.

- Add `-L` to the `nix build` argument list (unconditionally — on cached/no-op runs
  it adds essentially nothing; on real builds it streams the full logs that are the
  point of the capture).
- The child is spawned with `stderr(Stdio::piped())`; stdout still inherits (the
  out-paths line is irrelevant on failure and noisy to capture). We then call
  `io::copy(&mut child_stderr, &mut MultiWriter(log_file, io::stderr()))`, where
  `MultiWriter` forwards every chunk to both sinks. `io::copy` does the read/buffer
  looping; the only code we own is the ~6-line fan-out adapter. After the copy
  returns at EOF we `child.wait()` for the exit status.
- **Live-output nuance:** piping stderr makes it a non-TTY, so nix drops its TTY
  progress bar for plain `-L` log lines. This still streams live (locally and to the
  CI console) and is clearer for capture — an intentional, minor cosmetic change from
  today's progress bar.
- Log path: `.xtask/diagnostics/<check>/build.log` (the existing diagnostics
  convention; gitignored; created with `create_dir_all`; overwritten each run).
- On failure, `detail` references the saved log, e.g.
  `nix build .#checks.x86_64-linux.e2e exited with exit status: 1; full build log: .xtask/diagnostics/e2e/build.log`.
- On success the log file is written and simply not referenced (cheap; aids success
  diagnostics too). No schema change to `StepResult` — the path lives in the existing
  `detail: Option<String>`.

### Testable seams

xtask is coverage-exempt, but — mirroring #41 — the logic is split so it can be unit
tested without shelling out to nix:

- `struct MultiWriter<A: Write, B: Write>(A, B)` implementing `Write` by forwarding
  each `write`/`flush` to both inner writers (`write_all` to both, returning
  `buf.len()`). Tested by driving `io::copy` from an in-memory reader into a
  `MultiWriter` of two `Vec<u8>` sinks and asserting both receive the full input
  (including a large / multi-chunk input so a single `write` isn't assumed).
- `fn failure_detail(installable: &str, status: &std::process::ExitStatus, log_path: &str) -> String`
  — formats the failure message including the log path. Pure; tested for the exact
  shape (contains the installable, the status, and the path).
- `build_check` keeps the nix invocation, wiring the real child stderr + the log
  `File` + `io::stderr()` into `io::copy`/`MultiWriter`, and `failure_detail` into
  the `StepResult`. This thin shell-out is exercised by the existing
  `cargo xtask validate` gate, not a unit test.

### `rescue_diagnostics`

Kept as-is. It serves the coverage check's *structured* `emit-out/diagnostics`
bundle (nextest log, disk-usage, status sentinel — a data artifact, not a build
log). The new `-L` capture supersedes its (broken) role for e2e *logs*, so the e2e
prefix-mismatch becomes moot rather than something this issue fixes.

## Acceptance

- A failing Nix check leaves the full build log at `.xtask/diagnostics/<check>/build.log`,
  including the failing transitive derivation's output (verified by inducing a real
  e2e or coverage failure and confirming the failing test/builder text is present in
  the file).
- The failure `StepResult.detail` (and thus `.xtask/last-result.json`) names that
  log path.
- Live terminal output during a failing build is no worse than today (the tee still
  streams stderr).
- The captured log is sufficient to diagnose the failure **without** a rebuild.
- The log path is covered by ci.yml's existing `validate-diagnostics` upload glob
  (`.xtask/diagnostics/`) — confirmed by inspection (no workflow change), so the log
  is retrievable from a failed CI run's artifacts.
- Unit tests for `tee` and `failure_detail` pass; `cargo xtask validate --no-e2e`
  green.

## Scope

- **In:** the `build_check` capture change (all three current callers benefit via the
  single chokepoint), the `tee`/`failure_detail` seams + their tests, and an
  acceptance check of CI-artifact pickup by inspection.
- **Out:** changing `ci.yml` (the upload already covers the path); a structured
  `StepResult.log_path` field (low value — CI upload is path-based; can be a later
  polish if a machine consumer needs it); fixing `rescue_diagnostics`'s e2e glob
  (superseded, not in scope); the e2e VM-side artifact persistence (that is #49); any
  coverage/e2e logic.
