# Observability

This project emits OpenTelemetry traces from the backend, host-side seed
processes, and end-to-end test runner.

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
  - `/var/lib/jaunder/capture/otel-traces.jsonl` (inside the VM)
  - lifted per combo inside `capture-<backend>.tar.gz` (the same bundle that
    carries `diag.log` and the mail/websub JSONL — see below)
- Branch determinants are span attributes, not span-name suffixes. A span name
  identifies an operation; fields such as `registration.policy`,
  `registration.invite_present`, and `registration.outcome` explain the decision
  path. `InternalError` boundary failures and native swallowed-error reports
  also include `error.span_trace`, an operator-only snapshot of the active span
  stack. Retention is collector-side: configure OTel tail sampling to keep
  errored/slow traces rather than expecting Jaunder to dump buffered branch logs
  in-process.

### Backend Saturation Gauges

The backend exports these asynchronous gauge instruments through
`host::metrics`:

| Instrument                              | Unit | Meaning                                                       |
| --------------------------------------- | ---- | ------------------------------------------------------------- |
| `jaunder.feed.queue_depth`              | —    | Feed-regeneration rows currently claimable by the feed worker |
| `jaunder.backup.last_success_timestamp` | `s`  | Unix timestamp of the newest successful backup artifact       |
| `jaunder.db.pool.used`                  | —    | Database pool connections currently checked out               |
| `jaunder.db.pool.idle`                  | —    | Database pool connections currently idle                      |
| `jaunder.db.pool.max`                   | —    | Configured maximum database pool connections                  |
| `jaunder.media.storage_bytes`           | `By` | Database-declared bytes for local uploaded media              |

The sampler writes a shared snapshot; OpenTelemetry callbacks only read that
snapshot and emit a datapoint for fields that are present. A failed source
clears only its field and reports a fixed diagnostic context, so dashboards see
absence rather than a misleading zero. An unconfigured backup destination is
normal and therefore clears `jaunder.backup.last_success_timestamp` without a
diagnostic.

`jaunder.media.storage_bytes` is the storage table's declared upload total. It
does not walk the media directory and does not include cached remote media. The
separate on-disk media usage follow-up is tracked as #1103.

## End-to-End Tracing Layers

### What is a span and what is an attribute (#794)

This distinction is easy to get wrong — #788's write-up said "`action.timed`
×1233 **spans**", but 1233 was the _entry count inside one attribute_. To be
exact:

- **Spans**: `e2e.test.lifecycle`, `e2e.test`, `e2e.context_mint`, `e2e.page`,
  `e2e.teardown`, `e2e.flow.*` (browser side); `request`, `storage.*`,
  `crypto.*`, `site.serve` (server and seed-process side).
- **Per-test JSON attributes on `e2e.test`**: actions (`e2e.action_top_json`),
  navigations (`e2e.navigation_top_json`), resources
  (`e2e.resource_summary_json`), long tasks (`e2e.long_tasks_json`), slow
  requests (`e2e.request_top_slow_json`). `action.timed` / `action.failed` /
  `navigation.lifecycle` / `request.slow` are span **events**, not spans.

Counting "spans named `action.timed`" therefore finds nothing; the data is
inside the attribute blobs.

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

**Every `e2e.`-prefixed span must carry an `e2e.project` attribute.**
`traces analyze --project <name>` drops any `e2e.`-named span whose
`e2e.project` differs from the filter, so an unstamped span reads as "belongs to
another project" and vanishes from filtered analysis.

### The attribution floor — measured at ~176 ms/test

Some per-test time cannot be measured from inside the fixture doing the
measuring. Playwright tears fixtures down in reverse setup order, so
`_autoPerfSpan`'s teardown — where the spans are built and `exportSpans` POSTs —
runs _before_ `context.close()`. The OTLP export itself and the context teardown
are therefore outside every span, permanently.

Measured on `sqlite × chromium`, 127 tests
(`cargo xtask traces analyze --playwright-report …`):

|                                  | before (#788) | after (#794)                        |
| -------------------------------- | ------------- | ----------------------------------- |
| per-test time outside every span | **28–31 %**   | **3.9 %**                           |
| absolute residual, per test      | —             | mean 176 ms, p50 171 ms, max 373 ms |

**The residual behaves like a floor, not a cost.** Correlation between a test's
reported duration and its unattributed time is **0.21** — near zero. A
proportional overhead would correlate near 1; a fixed per-test cost correlates
near 0. That is the evidence for calling this structural rather than merely
un-instrumented, and it is why the high _percentages_ in the report all sit on
short tests (a 1.6 s test with a 270 ms floor reads as 16 %, the same 270 ms).

No threshold is gated on this. The number is recorded so a future change that
moves it is visible; it was measured after the mechanism existed rather than
guessed beforehand.

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

**Harvesting at `load` alone — the pre-#818 design — was dark on firefox
entirely.** Two compounding defects:

1. `load` frequently never fires. `goto` waits only for `domcontentloaded`, so
   the test navigates away or ends first. This capped chromium at ~34% of
   navigations.
2. When `load` _does_ fire on firefox, it lands before boot has reached
   `jaunder.boot.entry`. `csr/index.html` starts the wasm from a module script
   that never awaits `init(...)`, so the fetch does not block `load` — and Gecko
   lost that race every single time.

Measured over the whole preserved #792 corpus (`e2e.boot_marks_json`, all 210
navigations per combo, `e2e.navigation_top_dropped = 0` throughout):

| combo (sqlite)           | marks-per-nav histogram                                   |
| ------------------------ | --------------------------------------------------------- |
| b1 / b2 / b3 chromium    | `{0: 138, 4: 72}` / `{0: 135, 4: 75}` / `{0: 139, 4: 71}` |
| b1 / b2 / b3 firefox     | `{0: 210}` — all three                                    |
| a1 chromium (warmup arm) | `{0: 151, 4: 59}`                                         |
| a1 firefox (warmup arm)  | `{0: 210}`                                                |

Chromium was strictly all-or-nothing, last mark always `mount_done`. Firefox was
**0 on 210/210, in both arms, across every run** — a systematic blackout, not a
sampling shortfall. **That corpus therefore contains no firefox boot
decomposition at all**, which is why #818 could not be answered from it and had
to re-measure. Nothing noticed for two issues' worth of work because
`bootPhases` is written by the harness and read by nothing.

Coverage is now _reported_ per `(source, project)` by
`cargo xtask traces analyze` — navigations, mounted, full mark sets, dropped —
and an unthresholded e2e assertion (`end2end/tests/boot-marks.spec.ts`) reddens
the gate if the mechanism breaks outright. It is **not yet gated on a
threshold**, so gradual erosion would still pass; that is
[#831](https://github.com/jaunder-org/jaunder/issues/831).

### Truncation is reported, never silent

Five lists are capped. Each now emits a companion dropped-count, because raising
a cap only moves the cliff and OTLP attribute size limits are real:

| Attribute                   | Cap | Dropped count                   |
| --------------------------- | --- | ------------------------------- |
| `e2e.request_top_slow_json` | 20  | `e2e.request_top_slow_dropped`  |
| `e2e.action_top_json`       | 30  | `e2e.action_top_dropped`        |
| `e2e.navigation_top_json`   | 20  | `e2e.navigation_top_dropped`    |
| `e2e.resource_summary_json` | 20  | `e2e.resource_top_slow_dropped` |
| `e2e.long_tasks_json`       | 20  | `e2e.long_tasks_dropped`        |

The last two are the ones that were genuinely lossy — the first three could
already be derived against `e2e.*_count`. `long_tasks_json` is a **tail** slice,
so it is the _earliest_ long tasks that are discarded.

### Firefox reports zero long tasks — engine limitation, not a bug

Gecko implements no `longtask` `PerformanceObserver`, so `e2e.long_tasks_json`
is always empty on Firefox and `e2e.long_tasks_dropped` is always 0. There is
nothing to fix and nothing to chase: the column is empty because the data source
does not exist in that engine. Chromium reports normally.

### The two layers

- `e2e.test` (automatic, from `end2end/tests/fixtures.ts`)
  - one span per test
  - request timing summary
  - navigation lifecycle summary (`e2e.navigation_top_json`)
  - each navigation record includes `cacheWarmth` (`cold` for first document
    navigation in the test, `warm` for subsequent ones)
  - includes `commit -> mount` timing (commit → CSR mount-ready)
  - resource summary
  - timed action summary (`e2e.action_top_json`)
- `e2e.flow.*` (manual semantic phases, from `end2end/tests/perf.ts`)
  - opt-in for selected scenarios
  - mark-to-mark phase timing for domain-specific flow analysis

Both layers share one **trace id** (from `JAUNDER_E2E_TRACEPARENT`) so browser
and backend spans are correlated in a single trace. Since #681 the **parent span
id** is per test, not run-wide: `fixtures.ts` mints the `e2e.test` span id
before the test body and sends `traceparent: 00-<traceId>-<testSpanId>-01` on
every context the test uses, and the server adopts that as its request span's
parent. Server request spans therefore carry the id of the test that caused
them, which is the structural join the flow-coverage gate below walks.

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

Each e2e VM check also runs Playwright's `json` reporter and copies the result
out as a flat artifact (alongside the OTEL traces above):

- `playwright-report-sqlite.json` / `playwright-report-postgres.json`

It records every test's title, project (browser), status, retries, and duration.
This is the primary source for per-test timing comparisons across browsers (e.g.
the Firefox-vs-Chromium analysis in #152). On the
`cargo xtask e2e <backend> <browser>` path it lands per combo at
`.xtask/diagnostics/e2e-<backend>-<browser>/playwright-report-<backend>.json`
and is uploaded as the `e2e-diagnostics-<backend>-<browser>` CI artifact.

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

**`code.namespace` is the disambiguator, not the name.**
`web.<vertical>.<ident>` uses the module's _first_ segment, so `posts::api` and
`posts::api::listing` both render `web.posts.…`; the name alone could not
separate a same-named fn in each. `(module, ident)` cannot collide at all — Rust
forbids two items of one name in one module.

**Two lanes, and neither is sufficient alone.** Traces exist only in the e2e
lane; fast feedback only in the static one.

- **Static** (`cargo xtask check`, `validate --no-e2e`): committed snapshot +
  allowlist + `syn` inventory. No capture, so a new `#[server]` fn with no flow
  reddens the build without an e2e run.
- **E2e** (`cargo xtask e2e sqlite chromium`): regenerates from that run's
  capture and fails on any difference from the committed snapshot.

**Regeneration is per-combo only.** `checks.e2e` is a `symlinkJoin` over every
`e2e-*` check and both sqlite combos emit a file named `capture-sqlite.tar.gz`,
so in the joined output the two collide unpredictably. Only
`cargo xtask e2e sqlite chromium` regenerates or verifies; the local aggregate
`cargo xtask validate` skips it, and the static lane still runs there. In CI the
`{backend}×{browser}` matrix (ADR-0034) means the `sqlite`/`chromium` job is the
one that carries the drift check.

`sqlite × chromium` is authoritative because `chromium`'s `testIgnore` and
`chromium-admin`'s `testMatch` are exact complements over all spec files and no
test is browser- or backend-conditional — so one combo drops no coverage.

To regenerate and verify after adding a flow:

```bash
cargo xtask e2e sqlite chromium            # writes .xtask/diagnostics/e2e-sqlite-chromium/capture-sqlite.tar.gz
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
- `.xtask/diagnostics/e2e-<backend>-<browser>/capture-<backend>.tar.gz` (the
  capture dir tarred out per combo — it contains `diag.log`; uploaded in the
  same `e2e-diagnostics-<backend>-<browser>` CI bundle)

Each line is one JSON object. Tracing events use the `fmt().json()` shape;
**panic** records are distinguished by `"kind": "panic"` and carry the literal
`panicked at <location>` message plus a verbatim `location`. Enabled only when
`JAUNDER_CAPTURE_DIR` is set (the e2e VMs set it via `captureEnv` in
`flake.nix`, and the server writes `diag.log` within it — issue #227);
production leaves it unset, so the feature is inert there.

This is the artifact the **zero-panic gate** (ADR-0032) now reads for
`panicked at`, unioned with the journal and de-duped by panic location. The full
systemd journal (`jaunder-journal-<backend>.log`,
`system-journal-<backend>.log`) remains captured as the **last-resort fallback**
— reach for it only when the scoped log doesn't have what you need (e.g. a panic
that fired before the app installed its hook). See `docs/adr/` for the
app-driven scoped-capture decision.

## Analysis

Use `cargo xtask traces analyze` on one or more artifact files, for example:

```bash
# each otel-traces.jsonl is extracted from an e2e capture-<backend>.tar.gz bundle
# (member path capture/otel-traces.jsonl); `cargo xtask traces run` does this for you.
cargo xtask traces analyze \
  sqlite-otel-traces.jsonl \
  postgres-otel-traces.jsonl
```

The analyzer reports:

- slowest spans overall
- slowest `e2e.test` spans
- top e2e action hotspots
- top navigation phase hotspots and slow targets (including
  `navigation.commit_to_mount`, the commit → CSR mount-ready phase)
- per-project/browser e2e duration breakdown
- per-trace duration totals
- per-test span coverage: Playwright-reported duration vs the time covered by
  the lifecycle span tree, and the uncovered remainder
- **boot-decomposition coverage**, per `(source, project)`: navigations, how
  many mounted, how many carried a full mark set, and how many were dropped by
  `e2e.navigation_top_json`'s cap. Keyed on the trace **file** as well as the
  project because `projectName` is the browser and names no backend — keying on
  project alone would pool sqlite's navigations with postgres's. Certify this
  before drawing conclusions from a corpus (#818).

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

## #818 — why firefox is slower: wasm compile (findings, 2026-08-05)

**Verdict: actionable, and it is wasm compile+instantiate.** Firefox spends
~430–475 ms compiling the 5.1 MiB wasm bundle where chromium spends ~22–35 ms —
a 14–21× ratio that accounts for the whole boot gap. The Rust-side mount path is
**1.7–12.7 ms** and is not where the time goes. Lever and follow-up: #836.

### What had to be fixed first

#818 assumed the question was answerable from #792's corpus. It was not: the
boot marks were harvested at `load`, and **firefox recorded zero marks on
210/210 navigations of every run in that corpus**. See §"What the boot marks do
and do not cover" for the two defects and the fix. This section's data is a
fresh corpus collected after it.

### Method

12 captures, `sqlite × {chromium, firefox}`, on a quiesced host, all six runs
distinctly salted so nix could not replay a cached suite. Two settings sets,
interleaved run-by-run:

- **deciding** — the single-worker packages (`workers=1`), because #818 is a
  per-navigation question and the suspect phases are exactly the CPU-bound ones
  two workers on a 2-core VM inflate. **Fixed as the deciding set before
  collection**, so it could not be chosen after seeing which read better.
- **confirming** — the gate checks (`workers=2`), to show the phase ratios
  survive the worker count and to connect the finding to the gate's wall-clock.

Suite wall-clock (seconds), and note the two sets genuinely differ — which is
why the choice was pre-registered:

| set           | chromium              | firefox               | ff/chr             |
| ------------- | --------------------- | --------------------- | ------------------ |
| single-worker | 342.1 / 345.9 / 347.7 | 473.0 / 467.1 / 469.3 | 1.38 / 1.35 / 1.35 |
| gate          | 181.0 / 179.1 / 180.0 | 270.9 / 276.5 / 269.0 | 1.50 / 1.54 / 1.49 |

Within-arm spread <2%. Corpus certified before analysis: **100%** of mounted
navigations carried a full mark set, `dropped = 0`, **0 closure violations in
2496 navigations**.

### The analysis target is the document-frame boot total

`commit_to_mount` is Node-side `Date.now()`; every segment is document-relative
(`performance.timeOrigin`). Mixing them would charge cross-process, plausibly
engine-asymmetric harness latency to the app's boot phases — the rule is
[ADR-0100](adr/0100-measurement-frames-are-not-mixed.md). The target is
`bootTotalMs` = `jaunder.boot.mount_done`'s `startTime`, decomposed into six
segments that sum to it **exactly**.

### Where the gap lives (means, deciding set)

Means, not medians, for the share arithmetic: means are linear, so the segments
close to the total exactly. Medians are quoted alongside as robust cross-checks.

**cold** — boot total chr 557.5 ms → ff 891.8 ms (1.60×), gap **334.3 ms**:

| segment                      | chromium | firefox   | Δ          | share       |
| ---------------------------- | -------- | --------- | ---------- | ----------- |
| doc start → wasm fetch start | 272.3    | 318.5     | +46.3      | 13.8%       |
| wasm fetch                   | 250.1    | 99.6      | −150.5     | **−45.0%**  |
| **wasm compile+instantiate** | **24.0** | **467.1** | **+443.0** | **132.5%**  |
| rust boot (3 marks)          | 11.1     | 6.6       | −4.5       | −1.3%       |
| _residual_                   |          |           |            | **0.0000%** |

**warm** — chr 363.5 → ff 724.4 (1.99×), gap **360.9 ms**: compile+instantiate
22.2 → 475.4 ms = **125.6%** of the gap; wasm fetch −38.0%; rust boot +0.4%.

Shares exceed 100% because **firefox's wasm fetch is a negative contributor** —
2.5–4.7× faster than chromium's. A segment where firefox is faster is shown as
negative, never dropped.

### That split is suspect, so here is the robust version

Chromium fetch 250 ms / instantiate 24 ms against firefox 100 ms / 467 ms means
the fetch/instantiate boundary sits in a different place on each engine.
Combining the two neutralises the question of where each engine books that work:

| set / warmth     | chr fetch+compile | firefox  | share of gap |
| ---------------- | ----------------- | -------- | ------------ |
| deciding, cold   | 274.1 ms          | 566.6 ms | **87.5%**    |
| deciding, warm   | 196.5 ms          | 512.5 ms | **87.6%**    |
| confirming, cold | 207.6 ms          | 523.1 ms | **82.2%**    |
| confirming, warm | 145.4 ms          | 500.4 ms | **80.5%**    |

The conclusion does not depend on the fetch/instantiate boundary.

#### Why the boundary moves is still unexplained — three hypotheses, all refuted

This section originally said the split was "what streaming compilation looks
like", with chromium compiling during download. **That explanation was wrong and
is withdrawn.** Three candidate causes have now been tested and all three fail:

| hypothesis                                    | verdict          | evidence                                                                                                                                                                                        |
| --------------------------------------------- | ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| firefox falls back off `instantiateStreaming` | **contradicted** | The fallback triggers on a wrong MIME type and logs a `console.warn` naming it. `serve_site` sends `application/wasm`, asserted at two levels (`site.rs` unit + `serve_site` response test).    |
| brotli decode explains it                     | **refuted**      | Both engines send an identical `accept-encoding` on 230/230 wasm requests, and both report `decodedBodySize = 5 350 591` / `encodedBodySize = 862 755` — the on-disk artifacts, byte for byte.  |
| different caching behaviour                   | **refuted**      | Both engines send `if-none-match` on ~44% of wasm requests. Firefox's `transferSize` reads full-size on cached responses where chromium's reads ~300 B, but that is a **reporting** difference. |

Server-side cost is identical too: `busy_ns` median 0.12–0.13 ms on both
engines.

So the two engines are demonstrably fed the same bytes, over the same wire, by a
server doing the same work — and the remaining difference is internal to how
each engine decodes, compiles, and attributes that work against `responseEnd`.
**That is recorded as unexplained rather than given a fourth story.** Two
plausible mechanisms have already died here; the combined figure above is what
the finding rests on precisely so it does not need one.

### The verdict, against rules fixed before collection

**Diagnosis — `wasm compile+instantiate` dominates.** The pre-registered bar was
≥40% of the gap **and** ≥1.5× the next largest segment. Observed: 132.5% /
125.6% (deciding, cold/warm) and 102.8% / 94.2% (confirming), against a
next-largest of 45.0% / 38.0% / 20.6% / 19.1% — dominance factors 2.9× to 5.0×,
agreeing across cold and warm and across both settings sets.

**Disposition — actionable, not intrinsic.** SpiderMonkey's compile _throughput_
is not ours; the **volume** it must compile is: 5.1 MiB raw
(`cargo xtask audit-wasm`), ~88 ms of firefox compile per MiB.

> **Superseded by #836 — do not reuse the 88 ms/MiB figure.** It is an _average_
> (449 ms ÷ 5.10 MiB), and treating it as a _marginal_ rate overpredicts by ~3×.
> Measured directly against three build arms, firefox instantiate is
> `≈ 377 ms + 13.9 ms/MiB` — at the shipped size, 92% size-independent. The
> disposition above is therefore too strong: volume is a real but weak lever.
> See [#836](#836--shrinking-the-wasm-bundle-2026-08-06), "Observed".

Both follow-ups have now been taken up. The streaming question was **answered
and the claim withdrawn** — see
[`site.serve`](#siteserve--what-the-server-actually-served-818) and #840; three
candidate explanations were tested and all three failed, so the
fetch/instantiate split is recorded as unexplained. The volume lever was
exercised in #836 — see [#836](#836--shrinking-the-wasm-bundle-2026-08-06)
below, which cut the bundle 57.6% and, in doing so, found that most of the
removed bytes were never compiler input at all.

**This redirects #801.** #801 attacks CSR mount cost generally; the Rust mount
path is 1.7–12.7 ms. Sizing that work from `commit_to_mount` without the
decomposition would have aimed at the wrong target.

### Frame skew — reported, never decomposed

Median `commitToMountMs − bootTotalMs`, deciding set:

| engine   | cold     | warm     |
| -------- | -------- | -------- |
| chromium | 384.8 ms | 337.8 ms |
| firefox  | 174.0 ms | 195.0 ms |

Large, and **larger on chromium**, so it _shrinks_ the apparent gap when
measured in `commitToMountMs` — the #792-era `commitToMountMs` ratios understate
the document-frame gap. This is harness cost (event delivery + the mount→binding
round trip), not app boot, which is why it is never folded into a segment.

### Two pre-registered rules that were wrong, and what replaced them

Both are recorded because a rule fixed in advance is only worth something if
breaking it is visible.

- **The loadavg discard rule (>3.0 at either sample) was confounded.** Samples
  are taken between back-to-back runs, so they measure the _finishing run's own
  VM_, not ambient contention — systematically: after gate runs 2.69/3.32/2.25,
  after single-worker runs 1.40/1.52/1.42. One sample breached 3.0; re-taking
  would have re-rolled the same self-load. Replaced by within-arm consistency
  (<2% spread, the standard #792 used); the breaching run is not a duration
  outlier.
- **The 1% residual rule conflated per-navigation closure with median
  additivity.** Closure holds per navigation _exactly_ (0/2496 violations), but
  `median(a+b) ≠ median(a)+median(b)`, so median-based shares cannot close.
  Replaced by mean-based shares, which are linear and close to 0.0000%.

### Observer effect of the mount-ready harvest (AC16)

The fix added a `page.evaluate` at mount-ready, with mount-path fetches still in
flight. Against #792 arm B (also 2-worker), gate-settings subset:

| population    | #792 arm B | #818     | delta     | n     |
| ------------- | ---------- | -------- | --------- | ----- |
| chromium warm | 283.0 ms   | 285.0 ms | **+0.7%** | 37→36 |
| firefox warm  | 308.0 ms   | 309.0 ms | **+0.3%** | 39→40 |
| chromium cold | 90.0 ms    | 110.0 ms | +22.2%    | 6→5   |
| firefox cold  | 54.0 ms    | 70.0 ms  | +29.6%    | 12→15 |

No material effect on the well-powered warm populations. The cold populations
are **n=5–15** — too small to separate a real effect from noise, and the
absolute shifts (+16–20 ms) are small against ~1000 ms boots. Stated as
underpowered rather than passed or failed.

### Reproducing

```sh
# per run: set a distinct e2eSalt in flake.nix, then per browser
nix build --print-out-paths --no-link .#packages.x86_64-linux.e2e-sqlite-firefox-single-worker
tar -xzf <out>/capture-sqlite.tar.gz capture/otel-traces.jsonl
# then
cargo xtask traces analyze <files…>      # certify coverage first
cargo xtask traces boot-phases <files…>  # per-(source,project,warmth) medians
```

`traces boot-phases` reports medians; the mean-based signed shares above were
aggregated over the same JSONL. Corpus:
`~/measurements/jaunder/issue-818-firefox-boot-phases/`.

## #836 — shrinking the wasm bundle (2026-08-06)

Acting on #818's lever: firefox's wasm compile+instantiate is 80.5–87.6% of the
boot gap, at ~88 ms per MiB of raw wasm, so raw bundle size is a direct handle
on the e2e gate's long pole.

**Raw bytes are the target, not compressed.** Brotli governs transfer; the wasm
compiler's input is the decompressed artifact. This is the whole reason the
budget below measures raw — see ADR-0106.

### Attribution first, and why it mattered

`cargo xtask audit-wasm --breakdown` was built before anything was changed. It
reports every wasm section's on-disk span — asserted to sum exactly to the file
size — then a per-crate rollup within the code section with an explicit
`<unattributed>` bucket.

It measures `.#csrWasm` (pre-wasm-bindgen, unstripped), **not** the shipped
bundle, because attribution reads the name section and `wasm-opt` strips that
from what ships. Its total is therefore not the download weight, and the tool
says so in its own output.

Measuring first overturned the plan. Four `common` dependency clusters had been
scheduled for a move to `host` on the theory that they were bulking up the
bundle:

| cluster     | deps                                               | code bytes |
| ----------- | -------------------------------------------------- | ---------- |
| syndication | `rss`, `atom_syndication`, `quick-xml`             | **0**      |
| markup      | `orgize`, `pulldown-cmark`, `ammonia`, `html5ever` | **0**      |
| etag        | `sha2`                                             | **0**      |
| kdf         | `argon2`                                           | **0**      |

**All four were already gone.** LTO plus `opt-level = 'z'` had eliminated every
one; `common` itself contributes 27 KiB of a 2.6 MiB code section. The largest
and riskiest part of the plan would have bought nothing. Filed as #855 on
layering grounds, which stand independently of bytes; the structural version is
#847.

Where the code actually is (code section, 2.6 MiB):

| crate            | bytes   | share |
| ---------------- | ------- | ----- |
| `reactive_graph` | 683 KiB | 25.7% |
| `tachys`         | 419 KiB | 15.8% |
| `core`           | 349 KiB | 13.1% |
| `alloc`          | 157 KiB | 5.9%  |
| `serde_json`     | 145 KiB | 5.5%  |
| `<unattributed>` | 120 KiB | 4.5%  |
| `web`            | 111 KiB | 4.2%  |

The Leptos reactive runtime is the payload. `serde_json` is the only
application-level entry in the top ten — filed as #856.

**One caveat on the tool itself.** `<unattributed>` first measured 1.0 MiB
(40%). That was a defect in the attribution, not a property of the bundle:
trait-impl methods demangle to `<Self as Trait>::method`, which carries no crate
in leading position, and they are among the commonest symbols in any Rust
binary. Attributing them to the self type's crate (falling back to the trait's
for primitive self types) dropped the bucket to 120 KiB. The figures above are
post-fix.

### What actually worked: `wasm-opt`

The build had **never** run an optimisation pass. Adding one to
`devtool csr-bundle` — the single implementation shared by the host build and
the Nix derivation — cut the shipped `pkg/jaunder.wasm`:

| build     | raw bytes     | vs none    | brotli  |
| --------- | ------------- | ---------- | ------- |
| none      | 5 350 591     | —          | 860 KiB |
| `-O2`     | 2 390 164     | −55.3%     | 606 KiB |
| `-Os`     | 2 357 119     | −55.9%     | 599 KiB |
| **`-Oz`** | **2 267 063** | **−57.6%** | 603 KiB |

`-Oz` is pinned. Verified the optimised bundle still runs: e2e sqlite/firefox,
137 passed, 0 flaky.

Target features are passed explicitly as `(rustc cfg, binaryen flag)` pairs
because **the two vocabularies diverge** — rustc reports `nontrapping-fptoint`,
binaryen calls it `nontrapping-float-to-int` and rejects the rustc spelling with
`Unknown option`, failing the build outright rather than under-optimising
quietly.

### The saving is mostly metadata, and that matters

Splitting the two effects, by building `-Oz -g` (optimised, name section kept):

| arm | build    | raw bytes | saved vs previous |
| --- | -------- | --------- | ----------------- |
| A   | none     | 5 350 591 | —                 |
| B   | `-Oz -g` | 3 614 222 | 1 736 369 (code)  |
| C   | `-Oz`    | 2 267 063 | 1 347 159 (names) |

So **1.28 MiB of the 3.08 MiB saved is the wasm name section** — a custom
section engines skip rather than compile. Applying #818's 88 ms/MiB to the whole
delta would predict ~259 ms of firefox compile saved, but that figure assumes
every removed byte was compiler input, and 44% of them were not.

**Pre-registered prediction, written before any capture is run** (the #840
lesson: a claim inferred from shape is not a tested claim):

| contrast | bytes removed | naive 88 ms/MiB prediction | what we expect                                          |
| -------- | ------------- | -------------------------- | ------------------------------------------------------- |
| A→B      | 1 736 369     | ~146 ms                    | roughly holds — this is real code                       |
| B→C      | 1 347 159     | ~113 ms                    | **near zero** — engines do not compile the name section |
| A→C      | 3 083 528     | ~259 ms                    | ~146 ms, i.e. the A→B figure                            |

If B→C comes in near zero, the honest headline is that #836 cut **download
weight** by 57.6% and **compile time** by rather less. That would also mean the
88 ms/MiB constant from #818 is only valid over compiled bytes, and should be
restated that way for whoever uses it next.

Capture protocol is #818's, unchanged: single-worker sqlite × both browsers,
three runs per arm, distinct `e2eSalt` per run, quiesced host, arms interleaved
rather than run in blocks.

### Historical residual analysis — the name-section half held, the code half did not

Captured 2026-08-06, 18 runs, corpus at
`~/measurements/jaunder/issue-836-wasm-shrink/` (README carries the
certification and the limits). Coverage: full mark set on 100% of 3744 mounted
navigations, `dropped = 0`, 0 closure violations in 72 populations. Arms
verified in-trace — `wasmDecodedBytes` takes exactly one value per arm.

This historical analysis used the field formerly labelled `wasmInstantiateMs`:
the `responseEnd → boot.entry` residual, not a direct WebAssembly initialization
measurement. Its contrasts describe that delivery-mode-specific residual; they
do not establish compilation or instantiation cost. #887 supersedes the field
with direct initializer diagnostics.

Firefox, combined `wasmFetchMs + responseEnd → boot.entry` residual, mean of
three run-means:

| contrast | predicted (naive) | pre-registered  | observed cold        | observed warm        |
| -------- | ----------------- | --------------- | -------------------- | -------------------- |
| A→B      | ~146 ms           | "roughly holds" | **43.5 ms** (SE 8.1) | **54.8 ms** (SE 5.6) |
| B→C      | ~113 ms           | "near zero"     | **14.5 ms** (SE 4.1) | **8.4 ms** (SE 5.4)  |
| A→C      | ~259 ms           | ~146 ms         | **58.0 ms** (SE 7.0) | **63.2 ms** (SE 2.5) |

**B→C: prediction confirmed.** Removing 1.285 MiB of name section bought 8–15
ms. The warm figure is not distinguishable from run-to-run noise (|d|/SE 1.6).
Name section costs 6.5–11.3 ms/MiB against code's 26–33 ms/MiB — engines largely
do skip it, as expected.

**A→B: prediction refuted.** Removing 1.656 MiB of real code bought 43–55 ms,
not ~146 ms. Restricting the 88 ms/MiB constant to compiled bytes is **necessary
but not sufficient**: it still overpredicts by about 3× over bytes that are
unambiguously compiler input.

**Why the constant was wrong is arithmetic, not engine behaviour.** #818 derived
88 ms/MiB as an _average_ — 449 ms of firefox instantiate ÷ 5.10 MiB, which
reproduces to 88.0 exactly — and #836 applied it as a _marginal_ rate. Those are
the same number only if the cost is proportional to size. It is not. Fitting
firefox cold instantiate across the three arms:

```
instantiate ≈ 377 ms + 13.9 ms/MiB
```

At the shipped size, **92% of firefox's wasm instantiate is size-independent**.
The average rate is not a constant at all — it reads 88 ms/MiB at arm A and 189
ms/MiB at arm C, purely because the fixed term is divided by a smaller number.

**The 377 ms is left unexplained.** This capture was designed to test a size
contrast and can only bound the fixed term, not attribute it. Naming a cause
here would repeat exactly the error #840 had to withdraw a claim for. It is the
single largest item in CSR boot and is filed as its own question — **#864**.

**What #836 actually bought.** Download weight fell 57.6% (5.35 MB → 2.27 MB
raw; brotli 860 → 603 KiB), which is real and is the result the size budget
guards. Firefox boot fell ~58–63 ms per navigation, chromium ~45–55 ms. That is
worth having and is roughly a quarter of what the lever was thought to be worth.

**Consequence for #818's disposition.** "Actionable, not intrinsic" was too
strong. Bundle volume is a genuine but weak lever — at ~14–20 ms/MiB marginal,
halving the bundle again would buy ~30 ms against a ~380 ms floor. The floor,
not the volume, is where firefox's boot cost lives — see #864.

### Measured and deliberately not acted on

Two candidates were measured against the same 25 KiB materiality threshold and
both came in under it, so neither got a follow-up issue.

**`croner` — 9.7 KiB, and it stays.** It is the only application-level crate in
the wasm graph that could plausibly have been cut, but ADR-0065 requires the
client to validate `BackupSchedule` through the server's own `FromStr`. Removing
it would mean a second, divergent parser — the exact failure ADR-0065 exists to
prevent. Below threshold and load-bearing.

**Client log strings — ~0 bytes, and the premise was wrong.** `csr/src/lib.rs`
initialises `console_log` at `Level::Debug`, which was expected to compile every
`debug!`/`trace!` format string into the bundle. Building with
`log/release_max_level_info` moved the shipped wasm by **11 bytes** (2 267 076 →
2 267 065) — below the 13-byte reproducibility noise floor.

The reason: there are **zero** `debug!`/`trace!` call sites in `csr`, `web` or
`common`. There are no strings to strip. Muting client logs would have bought
nothing, which is a good thing, because #839 is trying to _start_ capturing
browser console output and muting it in the same cycle would have worked against
that.

### The size budget

`cargo xtask validate` now fails when raw `pkg/jaunder.wasm` exceeds
`WASM_RAW_CEILING_BYTES` (2 340 000 — 3.2% over the achieved size). The headroom
is bounded on both sides: it absorbs an innocent dependency bump, but sits
_below_ what `-Os` and `-O2` produce, so a downgrade of the optimisation level
fails the gate. `audit-wasm` also now errors outright if the shipped wasm
carries a name section, guarding the 1.28 MiB strip.

Rejected: `panic = "abort"`. `rustc --print cfg --target wasm32-unknown-unknown`
already reports `panic="abort"`, so it buys zero bytes — and profiles are
workspace-wide, so setting it would flip the _server's_ release build to abort,
where a panicking tokio task would kill the process. Recorded in the root
`Cargo.toml` at the profile someone would add it to.

## #864 — firefox's wasm initialization floor (pre-registration, 2026-08-22)

#836 found that firefox's historical `responseEnd → boot.entry` residual fit
`≈ 377 ms + 13.9 ms/MiB`; at the shipped size, most of the segment was
size-independent. That historical field is not a direct initialization
measurement — #887 replaced it with `direct-init-v1` diagnostics — so this cycle
tests the floor with the current instrument rather than re-reading the old
residual.

### Deciding protocol

Firefox is decisive; chromium is a control. The capture is sqlite,
single-worker, quiescent host, three runs per arm unless a pre-capture dry run
shows a larger variance that requires more. Every run uses a distinct `e2eSalt`
so Nix cannot replay a cached trace. Arm order is randomized or explicitly
counterbalanced run-by-run, and the realized order is recorded with the corpus.

No final capture may change the SPA shell delivery path: `/pkg/jaunder.wasm`,
the `/pkg/jaunder.js` import, `initMeasured`, content negotiation,
compression/precompression, request count, and `wasmInitPath` distribution must
remain comparable unless the change is the explicitly named candidate. A corpus
without current `direct-init-v1` coverage, closure, arm-order reconciliation,
host quiescence notes, and an independent in-trace arm discriminator is
exploratory only.

### Candidate A — module shape, not byte volume

Arms:

- `baseline`: the current shipped bundle.
- `shape`: same delivery path, but an experiment-only bundle with actual module
  shape counts changed and reported in trace: at minimum functions, imported
  functions, exports, tables, memories, code bytes, decoded bytes, and raw
  bytes.

Prediction if module shape explains the fixed term: firefox `wasmApiMs` and/or
`wasmInitMs` moves in the same direction as the shape-count delta even after the
decoded-byte delta is too small to explain the change at #836's marginal 13.9
ms/MiB. A null result is a measured elimination of the tested shape factor, not
proof that every untested shape factor is irrelevant. Chromium may move
differently; chromium-only movement does not close #864.

### Candidate B — per-navigation or per-module engine setup

This candidate is included only if implementation can preserve the delivery/API
invariants above while producing an independent setup-arm discriminator. The
intended contrast is not preload, cache busting, `arrayBuffer`/manual `compile`,
or a second request path; those change delivery or WebAssembly API mode and
would repeat #866's attribution failure. If no invariant-preserving setup arm
exists, this cycle stops and either selects another separable size-independent
candidate under the same corpus or returns for spec approval before reducing the
experiment to one candidate.

Prediction if the floor is mostly per-navigation setup: changing the module-side
shape in Candidate A does not move firefox materially, while an
invariant-preserving setup discriminator separates firefox `wasmApiMs` and/or
`wasmInitMs` by a large fixed amount that is not explained by decoded bytes. A
null setup contrast eliminates only the implemented setup mechanism.

### Candidate C — custom-section count stress

Added after the first ship review rejected the one-section probe as only one
candidate. This keeps the same delivery/API invariants as Candidate A but
changes the count of same-named post-`wasm-opt` custom sections from zero
to 200. This is not a general module-shape probe: it isolates custom-section
table iteration and metadata parsing pressure while keeping imports, exports,
tables, memories, request URL, `wasmInitPath`, compression, and shell import
constant.

Arms:

- `baseline`: current bundle, `customSections: 0`.
- `shape-count`: same bundle plus 200 `jaunder.shape` custom sections, verified
  in-trace as `customSections: 200`.

Prediction if per-custom-section metadata setup explains a material part of the
floor: firefox `wasmApiMs` and/or `wasmInitMs` increases in `shape-count` by a
fixed amount larger than chromium's change and larger than the decoded-byte
delta would predict from #836's 13.9 ms/MiB marginal slope. A null or
chromium-matched result eliminates this custom-section-count mechanism, not
function count, import/export count, table/memory shape, or code-section layout.

### Reporting rule

Report firefox first. Report means over run means with uncertainty and cold/warm
handling for every arm contrast. `wasmApiMs` and `wasmInitMs` are overlapping
direct diagnostics, not compile-only durations and not additive parts of the
ADR-0100 exclusive decomposition; the exclusive document-frame closure remains
`wasmInitStartMs + wasmInitStartToBootEntryMs + bootPhases = bootTotalMs`.

### Finding — custom-section module-shape probe (2026-08-22)

Corpus: `~/measurements/jaunder/issue-864-wasm-floor/`, extracted to
`traces/*.jsonl`. The host was quiesced before capture. Runs were sqlite
single-worker, three runs per arm, with realized order counterbalanced by run:
`baseline-1 firefox→chromium`, `baseline-2 chromium→firefox`,
`baseline-3 firefox→chromium`, `shape-1 chromium→firefox`,
`shape-2 firefox→chromium`, `shape-3 chromium→firefox`.

Salts were `issue864-baseline-{1,2,3}` and `issue864-shape-{1,2,3}`. Commands:
`nix build --print-out-paths --no-link .#packages.x86_64-linux.e2e-sqlite-{firefox,chromium}-single-worker`,
then `cargo xtask traces analyze <traces...>` and
`cargo xtask traces boot-phases <traces...>`. Certification artifacts:
`.xtask/run/1787433823720-3423114.out` (`traces analyze`) and
`.xtask/run/1787433844049-3423325.out` (`traces boot-phases`).

The implemented `shape` arm is deliberately narrow: it appends one tiny
post-`wasm-opt` custom section named `jaunder.shape`. It preserves
`/pkg/jaunder.wasm`, `/pkg/jaunder.js`, `initMeasured`, streaming delivery,
precompression, request shape, and `wasmInitPath`; it changes decoded wasm by
only **30 bytes** (2,296,722 → 2,296,752), encoded brotli by **103 bytes**
(623,283 → 623,386), and the in-trace module shape only by
`customSections: 0 → 1`. Imports, imported functions, exports, tables, memories,
and exported functions stayed unchanged. Therefore this is a valid probe for
“custom-section/module-metadata count” only, not for every module shape factor.

Corpus integrity held for every arm/browser/run: cold populations had 166
current direct-init completions per run with 2 unlabeled pre-test navigations;
warm populations had 43 completions per run with 1 unlabeled pre-test
navigation. Delivery shape matched by arm: cold runs were 164 streaming + 2
buffered completions; warm runs were 43 streaming + 0 buffered. Each labeled
population had one unique module shape, 100% shape completeness, and zero
document-frame closure violations in `traces boot-phases`.

Firefox moved downward, but the run-to-run variance is larger than the effect
estimate:

| browser  | warmth | arm      | `wasmApiMs` mean ± SE | `wasmInitMs` mean ± SE | exclusive boot total mean ± SE |
| -------- | ------ | -------- | --------------------- | ---------------------- | ------------------------------ |
| firefox  | cold   | baseline | 481.0 ± 50.4          | 630.1 ± 61.9           | 962.0 ± 99.3                   |
| firefox  | cold   | shape    | 434.3 ± 29.1          | 572.3 ± 36.0           | 873.6 ± 56.9                   |
| firefox  | warm   | baseline | 441.1 ± 48.3          | 578.0 ± 59.5           | 810.4 ± 93.5                   |
| firefox  | warm   | shape    | 405.3 ± 18.1          | 534.8 ± 27.6           | 742.8 ± 42.2                   |
| chromium | cold   | baseline | 154.9 ± 4.1           | 267.5 ± 7.1            | 471.3 ± 11.6                   |
| chromium | cold   | shape    | 151.4 ± 12.0          | 264.2 ± 20.0           | 469.7 ± 33.3                   |
| chromium | warm   | baseline | 123.6 ± 5.5           | 184.9 ± 7.3            | 337.8 ± 13.8                   |
| chromium | warm   | shape    | 119.8 ± 5.7           | 185.0 ± 8.7            | 332.7 ± 16.8                   |

Paired shape-minus-baseline deltas over run means:

| browser  | warmth | `wasmApiMs`     | `wasmInitMs`    | exclusive boot total |
| -------- | ------ | --------------- | --------------- | -------------------- |
| firefox  | cold   | -46.7 ± 21.8 ms | -57.8 ± 26.6 ms | -88.4 ± 43.1 ms      |
| firefox  | warm   | -35.8 ± 30.4 ms | -43.2 ± 32.1 ms | -67.5 ± 51.3 ms      |
| chromium | cold   | -3.5 ± 14.5 ms  | -3.3 ± 24.1 ms  | -1.6 ± 40.4 ms       |
| chromium | warm   | -3.7 ± 9.0 ms   | +0.1 ± 12.7 ms  | -5.2 ± 23.9 ms       |

Interpretation: the one-section custom-section arm does **not** close #864. The
firefox sign is consistent across cold and warm direct diagnostics, while
chromium is near zero, but the uncertainty is too large for a decisive mechanism
claim and the tested shape factor is only one custom-section metadata record.
The remaining unknowns are the larger module-shape factors originally suspected
by #864 — function count, import/export count, table/memory shape, and
code-section layout — plus any invariant-preserving per-module engine setup
discriminator.

### Finding — custom-section count stress probe (2026-08-23)

Corpus: `~/measurements/jaunder/issue-864-wasm-floor/`, extracted to
`traces-count/*.jsonl`. The host was quiesced before capture. Runs were sqlite
single-worker, three runs per arm, with realized order counterbalanced by run:
`baseline-count-1 firefox→chromium`, `shape-count-1 chromium→firefox`,
`baseline-count-2 chromium→firefox`, `shape-count-2 firefox→chromium`,
`baseline-count-3 firefox→chromium`, `shape-count-3 chromium→firefox`.

Salts were `issue864-baseline-count-{1,2,3}` and `issue864-shape-count-{1,2,3}`.
Commands:
`nix build --print-out-paths --no-link .#packages.x86_64-linux.e2e-sqlite-{firefox,chromium}-single-worker`,
then `cargo xtask traces analyze <traces-count...>` and
`cargo xtask traces boot-phases <traces-count...>`. Certification artifacts:
`.xtask/run/1787449986508-3680541.out` (`traces analyze`) and
`.xtask/run/1787450006807-3680762.out` (`traces boot-phases`).

The `shape-count` arm appends **200** same-name `jaunder.shape` custom sections
after `wasm-opt`. It preserves `/pkg/jaunder.wasm`, `/pkg/jaunder.js`,
`initMeasured`, streaming delivery, precompression, request shape, and
`wasmInitPath`; it changes decoded wasm by **6,000 bytes** (2,296,722 →
2,302,722), keeping #836's decoded-byte slope prediction under **0.1 ms**.
Imports, imported functions, exports, tables, memories, and exported functions
stayed unchanged. The in-trace discriminator was `customSections: 0 → 200`, so
this is the preregistered custom-section-count mechanism and still not a probe
for function count, import/export count, table/memory shape, or code-section
layout.

Corpus integrity held for every arm/browser/run: cold populations had 152
current direct-init completions per run; warm populations had 41 completions per
run. Each labeled population had one unique module shape, 100% shape
completeness, and zero document-frame closure violations in
`traces boot-phases`.

Firefox did **not** move upward when 200 custom sections were added:

| browser  | warmth | arm         | `wasmApiMs` mean ± SE | `wasmInitMs` mean ± SE | exclusive boot total mean ± SE |
| -------- | ------ | ----------- | --------------------- | ---------------------- | ------------------------------ |
| firefox  | cold   | baseline    | 457.5 ± 51.3          | 600.9 ± 63.9           | 887.0 ± 98.5                   |
| firefox  | cold   | shape-count | 439.2 ± 24.1          | 579.6 ± 31.5           | 855.7 ± 54.8                   |
| firefox  | warm   | baseline    | 423.9 ± 43.7          | 559.9 ± 55.8           | 761.7 ± 94.2                   |
| firefox  | warm   | shape-count | 409.1 ± 23.2          | 541.3 ± 32.0           | 729.3 ± 54.3                   |
| chromium | cold   | baseline    | 144.3 ± 1.9           | 251.5 ± 3.2            | 408.6 ± 3.0                    |
| chromium | cold   | shape-count | 144.0 ± 0.8           | 251.1 ± 1.5            | 410.1 ± 1.3                    |
| chromium | warm   | baseline    | 110.8 ± 1.1           | 170.7 ± 1.9            | 300.2 ± 0.5                    |
| chromium | warm   | shape-count | 110.4 ± 0.6           | 170.7 ± 0.8            | 300.5 ± 1.8                    |

Paired shape-count-minus-baseline deltas over run means:

| browser  | warmth | `wasmApiMs`     | `wasmInitMs`    | exclusive boot total |
| -------- | ------ | --------------- | --------------- | -------------------- |
| firefox  | cold   | -18.3 ± 29.3 ms | -21.3 ± 35.7 ms | -31.3 ± 46.2 ms      |
| firefox  | warm   | -14.7 ± 20.7 ms | -18.5 ± 23.8 ms | -32.3 ± 41.6 ms      |
| chromium | cold   | -0.3 ± 1.6 ms   | -0.4 ± 2.7 ms   | +1.5 ± 3.9 ms        |
| chromium | warm   | -0.4 ± 1.0 ms   | -0.0 ± 1.6 ms   | +0.3 ± 1.3 ms        |

Interpretation: custom-section count is not the size-independent firefox floor.
The preregistered stressor increases the relevant metadata count by 200 and does
not produce a firefox-specific positive delta. The one-section probe's downward
firefox sign was run-order/noise, not a mechanism. #864 remains open for the
larger module-shape factors: function count, import/export count, table/memory
shape, code-section layout, and any invariant-preserving per-module engine setup
discriminator.

## #866 — where the rest of boot goes (2026-08-09)

#836 established that page boot is **43–47% of e2e suite wall-clock** and that
wasm explains only part of it. This is the rest.

**Denominator, stated once.** Each suite run performs **211 navigations**, of
which **208 mount** and carry boot marks. Every per-suite second below is
`mean × 208`. **Means, never medians** — `median(a+b) ≠ median(a) + median(b)`,
and #818 saw a 2–7% apparent residual that was purely that artifact.

Source: #836's arm-C captures (the shipped bundle), single-worker sqlite, three
runs pooled per engine. No new collection — this re-reads segments the corpus
already carried.

### Historical six segments

| segment                             | chromium ms/nav | s/suite  | firefox ms/nav | s/suite   |
| ----------------------------------- | --------------- | -------- | -------------- | --------- |
| `document_start → wasm_fetch_start` | **212.5**       | **44.2** | **260.7**      | **54.2**  |
| `wasm_fetch`                        | 161.3           | 33.5     | 56.8           | 11.8      |
| `wasm_instantiate`                  | 14.1            | 2.9      | **405.0**      | **84.2**  |
| `boot.entry → seed_parsed`          | 1.5             | 0.3      | 1.3            | 0.3       |
| `seed_parsed → render_start`        | 0.6             | 0.1      | 1.3            | 0.3       |
| `render_start → mount_done`         | 3.6             | 0.7      | 2.0            | 0.4       |
| **boot total**                      | **393.6**       | **81.9** | **727.0**      | **151.2** |
| frame skew (`commitToMount` − boot) | 294.9           | **61.3** | 184.2          | **38.3**  |
| **`commitToMount`**                 | **688.5**       | 143.2    | **911.2**      | 189.5     |

Split by cache warmth, ms per navigation (n = 339 / 285 / 341 / 285):

| segment                             | chr cold  | chr warm  | ff cold   | ff warm   |
| ----------------------------------- | --------- | --------- | --------- | --------- |
| `document_start → wasm_fetch_start` | 257.9     | 158.5     | 310.2     | 201.4     |
| `wasm_fetch`                        | 189.0     | 128.3     | 77.0      | 32.6      |
| `wasm_instantiate`                  | 14.6      | 13.6      | 403.9     | 406.4     |
| `boot.entry → seed_parsed`          | 2.7       | 0.2       | 2.1       | 0.3       |
| `seed_parsed → render_start`        | 0.9       | 0.2       | 1.9       | 0.5       |
| `render_start → mount_done`         | 5.8       | 0.9       | 1.8       | 2.2       |
| **boot total**                      | **470.9** | **301.6** | **796.9** | **643.4** |

Note what does _not_ move with warmth: **firefox's historical `wasm_instantiate`
residual is 403.9 ms cold and 406.4 ms warm.** A warm HTTP cache removes the
download but this residual cannot establish what happens to compile cost: it is
derived from `responseEnd → boot.entry` and absorbs every activity in that
interval. #887 replaces it with direct initialization diagnostics.

Two things close here rather than deferring.

**The Rust boot path is ~1 s of a 300–450 s suite.** `seed_parsed`,
`render_start` and `mount_done` sum to 1.1 s (chromium) and 1.0 s (firefox).
#801 was right to be redirected; there is nothing there, and there is no point
looking again.

**`topSlow` is truncated and carries no shares.**
`e2e.resource_top_slow_dropped` sums to **54** across 822 arm-C spans, so
per-resource timings from `e2e.resource_summary_json` are duration-biased. They
may illustrate a single resource, never a population. The contrast with
`navigation_top_json` — `dropped = 0`, a genuine census — is exactly the
distinction that makes a top-N slice readable as a population if it is lost.

### The pre-fetch window

`document_start → wasm_fetch_start` is the **largest boot segment on chromium**
(54% of boot) and the second largest on firefox. It is entirely our own code.

**Observed**, from `csr/index.html`: a synchronous inline pre-paint auth script
(#181, ADR-0044) in `<head>`, then two `<link rel="stylesheet">`, then an inline
`<script type="module">` in `<body>` that imports `/pkg/jaunder.js` and only
then calls `init("/pkg/jaunder.wasm")`. So the wasm fetch cannot begin until 56
KiB of JS glue has been fetched, parsed and executed. That ordering is a fact
about the file.

The window shrinks 35–39% cold→warm — chromium 257.9 → 158.5 ms, firefox 310.2 →
201.4 ms — so it is part network and part not, with a substantial residual
surviving a warm cache in both engines.

**Hypothesised and explicitly not established:** a pending stylesheet blocks
script execution per spec, so those two stylesheets _may_ sit on the critical
path to the wasm fetch starting. **This was not measured**, and nothing here
rests on it. Filed as #870 to be tested rather than asserted — naming it as the
cause on the strength of waterfall shape is the mistake #840 had to withdraw a
claim for.

### Frame skew

61.3 s (chromium) / 38.3 s (firefox) per suite, and **larger on the faster
engine**. `commitToMountMs` is Node-side wall clock; the six segments are
document-relative. **ADR-0100 forbids decomposing one into the other** — they
are different clocks — so this cycle reported the magnitude and did not split
it. #868 added a bridge instrument; see the 2026-08-24 finding below.

Whether it is harness overhead that costs real users nothing, or real time the
document frame cannot see, is unknown. Those two answers point in opposite
directions, which is the argument for measuring rather than guessing.

### What this leaves

| lever                                | worth                         | status                     |
| ------------------------------------ | ----------------------------- | -------------------------- |
| firefox `wasm_instantiate`           | 84.2 s — the largest by far   | #864 (~377 ms fixed floor) |
| frame skew                           | 61.3 / 38.3 s                 | #868 — measured below      |
| pre-fetch window                     | 44.2 / 54.2 s                 | preload below; #870        |
| `wasm_fetch`                         | 33.5 / 11.8 s, ~44% are `304` | #869 — caching             |
| navigation count (211 for 137 tests) | ~190 s firefox                | #867                       |
| Rust boot path                       | ~1 s                          | **closed**                 |

#869 deserves particular note: `/pkg/*` sets no `Cache-Control`, only an `ETag`,
and `server/src/site.rs` records ~44% of wasm requests as conditional. Removing
those round-trips outright is plausibly worth more than the preload below, which
can only start them earlier.

Corpus limits are inherited from #836 and apply: n=3 per arm, host not perfectly
quiescent, arm confounded with position within a round. The last one threatens
cross-arm contrasts; this decomposition makes none.

### The preload, and a prediction registered before capturing it

`csr/index.html` and `render_head` now carry
`<link rel="preload" href="/pkg/jaunder.wasm" as="fetch" type="application/wasm" crossorigin>`
plus a `modulepreload` for the glue, so the wasm download starts during HTML
parse rather than after 56 KiB of glue has executed.

**`crossorigin` is load-bearing, and the reasoning that omitted it was wrong.**
An `as="fetch"` preload without it is a `no-cors` request; wasm-bindgen's
`init()` calls `fetch()`, which defaults to `cors`. Firefox will not reuse the
mismatched preload — it requested the identical URL **twice**, downloading 2.2
MB of wasm for nothing. Chromium coalesced it either way, so this only ever
reproduced on firefox, and only because the e2e guard counts requests on both.

#### Pre-registration

Written **before** the capture. The point is to be able to be wrong.

- **Decisive quantity:** `document_start → mount_done` (boot total),
  document-frame, ADR-0100-clean. **Not** the pre-fetch segment alone — the fix
  collapses the _later_ `wasm_fetch` into that window, so a per-segment reading
  would misattribute the change. **Not** suite wall-clock, which did not
  separate arms on firefox in #836 and cannot resolve an effect this size.
- **Ceiling.** The saving cannot exceed the wasm fetch time: **≤161 ms/nav
  chromium, ≤57 ms firefox**. It will not be reached — the moved download now
  contends with the glue and both stylesheets, and ~44% of these requests are
  conditional (#869), where preload only starts the round-trip earlier.
- **Floor.** The improvement must exceed **3 × SE** on boot total in at least
  one engine. Firefox boot total is ~727 ms with run-level SD ~3–9 ms at n=3, so
  SE ≈ 2–6 ms and 3 × SE ≈ 6–18 ms. A ceiling alone would be confirmed by every
  positive outcome; the floor is what can fail.
- **Directional prediction:** `document_start → wasm_fetch_start` falls sharply
  while `wasm_fetch` **rises**, because the download now overlaps rather than
  following. If `wasm_fetch` does not rise, the preload is not doing what is
  claimed even if the total improves.
- **Abort rule (D8):** below the floor, or a regression in either engine, or any
  double fetch → **the preload is reverted** and written up as a negative
  result. It does not land on the strength of the reasoning being sound.

**One disclosure, because it weakens half of this.** A single non-protocol
chromium run — made while proving the request-count guard could fail — already
showed `wasmFetchStartMs` 212.5 → 51.5 ms and `wasmFetchMs` 161.3 → 206.6 ms,
about 116 ms net. So **chromium's prediction is informed by prior observation
and is not a clean pre-registration.** Firefox's is: its timings have
deliberately **not** been inspected, precisely so the gate's critical-path
engine keeps an uncontaminated prediction. Firefox is the arm to read.

#### Observed — the abort rule fired, and the preload is reverted

Captured 2026-08-10, 12 runs, corpus at
`~/measurements/jaunder/issue-866-preload/`. Certified before analysis:
`dropped = 0` on all 24 rows, full marks on 100% of 2496 mounted navigations,
exactly 208 mounted per capture. Arm integrity by an independent signal — the
wasm resource's `initiatorType` is `fetch` on 100% of before-arm navigations and
`link` on 100% of after-arm ones, in both engines.

**Boot total, mean of three run-means:**

| engine   | warmth | before | after | delta | 3 × SE | verdict          |
| -------- | ------ | ------ | ----- | ----- | ------ | ---------------- |
| firefox  | all    | 762.0  | 743.1 | 18.8  | 38.8   | **below floor**  |
| firefox  | cold   | 829.8  | 820.1 | 9.7   | 40.0   | **below floor**  |
| firefox  | warm   | 680.8  | 651.7 | 29.0  | 38.4   | **below floor**  |
| chromium | all    | 410.4  | 394.0 | 16.4  | 16.3   | clears by 0.1 ms |
| chromium | cold   | 489.2  | 467.3 | 21.9  | 17.8   | clears           |
| chromium | warm   | 316.7  | 306.9 | 9.9   | 16.4   | below floor      |

**The preload does exactly what it was built to do.** The pre-fetch window
collapses — firefox 276.2 → 81.5 ms, chromium 220.4 → 54.4 ms. The mechanism is
not in doubt.

**And the engines give it all back.** Per navigation, all navigations:

| segment            | firefox           | chromium         |
| ------------------ | ----------------- | ---------------- |
| pre-fetch window   | 276.2 → **81.5**  | 220.4 → **54.4** |
| `wasm_fetch`       | 60.1 → 110.2      | 169.0 → 312.6    |
| `wasm_instantiate` | 421.0 → **546.0** | 15.3 → 21.1      |

The directional prediction held: `wasm_fetch` rose, as the moved download now
contends.

`wasm_instantiate` also rose 125 ms on firefox — **and that figure must not be
read as a compile regression.** It is **derived, not measured**: `fixtures.ts`
computes it as `boot.entry.startTime − wasm.responseEndMs`, because Rust cannot
observe its own fetch or instantiation. It is a **residual**, so it absorbs
whatever happens between those two points.

That makes it **not comparable across arms that move when the fetch starts**:

- **Before**, `init()` starts the fetch, so it runs only after the glue has
  loaded and executed. The residual is close to genuine instantiate.
- **After**, the preload finishes early but `init()` still waits on the glue, so
  the glue's load and execution fall **inside** the residual — reattributed out
  of the pre-fetch window, not newly incurred.

The arithmetic fits reattribution: firefox −194.8 (pre-fetch) +50.1 (fetch)
+125.0 (residual) nets −19.6 ms. A pure reattribution nets zero, so what is left
is a small genuine saving on top of a large reshuffle across segment boundaries.

**No claim is made about compile cost here, in either direction.** Whether
delivery affects firefox's real initialization was untested in this historical
trial; #887 supersedes its residual with direct WebAssembly API and initializer
measurements.

**This does not touch the decomposition above.** That was computed within a
_single_ delivery mode (arm C), where `init()` always starts the fetch, so the
residual means what it says; the same holds for its cold/warm split. The
residual only becomes uninterpretable when an experiment moves the fetch's start
— which is exactly what this one did, and what this caveat exists to record.

#### Verdict against the pre-registered rules

- **Firefox — the decisive, uncontaminated arm — does not clear the floor** in
  any warmth. 18.8 ms against a required 38.8 ms.
- **Chromium clears by 0.1 ms** on the pooled figure (16.4 vs 16.3), which is
  not a result, and clears more convincingly cold (21.9 vs 17.8). Chromium's
  prediction was also disclosed in advance as contaminated.
- A large, consistent component regression landed on the engine that is the e2e
  gate's critical path.

**D8 fires: the preload is reverted.** It does not land on the strength of the
mechanism being real — and the mechanism _is_ real: the serial boot chain was a
genuine 166–195 ms of waiting and the preload genuinely removed it. It bought
~19 ms of boot, because the glue load stays on the critical path either way.

**What this does _not_ establish.** An earlier draft of this section claimed the
experiment showed firefox's instantiate floor (#864) is "delivery-dependent".
**That claim is withdrawn**: it rested on the derived residual described above,
which reattributes the glue load between arms. The floor may or may not depend
on delivery; this capture cannot say, and #887 is now framed as a question about
the instrument rather than a report of a regression.

The pre-fetch window (#870's stylesheet question) was left open here and is now
answered by the 2026-08-24 follow-up below.

## #870 — render-blocking stylesheets delay wasm fetch (finding, 2026-08-24)

**Verdict up front: yes, the two render-blocking stylesheets delay the wasm
fetch.** The browser does not start the shell module until the blocking
stylesheets have finished, and the wasm fetch starts immediately after the
module begins. The remaining `module_before_init → wasm_fetch_start` gap is only
~2–3.5 ms, so the pre-fetch window is stylesheet-blocked rather than hidden in
the module script.

Corpus: `~/measurements/jaunder/issue-870-stylesheet-wasm-fetch/`, extracted to
`issue-870-q*.jsonl`. The host was quiescent before capture. Runs were SQLite
single-worker, three runs per engine, with realized order counterbalanced by
run:

- `issue-870-q1`: chromium→firefox
- `issue-870-q2`: firefox→chromium
- `issue-870-q3`: chromium→firefox

Salts were `issue870-q{1,2,3}`. Commands:
`nix build --print-out-paths --no-link .#packages.x86_64-linux.e2e-sqlite-{chromium,firefox}-single-worker`,
then
`cargo xtask traces boot-phases ~/measurements/jaunder/issue-870-stylesheet-wasm-fetch/issue-870-q*.jsonl`.

The deciding rows are the complete stylesheet diagnostics emitted by the new
document-frame mark `jaunder.module.before_init` plus the two stylesheet
`PerformanceResourceTiming.responseEnd` values. Historical or incomplete rows
remain non-decisive; they are counted by the analyzer and not backfilled.

Coverage and ordering:

| engine   | complete/total rows | coverage | ordering pass | `styleMaxResponseEndMs / wasmFetchStartMs` |
| -------- | ------------------- | -------- | ------------- | ------------------------------------------ |
| chromium | 630/636             | 99.1%    | 630/630       | 55.9%                                      |
| firefox  | 633/636             | 99.5%    | 633/633       | 52.9%                                      |

Per-cache timing:

| engine   | cache | rows | `style_done→module_before_init` | `module_before_init→wasm_fetch_start` |
| -------- | ----- | ---- | ------------------------------- | ------------------------------------- |
| chromium | cold  | 501  | 97.1 ± 1.3 ms                   | 3.5 ± 0.1 ms                          |
| chromium | warm  | 129  | 108.1 ± 1.4 ms                  | 2.0 ± 0.1 ms                          |
| firefox  | cold  | 504  | 152.8 ± 2.4 ms                  | 2.9 ± 0.1 ms                          |
| firefox  | warm  | 129  | 157.2 ± 3.5 ms                  | 3.0 ± 0.2 ms                          |

Interpretation: module evaluation is serialized behind the stylesheets, and the
wasm fetch is then launched promptly. That makes the pre-fetch window a shell
delivery problem: making the wasm request independent of render-blocking CSS
would recover roughly the stylesheet delay, subject to preserving ADR-0044's
pre-paint authentication invariant and ADR-0121's no-double-download guard.

This closes #870. It does **not** close #869, #895, #1103, or #1138. The finding
does not reinterpret Node-frame `commitToMountMs` as document boot time and does
not re-open wasm preload; ADR-0100 and ADR-0121 still apply.

## #868 — frame skew bridge (finding, 2026-08-24)

**Verdict up front: the frame skew is mostly a mount-ready bridge term, not an
application boot phase.** The bridge closes exactly:
`commitToDocumentStartMs + mountDoneToBindingMs + frameSkewRemainderMs = commitToMountMs - documentBootTotalMs`
for every mounted navigation in the deciding corpus. The document-frame boot
decomposition remains the one used for app boot; these Node/document bridge
terms are reported separately under ADR-0100.

Corpus: `~/measurements/jaunder/issue-868-frame-skew/`, extracted to
`bridge-q*.jsonl`. The host was quiescent before capture. Runs were SQLite
single-worker, three runs per engine, with realized order counterbalanced by
run:

- `bridge-q1`: chromium→firefox
- `bridge-q2`: firefox→chromium
- `bridge-q3`: chromium→firefox

Salts were `issue868-bridge-q{1,2,3}`. Commands:
`nix build --print-out-paths --no-link .#packages.x86_64-linux.e2e-sqlite-{chromium,firefox}-single-worker`,
then
`cargo xtask traces boot-phases ~/measurements/jaunder/issue-868-frame-skew/bridge-q*.jsonl`.

Each engine/run produced 196 recorded navigations, 189 mounted navigations, 189
complete bridge rows, and zero closure violations. This is a decisive bridge
certification corpus.

Paired per-run means:

| engine   | mounted rows/run | frame skew mean ± SE | `commit→document_start` | `mount_done→binding` | remainder |
| -------- | ---------------- | -------------------- | ----------------------- | -------------------- | --------- |
| chromium | 189/189/189      | 317.1 ± 4.7 ms       | -51.0 ± 0.6 ms          | 368.2 ± 5.3 ms       | 0.000 ms  |
| firefox  | 189/189/189      | 211.9 ± 6.3 ms       | -132.7 ± 3.8 ms         | 344.6 ± 9.4 ms       | 0.000 ms  |

Per-suite contribution over 189 mounted navigations/run:

| engine   | frame skew total | `commit→document_start` | `mount_done→binding` |
| -------- | ---------------- | ----------------------- | -------------------- |
| chromium | 59.9 s           | -9.6 s                  | 69.6 s               |
| firefox  | 40.0 s           | -25.1 s                 | 65.1 s               |

Interpretation: `mount_done→binding` is the dominant bridge term in both
engines. The negative `commit→document_start` term means the Node-side commit
timestamp is after the document's `performance.timeOrigin`; it reduces the net
skew rather than explaining it. The zero remainder proves the bridge terms close
the prior 38–61 s suite gap without hiding time in arithmetic drift.

This does **not** close #864, #870, #895, #1103, or #1138. The finding says only
that the old `commitToMountMs - bootTotalMs` skew is primarily e2e harness/frame
coordination between page mount completion and the Node fixture's binding
resolution. It is not a Firefox wasm initialization floor, and it should not be
subtracted from document-frame boot phase findings.

## #867 — removing navigations (findings, 2026-08-11)

**Verdict up front: the pre-registered floor is MISSED on the pre-registered
metric, and the reason is a defect in the pre-registration rather than in the
change.** Suite wall-clock fell 23.4 s on firefox against a floor of 33.9 s. On
a like-for-like measure — the tests that exist in both arms — the same change
saved **77.3 s on firefox**, which _exceeds_ the ceiling the prediction set.
Both numbers are below, and both belong in the record.

### Method

Deciding set per #818/#836/#866: **single-worker, sqlite**, both browsers, 3
runs per arm, `before`/`after` interleaved run-by-run, a distinct `e2eSalt` per
run so nix could not replay a cached suite. `before` is the branch's fork point
(`606a6399`), `after` is `3b505e6b`, each in its own worktree so no checkout
switching could contaminate a run. Targets were the single-worker packages
(`.#e2e-sqlite-{chromium,firefox}-single-worker`), not the gate checks.

`retries` was raised from 0 to 1 **in both arms identically** for the campaign.
At the config default of 0 a `flaky` count is structurally impossible, so the
guardrail could not have failed; reverted afterwards.

Corpus: `~/measurements/jaunder/issue-867-navcount/` — 12 runs, per-run trace
JSONL and Playwright report, README recording salts and host load.

**Certification.** `dropped = 0` on every span of all 12 runs. Navigation counts
are **byte-identical across all three rounds within each arm**, so the count is
deterministic and carries no run-to-run variance. Host 1-minute load was
0.90–1.84 at the start of every run except `after-chromium-1` (5.98, the
decaying tail of the previous run's teardown) — recorded, not corrected for.

### Suite wall-clock (`.stats.duration`, seconds)

| arm    | chromium              | firefox               |
| ------ | --------------------- | --------------------- |
| before | 373.6 / 331.3 / 360.5 | 494.0 / 489.1 / 521.1 |
| after  | 352.3 / 307.3 / 350.9 | 479.8 / 476.8 / 477.3 |
| mean Δ | **−18.3 s**           | **−23.4 s**           |

0 failed and 0 flaky in all 12 runs, with retries armed.

### Why that number understates the change

**The two arms do not run the same suite.** `before` runs 138 tests; `after`
runs 160, because this branch adds `bootBudget.spec.ts` and `navigate.spec.ts`.
Those tests perform their own document loads and consume their own wall-clock,
so whole-suite subtraction charges the change for work it added rather than
measuring the work it removed. All 138 `before` tests are present in `after` —
nothing was renamed or dropped — so the like-for-like quantity is well defined.

**Two different quantities appear here and must not be mixed.** The rows marked
_body time_ are **summed per-test duration** from the Playwright report; suite
wall-clock (`stats.duration`) additionally carries fixture and worker overhead
outside the per-test windows, so the two disagree even within one arm
(`before`/chromium round 1: 362.2 s summed vs 373.6 s wall-clock). Body time is
the only one that can be restricted to a subset of tests, which is why the
like-for-like comparison uses it; the headline verdict uses wall-clock, because
that is the metric the pre-registration named.

|                                     | chromium   | firefox    |
| ----------------------------------- | ---------- | ---------- |
| matched-test body time, before      | 344.2 s    | 486.3 s    |
| matched-test body time, after       | 287.4 s    | 409.0 s    |
| **saved on the pre-existing suite** | **56.8 s** | **77.3 s** |
| body time of the 22 added tests     | 37.4 s     | 53.4 s     |
| net suite wall-clock                | −18.3 s    | −23.4 s    |

The arithmetic closes: 23.4 + 53.4 ≈ 76.8 ≈ 77.3.

Matching is on `(file, title)`: **138 shared, 22 after-only, 0 before-only** in
every one of the six browser/round pairs — `before` is a strict subset of
`after`, nothing was renamed, and every test in a trace appears in the matching
report and vice versa. Per-test and per-file rollups are in the corpus
(`pertest-*.json`, `perfile-rollup.csv`). The largest movers are `posts.spec.ts`
(firefox 171.0 → 114.0 s body time, 95 → 42 test-attributed loads) and
`profile.spec.ts` (33.0 → 25.6 s, 22 → 15).

### Document loads

|                             | before  | after         |
| --------------------------- | ------- | ------------- |
| test-attributed             | 216     | 182           |
| secondary-page (`e2e.page`) | 20      | 21            |
| **total**                   | **236** | **203**       |
| matched tests only          | 236     | **171** (−65) |
| added tests                 | —       | 32            |

**The classification's baseline is confirmed exactly.** Subtracting #863's test
(absent from the #866 corpus, present at the fork point, 5 loads) gives **231
before** — the corpus figure to the load. Every file's reduction matches the
pre-registered per-file rows: profile −7, unicode-slug −2, authed-flash −1,
password_reset −1, visibility −1, posts −49.

### Against the pre-registration

- **Count check: MISSED by one.** Predicted 169, observed 170 corpus-comparable.
  The discrepancy is entirely in `posts.spec.ts` and traces to Amendment 1,
  which assumed that deleting a redundant reload of the already-displayed
  permalink would drop the trace's navigation count by one. It did not — the
  trace did not count that reload as a navigation in the first place. The
  deletion was still correct (it removed a real document load), but its effect
  on _this metric_ was zero, and the amendment claimed otherwise.
- **Floor: MISSED on the pre-registered metric.** −23.4 s firefox against a 33.9
  s floor. The floor was computed from a 62-load saving times `commitToMount`,
  then measured against whole-suite wall-clock — an instrument that does not
  hold the suite constant.
- **Ceiling: exceeded on the like-for-like measure.** 65 loads × 911 ms predicts
  59.2 s for firefox; the matched tests saved 77.3 s. So **a removed navigation
  is worth more than its `commitToMount`** — that figure omits request time,
  fixture and context overhead, and teardown. The next cycle to price a
  navigation should treat `commitToMount` as a lower bound, not a ceiling.
- **Guardrail: met.** 0 failed, 0 flaky across all 12 runs with retries armed.

### What this cost, stated plainly

The pre-registration was written before the branch existed and assumed the
change would only _remove_ things. A change that also adds tests cannot be
measured by whole-suite subtraction, and nothing in the protocol caught that —
not the spec, not the plan, not two rounds of soundness review. The floor was
therefore aimed at a quantity that could not answer the question, and it failed
for that reason rather than because the work underperformed.

Per the spec's abort rule, declared before collection: the idiom and the gate
land regardless, on test-design grounds independent of wall-clock. What fails is
the performance claim _as pre-registered_, and it is recorded here as a failure
rather than rescued by promoting the like-for-like number to the headline after
the fact.

### The confirming set — what the gate actually saves

Gate checks (`.#checks.x86_64-linux.e2e-sqlite-{chromium,firefox}`, 2 workers,
`fullyParallel` on), same shape: 3 runs per arm, interleaved, distinct salts.
Reported, not gated — the spec gives this set no pass criterion.

| arm    | chromium              | firefox               |
| ------ | --------------------- | --------------------- |
| before | 193.0 / 178.1 / 173.8 | 302.9 / 302.1 / 273.5 |
| after  | 181.0 / 167.1 / 166.3 | 293.7 / 289.0 / 268.6 |
| mean Δ | **−10.2 s**           | **−9.1 s**            |

0 failed, 0 flaky in all 12. Navigation totals are **identical to the deciding
set — 236 before, 203 after — in every run and per spec file** (all 24 files
cross-checked, zero differences). Worker parallelism does not change what the
suite navigates, which is what lets the deciding set's count check stand.

**The issue's headline estimate was wrong, and this is the number that corrects
it.** #867 projected "~65 s off the gate". The gate saves **~9–10 s per combo**.
Two reasons, both structural rather than the change underperforming:

- **Parallelism halves the conversion.** At 2 workers a body-time saving reaches
  wall-clock at roughly half. Firefox: shared-test body time falls 555.7 s →
  475.5 s (−80.2 s), the 22 added tests cost 61.0 s, so net body ≈ 19.1 s and ≈
  9.6 s of wall-clock — against 9.1 s observed. Chromium closes the same way.
- **The estimate assumed removal only.** It priced ~74 navigations at full
  `commitToMount` with no parallel discount and no offsetting additions.

**Do not pool these two sets.** At 2 workers summed body time _exceeds_ suite
wall-clock (firefox `before`: ~521–573 s of body time inside a ~274–303 s suite)
— the suite finishes sooner while each test's own window grows, because the
workers contend. Single-worker and gate numbers answer different questions and
averaging them is meaningless.

Incidental: the gate checks already set `JAUNDER_E2E_RETRIES=1` via `extraEnv`
(`flake.nix:968`), so the campaign's `retries` edit was redundant for this set.
It was applied identically to both arms anyway, to keep the protocol identical.

### Reproducing

```console
$ git worktree add <base-dir> --detach wt-base-issue-867
$ # per run: set flake.nix e2eSalt to a unique string, then
$ nix build .#e2e-sqlite-firefox-single-worker --print-out-paths --no-link   # deciding
$ nix build .#checks.x86_64-linux.e2e-sqlite-firefox --print-out-paths --no-link  # confirming
$ # revert e2eSalt to "" afterwards — the e2e-scaffold check forbids committing it
```

Corpus: `~/measurements/jaunder/issue-867-navcount/`. Deciding-set files are
unprefixed, confirming-set files carry a `gate-` prefix, and the README opens
with a table naming both sets and stating that they must not be pooled.

## #792 — the per-test warmup A/B (findings, 2026-08-04)

**Verdict: delete the warmup, both browsers.** It costs 113 s/combo (chromium)
to 139 s/combo (firefox) and buys back at most ~12 s. No flakiness cost.

### Method

Six runs of `cargo xtask traces run`, interleaved `A1 B1 A2 B2 A3 B3`, on a
quiescent host (session baseline `/proc/loadavg` 0.75, 2.10–2.60 during
collection). Arms differ in exactly one token at otherwise gate-identical
settings (`WORKERS=2`, `RETRIES=1`, `vmMemory = 3072`, `vmCores = 2`):

- **Arm A** — the gate as it stands, `JAUNDER_E2E_WARMUP=1`.
- **Arm B** — `e2eWarmup = false`; nothing else changes.

Each run used a distinct `e2eSalt` (`a1`…`b3`). **This is load-bearing**:
without it nix returns the cached derivation and never runs the suite, so runs 2
and 3 would have been byte-identical replays of run 1. Sequential per-combo
start times in the reports confirm all 24 combos genuinely executed.

Deciding data is **sqlite only**, both browsers (per the spec: the backend axis
is not what warmup touches). Postgres was collected at the same settings and is
retained for #817.

### The runs

Every combo of every run, as measured. `expected = 130` throughout;
`unexpected = 0` throughout. No run was discarded.

| run | salt | warmup | started (sq-chr) | loadavg after  | sq-chr | sq-ff | pg-chr | pg-ff           |
| --- | ---- | ------ | ---------------- | -------------- | ------ | ----- | ------ | --------------- |
| A1  | `a1` | on     | 22:27:40Z        | 2.18 2.34 1.96 | 229.9  | 329.6 | 228.0  | 336.2 (1 flaky) |
| B1  | `b1` | off    | 22:49:11Z        | 2.22 2.24 2.08 | 174.0  | 254.4 | 171.2  | 257.7           |
| A2  | `a2` | on     | 23:06:28Z        | 2.27 2.41 2.30 | 224.9  | 323.6 | 227.8  | 327.8           |
| B2  | `b2` | off    | 23:27:16Z        | 2.47 2.43 2.36 | 174.5  | 256.6 | 171.7  | 258.0           |
| A3  | `a3` | on     | 23:44:03Z        | 2.10 2.33 2.35 | 226.8  | 323.7 | 226.2  | 323.9           |
| B3  | `b3` | off    | 00:04:40Z        | 2.60 2.38 2.29 | 172.0  | 256.0 | 171.5  | 257.8           |

Durations in seconds. Session baseline `/proc/loadavg` before the first run:
**0.75 0.94 0.82**. Load was sampled **after** each run rather than both before
and after; since the runs were back-to-back, each row's figure is also
effectively the next run's starting load, and the band (2.10–2.60) never
approached a level that would have triggered the discard rule.

### Suite wall-clock (sqlite, `.stats.duration`, seconds)

| arm           | chromium            | median                | firefox             | median                |
| ------------- | ------------------- | --------------------- | ------------------- | --------------------- |
| A (warmup)    | 229.9, 224.9, 226.8 | **226.8**             | 329.6, 323.6, 323.7 | **323.7**             |
| B (no warmup) | 174.0, 174.5, 172.0 | **174.0**             | 254.4, 256.6, 256.0 | **256.0**             |
| **Δ**         |                     | **−52.8 s (−23.3 %)** |                     | **−67.7 s (−20.9 %)** |

Within-arm spread is ≤2.2 %; the between-arm gap is ~21–23 %. Both arms ran 130
tests, `unexpected = 0` throughout.

**Flakiness guardrail (spec D8): not triggered.** Summed `flaky + unexpected`
across each browser's three sqlite runs is **0 for both arms**. The session's
only flake was one postgres-firefox test in A1 — in arm **A**, and outside the
deciding set. Arm B is not buying speed with retries.

### Where the time goes (span sums per combo, sqlite)

| phase                | A chromium  | B chromium | A firefox   | B firefox |
| -------------------- | ----------- | ---------- | ----------- | --------- |
| `e2e.test.lifecycle` | 428.3 s     | 321.3 s    | 623.1 s     | 475.1 s   |
| `e2e.test`           | 285.4 s     | 297.1 s    | 374.5 s     | 372.3 s   |
| `e2e.warmup`         | **113.3 s** | —          | **139.5 s** | —         |
| `e2e.context_mint`   | 19.1 s      | 15.3 s     | 87.3 s      | 81.7 s    |
| `e2e.teardown`       | 4.4 s       | 3.7 s      | 4.5 s       | 4.4 s     |

The arithmetic closes, and it is worth doing explicitly because it explains the
wall-clock number:

- chromium: −113.3 s of warmup, **+11.7 s** of test time → −107 s of lifecycle →
  ÷2 workers → **−53.5 s wall**, against −52.8 s observed.
- firefox: −139.5 s of warmup, −2.2 s of test time → −148 s → **−74 s wall**,
  against −67.7 s observed.

So the warmup's cache benefit is real but tiny: it buys **~12 s per chromium
combo and nothing measurable on firefox**, for 113–139 s spent. It is a bad
trade by an order of magnitude, and the two-worker divisor is why the
suite-level saving (~53–68 s) looks smaller than the warmup's raw cost.

**The invisible envelope collapses.** `lifecycle − test` — #788's "28–31 %
outside the test span" — goes from **142.9 s (33 %) to 24.2 s (7.5 %)** on
chromium. Most of that envelope _was_ the warmup.

### Correcting #788 on the mechanism

#788 concluded warm ≈ cold (chromium 993 ms warm vs 876 ms cold), reading the
warmup as protecting nothing. **That comparison was confounded** — it set the
warm checks (2 workers) against the cold packages (1 worker). Within a single
arm here:

| arm        | navs | cold | warm | `requestMs` p50 | `commitToMountMs` p50 cold / warm |
| ---------- | ---- | ---- | ---- | --------------- | --------------------------------- |
| A chromium | 210  | 0    | 210  | 31–34 ms        | — / 635–672 ms                    |
| B chromium | 210  | 113  | 97   | 37–41 ms        | 819–880 / 602–630 ms              |
| A firefox  | 210  | 0    | 210  | 98–104 ms       | — / 890–902 ms                    |
| B firefox  | 210  | 113  | 97   | 107–119 ms      | 950–976 / 851–877 ms              |

Warm **is** faster than cold — ~200 ms of `commitToMountMs` on chromium plus
~6–15 ms of `requestMs`. #788 had this backwards. The conclusion survives
anyway, for a different reason than #788 gave: the warmup pays a **full mount
per test** to make ~1.6 navigations per test slightly cheaper. Note also that
arm A shows **zero cold navigations** — the warmup was hiding cold-start cost
from the traces entirely, which is its own argument for removing it.

### Incidental, for follow-ups

- **Firefox `e2e.context_mint` is ~5× chromium's** (p50 511–596 ms vs 96–115 ms;
  81–87 s vs 15–19 s per combo). After the warmup goes, this is the largest
  remaining envelope cost — see #819.
- **sqlite ≈ postgres**, holding across all six runs (e.g. B3 chromium 172.0 vs
  171.5 s). Evidence for #817.
- **Firefox is ~1.47× chromium in both arms**, so the warmup is not what makes
  it slow — see #818.
- The suite is **roughly twice as fast as #788 measured** (chromium 226.8 s vs
  420 s; firefox 323.7 s vs 658 s) — post-#791 seeding. Every percentage in
  #788's write-up describes a suite that no longer exists.

### Reproducing

```sh
# per run: set e2eSalt (distinct) and e2eWarmup in flake.nix, then
cargo xtask traces run
# locate each combo's outputs afterwards (paths differ per salt):
nix build --print-out-paths --no-link .#checks.x86_64-linux.e2e-sqlite-chromium
jq '.stats' <out>/playwright-report-sqlite.json
# traces for the span sums (traces run deletes its own TempDir):
tar -xzf <out>/capture-sqlite.tar.gz capture/otel-traces.jsonl
```

Span sums are `endTimeUnixNano − startTimeUnixNano` grouped by span name;
navigation figures come from each `e2e.test` span's `e2e.navigation_top_json`
attribute. `cargo xtask traces analyze` computes neither medians nor
percentiles, so these were aggregated directly over the JSONL.

## #155 — post-CSR Firefox e2e tax (findings, 2026-07-02)

Re-measurement of the #152 Firefox-vs-Chromium tax on the **leptos-CSR** build
(post-#180; no SSR, no hydration reconciliation). Method: the four warm
`e2e-{sqlite,postgres}-{chromium,firefox}` checks, per-test durations paired
from `playwright-report-<backend>.json`, attribution from
`scripts/analyze-otel-traces`.

**The tax barely moved after the CSR cutover.** Median per-test Firefox/Chromium
ratio:

| backend  | median ratio | mean | tests ≥1.4× | suite total (ff / ch)   |
| -------- | ------------ | ---- | ----------- | ----------------------- |
| sqlite   | **1.83×**    | 1.80 | 61/66       | 585.8s / 336.8s (1.74×) |
| postgres | **1.69×**    | 1.69 | 62/66       | 623.5s / 376.0s (1.66×) |

Compare #152's SSR-era median **1.90×**. Removing hydration did **not** collapse
the gap — strong evidence the cost was never hydration-specific but ongoing
WASM/JS execution + rendering, which CSR still runs in Firefox.

**The delta is uniform and client-side, not server-side.** Distribution peaks in
the 1.7–2.2× bucket (45/66 sqlite) with only 1–4 tests <1.4× — uniform, not a
few hot tests. Attribution (sqlite chromium vs firefox traces):

- `e2e.test` avg: firefox 6813ms vs chromium 3802ms (1.79×), with **identical**
  avg actions (13.78) and firefox making **fewer** requests (31 vs 37) — so it
  is not doing more server work.
- The delta lives in **`navigation.commit_to_mount`** (the commit → CSR
  mount-ready phase): firefox 1123ms vs chromium 559ms = **2.01×**. The
  `wait.mount` action (the mount-ready wait) is the single largest action bucket
  (655ms avg × 302 = 198s); the action was renamed from `wait.hydration` in
  #251.
- Server-side phases are browser-invariant and small: `navigation.request` ~88ms
  avg; API fetches (`/api/current_user` 27ms, etc.) are browser-independent.

**Verdict (AC2): the per-test Firefox tax is irreducible at the per-test level**
— inherent SpiderMonkey-vs-V8 WASM-execution cost, uniform across the suite,
with no hot test to optimize and no hydration left to remove. Therefore **worker
parallelism is the only lever on Firefox e2e wall-clock** (see #182, folded into
#155); per-test tuning is not pursued.

**Per-browser floor (for the Task-6 timeout reconciliation):** at workers:1,
firefox `e2e.test` avg 6.8s / max 21.2s, chromium avg 3.8s / max 11.9s; measured
ratio ~1.7–1.83× vs the `slowBrowserTimeoutScale = 2.2` — the scale is in the
right ballpark (a modest trim, not removal). The phase these budgets cover is
the CSR mount, not hydration; the scalers were renamed accordingly in #224.

## #155 — worker-parallelism safety probes (AC3, 2026-07-02)

Probed `JAUNDER_E2E_WORKERS>1` on CSR (env-driven worker count threaded through
`playwright.config.ts`), each failure mode at its worst case. **CI
`ubuntu-latest` is ~4 vCPU, so the CI-representative probes cap
`virtualisation.cores` at 4** (a 6-core guest oversubscribes a 4-core runner).
Results (sqlite+chromium unless noted):

| config                         | cores | result                         | wall-clock |
| ------------------------------ | ----- | ------------------------------ | ---------- |
| workers=1 (today)              | 1     | 66/66                          | 6.6m       |
| workers=2                      | 4     | **66/66 green**                | 2.0m       |
| workers=3                      | 4     | 1 failed (`posts.spec.ts:349`) | 1.7m       |
| workers=4                      | 4     | 2 failed (`:349`, `:305`)      | 2.0m       |
| workers=4                      | 6     | 1 failed (`:349`)              | 1.4m       |
| workers=4 postgres+**firefox** | 4     | **66/66 green**                | 3.5m       |

**Both fears refuted:**

- **SQLite write contention — refuted.** 4 concurrent workers hammering SQLite
  writes produced **zero** `SQLITE_BUSY` / `database is locked` (WAL + 5s
  `busy_timeout` + `BEGIN IMMEDIATE` absorb it). The `workers:1` comment's
  premise was never tested and is wrong.
- **Firefox OOM — refuted.** Firefox 66/66 clean at 4 workers on a 6 GB VM (the
  4 GB OOM in #61's notes was a smaller VM).

**The real limit is CPU oversubscription, not the DB.** Above workers=2, the
same one or two heavy timeline tests (`posts.spec.ts:349` "local timeline for
unauthenticated users" — a known CSR heavy-test flake — and `:305` "per-user
timeline pagination") exceed their per-test timeout: they create many posts then
render a paginated timeline, and under N-worker CPU contention the client WASM
render slows past the budget. Firefox _passes_ at workers=4 only because its
2.2× timeout scale already absorbs the slowdown; chromium at 1.0× has no
headroom.

**Decision (AC3): GO, uniform `workers=4`.** SQLite contention and OOM are both
non-issues; the flip is safe. The blocker is a timeout-headroom problem, fixed
by making the per-test budget worker-contention-aware for all browsers (Part C)
so the heavy chromium tests survive 4-worker load — chromium is ~1.8× _faster_
per test than firefox, so firefox's proven 2.2× headroom is more than enough for
chromium once applied. (An asymmetric firefox=4/chromium=2 config was considered
— it reaches the same ~3.5m gate with no test changes since the matrix isolates
browsers per VM — but uniform-4 was chosen for config simplicity.) Expected gate
≈ 3.5m (firefox-bound), down from ~10m+ (~65%).

## #155 — flip landed: `workers=2`, small VMs, Firefox slimming (AC4, 2026-07-03)

**Supersedes the AC3 "uniform `workers=4`" decision above.** #210 landed
(batch-seed for the heavy timeline tests); this branch rebased onto it, and the
heavy `posts.spec.ts` timeline tests now seed via `test-support`. With that in,
the flip was re-verified — and a fuller sweep on a real 16-core / 32 GB dev box
changed the chosen operating point.

**What the sweep showed.** At `workers=4` every combo is 71/71 green _in
isolation_ (~3 min Firefox), and CI is unaffected because its matrix runs one
combo per dedicated runner (ADR-0034). But the **local `cargo xtask validate`
aggregate** builds all combos in one `nix build`, and on a host with
`max-jobs>1` they realize concurrently. At `workers=4` each VM needs `cores=4`
(one core per worker or the guest starves — `cores=3` was _worse_, 12–19
failures/combo), so N concurrent VMs demand N×4 host cores; four of them
oversubscribe a 16-core box and trip already-scaled timeouts at random. The
per-VM footprint, not the DB or OOM, is the binding constraint.

Measured (all four combos, 16-core / 32 GB, live-loaded host):

| workers | cores | mem           | concurrency | wall-clock | result           | peak RAM  |
| ------- | ----- | ------------- | ----------- | ---------- | ---------------- | --------- |
| 4       | 4     | 6 GB          | 4-wide      | 6.6m       | flaky (host CPU) | 24 GB     |
| 4       | 3     | 6 GB          | 4-wide      | 12.6m      | badly flaky      | 24 GB     |
| 4       | 4     | 6 GB          | 2-wide      | 8.4m       | flaky            | 12 GB     |
| 4       | 4     | 4 GB+slim     | 2-wide      | 10.5m      | flaky            | 8 GB      |
| 2       | 2     | 4 GB+slim     | 4-wide      | 8.2m       | **green**        | 16 GB     |
| **2**   | **2** | **3 GB+slim** | **4-wide**  | **8.7m**   | **green**        | **12 GB** |

**Budget-bug correction (important — the `workers=4` "flaky" rows above are
tainted).** Those rows ran with a **worker-scaling bug**:
`workerContentionScale` in `fixtures.ts` re-read `JAUNDER_E2E_WORKERS` with its
own default of `1`, which diverged from the config's `workers` default, so when
the env was unset the budgets computed **zero** contention headroom while N>1
workers actually ran. Because the scale is applied as
`max(browserScale, workerContentionScale)`, Firefox (browserScale 2.2) was
unaffected but **chromium (browserScale 1.0) got no headroom at all** — which is
why the `workers=4` failures were overwhelmingly chromium timeouts. Fixed
structurally by deriving the scale from `testInfo.config.workers` (Playwright's
resolved count) so it can never diverge from the running worker count. A
corrected-budget re-test of **`workers=4` / `cores=4` / 6 GB / 2-wide** then ran
**71/71 green on every combo** (chromium 2.9–3.0 m). So `workers=4` is _viable_,
not unfixably flaky.

**Decision (AC4): `workers=2`, `cores=2`, 3 GB VMs, Firefox process-slimming —
chosen on the balance, not because `workers=4` fails.** With both configs green
on corrected budgets: `workers=2` / 4-wide ran the local aggregate in **8.7 m**
vs `workers=4` / 2-wide's **10.8 m** — running all four at once beats
2-at-a-time even though each `workers=4` combo is quicker. `workers=2` also
needs only 2 cores (so it packs 4-wide with no concurrency throttling), is far
less bursty on a shared host (2 browser instances/combo vs 4), and — via the
Firefox `firefoxUserPrefs` slimming (Fission off, single content process,
trimmed caches, transparent to the app-level tests) — fits **3 GB** VMs, ≤12 GB
peak. (`cores` must be `≥ workers` or the guest CPU-starves:
`workers=4`/`cores=3` was _worse_, 12–19 failures/combo.)

**CI tradeoff, accepted:** `workers`/`cores`/`mem` are baked into the shared
`e2eWarmChecks` derivation, so CI's per-combo matrix uses the same values.
`workers=4` would give a slightly faster _isolated_ CI combo (the re-test put
the gap at **~1 min** — ~3 m vs `workers=2`'s ~4–5 m, not the ~4 min first
estimated), but only on CI where each combo has its own runner; locally it is
slower and would need the `--max-jobs 2` throttle re-added. Both configs are a
large reduction from the old ~12 min Firefox long pole (#155's acceptance), so
the ~1 min of CI headroom is worth trading for the simpler, faster, gentler
local story. `--max-jobs` is the only local-only lever (cores/mem/workers can't
diverge local-vs-CI without impurity), and at `workers=2`'s small per-VM
footprint it isn't needed — the host's own `max-jobs` schedules the four 2-core
VMs safely.

**Marginal-test budget fixes (kept from the `workers=4` work):** two tests
bypassed the worker-contention budget and were fixed — the `verifiedUser`
fixture now scales its own timeout at setup time (an in-body `test.setTimeout`
runs too late to cover fixture setup), and `posts.spec.ts` "draft lifecycle"
scales the post-navigation `.j-post-body` assertion (it used the global 5 s
`expect` timeout). Both help any `workers>1` run and give CI headroom.

## Timeout Budgeting

Whole-test budgets are ambient: an auto fixture gives every test a scaled
`DEFAULT_TEST_BUDGET_MS`, and it covers the whole suite — #270 deleted 18 of the
20 per-test budgets after measuring that they guarded nothing. The two that
remain derive their budget from polling deadlines that genuinely exceed the
ambient one.

For an individual assertion that needs longer on a slow browser, use
`slowBrowserTimeoutMs(testInfo, chromiumBudgetMs)` from
`end2end/tests/fixtures.ts` instead of a hard-coded timeout number.

For first document navigation in a test (typically the coldest path), use
`slowBrowserFirstNavigationTimeoutMs(testInfo, chromiumBudgetMs)`.

This applies a project-aware multiplier derived from observed p90 CSR-mount
latency so Firefox/WebKit runs get realistic budgets without increasing Chromium
timeouts unnecessarily.

There is no per-test warmup: every test's first navigation is a genuine cold
load, and the traces report it as one. A warmup existed from 2026-04 to 2026-08
and was removed in #792 after measurement showed it cost 113–139 s per combo to
save at most ~12 s — see the findings section above and
[ADR-0099](adr/0099-e2e-does-not-pre-warm.md).

### Heavy timeline fixture seeding (#210)

The three heavy timeline tests (`posts.spec.ts` `:305`/`:349`/`:410`) seed their
paginated fixtures through the `test-support` binary (ADR-0046) — one in-process
storage write per post — rather than a sequential loop of
`POST /api/posts/create` round-trips. That removes the setup cost `#155`
mitigated with worker-contention timeout headroom (`workerContentionScale` in
`end2end/tests/fixtures.ts`), so that headroom is now a candidate for reduction
once `workers>1` is unblocked (`#173`). The before/after measurement is driven
separately by the `#152` trace-analysis harness (`cargo xtask traces run`); the
timeouts are not re-tuned here.

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
