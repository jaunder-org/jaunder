# Local E2E OTLP Capture Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for an individual
> task when useful. This outline exists because a second supervised process,
> graceful telemetry flush, ephemeral receiver allocation, and one collector
> configuration shared across Nix and host execution create concurrency and
> cross-subsystem contract risk.

## Scope

In:

- One portable collector configuration used by the VM E2E gate and `e2e-local`.
- Pinned host availability of `otelcol-contrib`.
- Per-lifecycle collector supervision, endpoint injection, strict failure
  reporting, and unique retained captures.
- Focused lifecycle tests, the real `feeds.spec.ts` smoke path, trace analysis,
  and the existing local-E2E documentation affected by the behavior.

Out:

- PostgreSQL or backend changes to `e2e-local`.
- Changes to `devtool pg run` or Rust unit/integration test environments.
- New spans, trace schemas, analysis gates, or automatic analysis.
- Changes to the authoritative VM E2E pass/fail policy.

## Task outline

- [x] Task 1: Make the E2E collector configuration portable without changing VM
      behavior.
  - Contract: a checked-in collector configuration is the single source for both
    environments. Its receiver endpoints use the collector's literal
    `${env:OTELCOL_GRPC_ENDPOINT}` and `${env:OTELCOL_HTTP_ENDPOINT}` provider
    syntax; its exporter uses `${env:JAUNDER_CAPTURE_DIR}/otel-traces.jsonl`.
  - Contract: the SQLite and PostgreSQL VM services set the receiver variables
    to `127.0.0.1:4317` and `127.0.0.1:4318`, while the host supplies ephemeral
    loopback endpoints. `otelcol-contrib` is pinned in the host shell that runs
    xtask.
  - Verification: validate the shared configuration with the pinned collector
    and prove both VM definitions consume it while retaining their existing
    endpoints and capture path.

- [ ] Task 2: Provide an independently tested host collector guard.
  - Contract: the guard owns one `otelcol-contrib` child, distinct loopback
    receiver endpoints, stderr diagnostics, and a temporary capture directory.
    It exposes the gRPC exporter URL and browser HTTP trace URL only after both
    receivers are listening and the child remains alive.
  - Contract: explicit shutdown requests graceful termination, waits for the
    collector to flush, records a non-successful exit, and reaps the child. Drop
    is the fail-safe kill/reap path and cannot stand in for successful flushing.
  - Verification: focused xtask tests exercise readiness, early exit, graceful
    shutdown, Drop cleanup, and two simultaneous guards with distinct ports and
    capture directories.

- [ ] Task 3: Integrate one collector guard into each existing local E2E
      lifecycle.
  - Contract: one guard belongs to one `run_lifecycle` interval, starts before
    Jaunder and seeding, and outlives every Playwright invocation and fixture
    teardown in that interval.
  - Contract: Jaunder receives
    `JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT=http://<grpc-endpoint>`; Playwright
    receives `JAUNDER_E2E_OTLP_HTTP_ENDPOINT=http://<http-endpoint>/v1/traces`
    plus the existing trace-context inputs. Multiple Playwright invocations in
    one lifecycle share one collector and artifact.
  - Contract: collector readiness, early exit, and shutdown failures are
    recorded alongside Playwright and panic-gate results without masking an
    earlier failure.
  - Verification: focused xtask tests cover endpoint wiring, lifecycle
    cardinality, ordering, and combined result recording. Use the
    repository-native xtask test lane with `--manifest-path xtask/Cargo.toml`.

- [ ] Task 4: Retain and report every completed or failed lifecycle capture.
  - Contract: after collector shutdown, copy the whole capture directory across
    filesystem boundaries into
    `.xtask/e2e-local/<unique-run>/<browser>/capture`, then print the trace path
    when present or the retained run directory plus a missing-file diagnostic.
  - Contract: a copy or finalization failure is an infrastructure failure
    recorded alongside prior failures. The temporary source is kept and its path
    reported as the fallback artifact instead of being deleted or masking an
    earlier Playwright, panic-gate, or collector failure.
  - Verification: focused tests cover successful cross-filesystem-safe copying,
    missing trace output, copy/finalization failure, fallback source retention,
    unique destinations, and multi-failure reporting.

- [ ] Task 5: Prove and document the local trace workflow.
  - Verification: the real `cargo xtask e2e-local feeds.spec.ts` path retains
    and prints each artifact; `cargo xtask traces analyze <printed-path>` parses
    it; inspection proves an `e2e.test` and its server `request` share a trace
    ID and request-to-test parent link.
  - Verification: a controlled Playwright failure retains partial capture before
    failure is returned, and collector failure diagnostics preserve that primary
    failure.
  - Contract: after the behavior is proven, update the existing local-E2E and
    observability documentation with the retained-path and manual-analysis
    workflow; do not describe local capture as replacing the VM gate.

## Risk checks

- Port allocation must be loopback-only, retry-safe, and collision-free for
  concurrent checkouts; no fixed host OTLP receiver is introduced.
- Collector readiness must prove both receivers are listening and the child is
  still alive before starting Jaunder.
- Graceful collector termination must flush before the capture is moved; Drop
  remains a reap guarantee, not the success-path flush mechanism.
- Capture-before-assert ordering must preserve partial diagnostics and every
  primary/secondary failure, matching ADR-0037.
- The single capture directory and `otel-traces.jsonl` name remain the ADR-0057
  contract; VM seed-span assertions and VM gate authority remain unchanged.
- No lint suppression may be introduced without explicit user approval.
