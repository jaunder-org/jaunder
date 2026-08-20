# Spec — #353: harden `host::metrics` facade tests and init-order docs

**Status:** draft for approval.  
**Issue:** #353.  
**Scope:** `host::metrics` test assertions and the documented `MeterProvider`
initialization contract. No metric name, attribute value, emitter call site,
exporter setup, or runtime metrics behavior changes. A `Cargo.lock` h2 patch
update is allowed only because the per-commit gate's `cargo-deny` advisory check
failed on the branch base; it is gate hygiene, not part of the metrics design.

## Context

`host::metrics` is the native-only OpenTelemetry metric facade. It builds all
instruments once from `opentelemetry::global::meter("jaunder")` through a
`LazyLock`; exporter setup remains owned by `server::observability` per ADR-0011
and the #345 relocation to `host`.

The current facade already has one process-global metric-export test,
`every_emitter_exports_its_instrument`, because `global::set_meter_provider` is
process-global and install-once in practice. That test exports every instrument
and already asserts several derived attribute mappings, but it does not
explicitly prove that `login(LoginOutcome::InvalidCredentials)` records the
`outcome=invalid_credentials` datapoint named in issue #353.

The module docs say instruments are no-ops when no provider exists, but they do
not state the load-bearing ordering rule: because instruments are cached in
`LazyLock`, the real `MeterProvider` must be installed before any first metric
emission initializes `M`. If the facade is first touched while the global
provider is still the no-op provider, the cached instruments stay bound to that
provider and later exporter setup cannot repair that process.

## Required behavior

1. `host::metrics` documents the initialization-order contract at the module
   boundary:
   - `server::observability` / the binary must install the `MeterProvider`
     before any code path emits a metric.
   - No-provider mode remains allowed and intentional for processes without OTLP
     setup.
   - The cached-instrument consequence is explicit: first access determines the
     meter behind `M` for the process.
2. The existing process-global metric test asserts the login outcome attribute,
   not just instrument presence:
   - `login(LoginOutcome::InvalidCredentials)` must produce a
     `jaunder.auth.logins` counter datapoint with `outcome=invalid_credentials`.
   - A mutant that drops the attribute, renames `outcome`, or maps
     `InvalidCredentials` to the wrong string must fail this test.
3. The `email_send_result` branch assertions remain covered. The old issue note
   about those calls being unasserted is stale in the current tree; do not
   remove that coverage.
4. No second metric-exporter test is added unless the design also removes the
   process-global hazard. Prefer extending
   `every_emitter_exports_its_instrument`.
5. No runtime guard is required for #353. A debug assertion would need a
   reliable way to distinguish “intentional no-provider process” from “server
   emitted before setup”; that seam is not present today, and adding it would
   exceed this hardening issue.

## Non-goals

- Do not change metric names, attribute vocabularies, enum variants, or
  dashboard-facing strings.
- Do not move exporter setup out of `server::observability`.
- Do not add async observable gauges; #13 owns saturation gauges.
- Do not introduce a metrics feature gate; ADR-0058/#345 keep `host` native-only
  by crate structure.
- Do not make CLI/test-support telemetry writes visible in e2e traces; #769 owns
  that.

## Gate precondition discovered during implementation

`cargo xtask check --no-test` initially failed before the #353 code could be
committed because `cargo-deny` reported the h2 unbounded empty DATA frames
advisory in the branch-base lockfile. The implementation may update only h2 in
`Cargo.lock` to the latest compatible patch release so the required gate can
certify the metrics change. Do not change manifest requirements or unrelated
dependencies.

## Acceptance

- `host/src/metrics.rs` module docs include the provider-before-first-emit
  contract and reference the cached `LazyLock` instruments.
- `host::metrics::tests::every_emitter_exports_its_instrument` fails if `login`
  emits no `outcome` attribute or maps `InvalidCredentials` incorrectly.
- The test still uses the single in-memory exporter/provider install already
  present in the module.
- `devtool run -- cargo nextest run -p host metrics::tests::every_emitter_exports_its_instrument`
  passes.
- `devtool run -- cargo xtask check` passes before commit.
