# Spec — #145: scoped nix-build failure-excerpt

- **Issue:** jaunder-org/jaunder#145 (milestone "Verify-gate hardening")
- **Companion:** #144 (app-driven e2e scoped diag capture). Together: scoped
  artifacts at the source, full logs demoted to last-resort.
- **Status:** approved

## Context

On a failed Nix check, `build_check` (`xtask/src/steps/nix.rs`) fans the
`nix build -L` stream to the live terminal **and**
`.xtask/diagnostics/<check>/build.log`, then names that log in the `StepResult`
detail. But the `-L` stream is a firehose — it interleaves every (transitive)
derivation, the VM kernel console, and app output — so an agent must grovel it
to find the actual failure. This issue adds a **scoped excerpt** written
alongside the full log, so the failure is readable at a glance; the full
`build.log` stays as the fallback artifact.

## Load-bearing finding (verified empirically)

A deliberately-failed `nix build -L` on this repo's nix was captured. Two facts
shape the design:

1. **`-L` does NOT suppress nix's own error block.** Even while streaming all
   build logs, nix still prints a self-contained summary at the end:

   ```
   error: Cannot build '/nix/store/…-fail-probe-0.1.0.drv'.
          Reason: builder failed with exit code 3.
          Output paths:
            /nix/store/…-fail-probe-0.1.0
          Last 25 log lines:
          > build-output-line-37
          …
          > FATAL_ERROR_MARKER
          For full logs, run:
            nix log /nix/store/…drv
   ```

   This block **names the failing derivation and includes its _de-interleaved_
   tail** (nix's `Last N log lines` are the failing builder's own, not the
   interleaved stream) — exactly the scoped content #145 wants. It also covers
   eval errors (`error: <msg>` + trace).

2. **The `-L` line prefix drops the version** (`fail-probe-0.1.0.drv` streams as
   `fail-probe> …`), so parsing prefixes to isolate the failing builder
   ourselves would be fragile. We don't need to — nix already isolates it in the
   error block.

`--log-lines N` controls the block's tail size; verified `--log-lines 50` yields
`Last 50 log lines` under `-L`.

## Decisions (resolved in design interview)

1. **Excerpt = carve nix's error block, don't parse prefixes.** The excerpt is
   the region from the first line that starts with `error:` (column 0 — builder
   lines are prefixed `<name>> `, so they never match) through EOF. This is
   robust (nix de-interleaves the tail for us), handles builder _and_ eval
   failures, and needs no drv/prefix parsing.

2. **Tail size: add `--log-lines 50`** to the `nix build` args, so the error
   block carries the failing builder's last 50 lines (the issue's "~50").
   Affects only failure output; no cost on success. The carve is independent of
   the count.

3. **Fallback when no `error:` line** (an unusual failure — truncated log, odd
   exit): the excerpt is the **last 50 lines of `build.log`**, prefixed with the
   marker line ``=== no `error:` block in build log; last 50 lines: ===``. The
   excerpt is never empty. Note this `50` is a _different_ mechanism from
   Decision 2's `--log-lines 50`: `--log-lines` sizes nix's de-interleaved
   builder tail _inside_ the error block (the normal path); this fallback is our
   own raw slice of `build.log` used _only when nix emitted no `error:` block at
   all_. Same number, chosen for consistency.

4. **`failure_detail` names the excerpt first** ("read first"), with the full
   `build.log` as the fallback it already names. Best-effort: if `build.log` is
   unreadable (the rare `File::create` failure in the drain), no excerpt is
   written and only the full log is named.

5. **No ADR; no separable concerns.** A diagnostics-visibility nicety within the
   existing `build_check`, per the standing "file diagnostic-visibility gaps as
   their own issues" policy (this _is_ that issue).

## Design

`xtask/src/steps/nix.rs`:

- **`--log-lines 50`** added to `build_check`'s `nix build` arg list.
- **`fn failure_excerpt(build_log: &str) -> String`** (pure) — the carve +
  fallback. Unit-tested.
- **`fn write_failure_excerpt(check: &str, log_path: &str) -> Option<String>`**
  (I/O) — reads the captured `build.log`, carves via `failure_excerpt`, writes
  `.xtask/diagnostics/<check>/failure-excerpt.log`, returns its path (or `None`
  if the log is unreadable).
- **`failure_detail`** gains an `Option<&str>` excerpt-path param and names it
  first when present.
- **`build_check` failure arm** calls `write_failure_excerpt` (alongside the
  existing `rescue_diagnostics`) and threads the result into `failure_detail`.

CI already uploads `.xtask/diagnostics/` (`validate-diagnostics` artifact), so
the excerpt ships without a workflow change. `.xtask/` is gitignored.

Docs: note "read `failure-excerpt.log` first" in `CONTRIBUTING.md`'s diagnostics
section (and/or the diagnostics doc, if one exists).

## Acceptance criteria (observable)

1. **AC1 — scoped excerpt written on failure.** A failing
   `cargo xtask check`/`validate` Nix check produces
   `.xtask/diagnostics/<check>/failure-excerpt.log` containing nix's `error:`
   block (the `error:`…EOF region), **not** the full interleaved `-L` stream.
2. **AC2 — detail points at it first.** The failed `StepResult` detail names the
   `failure-excerpt.log` (read-first) _and_ the full `build.log`.
3. **AC3 — richer tail.** `build_check`'s `nix build` args include
   `--log-lines 50`.
4. **AC4 — extraction unit-tested (pure).** `failure_excerpt` has host-side,
   deterministic tests over source strings: (a) a captured-sample `-L` log
   (interleaved prefixed lines + error block) → excerpt equals the `error:`…EOF
   block, excluding the interleaved head; (b) a log with no `error:` line → the
   last-50-lines fallback, asserting the marker substring
   (``no `error:` block``) is present and a known last line is included while an
   early line is not.
5. **AC5 — full log retained.** `build.log` is still written and still named as
   the fallback artifact (unchanged).
6. **AC6 — documented.** A **new** note "read `failure-excerpt.log` first" is
   added to `CONTRIBUTING.md` (no nix-build-`build.log` diagnostics section
   exists today; the #144 e2e "look here first" note at `CONTRIBUTING.md:~274`
   is the stylistic precedent, but it is a _different_ artifact — this adds a
   nix-build-diagnostics note).
7. **AC7 — no regression.** `cargo xtask check` is green; the existing
   `failure_detail` test is updated to the new signature and still asserts the
   full-log path is named.

## Out of scope

- Parsing/prettifying the error block beyond carving it (no drv-prefix
  isolation, no colorization).
- Changing `rescue_diagnostics`, the `-L` streaming, or CI artifact upload.
- The #144 app-driven e2e capture (separate issue).

## Testing / verification ladder

- `failure_excerpt` unit tests (AC4) via
  `cargo nextest run --manifest-path xtask/Cargo.toml steps::nix` (or the
  module's test path).
- A real failure was captured during design; a representative sample is embedded
  as the test fixture.
- `cargo xtask check` green before ship (AC7).
- **AC1/AC2/AC5 are integration outcomes** (excerpt file written; detail names
  both paths; full log retained) — verified by a **deliberately-failed** Nix
  build during implementation (the design already captured one), not the unit
  suite. **AC3** (`--log-lines 50` in the args) is a source-inspection check.
  The pure `failure_excerpt` core (AC4) carries the algorithmic risk.
