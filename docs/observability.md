# Observability

This project emits OpenTelemetry traces from both the backend and end-to-end
test runner.

## Backend

- Backend spans are produced via `tracing` + OpenTelemetry in the `server`
  crate.
- In e2e VM checks, traces are exported to the in-VM collector and written under
  the capture-dir contract (#332):
  - `/var/lib/jaunder/capture/otel-traces.jsonl` (inside the VM)
  - lifted per combo inside `capture-<backend>.tar.gz` (the same bundle that
    carries `diag.log` and the mail/websub JSONL — see below)

## End-to-End Tracing Layers

### What is a span and what is an attribute (#794)

This distinction is easy to get wrong — #788's write-up said "`action.timed`
×1233 **spans**", but 1233 was the _entry count inside one attribute_. To be
exact:

- **Spans**: `e2e.test.lifecycle`, `e2e.test`, `e2e.context_mint`, `e2e.page`,
  `e2e.teardown`, `e2e.flow.*` (browser side); `request`, `storage.*`,
  `crypto.*` (server side).
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

Marks are harvested per navigation at that document's `load` event. `goto` waits
only for `domcontentloaded`, so **a navigation whose test finishes before `load`
fires records no marks** — in the measured run, 73 navigations across 59 of 127
tests carried them. Those navigations report the marks as _absent_, never as
zeros, so a missing decomposition cannot be mistaken for an instant one.

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
above rather than asserted. Three committed artifacts under `docs/coverage/`:

| File                        | Owner        | Contents                                                          | Compared      |
| --------------------------- | ------------ | ----------------------------------------------------------------- | ------------- |
| `server-fns.json`           | generated    | the covered fn set, plus an orphan bucket keyed by reason         | byte-for-byte |
| `server-fns-evidence.json`  | generated    | server fn → the named tests that drove it                         | no            |
| `server-fns-allowlist.json` | hand-written | one entry per knowingly-uncovered fn: fn name, reason, issue link | n/a           |

**The split is load-bearing (#745).** The gate asserts the fn _set_; the test
titles are evidence for a reader. Titles do not reproduce — two runs of the same
e2e derivation on the same tree disagree, because a test that ends
mid-navigation leaves its page booting and the boot is truncated at a different
point each run — so byte-comparing them reddened the build on unrelated PRs. The
attribution itself is sound; nothing is bound to a test that did not cause it.
The static lane cross-checks that the two generated files name the same fns, so
the evidence cannot silently fall out of step; it does **not** check the titles,
which is how a renamed test can leave a stale one behind (#757).

A fn is identified by the **union** of two signals: its **span name** with a
matching `code.namespace`, or a request **`uri`** resolving to the fn's declared
endpoint. Attribution is an **ancestor walk** up `parent_span_id` to a known
`e2e.test` span — `uri` hits resolve in one hop, span-name hits in two.

**The span name is matched forward, from the inventory — never inverted out of
the name.** This repo has already had two naming regimes: `server-fn-tracing`
writes `web.<vertical>.<ident>` today (#511, ADR-0011), while omitting the
explicit `name` derives `__server_<ident>`, because `#[server]` relocates the
annotated body — and its `#[tracing::instrument]` — into a generated fn of that
name (`server_fn_macro`'s `to_dummy_ident`). The extractor computes every
candidate for each inventory fn and accepts any, so a regime change is a code
update rather than a silent outage. An earlier version matched one shape only
and therefore matched **nothing**, silently: `uri` covered the same fns, so the
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

To regenerate after adding a flow:

```bash
cargo xtask e2e sqlite chromium            # writes .xtask/diagnostics/e2e-sqlite-chromium/capture-sqlite.tar.gz
cargo xtask server-fn-coverage regenerate  # rewrites server-fns.json AND server-fns-evidence.json
cargo xtask server-fn-coverage verify      # what the e2e lane runs: fails on snapshot drift
```

`regenerate` always writes **both** generated files; `verify` compares only
`server-fns.json`. Note that a second `cargo xtask e2e sqlite chromium` on an
unchanged tree replays the cached Nix derivation in seconds and re-lifts the
same capture, so it cannot be used to confirm that a regenerated artifact is
stable — that is a statement about _different_ runs, and `nix build --rebuild`
is what forces one.

**Everything fails closed.** A missing, empty, or unparseable capture is an
error, never "no uncovered fns" — otherwise the failure mode the gate guards
against and the failure mode of its own plumbing would look identical. The same
rule covers a missing or unparseable evidence file: it is an error, not an empty
one.

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

It is reported, not failed. Expect exactly the app-shell fns (`session`,
`list_local_timeline`, and the two warning-visibility fns), each carrying the
single reason `unknown-parent:` naming the run-wide traceparent's span id. That
is app-shell traffic issued in the **pre-test window** — after the browser
context exists but before `applyTestTraceparent` stamps it — which is
deliberately unattributed per #681. A different reason, an unfamiliar parent id,
or any other fn appearing means a context lost its traceparent or the capture is
truncated.

**These four survived #792**, which is worth recording because it was predicted
they would not. The per-test warmup's `/` load was assumed to be the bucket's
source, so removing the warmup should have emptied it and drifted this snapshot.
It did neither: the byte-compare passed unchanged against a post-removal run.
The orphan bucket's source is the pre-test window itself, not any particular
thing that used to occupy it — which is exactly why the mechanism is structural
and stays.

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
