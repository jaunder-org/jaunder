# ADR-0121: No `<link rel="preload">` for the wasm bundle

- Status: accepted
- Date: 2026-08-11

## Context

The SPA shell boots by fetching the wasm bundle after the JS glue. A
`<link rel="preload">` in the served `<head>` looked like an easy win: start the
2.2 MB download during HTML parse and collapse the serial pre-fetch window
(#866).

## Decision

The head carries **no wasm preload**, by measurement, under a pre-registered
abort rule. The trial (#866) collapsed the serial pre-fetch window exactly as
intended — firefox 276.2 → 81.5 ms per navigation — and bought nothing: the time
reappeared as fetch contention and a historical 125 ms increase in the
response-end residual then called `wasm_instantiate`. Boot total improved 18.8
ms against a pre-registered floor of 38.8 ms, so the abort rule fired and the
preload was reverted.

That retained residual is **not** a direct compile or initialization
measurement. #887 supersedes it with direct `wasmApiMs` and `wasmInitMs`
diagnostics; neither is added to the exclusive boot decomposition. Do not re-add
preload without reading `docs/observability.md` §"#866" and #887. If it is ever
re-added, `crossorigin` is mandatory: without it firefox downloads the bundle
twice.

## Consequences

- `render_head` stays preload-free; the comment there points here.
- The `WASM_URL`/`GLUE_URL` constants and their drift guards remain — a preload
  URL that drifts from the `init()` target would not fail but silently
  double-download, which is the failure mode any future retrial must guard
  first.
