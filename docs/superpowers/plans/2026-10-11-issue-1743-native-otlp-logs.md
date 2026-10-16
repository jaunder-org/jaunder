# Issue #1743 — native OTLP logging implementation outline

> Qualified revision: JAU-1743-COLLECTOR-AMENDMENT-001, human “both approved”,
> acknowledged by Lead against technical contract SHA256
> `c5ee2ad723645acbff1cda627e3154282f95e468ff04991ae8a28cec879580f6` and the
> latest decision card. The
> [spec](../specs/2026-10-11-issue-1743-native-otlp-logs.md) remains
> authoritative. This outline records implementation and acceptance obligations,
> not production activation, shipping or merge approval.

## Scope and sequence

Stock appender/SDK wiring replaces application privacy/admission visitors, Debug
parsing, path enrichment and private exporter classification. One shared
producer-qualified production/test Collector policy precedes batch and durable
log/trace sinks. Local output, source-admitted spans, operation determinants,
metrics, browser evidence, correlation, lifetime and failure safety remain.
Approved losses are parser state/EOF/future-warning detail, free-text status
descriptions, direct-log paths and private per-kind/coalescing/256-byte
promises. No new guarantee loss or public seam follows from implementation.

No production activation, historical repair, credentials, retention/budget
changes, storage Collector work or unrelated UI changes. The accepted browser
fixture repairs preserve assertions and test identities: Post-ID-owned image
selectors, exact routes, and Site Theme reset/restoration inside existing serial
configuration projects. They do not authorize font/pixel redesign or retries to
green.

## Behavior packets

- [x] A. Calibrate the selected stock composition before relying on removals.
  - Admitted synthetic wire/configuration/failure proofs establish ordinary and
    facade producers, scalar carriers, context, selected parser policy,
    secondary-copy removal and safe persisted failure diagnostics.
  - Negative SDK diagnostics remain visible with safe detail; Collector parse
    and actual output-write failures remain failures. Source-input taps are test
    stimulus, not production observers or diagnostic-output exemptions.
  - These focused cases do not promise volume recovery, universal sanitization
    or exhaustive workload coverage.
- [x] B. Integrate stock emission and minimal demonstrated mechanics.
  - Exact stock appender 0.32.0, no experimental metadata/span copying; ordinary
    batch/provider semantics. Application visitors, Debug parsing, direct-path
    enrichment, per-kind classification/coalescing and size policy are absent.
  - Retain owning public event-context adapter, suppression-preserving
    Activation, SDK-scope-only dynamic trace-layer gate and process-owned
    transport runtime. Process-global subscriber, not scoped worker-dispatch
    propagation; repeated-ID latest-exit and residual-work limits remain.
  - Six pinned remote-detail SDK identities use SafeSdkFormat at both host fmt
    variants and independent server WARN+ JSON. Other records delegate
    unchanged; opaque setup/nonrecursive shutdown fallbacks remain.
  - Root guard ordering is meter → tracer → logger → runtime. Public
    subscriber/lifecycle/failure tests, provider-presence regressions and
    independent source reviews support this packet, not final coverage.
- [x] C. Install the canonical shared Collector policy.
  - `nix/otel-processors.json` defines `transform/jaunder_privacy` with
    `error_mode: propagate`. Production and local/VM assemblies use privacy →
    batch for logs/traces; metrics remain unchanged. Closed checker, module
    assertions, launcher and source-closure inputs agree.
  - Producer-qualified retention is not a global field union. Every selected
    parser occurrence/severity/context survives with fixed text; status codes,
    operation/browser evidence and stock integer/string carriers remain.
  - Invalid policy has no raw fallback; `propagate` is not diagnostic redaction.
- [ ] D. Complete integrated behavior proof.
  - Both real Victoria backends verify `scope.name`, meaningful `_msg`, INFO
    severity and string-valued status `"401"`; exact log span ID joins the
    stored request span with URI/method/remote parent. SQLite additionally
    verifies persistence. No legacy target/path attribute or timestamp join.
  - Current-source SQLite Chromium auth flow is independently proven. This is
    not PostgreSQL Chromium proof, a full browser matrix, or a replay of private
    parser fixtures when the live workload emitted none.
  - Keep comparable before/after message-presentation evidence and original
    zero-panic/boot/determinant obligations. Record first failures and
    unexecuted work honestly; no unchanged retries or expectation weakening.
- [ ] E. Review and certify the final current tree.
  - Reconcile these seven working/current documents with actual implementation,
    retaining numberless/proposed ADR status and the same approved contract.
  - Prove accepted browser repairs through their existing projects on Chromium
    and Firefox with appropriate backend parity; source/compiled routing alone
    is not browser proof.
  - Review the integrated diff before applicable broad validation, authoritative
    coverage/CSR checks and current-tree CI. Historical green does not certify
    changed source. Use the normal CONTRIBUTING ladder, not a new framework.
  - Commit/push/merge/activation are separate authority. Retire spec/outline
    only after conformance under the established artifact lifecycle; merge
    requires explicit per-PR approval.

## Risk and proof discipline

The approved privacy seam accepts raw SDK/transport/trusted Collector input, not
raw durable output or persisted diagnostic leaks. New content-bearing or
attributed-link producers need review; current links have empty attributes.
Message constructors, TraceField admission and ADR-0168's isolated synthetic
browser qualification remain in force.

Collector policy cannot protect SDK stderr or solve generation/feedback/queue
pressure. Bounded shutdown acknowledgment is not durable-delivery or worker
containment proof. Runtime failure, timeout, truncated capture and missing
artifacts remain failed or incomplete evidence. Diagnose the owning seam and
return out-of-scope product changes to Lead rather than changing assertions,
resource budgets, admission or policy semantics.

Use supported `cargo xtask e2e-local <spec>` for focused SQLite iteration and
normal Nix/xtask backend/browser lanes for hermetic parity. Full Firefox
verdicts require every retained producer plus reconciliation. CSR bundle
integrity and source provenance are distinct from CSR lint and coverage. Final
broad gates remain after writer settlement; no stale e2e sidecar substitutes for
a gate.

Execution logs, inventories, approval records and point-in-time test counts stay
in ignored evidence storage and review handoffs, not this enduring contract.
