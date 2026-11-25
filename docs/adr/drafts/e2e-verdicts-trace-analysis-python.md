# ADR-DRAFT: E2E verdicts and trace analysis are Python in `dev`

- Status: proposed
- Date: 2026-11-25
- Issue: [#2100](https://github.com/jaunder-org/jaunder/issues/2100)

## Context

The e2e verdicts ran as host-proof's Rust `e2e-evidence` executable: lane
verify, reconcile, and server-fn coverage `verify`/`regenerate`. `dev e2e-*`
built `packages.host-proof` and parsed its one-JSON report. The manual trace
diagnostics were a second executable, `trace-analysis`, with its own copy of the
OTLP span reader. `trace-analysis run` no longer worked: it looked for e2e check
names that the per-lane catalog had replaced.

[ADR-0261](../0261-tool-language-by-maintainability.md) chooses the language for
infrastructure by maintainability. These tools read JSON, JSONL and gzip tar
files, compare sets and render tables. None needs Rust-specific parsing. Their
only Rust-specific need was the `#[server]` inventory of `web/src`, which
`repository-policy` already owns with `syn`. They also linked `host` only to
learn the capture layout's file names (`host::capture`).

## Decision

**The e2e verdicts and trace analysis are Python in `dev`.** `dev/e2e.py` calls
the verdicts in-process. The commands, arguments, file layout, CI jobs, check
order and exit statuses do not change. Verdicts follow `dev`'s step convention:
steps rendered by `report.render_report`, then `StepFailed`. They do not use the
one-JSON-on-stdout host-tool report.
`dev trace-analysis analyze | boot-phases | run` keeps its flags, sections,
columns and number formats. Tables render with `tabulate`, with number parsing
off so pre-formatted cells stay verbatim.

**One trace core.** `tools/dev/dev/traces.py` is the only reader of e2e
captures. It holds the OTLP JSONL span parser with its fail-closed errors, the
attribute readers, the gzip capture reader, and the boot decomposition outcome
under [ADR-0165](../0165-playwright-report-defines-trace-gate-population.md).
Every verdict and trace-analysis command uses it.

**Product facts come through product interfaces.**

- The server-fn inventory is `repository-policy`'s
  `source-contracts server-fn-inventory`, which prints the `syn` inventory of
  `web/src` as JSON. `dev` runs the Nix-built tool and parses the listing
  without re-validating it, as
  [the perf-data reader rule](../drafts/perf-data-python-reads-validated-data.md)
  allows. The listing skips untracked-input admission because it only reads.
- The capture layout (`instance-`, `diag.log`, `otel-traces.jsonl`) is spelled
  once in Python. Each spelling is a pair in the
  [ADR-0109](../0109-cross-language-literal-agreement.md) table against
  `host/src/capture.rs`, so drift fails the static gate. No host-proof code uses
  `host::capture`.

**The endpoint-derivation check stays live in Rust.** Under
[ADR-0120](../0120-no-endpoint-drift-check.md) the seed's observed URIs are the
only check that derived endpoints match what the server serves.
`repository-policy` owns the derivation, so the check is its test: it runs over
the seed's recorded `(module, span name, URI path)` tuples against the live
`web/src` inventory. The extractor's own seed checks run in Python against the
seed and an inventory recorded with it.

**`trace-analysis run` builds per-lane checks.** By default it builds every
enabled catalog lane check (`checks.e2e-<identity>`) for both backends and the
selected browsers. With `--single-worker` it builds the
`e2e-<backend>-<browser>-single-worker` packages, identity
`<backend>-<browser>-unsplit`. It reads each `capture-<identity>.tar.gz` with
its `playwright-report-<identity>.json` and analyzes them by identity.

## Consequences

- The `e2e-evidence` and `trace-analysis` executables are deleted, with every
  host-proof module only they used. About 5k lines of Rust become Python held to
  the `dev-python` check's 100% branch coverage.
- This amends [ADR-0246](../0246-retire-xtask-devtool-endpoints.md) and
  [ADR-0247](../0247-host-tool-json-report-contract.md): the two executables
  leave host-proof and the one-JSON host-tool report contract.
- It amends [ADR-0258](../0258-dev-runs-nix-built-host-tools.md): they leave the
  Nix-built host tools. Server-fn coverage builds `repository-policy` instead
  for its inventory, and the reconcile job still builds no VM.
- It amends the tool references in ADR-0165 and ADR-0120. ADR-0165's population
  rule is unchanged; ADR-0120's live check moves to `repository-policy`.
- It widens ADR-0109, written for Rust and TypeScript, to Python sites.
- `tabulate` joins the pinned `dev` dependencies.
- `host-proof` keeps `wasm-audit`, `source-diagnostics`, `admit-inputs` and
  `csr-prerequisite`, which still use `host`'s manifest and theme modules.
