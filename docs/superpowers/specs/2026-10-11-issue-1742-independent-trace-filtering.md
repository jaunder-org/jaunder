# #1742: Independent ordinary-log and trace filtering

Issue: <https://github.com/jaunder-org/jaunder/issues/1742> Decision:
[Independent ordinary-log and trace filtering](../../adr/drafts/independent-log-and-trace-filtering.md)

## Outcome

A production instance can keep its existing quiet console settings while
collecting useful INFO-level request and operation traces. Intentionally
filtered request spans do not generate misleading parent-assignment warnings;
genuine failures retain safe, actionable diagnostics.

## Load-bearing decisions

- Ordinary-log admission and OTLP trace filtering are independent. Log policy
  preserves `JAUNDER_LOG_FILTER`, then `RUST_LOG`, then the established ordinary
  or `--verbose` defaults, including the storage INFO exception.
- Log policy governs whichever ordinary-log sink is active. #1742 keeps stdout
  routing unchanged; #1743 owns native OTLP logs, no-mirroring routing, logger
  lifetime and removal of journal ingestion. Its active log sink consumes the
  same log policy, not the trace filter or a separate native-log filter.
- Add `JAUNDER_TRACE_FILTER` using the existing tracing filter grammar. Its
  absent-value default is INFO; valid DEBUG, WARN, target directives and `off`
  are supported. Ordinary-log overrides and `--verbose` do not change it.
- Blank, malformed or unreadable trace-filter input emits a fixed local warning
  without the raw value and falls back to INFO. It does not fail startup or a
  command. Missing input is normal and emits no warning.
- An endpoint is still required for export. Explicit trace `off` prevents trace
  export without turning off ordinary logs, metrics or scoped diagnostics.
  Ordinary-log `off` does not disable traces or diagnostics. Endpoint
  precedence, sampling and provider shutdown semantics do not change.
- The policy is shared by the server, production CLI commands and test-support.
  Resolve inherited input once at the periphery and inject immutable typed
  policy; do not add deep environment reads or in-process environment mutation.
- Existing log-filter overrides cease to control traces. Document this
  intentional compatibility change, including explicit tracing disablement and
  debugging.
- Intentional trace filtering is normal absence. Enabled request spans adopt
  valid inbound W3C trace context; absent or invalid parent headers remain
  normal extraction cases, not synthetic assignment failures.
- Genuine assignment failures remain visible through a nonrecursive local
  fallback with bounded reasons. Do not use the failing telemetry path to report
  itself, dump request headers, or merely suppress all assignment errors.
- Filtering or replacing an ordinary-log sink must not remove error SpanTrace
  stacks or safely formatted fields, including late-recorded fields. Span-field
  retention must not depend on stdout being an active log sink. Preserve the
  server's independent WARN+ diagnostics and panic fallback contracts. Local
  SDK/export and assignment diagnostics must not re-enter native OTLP logs.
- Existing PII/secret exclusions and sink-specific field admission remain
  mandatory. More trace collection does not admit new payload classes.
- No schema, storage/backend behavior, application endpoint, protocol, public
  presentation, or domain vocabulary change is intended. No glossary change is
  needed. Record the novel sink-policy decision in a numberless ADR draft and
  its architecture projection; promotion remains CI-owned.

## Acceptance

- Lock the original warning down through actual request middleware/subscriber
  composition before changing behavior. With OTLP configured and quiet console
  settings, ordinary anonymous requests succeed without the repeated false
  warning and export an INFO request span. Preserve an original deployed-binary
  control and prove the working-tree behavior on the current branch.
- A controlled valid W3C parent exports the expected trace ID and parent span ID
  on its request span, with child operation spans in that trace. Missing and
  invalid headers do not produce the assignment warning.
- Prove independent sink admission with real subscriber/exporter observations:
  log WARN + trace INFO collects INFO spans without INFO ordinary-log output;
  log INFO/DEBUG + trace WARN does not widen trace admission; explicit trace
  `off` exports no traces while permitted log events and metrics survive; log
  `off` does not suppress enabled traces or independent WARN+ diagnostics. #1742
  proves its existing stdout sink; #1743 proves its native log sink against the
  same admission contract. Test target-specific filters, not only levels.
- Deliberately disabled request tracing produces no false assignment diagnostic,
  including when another sink keeps the tracing span active. Genuine layer or
  lifecycle assignment failures produce a bounded actionable local diagnostic
  rather than silence or recursive export.
- Resolver tests cover absent, valid, blank, malformed and unreadable trace
  inputs; ordinary-log precedence/defaults, `--verbose`, and endpoint precedence
  retain their existing behavior. Child-process tests prove inherited wiring for
  both executable roots without mutating the test process environment.
- Through real layer composition, an INFO span hidden from the ordinary-log sink
  retains its safe initial and late-recorded fields in an error SpanTrace. Prove
  that removing the stdout log sink does not remove formatted span fields. WARN+
  capture remains independent of log/trace `off`, and panic capture remains
  nonrecursive and independently written. No secret-bearing fields are added.
- Verify short-lived production CLI and seed-process trace flushing and
  unchanged metrics/shutdown behavior. Tests use injected configuration or
  isolated child environments and retain the established capture artifact
  scope/layout.
- Run focused host and HTTP integration proofs first. Exercise a real
  browser-to-server flow with the ordinary quiet console and default trace
  policy; exported evidence must still support request correlation and existing
  e2e/boot/coverage consumers. Preserve SQLite/PostgreSQL proof for any
  storage-backed HTTP assertions. Do not weaken gates, capture requirements,
  retries or timeouts to obtain a passing result.
- Execute required proof and keep its logs/captures under ignored `.xtask/` or
  session storage, not as tracked delivery reports. Commit only the approved
  spec, decision/projection, implementation, regression tests/minimal maintained
  fixtures and current operator documentation. Retire completed planning docs
  after conformance review; retain the ADR and current guidance.
- Operator documentation states both filter controls, precedence, defaults,
  invalid-value behavior, explicit `off`, increased trace volume/retention
  responsibility, and the intentional change to legacy log-filter semantics.

## Boundaries

This includes shared host telemetry, request-parent failure handling, associated
regression coverage, ADR/projection and operator guidance. It does not fix
VictoriaLogs message-field mapping (#1743), introduce an OTLP log pipeline or a
new browser telemetry SDK, alter sampling/retention policies, or change console
verbosity defaults. No production instrumentation or deployment change is
required to demonstrate the contract. An implementation outline is appropriate
for the subscriber/configuration architecture boundary, but follows spec
approval and has its own approval gate.

Coordination proposal: #1742 owns shared filter resolution/admission and lands
first; #1743 rebases before final subscriber wiring and combined regression
proof of logs/traces/metrics, SpanTrace/capture and one-shot shutdown. Confirm
this interface/order with the #1743 owner before implementation; this text
creates no native blocking link and does not claim the peer has already accepted
it.
