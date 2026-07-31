# ADR-0011: Unified Observability Strategy

- Status: accepted
- Deciders: mdorman, Gemini CLI
- Date: 2026-05-15

## Context and Problem Statement

Jaunder is a full-stack application with complex interactions between the
backend (SSR, server functions) and the frontend (hydration, client-side
routing). Traditional logging is insufficient for diagnosing performance
bottlenecks or understanding the full lifecycle of a user request across these
boundaries, especially during end-to-end testing.

## Decision Drivers

- Visibility: End-to-end tracing from the test runner through the browser to the
  backend.
- Performance: Ability to identify hydration hotspots and slow database queries.
- Consistency: Using industry-standard protocols (OpenTelemetry).

## Decision Outcome

Chosen option: "Unified Observability with OpenTelemetry", because it provides a
standard way to correlate spans across different environments and languages.

### Implementation Details

- **Backend**: Uses the `tracing` crate with `tracing-opentelemetry` in the
  `server` crate.
- **E2E Test Runner**: Playwright fixtures in `end2end/tests/fixtures.ts`
  generate spans and inject trace context.
- **Correlation**: The `JAUNDER_E2E_TRACEPARENT` environment variable is used to
  propagate trace context from the test runner to the backend.
- **Layered Tracing**:
  - `e2e.test`: Automatic, captures one span per test with resource and
    navigation summaries.
  - `e2e.flow`: Manual, captures domain-specific semantic phases (e.g., "login
    flow").
- **Artifacts**: Traces are exported as JSONL files (`otel-traces.jsonl`) during
  CI and VM test runs for offline analysis.
- **PII discipline**: Span fields and the structured error boundary
  (`error.source`, `error.context` in `web/src/error.rs`) are operator-only but
  are still exported to trace backends, so they MUST NOT carry user PII or
  secrets — email addresses, session/verification tokens, passwords, or post
  bodies. Record stable, non-sensitive identifiers instead (`user_id`,
  `db.system`, `error.kind`/`error.class`); usernames are public identifiers and
  acceptable. The preserved error source chain is built from typed errors
  (`sqlx::Error`, `io::Error`, parse errors), which carry structural/diagnostic
  text — not bound parameter values — so the chain is PII-free as long as
  constructors keep raw user input out of error messages.

## Consequences

- Good: Deep visibility into hydration timing and backend performance during
  tests.
- Good: Correlated traces make it easy to see exactly what happened in the
  backend during a specific E2E test step.
- Bad: Adds some complexity to the test runner and backend initialization.
- Bad: Generates large trace files that require specialized analysis scripts.

## Addendum (2026-06-18): Event metrics pipeline (jaunder-kq8w.21, pre-GitHub bead tracker)

Traces answer "what happened in this one request"; they do not answer "how
often, and is the rate abnormal". This addendum adds an OpenTelemetry
**metrics** pipeline alongside the existing tracer for operational signals —
auth abuse, silent email/WebSub failures, backup health, upload pressure, and an
overall error rate. The full instrument catalog lives in the design spec
(`docs/archive/2026-06-18-otel-metrics-pipeline-design.md`); this records the
conventions and architecture.

### Pipeline

- The OTLP `MeterProvider` is installed in `server::observability` next to the
  tracer (`build_otel_meter`), behind the **same** OTLP-endpoint gate
  (`JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT` / `OTEL_EXPORTER_OTLP_ENDPOINT`). One
  endpoint feeds both traces and metrics; setup failure is non-fatal.
- When no endpoint is set (or in any non-server process — wasm, the CLI), no
  provider is installed and every instrument is a no-op, exactly like traces.

### Emit facade: `host::metrics` (originally `common::metrics`)

> **Superseded (2026-07-09, #345):** the facade now lives in
> **`host::metrics`**, declared unconditionally. `host` is native-only
> (ADR-0058), so `opentelemetry` is kept out of the wasm bundle by crate
> structure rather than the feature gate described below. The text that follows
> is the original, correct-for-its-time rationale; see the 2026-07-09 addendum
> for the move.

- A single facade in `common`, behind an optional **`metrics` feature** (enabled
  by `server` and `web/ssr`, never by the wasm/hydrate build). This is a feature
  gate, not a `target_arch` gate, so `common` stays free of `target_arch` cfgs
  (jaunder-kq8w.10) and the wasm build never pulls `opentelemetry`. `common` is
  the only crate reachable by every emitter (`server → web → common`), including
  the CLI.
- **Cardinality is enforced by the type system**: every helper takes bounded
  Rust enums (e.g. `metrics::login(LoginOutcome::InvalidCredentials)`), each
  mapping to a `&'static str` attribute. A call site cannot pass an unbounded
  label (username, id, URL). Instruments are built once from
  `global::meter("jaunder")` via a `LazyLock`.

### PII discipline

The PII rules above apply unchanged: metric attributes are bounded enums or
fixed operation names, never user input, so metrics carry no PII or secrets.

### Refinements over the spec catalog

- `jaunder.auth.registrations` gained a `closed` policy value (the site can have
  a `Closed` registration policy) and a `cli_bypass` policy for CLI-created
  users.
- `jaunder.media.uploads` gained an `error` outcome for unexpected (non-
  `MediaError`) upload failures.
- `jaunder.posts{event}` counts **web-UI** post lifecycle actions; posts created
  or mutated over `AtomPub` are reflected in `jaunder.atompub.requests` instead,
  so the two never double-count.
- `jaunder.atompub.requests{op,result}` is emitted from a single router-level
  middleware: `op` comes from the matched route template + method (a bounded
  set), `result` from the response status class.
- Email metrics are emitted at the `web` send call sites (where the message
  `kind` is known), not inside the generic mailer.

### Testing

Facade and mapping helpers are unit-tested against an in-memory metric reader
(`opentelemetry_sdk` `testing` feature): install a `MeterProvider` backed by an
`InMemoryMetricExporter`, call a helper, `force_flush`, and assert the named
instrument was exported. Branch-mapping helpers (error kind/class, session
outcome, upload/serve outcome, AtomPub op/result) are additionally covered by
exhaustive table tests so every attribute mapping is exercised regardless of
which request paths a given integration test happens to hit.

### Deferred

- Saturation **gauges** via async observable callbacks (queue depth, pool
  utilization, storage bytes, time-since-last-backup) — jaunder-kq8w.24.
- Making one-shot **CLI** emits actually export (provider init + `force_flush`)
  — jaunder-kq8w.25. CLI emit sites exist but are no-ops without a provider.

## Addendum (2026-06-27): Flushing telemetry from one-shot processes (issue #12)

The metrics addendum above installs an OTLP `MeterProvider` (and the tracer
installs a `TracerProvider`) gated on the OTLP endpoint. Diagnosis of the CLI
case (the kq8w.25 item deferred above) found the providers _were_ already
installed for one-shot commands — `run()` calls `init_tracing` for every
non-`serve` command — but both exporters are deferred: the metric reader is
periodic and the span processor batches, so they only export on an interval the
long-running server easily reaches and a one-shot CLI command exits long before.
The CLI's metric and span emits were therefore installed correctly yet silently
dropped. The fix is to flush before exit, not to install a provider.

Convention: `init_tracing` returns a `#[must_use]` `TelemetryGuard` owning the
installed providers. A process holds the guard for its working scope; the
guard's `Drop` calls `shutdown()` (force-flush + shutdown) on each provider,
exporting buffered telemetry on every exit path — success, `?` error-return, and
panic unwind. A single binding at the `run()` dispatch boundary owns telemetry
for _every_ command — `serve` included — so command bodies (including
`cmd_serve`) carry no telemetry-lifecycle code; for `serve` the guard is simply
held for the process lifetime and flushes at shutdown. Export failures (e.g. an
unreachable collector) are logged, never propagated — a telemetry failure must
not change a command's exit status. This closes the "CLI export" item the
metrics addendum deferred (metrics **and** traces, since both shared the
drop-on-exit defect).

## Addendum (2026-07-09): Metrics facade relocated to `host::metrics` (issue #345)

The 2026-06-18 addendum placed the emit facade in `common` behind an optional
`metrics` feature because, at the time, `common` was the lowest crate reachable
by every emitter (`server → web → common`) and a feature gate kept
`opentelemetry` out of the dual-target crate's wasm build.

ADR-0058 has since introduced `host` — the native-only sibling of `common` — and
the emitter set has grown to include `host` and `storage`. Every crate that
emits metrics now depends on `host` (`storage → host`, `web → host` under its
`server` feature, `server → host`, and `host` itself), and `host` is never in
the wasm dependency closure. The facade therefore moves to **`host::metrics`**,
declared **unconditionally**: `opentelemetry` is excluded from wasm by crate
structure, so the `metrics` feature on `common` — and the `common/metrics` /
`features = ["metrics"]` opt-ins in `host`, `server`, and `web` — are deleted.
This also removes `storage`'s prior reliance on Cargo feature unification to see
the metrics `SessionOutcome` enum (it now references `host::metrics` on a direct
dependency).

No behavior change: the instrument catalog, bounded-enum cardinality discipline,
PII rules, and no-op-without-a-provider semantics are unchanged; only the
facade's crate home moves. Exporter setup remains in `server::observability`.
This is an application of ADR-0058's charter ("any strictly-host-focused shared
code... including production machinery pushed down out of `web`"), not a new
observability decision — hence an amendment here rather than a new ADR.

## Addendum (2026-07-29): Web server-fn spans and the recordable-type allowlist (issue #511)

The PII discipline above told authors what a span field must not carry; nothing
made them write a span at all. **44 of the 55 `#[server]` fns in `web/src` had
none** — a request into `posts::create` or `audiences::create` produced no
top-level span to correlate, and the 11 that existed disagreed with each other
on naming. An unenforced convention is what allowed that, so the convention is
now a gate: `server-fn-tracing`, in `cargo xtask check` and
`cargo xtask validate`.

> **Partly superseded (2026-07-30, #714/#722):** the span-name half of this
> addendum no longer describes the code. `server-fn-tracing` survives, but only
> as the recordable-type allowlist and its sibling default-denies — it neither
> writes nor checks the span name, and it has no fix mode at all. The name is
> derived by `#[macros::server]` instead. The **values** and the **rule** are
> unchanged; the text below is the original, correct-for-its-time rationale for
> who maintained them. See the 2026-07-30 addendum.

### The span

Every `#[server]` fn in `web/src` carries `#[tracing::instrument]`, placed
**after** `#[server]` (the arrangement the pre-existing sites use and that is
known to wrap the server-side body), named:

```
web.<first path segment under web/src>.<fn ident verbatim>
```

The name is a pure function of source location and identifier, so **the gate
writes it**: `cargo xtask check` fills the `name = "…"` in (the same fix-mode
contract `fmt` has) and `cargo xtask validate` verifies it by equality without
mutating. An author writes `#[tracing::instrument(skip_all)]` and the derived
name lands in the source, so nothing is left to judgment and an operator reading
a span name can still grep for the literal. `web/src/posts/api/listing.rs`
yields `posts`, not `api`. A `#[server]` fn directly under `web/src` has no
vertical directory and is a hard error rather than a guessed name.

Only the name is written. A missing `#[tracing::instrument]` stays a _reported_
failure: inserting one would mean guessing the `skip(...)` list, which is the
one judgment the gate refuses to make on an author's behalf.

Deriving the name rather than writing it has a payoff: the fn idents carried a
vertical noun the module path restated (`audiences::create_audience`), and when
#684 shed those nouns, re-running `check` rewrote all 55 span names with no hand
edit and no gate change.

Spans use `#[tracing::instrument]`'s default **INFO** level, and no site sets an
explicit `level`. Stated because it means operator configuration reaches trace
backends at INFO — permitted, since that data is the operator's own, but a
deliberate choice rather than an inherited default. This one is a **convention,
not an invariant**: the gate tolerates an explicit `level`, since level changes
verbosity rather than what is recorded.

### What may be recorded

Every argument must be either named in `skip(...)`/`skip_all` or have a type on
an explicit **recordable** allowlist. The list is **default-deny**: an unlisted
type is not recordable, so a newly-introduced argument type fails the gate until
someone classifies it. The PII decision is forced when it arises rather than
left to a reviewer noticing.

The criterion is **"is this value already visible to the trace's reader, or
bounded by its own type?"** — not "did a user author it". Four grounds admit a
type:

1. **Bounded by the type itself** — it admits no free text: ids, hashes,
   pagination counts, bounded enums, timestamps, `u32`, `bool`.
2. **Operator configuration** — the backup destination, site title, base URL,
   backup schedule. The rule above prohibits _user_ PII and secrets; an
   operator's own settings are neither, and the operator _is_ the trace's
   audience. These are the informative content of the settings write-paths,
   whose spans previously recorded nothing at all.
3. **Already published** — a component of a public permalink (`Slug`,
   `PermalinkDate`, `Tag`), so already in any reverse-proxy access log.
4. **Permitted outright by this ADR** — `Username`, per "usernames are public
   identifiers and acceptable" above. Its own ground rather than part of (3),
   because `login` and `password_reset::request` take a username in a POST body.

Everything else is skipped: secrets, `Email`, `Bio`, `DisplayName`,
`AudienceName`, `SessionLabel`, `Filename`, request-body structs, and bare
`String`.

Note the test is **not** newtype-ness. `Filename` and `AudienceName` are
newtypes that validate a value's _shape_ while carrying arbitrary user text;
`u32` is a primitive that bounds its contents completely. `Filename` in
particular is skipped despite appearing in `media_url()` — a media item's URL is
only discoverable once a published post references it, so an
uploaded-but-unreferenced file's name is published nowhere, and `media::delete`
would otherwise record something like `mri-results-2026.pdf`.

This ADR states the rule; **the gate holds the list** (`RECORDABLE_TYPES` in
`xtask/src/steps/server_fn_tracing_check.rs`, each entry carrying its ground),
so adding a type is a code change that shows up in a diff. Like ADR-0066's
registrar guard, the requirement is mandatory with **no per-fn opt-out**.

`fields(...)` value expressions are held to the same allowlist, since
`skip(email)` paired with `fields(who = %email)` would otherwise satisfy an
argument-level check while recording the email anyway. The field _name_ (left of
`=`) is not checked — a field may be named after a skipped argument as long as
its value does not read it.

### Two caveats, recorded deliberately

- **`Bio`/`DisplayName` are skipped because nothing publishes them _today_.**
  They are reachable only through `profile::get`/`profile::update`. If a public
  `/@username` page later renders them, the classification warrants revisiting —
  the gate will not notice on its own.
- **Allowlisting `u32` leaves a narrow hole**: a numeric OTP or PIN would pass.
  ADR-0063 closes it by convention, since such a value would arrive as a newtype
  (`OtpCode`), not a bare `u32`. Recorded rather than solved with gate
  machinery.

### Deliberately out of scope

`#[instrument(err)]` and `ret` are **rejected** by the gate. Recording server-fn
failures as span errors is desirable but changes _what_ is recorded, and needs
its own PII review of the `WebError` `Display` chain — a different question from
span presence. Follow-ups filed as **#684** (path-based `#[server]` matching,
which unblocks dropping the vestigial fn-ident nouns) and **#685** (`login`'s
un-newtyped `label`).

## Addendum (2026-07-30): the span name is macro-derived, and `server_fn` retires (issues #714, #722)

The #511 addendum made the span name a pure function of source location and
identifier, then had **the gate write it into the source**. #714 moves the
derivation into `#[macros::server]` (`macros/src/server_fn.rs`), which emits
`#[::tracing::instrument(name = "web.<vertical>.<ident>")]` — the same value,
from the same inputs, by the same rule. This resolves
[#722](https://github.com/jaunder-org/jaunder/issues/722), which asked the
general question: when a literal restates what the source already encodes, do
you derive it or generate-and-gate it?

### Why derivation was rejected in #511, and why it is available now

Letting `#[tracing::instrument]` name the span itself was never an option on a
`#[server]` fn. `#[server]` relocates the annotated body into a generated fn
named `__server_<ident>` (`server_fn_macro-0.8.10/src/lib.rs:1578`), and the
instrument attribute — which must sit inside `#[server]` to wrap the server-side
body at all — derives its name from whatever fn it ends up on. Plain derivation
therefore yields `__server_create`: a macro-internal name, unusable as an
operator-facing span. That coupling is what forced an explicit `name = "…"`, and
having forced it, #511 gave the literal to the gate to maintain.
`#[macros::server]` breaks the coupling from the other side — it computes a
readable name from the file path and ident and writes it into its own expansion
— so the span name no longer depends on what `#[server]` calls its generated fn.

Superseded from the #511 addendum, item by item:

- **Nothing writes the name into the source, and nothing checks it there.**
  `Mode::Fix` for span names is gone; `cargo xtask check` no longer rewrites
  anything under `web/src` (see ADR-0082's #714 amendments).
- **The greppability payoff is deliberately given up.** #511 argued that landing
  the derived name in the source let "an operator reading a span name still grep
  for the literal". After #714 the literal exists only after macro expansion.
  That is a real loss, accepted because a string nobody writes cannot drift.
- **Presence is no longer a _reported_ failure — it is not representable.** A
  `#[macros::server]` fn always carries the instrument attribute, so the "a
  missing `#[tracing::instrument]` stays a reported failure" rule has nothing
  left to report.
- **`fields(…)`, `level`, `err`, and `ret` are rejected by the macro**, not
  tolerated or allowlisted by the gate. #511 held `fields(…)` value expressions
  to the recordable allowlist to close the `skip(email)` +
  `fields(who = %email)` hole; the macro closes it by construction instead, so
  the value-expression allowlist is deleted with it. The explicit-`level`
  tolerance goes the same way — no site set one. Re-admitting `fields` means
  re-admitting the value allowlist in the same change.
- **The placement rule narrows the derivation's input.** #511 noted
  `web/src/posts/api/listing.rs` yields `posts`, not `api`. #714 forbids that
  file shape outright: every `#[server]` fn lives in `web/src/<vertical>/api.rs`
  (ADR-0070), so the vertical is the only segment there is.

What `server-fn-tracing` retains is the substance: the `RECORDABLE_TYPES`
default-deny over every parameter, the nameless-parameter rule, and default-deny
on any argument it does not model. It now reads `skip(…)`/`skip_all` from
`#[macros::server(…)]`, since no `#[tracing::instrument]` survives in source.
The macro's own key list is a second, independent default-deny, so adding a key
there cannot silently widen what may be recorded.

### `server_fn` as a log field is retired

The structured error boundary emitted a `server_fn` field naming the failing
function, taken from the `boundary!` label. It is deleted along with the label
(`boundary!` itself is gone; `#[macros::server]` wraps every body in
`crate::error::server_boundary` unconditionally). The field was redundant with
span context and strictly less precise: the failure event is raised inside
`web.<vertical>.<ident>`, whereas the bare label was the fn ident alone — which,
after #684 shed the vertical nouns, is ambiguous across verticals (three
`create`s).

The redundancy is a property of both configured sinks, not a hope:

- The JSON formatter renders current span and span list by default
  (`display_current_span`/`display_span_list` are `true` —
  `tracing-subscriber-0.3.23/src/fmt/format/json.rs:334-342`).
- The plain-text `Format<Full>` walks `ctx.event_scope()` unconditionally
  (`format/mod.rs:985-1000`) — no flag guards it.

So no operator loses the function's identity on any sink. The other five fields
(`error.kind`, `error.class`, `error.public`, `error.source`, `error.context`)
are genuine per-failure data and are unchanged, as are the PII rules governing
them. No event's existence or level changes; one duplicated field goes.
