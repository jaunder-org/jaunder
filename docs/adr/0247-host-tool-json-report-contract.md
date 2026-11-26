# ADR-0247: Host tools report one JSON document on stdout

- Status: accepted
- Date: 2026-11-12
- Issue: [#1898](https://github.com/jaunder-org/jaunder/issues/1898)
- Amended: 2026-11-24 — `e2e-local` is no longer a JSON-report tool (#2066)
- Amended: 2026-11-25 — `wasm-audit` is no longer a JSON-report tool (#2101)
- Amended: 2026-11-25 — `admit-inputs` moves to host-tool; `csr-prerequisite` is
  retired (#2102)
- Amended: 2026-11-25 — nor are `e2e-evidence` and `trace-analysis` (#2100)
- Amended: 2026-11-25 — no host-proof JSON-report tool remains (#2105)

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

_Amended by #2066._ `e2e-local` has left this list and the host-proof list
below. It now runs inside `dev` (`tools/dev/dev/e2e_local.py`), which keeps the
same step list but prints only its rendered summary on stdout, records each step
as a span, and uses its exit status as the verdict. That status is 1 for a
failed lifecycle step and 128+n on a signal. Its admission and Nix build are
ordinary `dev` steps, which end the run with their own status and no summary. No
JSON report is written.

_Amended by #2101._ `wasm-audit` has also left this list and the host-proof list
below. `dev wasm-audit report` and `validate`'s wasm budget run as `dev`'s own
Python (`tools/dev/dev/wasm_audit.py`). It prints its tables directly, prints a
refusal on stderr, and uses its exit status as the verdict. Nothing read its
JSON report, so none is written.

_Amended by #2103:_ as of 2026-11-25, `source-diagnostics` had left this list
and the host-proof list below, retired with the cache-safety probe
([CI honours non-substitutable verdicts](0267-ci-honours-non-substitutable-verdicts.md));
current state: ARCHITECTURE.md §Residual tooling owners after xtask retirement.

_Amended by #2100._ `e2e-evidence` and `trace-analysis` have left this list and
the host-proof list below too. The e2e verdicts (each lane's `e2e-evidence`
step, `dev e2e-reconcile` and the server-function coverage verify and
regenerate) and `dev trace-analysis` run as `dev`'s own Python. A verdict
follows `dev`'s step convention: its steps render as `dev`'s step view and a
failed step ends the run with that step's status. Trace analysis prints its
tables directly. No JSON report is written, and CI's e2e reconcile runs no
Nix-built host-proof verifier.

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
diagnostics (`trace-analysis`, `source-diagnostics`); `tools/coverage`
(nondefault `host` feature) owns `coverage-check`; `tools/doctests` (nondefault
`host` feature) owns `doctest-check`; `tools/qualification` keeps
`qualification-host` and its own plain detail output under
[ADR-0239](0239-qualification-workflow-ownership.md). CI's e2e shell runs the
Nix-built `packages.host-proof` verifier.

_Amended by #2102._ `admit-inputs` is host-tool's one executable and renders no
report: a refusal is exit 1 and stderr. `csr-prerequisite` is retired.

_Amended by #2105:_ as of 2026-11-25, no host-proof JSON-report tool remained:
the `tools/host-proof` crate and the Nix-built `packages.host-proof` verifier
had been deleted, so the host-proof concern owner above had ended. The report
contract still lived in `tools/host-tool`, and `coverage-check`
(`tools/coverage`, nondefault `host` feature) was the one host tool whose
verdict `dev` rendered from it; current state: ARCHITECTURE.md §Residual tooling
owners after xtask retirement.

## Consequences

- One small typed report shared through `tools/host-tool` replaces the generic
  envelope; a new tool adds no rendering code.
- Machine consumers parse stdout directly; CI artifacts that need a file
  redirect it (or keep the human log, as the Firefox reconciliation does).
- Running a tool binary directly ends in a JSON document rather than a table;
  the Just recipes are the human frontdoor.
- A diagnostic whose product is a rendered table carries it in `report`; a
  versioned payload rides in `evidence`, not in ad hoc top-level fields.
