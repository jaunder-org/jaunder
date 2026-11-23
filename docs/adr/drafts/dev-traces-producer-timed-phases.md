# ADR-DRAFT: Dev traces the phases a sandboxed producer timed itself

- Status: proposed
- Date: 2026-11-23
- Issue: [#2031](https://github.com/jaunder-org/jaunder/issues/2031)

## Context

A hermetic producer runs inside a Nix build. There it cannot join the `dev`
trace: the sandbox has no `JAUNDER_TRACE_DIR`, and a trace written in the build
directory is discarded. In the trace, the Rust coverage producer was therefore
one `build` span. That span covered workspace resolution, the census compile,
the instrumented test run and the reports, plus Nix's fetch, unpack and install.
For #1981 (CI run 44383337174, 819.9 s) the breakdown had to be reconstructed by
hand from log timestamps and `status.json`. #2030 raises the same need for the
E2E lane phases in `e2e-phase.json`.

[ADR-0254](../0254-dev-trace-step-accounting.md) decided that a span times one
kind of work, and it added no new trace framework, only attributes on the
existing span records. Producers already time their own phases, but only as
durations in their sidecars.

## Decision

A `dev` step may write **after-the-fact child spans** from the timed evidence of
a Nix output that this run built.

- The producer records each phase's wall-clock start (Unix milliseconds) beside
  its duration, and records the two together or not at all. A phase that never
  started records neither and gets no span.
- After the build, `dev` reads the sidecar from the output. It writes one
  finished span per phase under the step's `build` child, with the producer's
  start and end times and a `jaunder.status` of `ok` or `failed`. These spans go
  through the same record writer, golden format and merge as every other span.
- Spans are written only when the output's `jaunder.cache` is `built`. A
  `reused` or `substituted` output's phases ran elsewhere or earlier, on another
  clock, and are never placed in this run.
- An unreadable or malformed sidecar yields no phase spans, all or nothing.
  Reading it never changes the step's status or output.

The first user is the Rust coverage producer's `status.json` (`started_unix_ms`
beside `duration_ms`).

## Consequences

- `dev trace summary` shows a producer's phases, and `build`'s untraced time is
  the Nix overhead around them (fetch, unpack, install).
- The placement assumes that the builder's clock is the host's. That holds
  because only a `built` output is traced, and a non-substitutable producer
  builds locally.
- #2030 can reuse the same helper with its own parent span, once the E2E phase
  sidecar records start times.
- Producers still never write span records themselves inside a sandbox, and
  nothing is exported over OTLP.
