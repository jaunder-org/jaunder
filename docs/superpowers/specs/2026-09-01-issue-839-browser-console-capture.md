# Issue #839: browser console and page-error capture

## Outcome

The Playwright trace harness records browser console warnings, console errors,
and uncaught page errors for every page attached through `attachTraceCapture`.
The default page exports test-phase diagnostics on `e2e.test`; additional
contexts export their test-phase diagnostics on their existing `e2e.page` span.
Diagnostics emitted before the `pretest` → `test` phase switch remain in the
pretest sink and are never attributed to the test.

This change adds observation only. Browser diagnostics do not fail a test.

## Capture contract

- `attachTraceCapture` remains the sole listener-registration seam. It attaches
  listeners to every page already in the context and every page subsequently
  emitted by the context.
- The existing warning-only sink becomes one ordered browser-diagnostic sink.
  Records receive the existing context-global `sequence` and an `emittedMs`
  timestamp at event delivery.
- Console capture deliberately includes only Playwright message types `warning`
  and `error`. `log`, `info`, `debug`, and other console levels are excluded.
- A console record contains:
  - `kind: "console"`
  - `type: "warning" | "error"`
  - `text`
  - Playwright's source `location` (`url`, zero-based `line`, zero-based
    `column`)
  - `sequence` and `emittedMs`
- An uncaught page-error record contains:
  - `kind: "pageerror"`
  - `name`, `message`, and the browser-provided `stack` when present
  - `sequence` and `emittedMs`
- Event payloads are normalized synchronously in the listener. No `JSHandle`
  values or live Playwright objects enter the sink.
- Phase attribution is decided when the event is delivered. Pretest records stay
  in the pretest sink; test records stay in the test sink.

## Trace schema

- Test-phase records are serialized, in sequence order, into `e2e.console_json`.
- At most the first 20 records are exported per span. Preserving the first
  records retains the likely root cause rather than a later cascade.
- `e2e.console_dropped` reports `max(0, total records - exported records)` on
  every span carrying `e2e.console_json`; truncation is never silent.
- The full in-memory sink remains available to test helpers. Capping happens
  only at trace serialization.
- The default context writes these attributes to `e2e.test`. Each
  `tracedContext` writes the same attributes to its existing `e2e.page` span.
  Span identity, timing ranges, parentage, and existing attributes/events remain
  unchanged.
- The Rust trace analyzer requires no schema change: it preserves unknown
  attributes and has no console-specific consumer.

## Test-data boundary

- Browser diagnostics are collected only by the Playwright harness while it
  drives the isolated E2E environment. Production browser code installs no
  listener and exports no browser console or page-error payload.
- Exact diagnostic text and stacks may contain the harness's synthetic users,
  tokens, passwords, or post bodies. Preserving those values is intentional: the
  E2E corpus exists to make failures in synthetic flows directly observable
  rather than reconstructing them indirectly.
- This exception does not admit real user data or infrastructure credentials.
  The harness must continue to target its disposable seeded environment; a
  capture against a non-test deployment is outside the supported contract.
- A proposed ADR amendment narrows ADR-0011's no-PII rule: production telemetry
  remains PII-free, while isolated E2E telemetry may contain synthetic
  application data needed to diagnose the test. The architecture projection
  changes with the draft.

## Compatibility and migration

- Replace `ConsoleWarningRecord` / `CaptureSink.consoleWarnings` with the
  discriminated browser-diagnostic record and sink. Migrate the existing
  client-telemetry ordering test to select its warning from the new sink; do not
  retain an alias or duplicate warning list.
- Update the observability schema and truncation table for the two new
  attributes and the 20-record first-retained policy.
- Add a proposed ADR draft and architecture projection that distinguish
  production telemetry from isolated E2E telemetry carrying synthetic test data.
  The implementation otherwise follows ADR-0096's context-level listener and
  phase-attribution boundary and ADR-0011's automatic browser-span export model.

## Verification

- A focused Playwright test emits a warning, an error, and an uncaught exception
  and observes three correctly typed, phase-attributed normalized records. It
  also proves excluded console levels do not enter the sink.
- The existing client-telemetry test still proves the local warning precedes the
  associated request using the shared sequence.
- A deterministic transform test proves the first 20 records are serialized in
  order and the exact dropped count is reported.
- Existing trace-schema checks accept `e2e.console_json` and
  `e2e.console_dropped` on both `e2e.test` and `e2e.page` without changing their
  span ranges or identities.
- Privacy-boundary tests or structural checks prove that listener installation
  and raw browser-diagnostic export remain inside the Playwright E2E harness;
  production client code gains no corresponding exporter.

## Non-goals

- Failing tests on console errors, warnings, or uncaught page errors.
- Guessing allowlists, suppression rules, or severity thresholds before captures
  establish the real distribution.
- Capturing ordinary `log`/`info`/`debug` output.
- Evaluating or serializing console argument handles.
- Teaching the Rust trace analyzer to interpret browser diagnostics.
