# Spec — #345: move `common::metrics` → `host::metrics`, delete the `metrics` feature

**Status:** design resolved (mechanical relocation within an existing charter).
**Applies:** ADR-0058 (host = strictly-host-focused shared code). **Amends:**
ADR-0011 (unified observability) — its "facade home: `common::metrics`
(feature-gated)" decision is superseded.

## Problem

The OTel metrics facade lives in `common/src/metrics.rs` behind `common`'s
optional `metrics` feature (`metrics = ["dep:opentelemetry"]`, gated
`#[cfg(feature = "metrics")] pub mod metrics`). The gate exists for one reason:
`common` is **dual-target** (host + wasm), and `opentelemetry` must never enter
the wasm bundle. So every server-side consumer must remember to enable the
feature, and the browser build is kept clean by a flag rather than by structure.

ADR-0058 has since introduced `host` — the **native-only** sibling of `common`,
chartered for "**any** strictly-host-focused shared code... including
_production_ machinery pushed down out of `web`." The metrics facade is exactly
that: native-only shared code that only makes sense on the server/CLI. Placed in
`host`, it needs no gate — `opentelemetry` stays out of wasm _structurally_
because `host` is never in the wasm dependency closure.

## Verified premises (why this is safe)

- **P1 — reachability.** Every crate that emits metrics already depends on
  `host`: `storage → host` (`storage/Cargo.toml:9`), `web → host` (SSR-only,
  optional, `web/Cargo.toml:19` under the `server` feature), `server → host`
  (`server/Cargo.toml:16`), and `host` itself (`host/src/error.rs`). `host` is
  the **lowest** crate reachable by all emitters _and_ excluded from wasm. ⇒ no
  new workspace-dependency edges are needed.
- **P2 — ADR-0058 invariant holds.** The invariant is "`host` depends on no
  _workspace_ crate except `common`." `metrics.rs` depends only on
  `opentelemetry` (+ `opentelemetry_sdk` in tests) — external _infrastructure_
  crates of exactly the permitted kind (like `host`'s existing `sqlx`/`http`).
  It references no workspace crate. ⇒ move is in-charter; no cycle.
- **P3 — no wasm regression.** `web`'s default/`csr`/hydrate build never enables
  the `server` feature, so it never links `host`, so it never pulls
  `opentelemetry` — the same guarantee the feature gave, now enforced by the
  crate graph instead of a flag. ⇒ `wasm-clippy` and the wasm bundle stay clean.
- **P4 — `common` is untouched otherwise.** The `metrics` feature gates _only_
  `pub mod metrics`; nothing else in `common` is conditional on it. ⇒ deleting
  the feature and the optional `opentelemetry` dep is clean and leaves `common`
  lighter (and preserves its "zero host-only carve-outs" invariant — this
  removes a feature-cfg, adds none).

## Target end state (acceptance)

1. `host/src/metrics.rs` is the facade's home, declared **unconditionally**
   (`pub mod metrics;`) in `host/src/lib.rs`. `common/src/metrics.rs` is gone.
2. `opentelemetry` is a plain (non-optional) dependency of `host`;
   `opentelemetry_sdk` is a dev-dependency of `host`. Both are removed from
   `common/Cargo.toml`, and the `metrics` feature and its
   `#[cfg(feature = "metrics")]` gate no longer exist anywhere.
3. All emit sites reference `host::metrics::*` — `web` (`posts/`, `auth/`),
   `storage` (`sessions.rs`), `host` (`error.rs`), `server`. No
   `common::metrics` path remains in the tree.
4. `web`'s `server` feature list no longer contains `"common/metrics"` (it
   already pulls `dep:host`); `host`/`server` no longer pass
   `features = ["metrics"]` to `common`.
5. `storage` references `host::metrics::SessionOutcome` on its **direct,
   unconditional** `host` dependency — the prior reliance on Cargo feature
   unification (`storage` used `common::metrics::*` without enabling the
   feature) is eliminated.
6. ADR-0011 is amended: its facade-home decision points at `host::metrics` and
   cross-references ADR-0058. The `common::metrics` module doc (moved to
   `host::metrics`) and any doc-links are updated. No new ADR — this is an
   application of ADR-0058, not a new decision.
7. `cargo xtask validate` is green: static + clippy + `wasm-clippy` + coverage +
   the full e2e matrix. (metrics.rs stays host-compiled and coverage-measured,
   as it is today.)

## Non-goals / notes

- **No behavior change.** Pure relocation + feature deletion; the emitted
  metrics, enum values, and no-op-without-MeterProvider semantics are unchanged.
- **Accepted cost:** `test-support` (a `host` dependent) now pulls the
  `opentelemetry` **API** crate transitively where it previously did not. This
  is acceptable — `test-support` is native e2e tooling, and the API facade is
  lightweight (the SDK stays a dev/server-side dependency, not a `test-support`
  build cost). No feature gate is reintroduced to avoid this: reinstating a gate
  would defeat the entire point, and `host` being native-only means there is no
  wasm hazard to gate against.
