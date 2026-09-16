# Bound seed trace verification

## Outcome

E2E VM seed-trace verification finishes within a short dedicated budget and
reports its own failure instead of exhausting the lane or appearing to be a
Playwright timeout. Failures retain the available capture evidence for
diagnosis.

## Load-bearing decisions

- Seed evidence is validated inside the guest by a `test-support` command that
  streams the trace JSONL rather than returning the complete file through the
  NixOS test-driver channel.
- The command accepts the capture directory and derives the canonical
  `otel-traces.jsonl` path through `host::capture`; the harness does not restate
  the capture filename.
- Validation exits zero only for complete evidence. Nonzero failures carry the
  stable reason tokens `seed-trace-missing`, `seed-trace-malformed`, or
  `seed-trace-incomplete`; incomplete failures also name the required seed
  processes lacking a `storage.*` span.
- The NixOS test driver owns a 30-second timeout around collector shutdown and a
  separate 30-second timeout around the verifier command. Those failures carry
  the stable reason tokens `seed-collector-stop-timeout` and
  `seed-trace-verifier-timeout`, respectively.
- Any collector-stop or verification failure retains the available whole capture
  directory using the existing `capture-<backend>.tar.gz` contract before the
  lane fails.
- After such a failure, the lane terminates without restarting services or
  invoking Playwright. Successful verification retains the existing collector
  and Jaunder restart/readiness sequence.
- This extends the accepted capture-before-failure and capture-directory
  contracts in ADR-0037 and ADR-0057; it introduces no new trace schema or
  diagnostic artifact format.

## Acceptance

- Seed verification never streams the complete trace capture through the
  test-driver command channel.
- Valid evidence for both `e2e.seed.jaunder` and `e2e.seed.test-support`, each
  containing a `storage.*` span, passes.
- Missing or empty, malformed, and incomplete evidence each exits nonzero with
  its specified reason token; an incomplete result names every missing required
  seed process.
- Collector-stop and verifier timeouts independently finish within their
  dedicated 30-second budgets and report their specified stage-specific token.
- Each collector-stop or verifier failure copies any available
  `capture-<backend>.tar.gz` before surfacing the failure, and Playwright is not
  started.
- Automated tests cover successful, missing or empty, malformed, and incomplete
  verifier outcomes at the narrowest viable seam.
- Separate harness regressions exercise collector-stop timeout and verifier
  timeout, each proving the configured bound, stage-specific token,
  capture-before-failure ordering, and absence of service restart and Playwright
  execution.
- The authoritative SQLite/Chromium and PostgreSQL/Chromium E2E lanes pass,
  covering both backend-specific generated-script placements without increasing
  the Playwright or global VM timeout.

## Boundaries

- No production capture behavior, OpenTelemetry schema, collector endpoint, or
  application storage behavior changes.
- No weakening of required seed-span evidence or fallback to text matching.
- No new diagnostic artifact alongside the existing whole-capture tarball.
- No Playwright test, assertion timeout, or whole-test timeout changes.
