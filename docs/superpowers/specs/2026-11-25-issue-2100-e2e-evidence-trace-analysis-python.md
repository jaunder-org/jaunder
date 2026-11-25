# Issue #2100: e2e-evidence and trace-analysis become Python in `dev`

## Outcome

The e2e verdicts are `dev e2e-producer`'s lane verify, `dev e2e-reconcile` and
server-fn coverage `verify`/`regenerate`. Together with the manual
`dev trace-analysis analyze | run | boot-phases` diagnostics, they run as Python
inside `tools/dev`. Each verdict decides exactly as before, and the committed
`docs/coverage/server-fns.json` regenerates byte for byte.

The Rust `e2e-evidence` and `trace-analysis` binaries, and every `host-proof`
module that only they use, are deleted. No `host-proof` code uses
`host::capture`; `wasm-audit` and `csr-prerequisite` still use `host`'s manifest
and theme modules. `dev trace-analysis run`, which cannot find today's per-lane
checks, works again. Everything lands in one PR that closes #2100.

## Approach and tradeoffs

**One trace core.** The core is a Python module and the only reader of e2e
captures. It holds:

- the OTLP JSONL span parser, with today's fail-closed errors;
- the attribute and JSON-attribute readers;
- the gzip capture reader;
- the `boot_phases` decomposition outcome, under ADR-0165.

The capture reader rejects a non-file member, a duplicate member and an empty
trace, and requires `instance-N/diag.log` numbered contiguously from 0. The
e2e-evidence verdicts and every trace-analysis command use this core. Its parsed
span keeps start and end Unix nanoseconds, span id and parent id, which are the
fields `dev/chrome.py`'s Perfetto track layout needs. That is all this PR does
toward a Perfetto view: rendering capture spans in the `dev` Perfetto trace is
#2115, which this one blocks.

**e2e-evidence becomes in-process `dev` code.** `dev/e2e.py` calls Python
functions instead of a `host-proof-build` step and an `e2e-evidence` subprocess.
The command names, arguments, file layout and CI jobs stay the same. Each check,
its order, and its message gist are preserved:

- lane population;
- the duration budget (fail at ≥ 80% utilization);
- boot decomposition (closure within 1 ms, at least 3 boot phases, zero dropped
  navigations, the report/trace project-set match);
- the ownership census and reconcile, which collects every failure rather than
  stopping at the first;
- phase sidecars (an unavailable required phase is a failure);
- span-loss refusal and the required `diag.log` (#2029, #2076);
- signal-1/signal-2 server-fn attribution.

`compare_control_population` and `validate_unsplit_control` are never called in
production, so they are dropped rather than ported. Verdicts follow the `dev`
conventions that `e2e-local` (#2066) and `perf` (#2068) use: steps rendered by
`report.render_report`, then `StepFailed`. They do not use the
one-JSON-on-stdout host-tool contract. Exit status stays 0 for pass and 1 for a
verdict failure.

**Product facts through product interfaces.**

- **Server-fn inventory.** `repository-policy`'s `source-contracts` gains
  `server-fn-inventory`. It prints the existing `syn` inventory of `web/src` as
  JSON, one entry per fn: qualified name, module, identifier and endpoint. `dev`
  runs it as the Nix-built tool from the commit's own checkout root. It is a
  read-only listing, so it skips `prepare_policy_checkout`'s untracked-input
  admission, and a local regenerate is never refused over an unrelated untracked
  file. Python parses it without re-validating it and without a schema
  (`docs/adr/drafts/perf-data-python-reads-validated-data.md`). The CI reconcile
  job already builds Nix tools, so it builds `repository-policy`. It still never
  builds or runs a VM.
- **Capture layout.** `instance-`, `diag.log` and `otel-traces.jsonl` are
  spelled once in Python. `e2e_local.py` uses the same constants instead of its
  own `instance-{index}`. Each spelling is added as a pair to the ADR-0109
  cross-language literal table against `host/src/capture.rs`, so drift fails the
  static gate. In `host`, `instance_directory` and `instance_index` spell
  `instance-` separately, so they first share one `INSTANCE_PREFIX` constant for
  the pair to anchor on. ADR-0109 and the table's module doc, both written for
  Rust↔TypeScript only, are widened to cover Python sites. Rejected alternative:
  keep a Rust reader for the layout. It would keep `host` linked only to learn a
  filename, which ADR-0261 names as the smell.

**trace-analysis.**

- `analyze` keeps its flags (`--top ≥ 1`, `--trace`, `--project`, repeatable
  `--playwright-report`, at least one file).
- It keeps its 13 sections, their order, columns, row rules, sorting and number
  formats (`.3`/`.1`/`.2`, `-` for empty).
- `boot-phases` keeps its per-population medians, status lines and segment
  table.
- Tables render with `tabulate`, added to the pinned `devPythonDeps` and
  `tools/dev/pyproject.toml`, with `types-tabulate` added to the `dev-python`
  check for `mypy --strict`. Its `simple_outline` style is the box-drawing style
  closest to today's `tabled` sharp tables. Cells are pre-formatted strings
  passed with `disable_numparse=True`, so tabulate never reformats a number. A
  test pins this, e.g. `1.000` stays `1.000`. The glyphs may differ slightly,
  which is accepted: no consumer parses them.
- Exit codes are kept: 0 on success, 1 for a malformed JSON trace attribute, and
  2 for a refusal (an unreadable file, or a conflicting Playwright duration).
  The one exception is `run`: a failed Nix build is a `dev` step failure and
  exits with nix's status (usually 1), not 2.
- `run` is fixed. For both backends and the selected browser(s), it builds:
  - **Default:** each enabled catalog lane check, `checks.e2e-<identity>`.
  - **`--single-worker`:** the `e2e-<backend>-<browser>-single-worker` package,
    with identity `<backend>-<browser>-unsplit`.

  It reads `capture-<identity>.tar.gz` and pairs it with
  `playwright-report-<identity>.json` when present. Each trace is labelled by
  its identity, and the result feeds `analyze`. Builds go through
  `steps.nix_build`, so cache reuse is recorded.

**Deletion.**

- **Rust code:** the two binaries, `e2e`, `e2e_evidence`, `e2e_ownership`,
  `playwright_report`, `traces/`, `server_fn_coverage/`, the three `steps`
  modules, and `test_support`.
- **Tests and fixtures:** `tests/e2e_workflow.rs` and the Rust fixtures move to
  `tools/dev/tests` as pytest. Fixtures that would fall out of use are dropped.
- **Dependencies:** crate dependencies left unused (`host`, `tabled`, and any of
  `flate2`/`tar`/`url`/`opentelemetry*`) are removed.
- **Packaging:** `nix/packages.nix` drops the two binaries.

**Server-fn seed cross-checks.** The 2.8 MB seed is a recorded capture.

- **Endpoint check stays live in Rust.** Today the seed is the only live check
  that the inventory's derived endpoints match the URIs the server really serves
  (ADR-0120). `repository-policy` owns that derivation, so this check stays
  there as a Rust test. It runs over a small committed fixture of the seed's
  observed `(module, span name, URI path)` tuples, against the live `web/src`
  inventory, and keeps today's minimum of 50 matches.
- **Extractor checks move to Python.** These are:
  - each attribution signal alone covers the full union;
  - `media::upload` is covered;
  - non-`/api` traffic is present.

  They run against the seed plus an inventory JSON recorded alongside it, so
  both are the same snapshot in time.

**Docs and ADRs.** A tracked numberless ADR draft (`jaunder-adr`) records "e2e
verdicts and trace analysis are `dev` Python". It amends:

- ADR-0246 and ADR-0247, which drop the two executables from the host-tool
  report contract;
- ADR-0258, which drops them from the Nix-built host tools;
- ADR-0165's and ADR-0120's tool references;
- ADR-0109, which widens to Python sites, as above.

`docs/ARCHITECTURE.md`, `docs/observability.md` and `CONTRIBUTING.md` are
updated, including the `boot-phases` and `--playwright-report` usage they omit
today and the corrected `run` description.

The cost we accept: about 5k lines of Rust become Python held to the
100%-branch-coverage `dev-python` check.

## Acceptance

- **Tests:** `dev-python` passes (ruff, `mypy --strict`, 100% branch coverage).
  The pytest suite reproduces each Rust test's behavior: every verdict's pass
  case and each listed failure message, the 80.00% boundary rejection,
  collect-all reconcile ordering, span loss in both modes, archive rejections,
  boot closure at 55.0 (pass) vs 56.5 (fail), snapshot determinism over
  run-a/b/c, and the `analyze` section order and formats.
- **Byte-identical regeneration:** `dev e2e-regenerate-coverage` (once, at the
  end) leaves `docs/coverage/server-fns.json` unchanged. Byte-equal `verify`
  passes in CI.
- **Real-artifact parity:**
  - A downloaded CI `e2e-lane-*` artifact set reconciles under the Python
    `dev e2e-reconcile` with the same verdict as the Rust binary on `main`.
  - `dev trace-analysis analyze` and `boot-phases` on its capture produce the
    same table contents as the Rust binary, apart from the glyphs.
- **`run` works:** `dev trace-analysis run --browser chromium` (cached lane
  builds) produces the analysis. This is proof to execute, not committed output.
- **Deletion:** in code, Nix, workflows and current docs (`CONTRIBUTING.md`,
  `docs/ARCHITECTURE.md`, `docs/observability.md`), no reference to an
  `e2e-evidence` or `trace-analysis` executable or to `host-proof`'s e2e/trace
  modules remains. Still allowed: `dev trace-analysis`, dev's `e2e-evidence`
  step name, and historical or amendment text in ADRs and specs. No `host-proof`
  source references `host::capture`. The `repository-policy` endpoint test
  passes against the live `web/src`. The CI-equivalent gate
  (`dev validate-no-e2e`) is green, and the PR's CI E2E lanes and reconcile are
  green.
- **Follow-ups filed:** #2116 reuses `tabulate` for `dev`'s existing tabular
  output. #2115 renders capture spans in the Perfetto trace. Both are blocked by
  this issue.

## Boundaries

- No change to the verdict rules, the lane catalog, the VM producers, artifact
  names or the CI job graph, beyond which binary a job runs.
- No new trace-analysis sections or flags. `run` is repaired, not extended.
- `wasm-audit`, `source-diagnostics`, `admit-inputs` and `csr-prerequisite` stay
  in `host-proof`. Retiring `host-proof` is its own blocked issue.
- The static `server-fn-coverage` leaf in `repository-policy` is unchanged,
  apart from sharing the inventory it already owns.
