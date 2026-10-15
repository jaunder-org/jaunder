# #1780 — Invocation-local E2E lane concurrency

## Outcome

A caller deliberately running full local `cargo xtask validate` can bound its
E2E producer fan-out to fit the CPU/RAM capacity admitted for that invocation.
The same complete enabled lane population and authoritative verdict remain
required; a smaller concurrency bound changes scheduling, not protection.

## Load-bearing decisions

### Caller contract

- Add `cargo xtask validate --e2e-jobs <N>`, accepting a positive integer.
- Omission preserves existing behavior: all enabled lanes may be dispatched
  concurrently. This is compatibility, not an assertion that the host admits it.
- Reject zero, negative, malformed, missing, and unrepresentably large values
  before validation work starts. Values above the lane count are valid.
- Reject explicit `--e2e-jobs` together with `--no-e2e`; do not silently ignore
  a caller's resource-control request.
- Before E2E dispatch, report the requested bound (or its omitted/default
  state), enabled lane count, and effective bound, capped at that count.
  Preserve this readback in the ordinary structured command result as well as
  human output.
- This option belongs only to full `validate`. Existing single-lane, browser,
  compatibility, control, trace, and non-E2E commands keep their interfaces and
  defaults. No environment variable, persistent configuration, or global lock is
  added.

### Scheduling and ownership

- The bound limits concurrently executing lane-build operations owned by this
  invocation, including their existing build-completion handling. It does not
  count guest browser workers, dependency derivations, or other invocations.
- Admit no more than the effective bound. When a build operation settles, its
  slot can admit another pending lane without waiting for unrelated active
  lanes. Bounds above one permit independent lane work in parallel.
- Derive the population from the existing enabled-lane catalog, not a copied
  roster. Every enabled lane is attempted exactly once; disabled lanes remain
  excluded. Retain deterministic catalog-order result presentation.
- A reported build/spawn/wait failure must not discard pending lanes, cancel
  healthy active lanes, or abandon their results. Await all attempted work and
  preserve failure as failure before aggregate completion.
- Retain the existing process supervision and teardown ownership. The limit does
  not add pressure monitoring, preemption, or a new cancellation protocol.

### Verdict and evidence

- Keep lane-qualified GC roots, logs, failure recovery, diagnostics, reports,
  manifests, phases, and captures on their existing paths and lifecycle.
- Lift available evidence after failures. For successful builds, retain the
  duration-budget and boot-decomposition checks in their current order.
- After all builds settle, retain both backend-specific Firefox reconciliations,
  including failed-lane evidence and exact required-population validation.
- Preserve aggregate failure propagation and the authoritative SQLite/Chromium
  server-function coverage verification and its existing eligibility rules.
- Keep timing evidence honest: scheduling bounded cohorts does not make all
  lanes simultaneously concurrent or turn their maximum duration into measured
  total E2E elapsed time. Do not invent missing timing observations.

### Host policy and Nix

- The caller chooses N from actual approved CPU/RAM admission. No fixed host
  size, automatic RAM estimate, or resource-safety guarantee is introduced.
- Nix retains control of dependency builds, substitutions, builders, and its own
  job/core scheduling. The new bound caps host lane-build dispatch, not
  host-wide VM/process count, actual CPU/RAM use, or Nix daemon concurrency.
- Document that distinction and the separate multi-checkout admission scope of
  #1753, which is related work rather than a prerequisite.
- Current source requests 4 GiB for ordinary Firefox guests and 3 GiB for other
  production lanes, each with two vCPUs. These are configuration, not measured
  host use or proof of any earlier resource incident.

## Acceptance

- Focused CLI tests cover valid N, omission, above-population values, invalid
  input, and conflict with `--no-e2e`, without invoking validation or VMs.
- Focused fake/build-adapter tests exercise the actual orchestration used by
  full validation: bound one, intermediate bounds, and bounds at/above the
  population; synchronize completions to prove peak active work never exceeds
  the bound, parallel work is admitted, and a freed slot admits pending work.
- The same tests prove complete exactly-once enabled-lane execution and stable
  result order despite out-of-order completion and an injected early failure.
- Focused lifecycle/evidence tests prove all active adapters settle, failure
  remains red, lane artifacts remain independent, lifts/validators retain their
  ordering, and reconciliation sees the full expected evidence set. Reuse real
  evidence consumers and maintained fixtures where their contracts are tested;
  do not reproduce VM, browser, database, or reconciliation logic in a fake.
- Verify readback and document the option, compatibility default, Nix limits,
  and caller admission responsibility in maintained developer guidance.
- Proof outputs stay in ignored run storage or PR evidence. Implementation,
  regression tests, and current guidance are deliverables; no checked-in audit
  report or VM-matrix performance measurement is required by this change.

## Boundaries

No global queue, host resource framework, #1753 dependency, cache collector, CI
topology change, lane/catalog/filter change, guest resizing, browser-worker
change, isolation relaxation, coverage waiver, or historical-causality claim.
This document proposes the contract for approval; it authorizes no
implementation.
