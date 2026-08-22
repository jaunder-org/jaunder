# Issue #16 — Decision-path tracing implementation outline

> Execute with `jaunder-iterate`, delegating slices through `jaunder-dispatch`
> when useful. This outline exists because the approved spec changes the
> observability architecture and error-carrier contract.

## Scope

In:

- Add operator-only span-stack capture to `host::error::InternalError`.
- Emit that span stack from boundary/continued error telemetry without changing
  public `WebError` projection.
- Migrate one representative branch-name path in registration to determinant
  fields.
- Decide and document the slow/error-only retention boundary for this issue.
- Update durable observability guidance and ADR material.

Out:

- New collector, dashboard, external sampling deployment, or telemetry backend.
- Repository-wide branch-span migration.
- Client-visible `SpanTrace` or API shape changes.
- Relaxing recordable-type, PII, or secret-redaction rules.

## Task outline

- [x] Task 1: Capture and expose operator span traces
  - Contract: add `tracing-error` as a host-owned observability dependency;
    `InternalError` stores a span-stack snapshot captured during construction.
    Keep `Display`, `Error::source`, `public_message`, and `operator_message`
    public behavior unchanged except for any explicit operator-only
    accessor/field used by emitters.
  - Contract: every `InternalError` constructor path, including `masked`,
    `validation_source`, `server_boxed`, and `From` conversions, must capture
    consistently through the common constructor path rather than ad hoc per
    variant.
  - Contract: wire `tracing_error::ErrorLayer` into the normal
    `host::telemetry::init_tracing` subscriber stack, not only into tests, so
    production `SpanTrace` capture sees the active span stack and recorded
    fields.
  - Verification: host error tests prove a nested instrumented failure captures
    both parent and child spans plus recorded fields, and existing
    masking/source-chain tests still pass.

- [x] Task 2: Emit span traces on error telemetry
  - Contract: `emit_boundary_failure` includes the span trace as an
    operator-only structured tracing field beside the existing `error.kind`,
    `error.class`, `error.public`, `error.source`, and `error.context` fields;
    `record_error` metric attributes stay bounded and unchanged.
  - Contract: continued-after-error reporting (`report_swallowed` /
    `report_client_swallowed`) must be addressed explicitly. Preferred
    implementation: native `report_swallowed` captures/emits equivalent active
    span context; client-swallowed telemetry remains bounded-client-data-only
    and documents why browser-side span stack is not available here.
  - Verification: existing boundary-event tests continue to prove the enclosing
    span is in scope; new tests assert boundary events include the span trace,
    native `report_swallowed` includes or deliberately omits active span context
    exactly as documented, `report_client_swallowed` has the documented bounded
    client-only rationale, and `WebError` projection is unchanged.

- [x] Task 3: Migrate registration branch spans to determinant fields
  - Contract: `web.registration.register` remains the macro-derived server-fn
    span; remove the branch-specific child span names
    `web.registration.register.create_user_open` and
    `web.registration.register.create_user_invite` as the representative
    migration.
  - Contract: registration records determinant fields on the owning span, at
    minimum `registration.policy`, `registration.invite_present`, and
    `registration.outcome`; values are bounded tokens and contain no invite
    code, password, email, or free-form source text.
  - Contract: storage/session child spans stay separate where they time
    meaningful called work (`storage.user.create_user`, invite-backed atomic
    create, session creation). Because the current invite path's only timing
    span is branch-named, replace it with a non-branch-named operation span
    before removing `web.registration.register.create_user_invite`; the
    migration moves branch identity into fields, not useful operation timing out
    of the trace.
  - Verification: a focused web/server test or trace-recorder unit asserts the
    determinant fields on the enclosing `web.registration.register` span for
    open, invite-present, invite-missing, and closed policy paths as reachable
    without over-mocking.

- [x] Task 4: Settle slow/error-only retention for this issue
  - Contract: if in-process dump-on-error is implemented, it must build on the
    existing `host::telemetry::SlowSpanLayer` close-hook shape and must not emit
    buffered branch logs for successful non-slow spans.
  - Contract: if collector-side tail sampling is documented instead, no
    in-process retention scaffolding lands; the docs must state that Jaunder
    emits determinant fields and SpanTrace, while collector policy decides trace
    retention.
  - Verification: either tests pin the slow/error emission behavior, or the
    delivered diff contains the explicit collector-side documentation and no
    inert retention scaffold.

- [x] Task 5: Record durable guidance
  - Contract: update `docs/ARCHITECTURE.md` observability text and contributor
    guidance with the determinant-field convention: span names identify
    operations; fields describe decisions; fields are per-span declarations
    recorded when known; use the narrowest owning span; avoid secrets, raw
    emails, request bodies, arbitrary source text, and high-cardinality struct
    dumps.
  - Contract: create a numberless ADR draft under `docs/adr/drafts/` extending
    ADR-0011 if implementation adds SpanTrace or changes server-fn
    instrumentation policy; project its consequence into `docs/ARCHITECTURE.md`
    immediately. `jaunder-ship`/ADR promotion owns numbering later.
  - Verification: docs name the `boundary!` macro only as deleted history and
    direct new automation to `#[macros::server]` or helpers.

- [x] Task 6: Certify and commit
  - Contract: update this outline checkbox state before the commit gate; use
    `jaunder-commit` discipline; no lint suppressions without explicit user
    approval; no `Co-Authored-By` trailer.
  - Verification: `devtool run -- cargo xtask check` passes after any auto-fixes
    are inspected and staged.

## Risk checks

- Public boundary invariant: `InternalError` operator details and span traces
  never cross into `WebError` or serialized client responses.
- Cardinality/PII invariant: determinant fields and span traces must not
  introduce passwords, tokens, raw emails, invite codes, request bodies, or
  arbitrary source text as span attributes.
- Trace shape invariant: meaningful called work can remain child spans; branch
  decisions become fields on the owning span rather than span-name suffixes.
- Host layering invariant: `host` may depend on infrastructure crates like
  `tracing-error`, but must not learn `web`/`storage` abstractions.
- Metrics invariant: `jaunder.errors` attributes remain the existing bounded
  enum strings; span trace text is tracing/log-only, not a metric attribute.
- ADR/doc invariant: any accepted instrumentation-policy change is reflected in
  the architecture materialized view before ship.
