# ADR-DRAFT: Dev traces the phases a sandboxed producer timed itself

- Status: proposed
- Date: 2026-11-23
- Issue: [#2031](https://github.com/jaunder-org/jaunder/issues/2031)

## Context

A hermetic producer runs inside a Nix build or a VM. There it cannot join the
`dev` trace: the sandbox has no `JAUNDER_TRACE_DIR`, and a trace written in the
build directory is discarded. In the trace, the Rust coverage producer was
therefore one `build` span. That span covered workspace resolution, the census
compile, the instrumented test run and the reports, plus Nix's fetch, unpack and
install. For #1981 (CI run 44383337174, 819.9 s), the breakdown had to be
reconstructed by hand from log timestamps and `status.json`. An E2E lane's
producer step hid its VM phases the same way. #2030 traced those phases from the
lane's lifted `e2e-phase-<lane>.json`.

[ADR-0254](../0254-dev-trace-step-accounting.md) decided that a span times one
kind of work. It added no new trace framework, only attributes on the existing
span records. Producers already time their own phases in their sidecars.

## Decision

A `dev` step may write **after-the-fact child spans** from the phases its
producer timed itself. This amends ADR-0254's "attributes only" rule for that
case alone. Every span still times one kind of work, and no new trace framework
is added.

- The producer records each phase's wall-clock start beside its duration, and
  records the two together or not at all. A phase without both gets no span.
- When the step's child ends, and before the step closes, `dev` reads the
  sidecar where the step left it. Coverage reads `status.json` through its GC
  root, under the `build` child of the `coverage` step. An E2E lane reads its
  lifted phase manifest, under `e2e-producer`. One helper writes each phase as a
  finished child span at the producer's start and end times, with a
  `jaunder.status` of `ok` or `failed`. These spans go through the same record
  writer, golden format and merge as every other span.
- Only phases inside the step's own time window are written. A reused or
  substituted output's phases, or an earlier run's left behind by a failed
  build, ran before the step started and are left out, not misplaced. No Nix
  store query decides this.
- A missing or malformed sidecar yields no phase spans and never changes the
  step's status or output. Each producer's reader decides how strictly it reads
  its own sidecar: coverage's reads all or nothing.

## Consequences

- `dev trace summary` shows a producer's phases. The parent's untraced time is
  the work around them, such as Nix's fetch, unpack and install.
- The window test assumes that the producer's clock is the host's. That holds
  for a local build or VM. A phase measured on a skewed remote clock is dropped
  or, if the skew is small, shifted.
- A further producer only needs its sidecar to record start times and a reader
  that turns them into phases.
- Producers still never write span records themselves inside a sandbox, and
  nothing is exported over OTLP.
