# Observability

Native processes emit OpenTelemetry logs, traces and metrics through a shared
Collector endpoint. The end-to-end test runner also emits correlated traces.

## Backend

- Backend spans are produced via `tracing` + OpenTelemetry. Shared host-process
  OTLP setup and shutdown live in `host::telemetry`; the server, production CLI
  commands, and `test-support` all hold the same guard. Server-scoped HTTP spans
  and e2e diagnostics stay in `server::observability`.
- When an OTLP endpoint is configured, `jaunder serve` also registers saturation
  gauges and starts a 30-second sampler owned by the serve lifetime. If no OTLP
  endpoint is configured, the sampler is not started.
- In e2e VM checks, the running server, production `jaunder site-config set`
  seed steps, and the `test-support` seed binary export to the in-VM collector.
  The collector writes under the capture-dir contract (#332):
  - `/var/lib/jaunder/capture/otel-traces.jsonl` (traces inside the VM)
  - `/var/lib/jaunder/capture/otel-logs.jsonl` (native logs, separate file
    exporter)
  - lifted per lane inside `capture-<lane>.tar.gz` (the same bundle that carries
    `diag.log` and the mail/websub JSONL — see below)
- `cargo xtask e2e-local` supervises the same collector pipeline on the host,
  using per-lifecycle ephemeral OTLP receivers. After Jaunder and Playwright
  finish, it flushes the collector, retains the complete capture at
  `.xtask/e2e-local/<run-id>/<browser>/capture/`, and prints the exact
  `otel-traces.jsonl` path. These local artifacts contain correlated
  `e2e.test`/server `request` spans for iteration; the VM matrix remains the
  authoritative gate.
- Branch determinants are span attributes, not span-name suffixes. A span name
  identifies an operation; fields such as `registration.policy`,
  `registration.invite_present`, and `registration.outcome` explain the decision
  path. `InternalError` boundary failures and native swallowed-error reports
  also include `error.span_trace`, an operator-only snapshot of the active span
  stack. Retention is collector-side: configure OTel tail sampling to keep
  errored/slow traces rather than expecting Jaunder to dump buffered branch logs
  in-process.

### Independent log and trace controls

The server, production CLI commands and `test-support` resolve the same
immutable telemetry policy at process startup. Without a resolved OTLP endpoint,
ordinary logs use the existing pretty/JSON stdout formatter and local filter.
With an endpoint, ordinary logs use native OTLP records only, including when
logger setup fails: stdout mirroring is not restored. CLI command results
written to stdout remain separate and usable.

| Control                               | Policy                                                                                           |
| ------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `JAUNDER_LOG_FILTER`                  | Preferred local ordinary-log filter; otherwise `RUST_LOG`, then ordinary or `--verbose` defaults |
| `JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT` | Preferred logs/traces/metrics endpoint; otherwise `OTEL_EXPORTER_OTLP_ENDPOINT`                  |

The local filter accepts tracing filter grammar, including levels, target/span
selectors and `off`. Ordinary defaults remain WARN for application targets and
INFO for storage; `--verbose` raises application/storage targets to DEBUG and
SQLx to INFO. A configured local override takes precedence over those defaults.
Invalid local input retains its existing default fallback. Pretty and JSON
formats retain the same local-output policy.

With an endpoint configured, the OTLP trace layer admits generated application
spans and permitted span events at every level, including DEBUG and TRACE,
without application-level severity, target or context filtering. For example,
`JAUNDER_LOG_FILTER=warn` limits stdout when used without OTLP, but does not
limit configured OTLP output. Native logs also offer all application severities,
TRACE through ERROR, without a level/target/context filter. Local `off` does not
disable native logs, OTLP spans, metrics, independent WARN+ diagnostic capture
or the panic hook. No endpoint means no OTLP provider/export; local logs and
diagnostic context remain available without a Collector.

The SDK uses `Sampler::AlwaysOn` for Jaunder's spans, even when a valid incoming
W3C parent's sampled flag is clear. Its trace ID and parent span ID are
preserved; Jaunder does not recover upstream spans the caller discarded.
Endpoint precedence and provider flushing remain intact. Export is best-effort:
bounded buffering, Collector outages, exporter failures or abrupt process death
can lose data. Unfiltered admission covers instrumentation present in the
binary, not instrumentation a compiler or dependency never generated.

**Compatibility change:** `JAUNDER_LOG_FILTER`, `RUST_LOG` and `--verbose`
control local output only, not OTLP retention. There is no application trace
filter or sampling knob. Quiet local logs can accompany increased Collector
volume, including DEBUG/TRACE logs, spans and events. Budget Collector storage
and configure Collector-side sampling, filtering and retention there.

HTTP parent extraction/adoption follows the installed tracer/subscriber
capability. Without a tracer, requests do not attempt assignment. With the
unfiltered tracer, requests adopt valid W3C parents before SDK context starts;
missing/invalid headers remain ordinary root-context cases. HTTP `x-request-id`
values are preserved or generated and propagated in response headers, not
exported as span attributes. OTLP correlation uses the top-level trace ID, span
ID and parent span ID, including same-trace child operations.

Genuine assignment failures use local stderr with bounded reasons
`otel-layer-missing`, `span-already-started` or `span-disabled`, never raw
headers or arbitrary source text. SDK/exporter, configuration, formatter and
shutdown fallbacks likewise remain local and outside failing/native export paths
to avoid recursion. Error `SpanTrace` initial and late-recorded fields survive
filtered or absent stdout in either pretty or JSON mode. Unfiltered output does
not broaden privacy, redaction or sink-specific field admission: HTTP fields
remain method, path-only URI, version and bounded trusted-proxy outcome, never
queries, whole headers or raw parent input. Independent diagnostics are not OTLP
logs and must not be re-exported as ordinary application records.

See the
[Collector-owned retention decision](adr/drafts/independent-log-and-trace-filtering.md)
and [native OTLP process logs decision](adr/drafts/native-otlp-process-logs.md).

### Shared Collector privacy policy

The qualified #1743 amendment selects stock appender 0.32.0/SDK batching and
removal of application privacy/admission visitors, Debug parsing, classifiers
and the proposed SpanProcessor decorator. The
[native OTLP decision](adr/drafts/native-otlp-process-logs.md) records the
approval provenance and precise producer-qualified policy. The stock bridge and
shared policy are implemented and have focused seam proofs; this is not full
matrix/coverage/CI certification or production activation.

The canonical `nix/otel-processors.yaml` fragment supplies
`transform/jaunder_privacy` before `batch` and every durable log/trace exporter.
Production imports the fragment's processor value into the ordinary NixOS
`services.opentelemetry-collector.settings` surface, so operator tuning
continues to merge normally. Importing this fixed trusted YAML requires
evaluation-time IFD: a pinned build-platform `yq-go` converts only policy data
to JSON, without interpolating environment placeholders. Evaluation therefore
needs IFD enabled and a builder for the configuration's build platform; it does
not run the Collector just to inspect settings. The upstream NixOS module
generates, validates and runs one settings file with its normal hardening. Local
and VM launchers load the canonical YAML directly as the second Collector
configuration. Explicit arrays enforce privacy → batch; production metrics
remain batch-only. The closed checker parses YAML into the reviewed JSON value
and rejects missing/changed policy or bypass surface. Pinned Collector
validation uses the complete composed configuration. Ordinary producers match
`instrumentation_scope.name`; only facade scope `log` uses `log.target`.
Retained fields are qualified by producer, never a global union. Unknown event
fields, raw stack/header/request/body aggregates and secondary message copies
are removed. Existing safe message/typed-error constructors remain trusted;
unselected messages are not universally sanitized. Source-admitted operation
spans/determinants, browser boot/test evidence, resources, metrics and IDs
remain unchanged. Current link attributes are empty; no universal link sanitizer
is required for this composition.

Exactly `html5ever::tokenizer`, `html5ever::tree_builder` and
`html5ever::serialize` retain every log/trace-event occurrence at all severities
and context, but use `HTML parser diagnostic; text withheld` as complete body/
event name, with only producer/location/level metadata. No parser-event dropping
is selected. Safe state/EOF wording and future warning explanation are lost;
`char_ref` is outside the exact selection. Span status descriptions are cleared,
not status codes. Stock integer/string carriers remain, including decimal
`%UserId` and overflow strings; custom integer/Debug-Option parsing retires.

**Stored query interface:** VictoriaLogs exposes the native producer as
`scope.name`, message as `_msg`, and IDs as `trace_id`/`span_id`. Query
`scope.name:"tower_http::trace::on_response"` with a known `trace_id`, then
match the log's exact span ID to the stored `request` span for
URI/method/remote-parent context. Do not approximate by timestamps. Both real
backend fixtures verify `_msg="finished processing request"`, severity
`"INFO"`/`"9"` and status `"401"`. VictoriaLogs strings do not establish native
OTLP value types. Neither `jaunder.target` nor `jaunder.request.uri` is
exported. Explicit/contextual/root and sampled/unsampled parent semantics
remain; roots legitimately have no request link.

Raw application SDK data, OTLP transport and trusted Collector input are
accepted processing locations, explicitly amending ADR-0011's pre-Collector
privacy interpretation. Durable Victoria/file output **and persisted Collector
error/debug/export-failure diagnostics** must exclude withheld data. Invalid
policy fails startup without raw fallback. Disable raw debug exporters/context
logging; `error_mode: propagate` is not diagnostic redaction and may
reject/retry other batch data. Synthetic secondary-copy and failure-path proof
is mandatory before delivery, not permission to share raw captures.

The demonstrated composition retains an owning public event-context adapter,
suppression-preserving Activation, an SDK-scope-only dynamic trace-layer gate
and process-owned transport runtime. These are context/self-observation
mechanics, not an application privacy/admission filter. Preserve process-owned
bounded shutdown/one-shot draining, nonfatal outcomes,
credential/endpoint/payload-safe and nonrecursive direct diagnostics. Exact
private per-kind, coalescing and 256-byte promises retire. Collector policy
cannot sanitize SDK stderr or stop client feedback/queue pressure, and
guarantees neither volume recovery nor delivery. Local stdout, CLI results,
SpanTrace fields and independent WARN+/panic capture remain separate.

### Native log records and lifetime

Stock appender 0.32.0 owns field/body/severity conversion without experimental
metadata/span-map copying. There is no application privacy visitor, Debug/Option
parser, target admission rule or direct-log path enrichment. The Collector owns
the producer-qualified durable field profile described above.

An explicit/formatted application message becomes the string body, not a dump of
structured fields or the span stack. Native severity retains the actual tracing
level. In the supported VictoriaLogs OTLP ingestion path, severity is stored as
`severity_text` and `severity_number` (for INFO, `"INFO"` and `"9"`), not as a
`severity` attribute. These are record severity fields, separate from the
Collector's producer-qualified attribute profile:

- `error.kind`, `error.class`, `error.public`, `error.source`, `error.context`,
  `error.disposition`, `telemetry.origin`: reviewed `host::error` carriers only.
- `db.system`: scalar from application/seed producers; `user.id`: audited
  numeric identifier from registration, password-reset and email
  credential-consumption events; `status`: reviewed Tower response-completion
  target only.

Stock scalar integers/strings retain their carriers; unsigned overflow retains
exact decimal text instead of wrapping. Arbitrary event fields, Debug
aggregates, inherited span maps, headers, request aggregates, credentials,
bodies and raw stacks (including `error.span_trace`) are excluded at the shared
Collector, not by an application visitor. Message and typed error-source
constructors still owe ADR-0011's safe-text discipline; this is not blanket
sanitization of arbitrary producer text.

Trace/span correlation follows the event's resolved tracing parent: explicit
parents win over current spans, contextual events use their active parent, and
explicit roots or missing/unavailable tracer context acquire no fabricated or
ambient SDK IDs. Repeated entries of the same tracing span exited out of order
retain the pinned bridge's external-context/suppression limitation: arbitrary
entry-instance restoration is not promised. This is an accepted limitation, not
an additional caller-ordering rule.

The event adapter resolves the actual parent via owning weak dispatch and public
OTel context, even when a different dispatch is current. Activation preserves
SDK suppression while entering application spans; the dynamic trace gate
excludes only SDK-scope instrumentation from OTel, not local diagnostics. The
two-worker/two-blocking-worker transport runtime uses the process-global
subscriber; scoped worker-dispatch propagation is not promised.

All providers share the process resource, endpoint and suppressed exporter
runtime. Guard shutdown runs meter → tracer → logger → exporter runtime, keeping
transport available for one-shot draining and server teardown. Logger shutdown
uses the pinned SDK's five-second default, with no additional unbounded
pre-shutdown flush. Runtime shutdown has its own five-second best-effort budget;
neither is a whole-process deadline. Acknowledgment is not delivery proof, and
residual blocking export work can outlive a timeout.

Native batching receives a complete owned SDK `BatchConfig` snapshot resolved at
initialization. The pinned public processor builder constructs a discarded
environment-derived default before `with_batch_config` replaces it. That
constructor-only third-party read cannot change effective policy; application
callbacks, workers, reconnects and teardown do not resolve it again. Do not
infer a literal zero-third-party-read guarantee. Ordinary pinned batching
supports queue size, batch size and scheduled delay, not experimental async
export-timeout policy. Without inherited overrides, SDK defaults bound the queue
to 2048 records and batches to 512; saturation can drop records.

`SafeSdkFormat` wraps both host fmt variants and the independent server WARN+
JSON writer. At target `opentelemetry-otlp`, precisely
`Tonic{Logs,Traces,Metrics}Client.{ExportFailed,PartialSuccess}` retain actual
level/target/name, a closed set of 17 tonic code names or signed scalar rejected
counts, and fixed `remote detail withheld` message/details. Selected records
omit span copies; unrelated SDK/application records delegate unchanged,
including normal filtered context. This is not universal SDK sanitization or
global SDK hiding. Opaque setup and nonrecursive shutdown fallbacks remain.

Stock partial-success semantics remain stock: rejection counts stay visible but
are not promoted into a private exporter error. There is no per-kind/coalescing/
256-byte contract. Source-input tapes exist only in tests, not production and
not as diagnostic-output privacy exemptions. These best-effort failures do not
change application results or exit status and do not restore ordinary stdout.
See the
[deployment procedures](DESIGN.md#native-log-routing-and-message-verification)
for message queries and failure investigation.

The shared E2E Collector writes logs only to `otel-logs.jsonl`, isolated from
`otel-traces.jsonl`, `diag.log`, mail and WebSub evidence. Whole-directory
retention carries both OTLP files without changing trace analysis, boot coverage
or zero-panic consumers. `JAUNDER_CAPTURE_DIR` remains test-only; it is not a
production fallback or a browser log-export feature.

### Verifying telemetry changes

Validate the same merged Collector configuration the local launcher uses:

```bash
devtool run -- otelcol-contrib validate --config end2end/otel-collector.yaml --config nix/otel-processors.yaml
```

For a changed browser flow, use the supported focused SQLite loop, for example
`devtool run -- cargo xtask e2e-local auth.spec.ts`. This builds and validates
current CSR assets and exercises a real server/Collector/browser lifecycle; it
is not PostgreSQL parity, complete CSR lint, authoritative source coverage or a
full matrix. Use `devtool run -- cargo xtask e2e postgres chromium` for that
backend's full hermetic Chromium lane. Firefox uses the retained `e2e-lane`
producers plus `e2e-reconcile`, not a serial-special result alone. The complete
commands and final validation ladder remain in
[CONTRIBUTING](../CONTRIBUTING.md#running-e2e-tests-locally).

Inspect actual records and complete captures, not only acknowledgments. A live
workload with no selected parser events cannot certify parser occurrence
coverage; SDK isolation tests do not establish every workload's volume or
resource behavior. Preserve a failed/incomplete lane as such and diagnose its
owning source rather than treating an earlier green capture as current proof.

### VictoriaMetrics metrics transport

The supported NixOS stack sends metrics from the Collector through
`otlphttp/victoriametrics` to
`http://127.0.0.1:8428/metrics/opentelemetry/v1/metrics`. The exporter uses its
pinned protobuf default; application → Collector OTLP is unchanged.
VictoriaMetrics uses `-opentelemetry.usePrometheusNaming` so existing stored
instrument names, unit suffixes and monotonic-counter suffixes remain available.
No compatibility transform or synthetic metadata series is inserted.

The operator-selected native OTLP metadata contract uses `service_name` and
`scope_name` instead of remote-write's `job` and `otel_scope_name`. Queries that
filter those labels must use the native names. Scope metadata includes
`scope_version` (`unknown` when absent), and SDK resource attributes such as
`telemetry_sdk_language`, `telemetry_sdk_name` and `telemetry_sdk_version` are
labels on instrument series. Prometheus-generated `target_info` is absent by
design; it is not an instrument-data series. Datapoint attributes such as
bounded `kind` and `result` remain unchanged.

Native protocol defaults promote resource attributes and scope metadata. This is
not legacy-identical label/cardinality behavior or a universal bounded-label
guarantee: keep resource/scope values within the existing trusted
instrumentation discipline, and review new attributes before adding them. No
global promotion option is added. Capture-only local/E2E profiles still have no
metrics backend or metrics pipeline.

The pinned Collector/VictoriaMetrics boundary is exercised with controlled
points shaped like current host counters, gauges and cumulative histograms. That
proves the selected stored schema and instrument-data comparison, not live Rust
SDK emission, exhaustive datatype/temporality coverage, lossless delivery or
clean draining. The routing decision is recorded in
[ADR-0196](adr/0196-single-host-nixos-deployment-stack.md).

### Backend Saturation Gauges

The backend exports these asynchronous gauge instruments through
`host::metrics`:

| Instrument                              | Unit | Meaning                                                                                 |
| --------------------------------------- | ---- | --------------------------------------------------------------------------------------- |
| `jaunder.feed.queue_depth`              | —    | Feed-regeneration rows currently claimable by the feed worker                           |
| `jaunder.backup.last_success_timestamp` | `s`  | Unix timestamp of the newest successful backup artifact                                 |
| `jaunder.db.pool.used`                  | —    | Database pool connections currently checked out                                         |
| `jaunder.db.pool.idle`                  | —    | Database pool connections currently idle                                                |
| `jaunder.db.pool.max`                   | —    | Configured maximum database pool connections                                            |
| `jaunder.media.storage_bytes`           | `By` | Database-declared bytes for local uploaded media, used for upload accounting and quotas |
| `jaunder.media.filesystem_bytes`        | `By` | Logical length of every regular directory entry below `<storage_path>/media`            |

The sampler writes a shared snapshot; OpenTelemetry callbacks only read that
snapshot and emit a datapoint for fields that are present. A failed source
clears only its field and reports a fixed diagnostic context, so dashboards see
absence rather than a misleading zero. An unconfigured backup destination is
normal and therefore clears `jaunder.backup.last_success_timestamp` without a
diagnostic.

`jaunder.media.storage_bytes` remains the storage table's declared upload total:
the database-only value used for upload accounting and quotas. It does not walk
the media directory and does not include cached remote media.

`jaunder.media.filesystem_bytes` is a separate physical-drift diagnostic. It
walks the complete `<storage_path>/media` tree and sums the logical length of
every regular directory entry, including `upload`, `cached`, `tmp`, orphaned
files, and future descendants. Each hard-linked entry contributes its own
length: this is namespace-level logical usage, not allocated-block,
deduplicated-physical-storage, or filesystem-quota usage.

The serve-owned 30-second sampler performs an immediate first collection and
subsequent filesystem walks on Tokio's blocking-work facility, never on an HTTP
request path or an async runtime worker. It awaits each walk before starting the
next, so only one filesystem scan is in flight. The snapshot callback remains a
synchronous read.

The filesystem sample is all-or-nothing. A missing or unreadable path, traversal
or metadata error, symlink, or non-regular non-directory entry reports the
bounded `server.metrics.media_filesystem_bytes` diagnostic and clears the
snapshot field, publishing no datapoint rather than zero or a partial value.

## End-to-End Tracing Layers

### What is a span and what is an attribute (#794)

This distinction is easy to get wrong — #788's write-up said "`action.timed`
×1233 **spans**", but 1233 was the _entry count inside one attribute_. To be
exact:

- **Spans**: `e2e.test.lifecycle`, `e2e.test`, `e2e.context_mint`, `e2e.page`,
  `e2e.teardown`, `e2e.flow.*` (browser side); `request`, `storage.*`,
  `crypto.*`, `site.serve` (server and seed-process side).
- **Per-test JSON attributes on `e2e.test`**: actions (`e2e.action_top_json`),
  default-page navigations (`e2e.navigation_top_json`), resources
  (`e2e.resource_summary_json`), long tasks (`e2e.long_tasks_json`), slow
  requests (`e2e.request_top_slow_json`), and default-context browser
  diagnostics (`e2e.console_json`). **Per-secondary-page JSON attributes on
  `e2e.page`**: secondary-page navigations (`e2e.navigation_top_json`) and
  browser diagnostics (`e2e.console_json`). Both diagnostic owners also carry
  `e2e.console_dropped`. `action.timed` / `action.failed` /
  `navigation.lifecycle` / `request.slow` are span **events**, not spans.

Counting "spans named `action.timed`" therefore finds nothing; the data is
inside the attribute blobs.

### Browser diagnostics are isolated E2E observations

The Playwright harness records only browser console messages whose Playwright
type is `warning` or `error`, plus uncaught `pageerror` events. It excludes
`log`, `info`, `debug`, and every other console level. Listener delivery
synchronously normalizes each event into the ordered `BrowserDiagnosticRecord`
union; no `JSHandle` or live Playwright object enters the sink:

```ts
type BrowserDiagnosticRecord =
  | {
      kind: "console";
      type: "warning" | "error";
      text: string;
      location: { url: string; line: number; column: number };
      sequence: number;
      emittedMs: number;
    }
  | {
      kind: "pageerror";
      name: string;
      message: string;
      stack?: string;
      sequence: number;
      emittedMs: number;
    };
```

At the `pretest` → `test` phase switch, the default test sink owns the records
exported on `e2e.test`; every `tracedContext` test sink is exported on its
existing `e2e.page` span. Records delivered before that switch remain in the
pretest sink and are never exported on either test span. When the test body
ends, both default and secondary captures enter a sinkless teardown phase before
settlement, so diagnostics delivered after the recorded span end are not
misattributed to it. Diagnostics are observation-only: warnings, errors, and
page errors do not fail a test.

The harness serializes the first 20 test-phase records in sequence order as the
`e2e.console_json` JSON-string attribute, and writes
`e2e.console_dropped = total records - exported records`. Its raw text and stack
may contain synthetic application values from the disposable seeded E2E
environment. That limited diagnostic exception is not production telemetry:
production browser code captures or exports neither console nor page-error
payloads, and real-user data and infrastructure credentials remain forbidden.
See the
[isolated E2E browser-diagnostic payload decision](adr/0168-isolated-e2e-browser-diagnostic-payloads.md).

### The per-test span tree

```
e2e.test.lifecycle              first auto-fixture stamp → just before OTLP export
├── e2e.context_mint            browser context + page creation
├── e2e.test                    the test body — unchanged span id, range, attributes
│   └── request, storage.*, …   server spans, attributed by traceparent
├── e2e.page                    one per extra context opened via `tracedContext`
└── e2e.teardown                span assembly, perf read-back
```

`e2e.test` was deliberately **not** widened to cover the lifecycle. Its span id
is the #681 attribution join and its time range is what every existing analysis
— including all of #788's numbers — means by "in-span time"; widening it would
have silently redefined all of them. Reparenting it under the envelope is safe:
the analyzer matches the exact name `e2e.test` (so `e2e.test.lifecycle` cannot
collide), and the coverage extractor walks `parent_span_id` _upward_ to an
`e2e.test`-named span, so an extra ancestor changes nothing.

**Which `e2e.test` numbers remain comparable to #788.** Its span id and time
range are unchanged, and so are `e2e.request_count` and `e2e.navigation_count` —
that is exactly what the phase-tagged capture sink protects: anything a fixture
does before the test body is filed under the `pretest` phase, never in
`e2e.test`'s arrays. But `e2e.action_count` and `e2e.action_top_json` **are not
comparable**: #794 delimited the composite flows (`flow.login`,
`flow.verify_email`, …) and wrapped the previously-invisible waits, so the
action count legitimately rose. Diff the request and navigation counts across
that boundary; do not diff the action counts.

Secondary-page navigation counts are deliberately separate. `e2e.test`'s
`e2e.navigation_count` and `e2e.navigation_top_json` remain default-page-only
for ADR-0096 comparability; each instrumented `e2e.page` span carries the same
navigation count/top-list vocabulary for its own page. The canonical document
load total for an e2e trace corpus is therefore:

```
sum(e2e.test.navigation_count) + sum(e2e.page.navigation_count)
```

`cargo xtask traces analyze` treats `e2e.test` and `e2e.page` spans carrying
`e2e.navigation_top_json` as navigation-bearing for URL/phase and boot-coverage
reports. Sections that say `e2e.test` alone remain default-page-only.

**Every `e2e.`-prefixed span must carry an `e2e.project` attribute.**
`traces analyze --project <name>` drops any `e2e.`-named span whose
`e2e.project` differs from the filter, so an unstamped span reads as "belongs to
another project" and vanishes from filtered analysis.

### Attribution boundaries

Some per-test time cannot be measured from inside the fixture doing the
measuring. Playwright tears fixtures down in reverse setup order, so
`_autoPerfSpan`'s teardown—where spans are built and exported—runs before
`context.close()`. OTLP export and context teardown are outside those spans.
Compare current Playwright durations with attributed span time when
investigating this residual; a historical residual is neither a timing budget
nor a baseline.

### What the boot marks do and do not cover

Marks are harvested per navigation **when `data-mounted` is observed**, and
again at that document's `load`. The mount-ready harvest is the complete one, by
construction: `csr/src/lib.rs` emits every `jaunder.*` mark synchronously before
`mark_ready()` sets the attribute, so the observer that fires on it cannot see a
partial set — on any engine. The `load` harvest is kept because a navigation
that never mounts still reaches it and its `.wasm` resource timing is still
worth recording. The two are reconciled by `mergeDocumentTiming`, which keeps
whichever snapshot has more marks rather than whichever resolved last (#818).

Navigations that record nothing report the marks as _absent_, never as zeros, so
a missing decomposition cannot be mistaken for an instant one.

The mount-ready harvest is necessary: `goto` may finish at `domcontentloaded`
and the test may end before `load`; asynchronous WASM boot also need not finish
before `load`. Reconcile the complete executed population rather than trusting
the presence of a few captured marks.

Coverage is reported per `(source, project)` by `cargo xtask traces analyze` —
navigations, mounted, full mark sets, and dropped records — and the E2E VM gate
now fails closed on every successful backend×browser combo when the report's
executed project population and its trace evidence do not reconcile. The
unthresholded `end2end/tests/boot-marks.spec.ts` remains a mechanism check; the
host gate additionally requires current `direct-init-v1` document-frame
decomposition, 1 ms closure, and zero dropped records.

### Truncation is reported, never silent

Six lists are capped. Each emits a companion dropped-count, because raising a
cap only moves the cliff and OTLP attribute size limits are real:

| Attribute                   | Cap | Dropped count                   |
| --------------------------- | --- | ------------------------------- |
| `e2e.request_top_slow_json` | 20  | `e2e.request_top_slow_dropped`  |
| `e2e.action_top_json`       | 30  | `e2e.action_top_dropped`        |
| `e2e.navigation_top_json`   | 20  | `e2e.navigation_top_dropped`    |
| `e2e.resource_summary_json` | 20  | `e2e.resource_top_slow_dropped` |
| `e2e.long_tasks_json`       | 20  | `e2e.long_tasks_dropped`        |
| `e2e.console_json`          | 20  | `e2e.console_dropped`           |

Resource summaries, long tasks, and console diagnostics are genuinely lossy; the
first three lists can already be derived against `e2e.*_count`.
`long_tasks_json` is a **tail** slice, so it discards the earliest long tasks;
`e2e.console_json` deliberately preserves the **first** 20 records, retaining
the likely root cause rather than a later cascade. Its dropped count is the
exact remainder, never a rounded or approximate value.

### Firefox reports zero long tasks — engine limitation, not a bug

Gecko implements no `longtask` `PerformanceObserver`, so `e2e.long_tasks_json`
is always empty on Firefox and `e2e.long_tasks_dropped` is always 0. There is
nothing to fix and nothing to chase: the column is empty because the data source
does not exist in that engine. Chromium reports normally.

### Browser-side e2e spans

- `e2e.test` (automatic, from `end2end/tests/performance.ts`, composed into the
  suite surface by `end2end/tests/fixtures.ts`)
  - one span per test
  - request timing summary
  - default-page navigation lifecycle summary (`e2e.navigation_top_json`)
  - each navigation record includes `cacheWarmth` (`cold` for first document
    navigation in the test, `warm` for subsequent ones)
  - includes `commit -> mount` timing (commit → CSR mount-ready)
  - resource summary
  - timed action summary (`e2e.action_top_json`)
- `e2e.page` (automatic for extra contexts opened through `tracedContext`)
  - one span per secondary capture, normally one secondary Playwright page
  - secondary-context navigation lifecycle summary (`e2e.navigation_top_json`)
  - contributes to document-load URL/phase analysis, but not to `e2e.test`
    request/action counts
- `e2e.flow.*` (manual semantic phases, from `end2end/tests/perf.ts`)
  - opt-in for selected scenarios
  - mark-to-mark phase timing for domain-specific flow analysis

These browser-side spans share one **trace id** (from `JAUNDER_E2E_TRACEPARENT`)
so browser and backend spans are correlated in a single trace. Since #681 the
**parent span id** is per test, not run-wide: `performance.ts` mints the
`e2e.test` span id before the test body and sends
`traceparent: 00-<traceId>-<testSpanId>-01` on every context the test uses. The
server adopts that as its request span's parent. Server request spans therefore
carry the id of the test that caused them, which is the structural join the
flow-coverage gate below walks.

The run-wide `JAUNDER_E2E_TRACEPARENT` value remains installed as
`playwright.config.ts`'s static `use.extraHTTPHeaders`, so it is still what
pre-attribution traffic carries — anything issued between context creation and
the per-test traceparent being applied. That traffic is deliberately _not_
attributed to any test. Until #792 the per-test warmup was its main source; the
bucket remains because the window it covers is structural, not because the
warmup was.

A context built with `browser.newContext()` does **not** inherit config-level
`extraHTTPHeaders`, so specs must use the `tracedContext` fixture; the
`traced-context` static check enforces it.

## Per-test timing report

Each E2E VM check also runs Playwright's `json` reporter and copies the result
out as a lane-qualified flat artifact alongside the OTEL traces:

- `playwright-report-<lane>.json`

It records every test's title, project, status, retries, and duration. This is
the primary source for per-test timing comparisons across browsers. The report
lands at `.xtask/diagnostics/e2e-<lane>/playwright-report-<lane>.json`.
Chromium's unsplit producer uploads `e2e-diagnostics-<backend>-chromium`; each
retained Firefox producer uploads `e2e-firefox-<backend>-<partition-or-shard>`.

The report is paired with `duration-budget-manifest-<lane>.json`; the copied
report and manifest are the duration-pressure gate's source, not trace spans or
suite wall-clock time. For an otherwise-successful lane, their selected tests
and attempts must reconcile exactly; missing, malformed, incomplete, or
inconsistent input fails closed after diagnostics are captured. The gate
evaluates every reported attempt, including retries: any attempt at **80% or
more** of its effective whole-test timeout fails, even if a later retry passes.

Firefox adds one backend-level host reconciliation after all three lane
producers settle. It consumes the independent unsharded census plus every
lane-qualified report, manifest, phase sidecar, and capture; it rejects missing
or duplicate source identities, swapped shard evidence, failed phases, partial
traces, and incomplete retry/duration populations. The reconciliation artifact
contains only `.xtask/last-result.json`: producer artifacts already retain the
full diagnostics, so operators should inspect the named failed lane rather than
a duplicated aggregate bundle.

This is a per-combo headroom detector, not a timeout-sizing policy or aggregate
duration history. Whole-test budgets remain ambient except where a budget is
derived from a polling deadline that exceeds it; observed duration does not
justify re-sizing that deadline-derived budget.

## E2E evidence gates

The successful-combo host checks serve distinct purposes and do not substitute
for one another:

- **Duration pressure** reconciles Playwright attempts with the duration
  manifest and rejects attempts using 80% or more of their effective timeout.
- **Boot-decomposition coverage** reconciles the Playwright report's executed
  project set with `e2e.test` and `e2e.page` trace navigation evidence in that
  combo's capture archive. It certifies complete, current, non-dropped,
  document-frame-closing evidence; it sets no boot-duration budget.
- **Source coverage** is the Rust coverage denominator and measures exercised
  source lines, not browser trace completeness.
- **`#[server]` flow coverage** derives which server functions browser traffic
  reaches. It is request attribution coverage, not document boot evidence.

## `#[server]` flow coverage (#681)

Which server fns a real browser session actually drives, derived from the traces
above rather than asserted. The sole committed artifact under `docs/coverage/`
is generated and compared byte-for-byte:

| File              | Contents                                                |
| ----------------- | ------------------------------------------------------- |
| `server-fns.json` | the covered fn set, plus orphan buckets keyed by reason |

Per-test attribution remains part of extraction: an ancestor walk from a request
up `parent_span_id` to a known `e2e.test` span distinguishes test-driven traffic
from unattributed orphans. It is deliberately not persisted (#757). Test-title
sets do not reproduce — two runs of the same e2e derivation on the same tree can
disagree because a test that ends mid-navigation leaves its page booting and the
boot is truncated at a different point each run. The attribution itself is
sound, but an uncompared title list can churn or retain stale names and
therefore cannot serve as durable flow proof. Inspect a fresh capture when
per-test attribution is needed. This retires #745's two-file compromise without
changing the trace-derived coverage decision
([ADR-0081](adr/0081-empirical-server-fn-flow-coverage.md)).

A fn is identified by the **union** of two signals: its **span name** with a
matching `code.namespace`, or a request **`uri`** resolving to the fn's declared
endpoint. Attribution is an **ancestor walk** up `parent_span_id` to a known
`e2e.test` span — `uri` hits resolve in one hop, span-name hits in two.

**The span name is matched forward, from the inventory — never inverted out of
the name.** This repo has already had two naming regimes: `#[macros::server]`
derives `web.<vertical>.<ident>` today (#714), while omitting the explicit
`name` derives `__server_<ident>`, because `#[server]` relocates the annotated
body — and its `#[tracing::instrument]` — into a generated fn of that name
(`server_fn_macro`'s `to_dummy_ident`). The extractor computes every candidate
for each inventory fn and accepts any, so a regime change is a code update
rather than a silent outage. An earlier version matched one shape only and
therefore matched **nothing**, silently: `uri` covered the same fns, so the
union looked healthy. That is why
`each_signal_finds_fns_on_its_own_in_the_real_capture` measures the two signals
**separately** against the committed capture, asserting each alone covers
everything the union does.

**`code.namespace` corroborates the current explicit name and disambiguates
compatibility names.** For valid current `#[macros::server]` functions,
`web.<vertical>.<ident>` is unique: the macro requires
`web/src/<vertical>/api.rs` and rejects deeper server-function modules, so there
cannot be a current `posts::api::listing` counterpart to `posts::api`. Matching
`code.namespace` still conservatively rejects foreign or malformed trace
evidence without making extraction depend indirectly on that compile-time
placement rule. The retained `__server_<ident>` and bare `<ident>` compatibility
forms omit the vertical, so `code.namespace` remains their load-bearing
disambiguator. `(module, ident)` cannot collide — Rust forbids two items of one
name in one module.

**Two lanes, and neither is sufficient alone.** Traces exist only in the e2e
lane; fast feedback only in the static one.

- **Static** (`cargo xtask check`, `validate --no-e2e`): committed snapshot +
  allowlist + `syn` inventory. No capture, so a new `#[server]` fn with no flow
  reddens the build without an e2e run.
- **E2e** (`cargo xtask e2e sqlite chromium`): regenerates from that run's
  capture and fails on any difference from the committed snapshot.

**Regeneration is per-combo only.** Every output is lane-qualified, so the
aggregate has no filename collisions. The `sqlite`/`chromium` lane remains the
single authoritative empirical source: `cargo xtask e2e sqlite chromium`
verifies it directly, while local `cargo xtask validate` resolves that already
realized lane after all eight builds. In CI the `sqlite`/`chromium` producer
carries the drift check ([ADR-0034](adr/0034-ci-e2e-matrix-distribution.md)).

`sqlite × chromium` is authoritative because `chromium`'s `testIgnore` and
`chromium-admin`'s `testMatch` are exact complements over all spec files and no
test is browser- or backend-conditional — so one combo drops no coverage.

To regenerate and verify after adding a flow:

```bash
cargo xtask e2e sqlite chromium            # writes .xtask/diagnostics/e2e-sqlite-chromium-unsplit/capture-sqlite-chromium-unsplit.tar.gz
cargo xtask server-fn-coverage regenerate  # rewrites server-fns.json only
cargo xtask server-fn-coverage verify      # compares the capture-derived snapshot byte-for-byte
```

`server-fns.json` is the sole generated server-fn coverage artifact.
`regenerate` writes it; `verify` recomputes and compares it. Per-test
attribution remains internal to extraction and is not persisted. Note that a
second `cargo xtask e2e sqlite chromium` on an unchanged tree replays the cached
Nix derivation in seconds and re-lifts the same capture, so it cannot be used to
confirm that a regenerated artifact is stable — that is a statement about
_different_ runs, and `nix build --rebuild` is what forces one.

**Everything fails closed.** A missing, empty, or unparseable capture is an
error, never "no uncovered fns" — otherwise the failure mode the gate guards
against and the failure mode of its own plumbing would look identical. The same
rule covers a missing or unparseable committed snapshot: it is an error, not an
empty one.

**The seed capture is committed, reduced and re-runnable.** The allowlist claims
each of its entries names a fn the real suite does not drive, so the capture
that claim came from is checked in as
`xtask/src/server_fn_coverage/testdata/otel-traces-seed.jsonl` — the extractor's
unit tests run against it, rather than against hand-authored spans only. A 25 MB
capture cannot be committed, so `testdata/reduce-otel-capture.mjs` cuts it to
~610 KiB and is committed beside it: without the reduction in the repo, "the
reduction preserved the hit set" is unfalsifiable, and an entry's absence from
the hit set would be indistinguishable from the reduction having dropped it. The
script's header states exactly what it keeps and why. It keys on each span's own
`name`, `uri`, and `code.namespace` and never re-derives the `#[server]`
inventory — an earlier version did, misread `upload_media`'s
`#[server(input = MultipartFormData, endpoint = "/upload_media")]`, and silently
dropped it while every test still passed.

**The orphan bucket** records, per fn, the distinct **reasons** its unattributed
hits ended with — `unknown-parent:<span id>`, `no-parent`, or `depth-exceeded`.
Two properties, both deliberate:

- **Reasons, because "outside any test" and "attribution is broken" are the same
  _shape_ of result but opposite in meaning.** A bucket that cannot tell them
  apart hides the very failure this gate exists to catch.
- **Not counts, because a count tracks how many tests ran.** Warmup orphans
  twice per test, so any PR adding or removing an e2e test anywhere would move
  the numbers — and since the snapshot is compared byte-for-byte, that would
  make this artifact a tax on unrelated work. A reason set is a function of the
  code.

It is reported, not failed. **Since #792, expect it to be empty.** Until then it
held exactly four app-shell fns (`get_session`, `list_local_timeline`, and the
two warning-visibility fns), each carrying the single reason `unknown-parent:`
naming the run-wide traceparent's span id — that was the per-test warmup's `/`
load, issued before `applyTestTraceparent` stamped the context. Removing the
warmup removed the traffic, and the regenerated snapshot's `orphans` is `{}`.

**The mechanism stays even though its occupant is gone.** The pre-test window —
after the browser context exists, before the traceparent is applied — is
structural, and anything that ever lands in it must not be attributed to a test.
An empty bucket is the correct steady state, not dead code. An fn appearing here
again means something now issues traffic before the test proper starts; a
different reason or an unfamiliar parent id means a context lost its traceparent
or the capture is truncated.

## Server-side scoped diagnostic log — look here first (#144)

When an e2e combo fails, **read the scoped diagnostic log before the journal.**
The server writes a small, low-noise JSONL file of only its own **WARN+ events
and panics** — no kernel boot spam, no INFO request lines. It lands per combo
at:

- `/var/lib/jaunder/capture/diag.log` (inside the VM)
- `.xtask/diagnostics/e2e-<lane>/capture-<lane>.tar.gz` (the capture dir tarred
  out per lane — it contains `diag.log`; uploaded in that producer's
  lane-qualified CI artifact)

Each line is one JSON object. Tracing events use the `fmt().json()` shape;
**panic** records are distinguished by `"kind": "panic"` and carry the literal
`panicked at <location>` message plus a verbatim `location`. Enabled only when
`JAUNDER_CAPTURE_DIR` is set (the e2e VMs set it via `captureEnv` in
`nix/nixos.nix`, and the server writes `diag.log` within it — issue #227);
production leaves it unset, so the feature is inert there.

This is the artifact the **zero-panic gate** (ADR-0032) now reads for
`panicked at`, unioned with the journal and de-duped by panic location. The full
systemd journal (`jaunder-journal-<lane>.log`, `system-journal-<lane>.log`)
remains captured as the **last-resort fallback** — reach for it only when the
scoped log doesn't have what you need (e.g. a panic that fired before the app
installed its hook). See `docs/adr/` for the app-driven scoped-capture decision.

## Analysis

Use `cargo xtask traces analyze` on one or more artifact files, for example:

```bash
# Local e2e-local capture; use the exact path printed by the command.
cargo xtask traces analyze \
  .xtask/e2e-local/<run-id>/chromium/capture/otel-traces.jsonl

# VM captures are extracted from capture-<lane>.tar.gz; traces run does this.
cargo xtask traces analyze \
  sqlite-otel-traces.jsonl \
  postgres-otel-traces.jsonl
```

The analyzer reports:

- slowest spans overall
- slowest `e2e.test` spans
- top e2e action hotspots
- top navigation phase hotspots and slow targets from both `e2e.test` and
  `e2e.page` navigation JSON (including `navigation.commit_to_mount`, the commit
  → CSR mount-ready phase)
- per-project/browser e2e duration breakdown
- per-trace duration totals
- per-test span coverage: Playwright-reported duration vs the time covered by
  the lifecycle span tree, and the uncovered remainder
- **boot-decomposition coverage**, per `(source, project)`: `e2e.test` plus
  `e2e.page` navigations, how many mounted, how many carried a full mark set,
  and how many were dropped by `e2e.navigation_top_json`'s cap. Keyed on the
  trace **file** as well as the project because `projectName` is the browser and
  names no backend — keying on project alone would pool sqlite's navigations
  with postgres's. Certify this before drawing conclusions from a corpus (#818).

### `site.serve` — what the server actually served (#818)

Static assets are content-negotiated, so **what the client asked for and what
the server sent are different facts**, and only the first was recorded: the
`request` span carries `accept-encoding`, but the chosen representation had to
be re-derived from `choose_encoding`'s logic plus which `.br`/`.gz` variants the
bundle happens to embed. #818 had to do exactly that to rule out "the two
browsers were fed different bytes" as a cause of a fetch-duration asymmetry — a
question the traces should have answered directly.

`site.serve` records it: `site.path`, `site.encoding` (`br`/`gzip`/`identity` —
never absent, so a deliberately uncompressed response cannot read as a missing
field), `site.bytes`, `site.embedded`, and `site.status`.

Three things to know before using it:

- **`site.bytes` is the selected representation, not bytes on the wire.** It is
  recorded before the conditional check, so a `304` still reports the full size
  while sending no body — ~44% of `pkg/jaunder.wasm` requests are conditional.
- **`site.status` is the authoritative body-or-no-body signal.** `304` means
  nothing was sent, whatever the client later reports.
- **`site.embedded = false` is the SPA-shell fall-through**, which is how an
  asset that silently stopped being embedded becomes visible instead of
  surfacing as an unexplained shell response.

The client side pairs with it: `wasmDecodedBytes` / `wasmEncodedBytes` /
`wasmTransferBytes` on each navigation record. `decoded` is the wasm compiler's
actual input — the number bundle-size work turns on (#836) — and `decoded` far
exceeding `encoded` is how you confirm a precompressed variant was served.

**Do not write a cache check against `transferSize`.** On a revalidated response
firefox reports the full body size where chromium reports ~300 B, while both
engines send `if-none-match` at the same rate — so the field reads as a browser
behaviour difference that is not there. #818 briefly mistook it for one.
`site.status` is the engine-independent answer; use that.

### `commit_to_mount` stops at `data-mounted` — read `mount_to_settled_ms` too

**`commit_to_mount` does not include the mount-path fetches.** `csr/src/lib.rs`
sets `data-mounted` the instant `mount_to_body` returns:

```rust
mount();        // mount_to_body returns with Suspense fallbacks still in place
mark_ready();   // data-mounted set HERE
```

`goto` / `waitForMount` — and therefore `commit_to_mount` — end at that point.
The shell and route resources (`get_session`, the two warning-visibility checks,
the route's timeline) resolve _afterwards_. Anyone sizing "mount cost" from
`commit_to_mount` alone is measuring wasm fetch + compile + instantiate + init +
first render, and nothing else.

`mount_to_settled_ms` covers the remainder: mount-ready → the last mount-path
request to finish before the next navigation commits. Note the fetches are
**per-route and partly serialized** — `web/src/cockpit/component.rs` awaits the
session reconcile before fetching the timeline — so there is no single "app
settled" point in the app to hook, which is why this is derived from the request
records rather than marked. See #801 for the mount-cost work itself.

### Boot marks: prefix-discovered, unconditional

The CSR client emits `performance.mark`s at its boot boundaries via
`client::perf`. Two properties are deliberate:

- **Discovered by prefix, never by name.** The harness exports every mark
  matching `jaunder.`; the names live only in Rust. Adding a mark needs no
  TypeScript change — unlike `MOUNTED_ATTR`, whose cross-language agreement is
  only comment-enforced and can drift.
- **Unconditional, not behind a cargo feature.** They are a handful of
  microsecond-scale calls. Feature-gating them would mean the binary being
  measured is not the binary being shipped, which quietly invalidates every
  number they produce.

**Measure from `traces run`, never from `cargo xtask validate`.** `validate`
builds the `e2e-checks` aggregate, so nix realizes the four combo derivations
**concurrently** — four VMs at two workers each on one host. `traces run` builds
them one at a time (`traces/run.rs`'s nested loop). Measured 2026-08-05 on the
same tree: sqlite-chromium reported **436 s** under `validate` against **191 s**
serial, and all four `validate` combos started within 8 seconds of each other. A
suite duration read out of a `validate` log is a contention artefact, inflated
enough (~2.5×) to look like a catastrophic regression.

Host quiescence matters for the same reason: sample `/proc/loadavg` before and
after each run and discard any taken while other work — including other agent
sessions — was on the box.

To build both e2e VM checks and immediately analyze the produced traces, use:

```bash
cargo xtask traces run --top 25
```

When the question is what a single navigation costs rather than what the suite
costs, use the single-worker packages — one worker, so no contention distorts
the per-navigation numbers:

```bash
cargo xtask traces run --single-worker --top 25
```

Optional filters:

- `--top N` controls how many rows each section prints.
- `--trace TRACE_ID` restricts analysis to one trace id.
- `--single-worker` runs the per-browser single-worker packages
  (`e2e-{sqlite,postgres}-{chromium,firefox}-single-worker`) instead of the gate
  checks. These were the `-cold` family before #792, when "cold" meant "no
  warmup"; the gate is cold now too, so the worker count is the only difference
  left.
- `--browser chromium|firefox` restricts the run to one browser (default: both).
  Use this (not `--project`) to focus one browser, e.g. when debugging Firefox
  timeout pressure: `cargo xtask traces run --browser firefox`.

(`cargo xtask traces analyze` additionally accepts `--project NAME` to focus one
browser/project when analyzing already-collected trace files directly.)

### Backend axis: suite wall-clock versus server cost

Keep SQLite and PostgreSQL separate for storage/request-span analysis. Similar
suite durations do not establish similar backend costs: browser work and
parallel execution can conceal server differences. A suite-wall-clock
investigation may use one backend only after fresh measurements establish that
the omitted axis cannot change its decision. Re-establish that premise when
workload composition or the dominant client cost changes.

## Controlled experiments

Use matched control/treatment arms against named source revisions and record the
backend, browser, toolchain, dataset, worker count and host resources. Specify
the question, decision threshold and failure policy before collecting
measurements. Alternate arm order and retain repeated independent runs so cache
warming, host contention and temporal drift are visible. Distinguish whole-suite
wall-clock from per-navigation or server-only cost; do not sum overlapping
measurement frames.

Nix may reuse an already-built check without executing it. For a fresh VM
measurement use the temporary `e2eSalt` procedure in
[CONTRIBUTING](../CONTRIBUTING.md#e2esalt--the-measurement-cache-buster-792),
restore the empty salt before committing, and retain the actual commands,
revisions and outcomes with the run artifacts. Record failing arms honestly;
never silently discard retries, OOMs or failed checks to improve the result.

Keep raw captures and normalized run tables in ignored `.xtask/` or external
review artifacts. Put the conclusion and tested scope in the issue/PR. Promote
only reusable methodology and enduring decision rationale into maintained docs
or ADRs; a historical successful experiment is not a continuing regression gate.
For ongoing performance comparisons use the
[repeatable performance harness](../CONTRIBUTING.md#repeatable-performance-harness)
and its deliberately maintained baseline rather than a frozen experiment report.

## Timeout Budgeting

Whole-test budgets are ambient: an auto fixture gives every test a scaled
`DEFAULT_TEST_BUDGET_MS`, and it covers the whole suite — #270 deleted 18 of the
20 per-test budgets after measuring that they guarded nothing. The two that
remain derive their budget from polling deadlines that genuinely exceed the
ambient one.

For an individual assertion that needs longer on a slow browser, use
`slowBrowserTimeoutMs(testInfo, chromiumBudgetMs)` from
`end2end/tests/timeout-policy.ts` instead of a hard-coded timeout number.

For first document navigation in a test (typically the coldest path), use
`slowBrowserFirstNavigationTimeoutMs(testInfo, chromiumBudgetMs)`.

This applies a project-aware multiplier derived from observed p90 CSR-mount
latency so Firefox/WebKit runs get realistic budgets without increasing Chromium
timeouts unnecessarily.

There is no per-test warmup: every test's first navigation is a genuine cold
load, and the traces report it as one
([ADR-0099](adr/0099-e2e-does-not-pre-warm.md)).

### Heavy timeline fixture seeding (#210)

Heavy timeline tests seed their paginated fixtures through the `test-support`
binary ([ADR-0046](adr/0046-test-support-seed-binary.md)) rather than sequential
HTTP creation round-trips. Fixture setup is not a browser interaction
measurement; keep its cost separate when interpreting navigation traces.

## WASM Bundle Audit

Use `cargo xtask audit-wasm` to measure frontend bundle size from the
deterministic Nix `site` build output:

```bash
cargo xtask audit-wasm
```

This reports raw, gzip, and brotli sizes for:

- `pkg/jaunder.wasm`
- `pkg/jaunder.js`

Useful options:

- `--json` for machine-readable output
- `--site-path /nix/store/...-jaunder-site` to reuse a previously built site
  output

The Jiff time-model migration (#1272, 2026-09-06) requires a bundled IANA TZDB
for browser-local named-zone conversion. It increased the `-Oz` artifact to 3
102 495 bytes, so the budget was deliberately recalibrated to 3 200 000 bytes
(3.1% headroom). The ceiling remains below the same bundle's measured `-Os` (3
236 295) and `-O2` (3 279 507) outputs; losing `-Oz` therefore still fails the
gate. This is an intentional feature cost, not unexplained bundle drift.
