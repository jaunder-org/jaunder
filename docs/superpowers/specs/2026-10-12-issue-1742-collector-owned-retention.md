# #1742 revision: Collector-owned OTLP retention

Issue: <https://github.com/jaunder-org/jaunder/issues/1742>

## Outcome

Small installations can use local logging without a Collector. When an OTLP
endpoint is configured, Jaunder records and offers its own spans to the
Collector without application-level severity/target filtering or head sampling.
The Collector owns preservation decisions. Local verbosity does not determine
OTLP output. Future native OTLP logs follow the same unfiltered-output contract.

This is a new active revision, not a restoration of completed planning. It
supersedes the unlanded independent trace-filter proposal reviewed at
`872ea71043455f48abed55e1792cf6bf2bbca139`. PR #1756 remains draft while this
revision is approved and implemented; prior conformance does not certify the new
outcome.

## Decisions

- `JAUNDER_LOG_FILTER`, then `RUST_LOG`, then established ordinary/`--verbose`
  defaults govern local ordinary-log output only. Preserve the storage INFO
  exception, pretty/JSON formats and current local-output behavior.
- The OTLP trace layer has no application-level level, target or context filter.
  Remove the unlanded `JAUNDER_TRACE_FILTER` input/resolver/accessor/fallback,
  filter-specific fixtures and host per-span admission marker/query. Do not
  replace them with a new transport-specific verbosity switch or comparable
  bookkeeping.
- Configure the SDK with `Sampler::AlwaysOn` for Jaunder's spans. Preserve valid
  incoming W3C trace IDs and parent span IDs, but do not suppress Jaunder's
  recording/export because an incoming sampled flag is clear. The caller may
  discard its own data; that does not govern Jaunder. Missing/invalid headers
  remain normal root-context cases. No recovery of the caller's omitted spans is
  promised.
- Export remains best-effort: existing exporter failures, bounded buffering,
  shutdown and transport behavior remain. "Unfiltered" is not a guarantee of
  delivery when the Collector is unavailable or the process dies. It applies to
  instrumentation present in the binary, not instrumentation the compiler or a
  dependency never produced. Exporter-only runtime shutdown follows both
  provider shutdowns with a fixed five-second best-effort budget, without a
  configuration knob. Unfinished blocking work may remain until it returns or
  the process exits, including when the telemetry guard is dropped while the
  process continues. Joining the runtime owner does not prove that work stopped;
  Tokio's shutdown API reports no timeout outcome.
- No endpoint means no OTLP provider/export. Continue local logging, independent
  WARN+ diagnostics, panic capture and error span context. Parent extraction and
  adoption follow the actual installed tracer/subscriber capability; with no
  tracer, do not attempt assignment. With an installed unfiltered layer, normal
  requests adopt their parent without the removed per-span marker.
- Genuine assignment failures retain bounded, fixed local stderr reasons without
  recursion or raw header/source text. Preserve middleware ordering, request IDs
  and safe field admission.
- Preserve initial and late error `SpanTrace` fields with filtered, absent,
  pretty or JSON local output. The existing distinct field cache/updater solves
  this independently of OTLP filtering; simplifying trace policy must not lose
  error context. Preserve slow-span reporting, metric behavior, diagnostic/panic
  independence and provider lifetime/flush contracts.
- #1743 owns native OTLP log export, physical sink selection, no-mirroring and
  logger lifetime. Its OTLP ordinary-log sink admits every application log
  severity independently of local verbosity. It must not consume the local
  filter accessor. Agree a clearly local-only accessor contract with its owner;
  do not introduce a second OTLP log filter. This supersedes the earlier shared
  ordinary-log policy for whichever physical sink is active.
- Unfiltered output does not bypass privacy/redaction/sink-specific field rules
  or turn independent local diagnostic transports into OTLP logs. A fixed,
  non-configurable per-OTLP-layer SDK-context suppression guard is permitted
  solely for SDK self-observation isolation, not application retention. Preserve
  suppression through application-span activation and exporter-only transport
  execution; ordinary application telemetry remains unfiltered. Known activation
  limitation: repeated entries of the same tracing span exited out of order can
  expose an interleaved external OpenTelemetry context early, and can lose
  suppression after that external scope exits. Callbacks have no entry-instance
  identity; the pinned bridge has the same latest-span-ID exit behavior. This is
  documentation, not an API restriction or enforcement rule; no arbitrary
  entry-instance restoration is promised. Keep SDK, exporter, configuration,
  shutdown, field-format and parent-assignment local fallbacks outside the
  failing/native export pipeline to avoid recursion.
- Update the unlanded numberless ADR and architecture projection to the selected
  policy, and document local versus OTLP behavior, explicit AlwaysOn semantics,
  endpoint enablement, compatibility and Collector volume/retention ownership.
  Resolve process inputs at the periphery; add no deep ambient reads or
  in-process environment mutation.

## Acceptance

- Establish red focused tests on the current implementation for DEBUG/TRACE
  request/operation admission under local WARN/off and for a valid unsampled
  incoming W3C parent. Then demonstrate all admitted Jaunder spans export with
  the expected IDs and parent relationships, independent of local severity,
  target/context directives and `--verbose`.
- Through real production subscriber/SDK composition, local INFO/DEBUG/TRACE
  events are filtered as configured while their spans and permitted span events
  remain available to OTLP. Local `off` does not disable OTLP spans, metrics or
  independent diagnostic/panic capture. No standalone OTLP filter or sampling
  knob remains in the implemented interface or operator documentation.
- Real HTTP/router proof covers sampled and unsampled upstream parents, absent
  and invalid headers, child operation correlation, request IDs and safe fields.
  Without a tracer, ordinary requests succeed without parent-assignment
  warnings. With the unfiltered tracer, assignment succeeds before SDK context
  starts; actual genuine lifecycle failures remain diagnosed.
- Both executable roots collect only the remaining immutable inputs. Inherited
  local overrides do not clamp OTLP spans or alter AlwaysOn recording. Retain
  endpoint precedence and statuses without unsafe test-process environment
  mutation.
- Existing real field-retention, initial/late formatter-failure, pretty/JSON,
  absent-output, WARN+/panic and metric controls stay green. No privacy payload
  classes are broadened; no diagnostic is routed recursively through OTLP.
- Actual short-lived production CLI and test-support export their generated
  spans after exit; provider/metric shutdown and capture scope/layout remain
  intact. Rerun the original working-tree quiet-log request loop and the focused
  quiet-log/default-recording auth browser flow, checking exported correlation
  and existing boot/capture consumers. Retain the original pre-change/deployed
  controls. Preserve SQLite/PostgreSQL parity for any storage-backed assertions.
- #1743 owner explicitly acknowledges the new local-only versus unfiltered-OTLP
  contract and #1742-first/rebase order. #1742 does not certify native log
  export before that sink exists; #1743 supplies actual
  all-severity/no-mirroring, diagnostic exclusion and logger-shutdown proofs
  after rebasing.
- Operator docs name local controls, endpoint requirements, AlwaysOn behavior
  despite upstream sampled flags, unchanged privacy/fallbacks and potential
  increased volume. SDK/Collector retention controls are operational decisions,
  not Jaunder verbosity settings. No application-level span-dropping substitute
  is introduced.
- Independent whole-branch review/conformance and final required machine gates
  cover this revised delivered tree. Keep proof artifacts ignored; commit the
  approved active spec and risk-triggered outline before behavior changes, then
  retire them after revised conformance. Do not weaken gates or preserve old
  filtering tests that assert the now-rejected policy.

## Boundaries

No production deployment, new reliable-delivery mechanism, Collector policy,
metrics filter, storage/schema, API/protocol response or user-visible
presentation change. #1755's existing local seed receiver-selection defect
remains separate; prove actual executable flushing with an owned healthy
Collector rather than claiming missing seed capture is green. Native
logger/routing implementation remains #1743 work, but its unfiltered-OTLP
contract is part of this coordinated revision. AlwaysOn intentionally replaces
the prior SDK parent-based sampling behavior; the old "sampling unchanged"
promise is superseded explicitly.
