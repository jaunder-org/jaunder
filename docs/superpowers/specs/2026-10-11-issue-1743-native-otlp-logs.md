# Issue #1743 — native OTLP logs and meaningful VictoriaLogs messages

> Qualified Collector revision: JAU-1743-COLLECTOR-AMENDMENT-001. The human
> answer “both approved” applied to the exact #1784/#1743 two-item frontier;
> only the #1743 amendment applies here. Lead acknowledged the approval of
> technical contract SHA256
> `c5ee2ad723645acbff1cda627e3154282f95e468ff04991ae8a28cec879580f6` and its
> latest qualified decision card. This supersedes the earlier approved
> application-projection implementation contract, not its historical evidence.
> This is the working contract, not production activation or merge approval.
> Admitted synthetic calibration, accepted source integration and scoped real
> Victoria proofs support the implemented seams; final integrated-tree
> validation remains distinct.

## Outcome and ownership

Jaunder exports logs, traces and metrics through its configured OTLP Collector.
VictoriaLogs presents explicit application messages in `_msg`, not a
missing-message placeholder. Stock dependency events without bodies can still
have Victoria's placeholder; the driven response event has a real body. Without
OTLP, ordinary logs retain existing stdout formatting and local filters. With
OTLP, ordinary logs are not mirrored to stdout/journald; CLI results and
independent local WARN+/panic capture remain separate. Endpoint precedence and
process resource ownership are unchanged.

The delivered #1742 foundation supplies local-only
`TelemetryConfig::local_log_filter()`, unfiltered spans/events and AlwaysOn
recording with valid sampled/unsampled upstream IDs. #1743 consumes that
foundation, but its private runtime/context mechanics are not permanent
requirements. Local `JAUNDER_LOG_FILTER`, `RUST_LOG`, `--verbose`, `off` and
restrictive target/context directives must not clamp OTLP or independent
capture. Existing metrics, operation determinants, browser boot/test evidence,
resources, retry/time/resource budgets and zero-panic rules remain unchanged.

## Approved replacement contract

- Remove application privacy/admission visitors, target-retention rules, Debug
  parsing, classifiers and the proposed SpanProcessor decorator. Use stock
  `opentelemetry-appender-tracing` 0.32.0 (no experimental metadata/span-map
  copying) and ordinary SDK batching/providers/export. Replace deleted-helper
  tests with proofs through the actual subscriber/export/Collector interfaces.
- Use one shared production/test Collector privacy policy before `batch` and
  every durable log/trace exporter; metrics are unchanged. Canonical shared
  fragment: `nix/otel-processors.json`, processor `transform/jaunder_privacy`,
  `error_mode: propagate`. Explicit merged pipeline arrays must reference policy
  then batch; no arbitrary processor allowance or raw bypass. Update the closed
  checker, both-backend module assertions, local/VM launchers and source
  closures together during implementation.
- Raw application SDK data, OTLP transport and Collector input are accepted
  trusted processing locations. This explicitly amends ADR-0011's pre-Collector
  privacy interpretation, not permission for raw downstream retention. Durable
  Victoria/file outputs **and persisted Collector error/debug/export-failure
  diagnostics** must not contain withheld data. Disable raw debug exporters and
  context logging. Invalid policy fails startup without raw fallback;
  `propagate` may lose/retry other batch data and is not diagnostic redaction.
- Producer matching is qualified: ordinary OTLP `instrumentation_scope.name`
  identifies the producer; only within scope `log` does `log.target` identify
  the facade producer. An attribute on another scope cannot override it. This
  assumes trusted instrumentation, not hostile-producer authentication.
- Retain verified producer/location/level metadata and only the applicable
  fields below, never a global union allowlist. Discard unknown event fields,
  `error.span_trace`, span/stack/request/header/body aggregates and secondary
  message copies. Existing message/typed-error constructors remain trusted under
  ADR-0011/0156; this is not universal text sanitization.

| Event field                                                                                                           | Required producer                                                               | Representation                                                                                                                 |
| --------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `error.kind`, `error.class`, `error.public`, `error.source`, `error.context`, `error.disposition`, `telemetry.origin` | Exactly `host::error`                                                           | Existing reviewed scalar/string carriers; no map/list/aggregate                                                                |
| `db.system`                                                                                                           | Prefix `host::`, `storage::`, `web::`, `jaunder::`, `test_support::`            | Existing scalar values, not arbitrary Debug aggregates                                                                         |
| `user.id`                                                                                                             | Exactly `web::registration::api`, `web::password_reset::api`, `web::email::api` | Actual integer or reviewed bare signed-decimal `%UserId` string; no casts, malformed/None/aggregate forms or invented ID names |
| `status`                                                                                                              | Exactly `tower_http::trace::on_response`                                        | Actual scalar integer or reviewed bare decimal string; no synthetic Debug `Some(decimal)` parser                               |

Stock integer/string representations are intentional: `%UserId` remains decimal
text; unsigned overflow remains exact decimal text, never wrapped or unchecked
cast to signed integer. The custom integer-query/Debug-Option guarantees retire.

For exactly `html5ever::tokenizer`, `html5ever::tree_builder` and
`html5ever::serialize`, retain **every native log and trace-event occurrence at
all severities with context**, replacing the body/event name with
`HTML parser diagnostic; text withheld`. Retain producer/location/level metadata
only on those parser events. No event dropping, regex/state classifier or
source-line matching is selected; `char_ref` is outside this exact selection.
Known-safe state/EOF wording and future warning explanation are deliberately
lost, not silently promised. Other explicit messages remain under existing
constructor/source discipline; newly identified content-bearing producers need
review, not a speculative application filter.

Trace-event fields receive the same producer-qualified rules plus bridge target,
level and location metadata. Clear free-text span status descriptions while
retaining status codes. Preserve generated operation names, source-admitted span
attributes/determinants, IDs/times, metrics and existing browser/resource
contracts; do not introduce a global span allowlist. Remove explicit raw
stack/header/request-aggregate copies, not safe paths/determinants. Current
bridge links have empty attributes: preserve link IDs and that behavior, without
inventing a universal link sanitizer. New attributed-link producers require
review before inclusion.

## Correlation, lifetime and approved losses

Preserve explicit-parent precedence, contextual parents and explicit roots;
absent/unavailable context must not acquire fabricated or ambient IDs. Preserve
sampled/unsampled upstream correlation. Retire bespoke direct-log
`jaunder.request.uri` enrichment if it requires a metadata Layer: query the
correctly linked request trace for its safe path instead, never approximate
correlation by timestamps. Root logs legitimately lack request context. Replace
`jaunder.target` queries with calibrated standard scope/facade producer fields;
VictoriaLogs stores the native producer as `scope.name`, message as `_msg` and
correlation as `trace_id`/`span_id`. The real response fixture stores INFO as
`"INFO"`/`"9"` and status as string `"401"`, not a native OTLP type claim.

Keep process-owned providers, ordinary bounded shutdown and one-shot draining;
no unbounded flush/retry or stdout fallback. Failures remain nonfatal,
credential/endpoint/payload-safe and nonrecursive. Retire private per-kind SDK
classification, partial-rejection promotion, once-per-kind coalescing and the
256-byte line guarantee, **not** that safety floor. Collector policy cannot
sanitize SDK stderr or prevent producer feedback. Delete custom runtime,
activation, suppression and context machinery unless stock integration
**demonstrates** a necessity; retain only a separately reviewed minimal
non-filtering mechanism for an observed incompatibility. No synthetic spans,
invented IDs, permanent private-runtime assumption or custom filter revival.

## Implemented mechanics and confidence limits

The demonstrated minimum retains the owning weak-dispatch/public-context event
adapter, suppression-preserving Activation, SDK-scope-only dynamic trace-layer
gate and process-owned two-worker/two-blocking-worker transport runtime. It uses
the process-global subscriber, not scoped worker-dispatch propagation. Repeated
same-span IDs retain the pinned latest-exit limitation, not arbitrary
entry-instance restoration. Shutdown order is meter → tracer → logger → runtime;
private diagnostic producer ordering is not production ordering.

`SafeSdkFormat` wraps both actual host fmt variants and the independent server
WARN+ diagnostic JSON writer. Exactly six pinned `opentelemetry-otlp` metadata
identities, `Tonic{Logs,Traces,Metrics}Client.{ExportFailed,PartialSuccess}`,
preserve actual level/target/name, one of 17 closed tonic codes or signed scalar
rejection counts, and fixed untrusted message/details. They omit SDK span
copies. Other records delegate unchanged; no universal SDK sanitizer, global SDK
hide or per-kind/coalescing/size policy is implied. Opaque setup and
nonrecursive shutdown fallbacks remain. Source-input taps exist only in tests.

The canonical policy is installed by production, local and VM launchers before
batch for logs/traces; production metrics remain batch-only. Focused root
subscriber/failure/checker proofs and both-backend real Victoria proofs support
these seams. Exact log span-ID linkage supplies request URI/method/remote
parent; no timestamp join or synthetic log path is used. SQLite also proves
persistence. Browser workloads with no selected parser events do not replay the
private parser census. Source/bundle build proof is not full CSR lint, coverage,
CI or shipping proof.

## Acceptance obligations

1. After separately admitted bounded synthetic calibration, confirm actual
   ordinary/facade wire fields, producer branches, integer/string/overflow
   carriers, merged OTTL behavior, secondary-copy removal, correlation and
   persisted error diagnostics. An appendix/source review is not an installable
   policy or runtime proof. If normal failure paths leak payloads, hold delivery
   and select the smallest supported correction rather than weaken the floor.
2. Prove stock subscriber/provider all-level output under local WARN/off,
   restrictive directives and ordinary/verbose defaults;
   explicit/contextual/root and sampled/unsampled IDs; unchanged CLI results,
   metrics, local initial/late SpanTrace fields, WARN+/panic evidence, one-shot
   draining and nonfatal, safe/nonrecursive failures. Demonstrate any retained
   mechanical adapter.
3. Prove the shared merged policy before every durable sink; invalid
   predicates/types, bypass rejection, all parser occurrences/severities,
   non-parser controls, status codes and preserved operation/browser evidence.
   Both real-stack SQLite/PostgreSQL fixtures must query exact meaningful
   `_msg`, severity, qualified fields and correctly linked trace/span context,
   exclude synthetic private markers and duplicate ingestion, and preserve
   persistence and ingress assertions.
4. Preserve comparable before/after default VictoriaLogs message presentation
   for the deterministic safe fixture with the same query/state/viewport.
   Historical screenshots/host proofs/coverage remain evidence of their tested
   scope, not certification of this revision. Retain runtime evidence privately
   in ignored storage, not tracked reports.
5. Review the cumulative deliverable and conformance before applicable final
   current-tree proof/CI. Existing coverage, backend/browser, resource and
   failure obligations remain; merge needs fresh per-PR approval.

## Boundaries and document ownership

No production activation/injection, historical repair, storage/schema changes,
new public interface, generic forwarding/schema framework, blanket drops or
retention/resource/budget changes. This is the telemetry Collector, not #1753's
storage Collector. Collector policy does not eliminate generation, client
feedback/queue pressure or guarantee volume recovery.

The [outline](../plans/2026-10-11-issue-1743-native-otlp-logs.md) records the
risk-triggered implementation sequence. The
[numberless decision](../../adr/drafts/native-otlp-process-logs.md),
architecture and operator guides carry enduring rationale/procedures; no domain
vocabulary changes. Keep the working spec/outline through conformance, then
retire them under the established artifact lifecycle. This reconciliation
introduces no new contract change or additional human outline gate beyond the
qualified approval; material new choices return to the owner.
