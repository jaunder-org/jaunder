# ADR-0247: Host tools report one JSON document on stdout

- Status: accepted
- Date: 2026-11-12
- Issue: [#1898](https://github.com/jaunder-org/jaunder/issues/1898)

## Context

[ADR-0246](0246-retire-xtask-devtool-endpoints.md) retired the `xtask` omnibus
and moved its residual capabilities to concern owners. Every xtask command
reported through one envelope: `[ ok ]`/`[FAIL]` step lines, a
`.xtask/last-result.json` sidecar, a `--json` flag and an `xtask-done:` stderr
sentinel. Carrying that envelope into each new executable would have copied its
rendering and sidecar machinery into every concern package — more code, not less
— while keeping two output modes per command. Child processes (Git, Nix,
Playwright) also wrote to the same stdout, so a machine reader could not rely on
it.

## Decision

Every host-run tool executable that renders a verdict (`test-local`,
`e2e-local`, `trace-analysis`, `e2e-evidence`, `wasm-audit`,
`source-diagnostics`, `coverage-check`, `doctest-check`) prints **exactly one
JSON report on stdout** and its **exit status is the verdict**. The report is
the small `host_tool::report` structure: `command`, `ok`, `duration_ms`,
`steps[{name, ok, skipped, duration_ms, detail}]`, an optional `exit_override`,
an optional `report` string for rendered tables, and an optional `evidence`
value for a versioned machine payload. There is no sidecar, `--json` flag or
completion sentinel. A refusal or top-level error is still one report: a single
failed step with status 2.

The shared argument parser sends help and usage text to stderr and then points
the process's stdout descriptor at stderr, reserving the real stdout for the
report. Every child process and stray print therefore lands on stderr; only the
report reaches stdout.

Human views are derived from the JSON, never rendered by the tools:
`scripts/step-report.jq` prints the `report` text and one line per step, and
`scripts/run-report` (or the equivalent inline bash under `pipefail`) applies it
so the tool's status, not the filter's, remains the recipe's verdict.

The contract lives in its own small `tools/host-tool` library (with the shared
multi-checkout compile-cache environment), so repository-policy keeps owning no
reports or process execution, as ADR-0246 requires.

Concern owners for the residual executables: host-proof owns focused proof
(`test-local`, `e2e-local`, and `pg-run`, which only propagates its wrapped
command's status), artifact-only e2e verdicts (`e2e-evidence`) and host
diagnostics (`trace-analysis`, `wasm-audit`, `source-diagnostics`);
`tools/coverage` (nondefault `host` feature) owns `coverage-check`;
`tools/doctests` (nondefault `host` feature) owns `doctest-check`;
`tools/qualification` keeps `qualification-host` and its own plain detail output
under [ADR-0239](0239-qualification-workflow-ownership.md). CI's e2e shell runs
the Nix-built `packages.host-proof` verifier.

## Consequences

- One small typed report shared through `tools/host-tool` replaces the generic
  envelope; a new tool adds no rendering code.
- Machine consumers parse stdout directly; CI artifacts that need a file
  redirect it (or keep the human log, as the Firefox reconciliation does).
- Running a tool binary directly ends in a JSON document rather than a table;
  the Just recipes are the human frontdoor.
- A diagnostic whose product is a rendered table carries it in `report`; a
  versioned payload rides in `evidence`, not in ad hoc top-level fields.
