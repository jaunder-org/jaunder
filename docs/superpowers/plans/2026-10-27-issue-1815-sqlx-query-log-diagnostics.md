# SQLx query-log diagnostics implementation outline

> Execute with `jaunder-iterate`, delegating with `jaunder-dispatch`. Trigger:
> producer-qualified privacy exception and shared policy/proof ownership.
> Contract:
> [approved specification](../specs/2026-10-27-issue-1815-sqlx-query-log-diagnostics.md).

## Execution summary

First prove a real SQLx query becomes useful stored diagnostic data through the
stock native log bridge, canonical Collector policy, and VictoriaLogs. Reuse the
existing stack VM, not a second stack or a product-only diagnostic endpoint.
Then expand the privacy/carrier regression cases and publish operator guidance.
No additional bind capture or SQLx trace-retention exception is authorized.

## Task outline

- [x] Task 1: Store real normal and slow SQLx diagnostics in the existing stack.
  - Policy owner: `nix/otel-processors.yaml` and its reviewed policy digest in
    `xtask/src/steps/e2e_collector_config_check.rs`. Apply exactly the approved
    producer selection, field/carrier admission, and body precedence.
  - Proof owner: `nix/checks.nix` and the minimum test-only Rust fixture/package
    needed to execute synthetic SQLx queries through existing native telemetry
    initialization. Do not replace the bridge or add a product endpoint.
  - Shared contract: emitted full statement/summary bytes, original slow
    message, known query results/counts, actual duration fields, severity, and
    available trace/span IDs are the comparison inputs; no manually fabricated
    SQLx event may substitute for real query execution.
  - Use a short query such as `SELECT 1` to cover SQLx's empty-statement summary
    fallback, plus a longer synthetic query to exercise full-statement
    precedence. Configure separate fixture connections with ordinary versus zero
    slow-query thresholds; SQLx compares elapsed time with `>=`, so zero
    deterministically exercises slow logging without sleeps. Keep this setting
    fixture-local.
  - Verification: extend the existing captured-signals stack scenario to query
    VictoriaLogs and compare `_msg`, SQL attributes, counts, duration carriers,
    severity, and IDs. Preserve the original slow-query message byte-for-byte.
    Establish this tracer bullet before expanding the regression matrix.

- [x] Task 2: Lock down the narrow SQLx exception and unchanged privacy
      boundaries.
  - Extend behavior assertions within the existing Collector/stack proof seams;
    use synthetic OTLP records only for edge cases real SQLx cannot naturally
    generate (conflicting producers, unsigned overflow, malformed carriers).
  - Verify native `sqlx::query` and facade scope `log` plus matching
    `log.target`; unrelated scopes with a spoofed target must not gain SQLx
    fields or a body.
  - Cover absent/empty bodies, full-statement versus summary precedence,
    existing messages, stock numeric/string carriers, and unrelated-field
    rejection. Other producers and SQLx trace events/spans retain their existing
    treatment.
  - Keep canonical policy parsing, pre-export ordering assertions, and the
    reviewed semantic digest aligned. A matching digest is not behavior proof.
  - Verification: affected policy checker tests, merged Collector configuration
    validation, and extended real-stack assertions. All spec acceptance items
    1–4 map to Tasks 1–2; no permanent wiring/source-copy tests.

- [x] Task 3: Publish the implemented operator contract.
  - Update `docs/observability.md` and `docs/DESIGN.md`: SQLx query lookup,
    statement/message distinction, execution fields, and explicitly accepted SQL
    exposure. Non-SQLx bodyless records remain outside this fix.
  - Finalize the existing SQLx ADR draft and `docs/ARCHITECTURE.md` projection,
    replacing implementation-pending language only after runtime proof succeeds.
  - Verification: independent conformance review against the approved spec and
    actual stored-record evidence; evidence stays in ignored/session storage.

## Risk checks and execution boundaries

- Reuse `jaunder-stack-sqlite-bcrypt` and `jaunder-stack-postgresql` in
  `nix/checks.nix`; run the SQLite tracer bullet first, then both backend checks
  for final integration proof. No database schema or production configuration
  changes are needed for the fixture.
- Real-stack commands:
  `nix build .#checks.x86_64-linux.jaunder-stack-sqlite-bcrypt` and
  `nix build .#checks.x86_64-linux.jaunder-stack-postgresql`.
- Collector syntax check uses `otelcol-contrib validate` with
  `end2end/otel-collector.yaml` and `nix/otel-processors.yaml`, supplying the
  existing loopback endpoint/capture environment. It does not prove ingestion.
- Policy and VM owners have disjoint files; agree on the approved wire fields
  before parallel implementation. Documentation can proceed independently but
  cannot claim implementation or proof before Tasks 1–2 pass.
- Preserve unrelated staged #1834 documents and the untracked #1804 spec. Commit
  only issue #1815 artifacts through `jaunder-commit`; do not sweep the
  pre-existing index into this issue's commits.
