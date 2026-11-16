# ADR-DRAFT: A Python `dev` runner with OpenTelemetry tracing replaces Just

- Status: proposed
- Date: 2026-11-16
- Issue: [#1965](https://github.com/jaunder-org/jaunder/issues/1965)

## Context

[ADR-0237](../0237-just-owned-gate-workflows.md),
[ADR-0238](../0238-just-e2e-workflow-ownership.md),
[ADR-0239](../0239-qualification-workflow-ownership.md) and
[ADR-0240](../0240-just-frontdoors-host-pr-observer.md) made the root `Justfile`
and `recipes/*.just` the owner of developer, hook, CI and sandbox workflows.
Milestone 33 then asked for per-step timing, outcome and cache evidence. The
plan was to extend traced-run's custom span mechanism, which passes context in
`JAUNDER_TRACED_RUN_CONTEXT` and writes Chrome JSON fragments.

Just 1.58 has no per-run or per-recipe hook. `set shell` is global and also
wraps backtick evaluation, and a recipe line can run outside any root span and
record nothing. Making every step traced, with its exit status and cache state,
would mean wrapping every line and trusting that no line escapes the wrapper.
Shell and Just recipes also handle argv quoting, nested composition and signal
forwarding poorly.

We compared real recipe ports in Just, bash and Python. Python with click
expressed steps, nesting, exit status and argv as ordinary code. Bash still
needed a hand-written span protocol, and invoke loses argv quoting. The
OpenTelemetry API/SDK already provides spans, context and environment
propagation (`TRACEPARENT`), so a custom span protocol is unnecessary. Perfetto
reads Chrome JSON and its own protobuf, but not OTLP.

## Decision

A Python package in `tools/dev/`, built with click and the OpenTelemetry API/SDK
and packaged by Nix as the `dev` command, owns every workflow the Justfile and
`recipes/*.just` owned. That covers developer gates, Git hooks, CI lanes, the
commands the Nix checks run inside the sandbox, and the remaining utility
recipes. Just is removed from the repository and the devShell.

Each gate and step is an OpenTelemetry span. Its attributes are its exit status,
its command text, and a `cache` value where one applies: `reused`, `built` or
`substituted` for Nix outputs, and nothing for any other step. Spans are
exported only to files: every process writes plain span start and end records
under `.xtask/run/<run_id>.trace.d/`. The root process merges them into one
Chrome trace JSON file, `<run_id>.trace.json`, for Perfetto, and marks spans
with no end or no parent as incomplete. Child processes join through the
OpenTelemetry environment carrier. host-proof uses the Rust OpenTelemetry SDK
with the same record format. `dev` runs from the checkout on the host and is
built from a filtered source in the sandbox. `dev trace summary` reads the
records locally or from CI artifacts.

Tracing never changes a step's exit status or output. traced-run keeps only its
output-parking runner; its launcher, span, recorder and workflow-folding code is
retired.

This supersedes the Just-ownership parts of ADR-0237, ADR-0238, ADR-0239 and
ADR-0240, and the traced-run span-mechanism constraint in milestone 33.

## Consequences

- Python, so far used only by the ADR promoter, now owns every workflow and gets
  its own gates: ruff, `mypy --strict`, pytest, and at least 90% branch
  coverage.
- The hermetic checks' closures gain python3, click and opentelemetry.
- Two small span-record writers, Python and Rust, must keep writing the same
  format. A shared golden-file test locks it.
- Any edit to `tools/dev/` rebuilds the hermetic checks that run `dev`.
- Behaviour tests that drove `just` with fake tools move to pytest against
  `dev`. Tests of Just itself and of Justfile text are deleted.
- The `no-just` source contract fails if Just command text (an invocation, a
  `Justfile` or `*.just` file name) reappears in `.githooks`, `.github`, `nix`,
  `scripts` or the tools' sources.
- Adding OTLP export later is a configuration choice, not a redesign.
