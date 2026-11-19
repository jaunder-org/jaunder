# ADR-DRAFT: Split Chromium E2E lanes and reconcile flow coverage

- Status: proposed
- Date: 2026-11-19
- Issue: [#1969](https://github.com/jaunder-org/jaunder/issues/1969)

## Context

[ADR-0203](../0203-distribute-ci-validation-and-firefox-e2e-lanes.md) split
Firefox into three isolated lanes per backend and kept Chromium as one unsplit
lane per backend. By November 2026 that Chromium lane had become the E2E long
pole:

- On AWS-hosted GitHub runners it took about 1484 s against the shared 1500 s
  Playwright cap, and some runs were killed with no failing test (#1985).
- About 637 s of that ran at concurrency 1: the serial admin-site and invite
  projects that wait for the parallel Chromium project (#1969).
- PostgreSQL/Chromium had already needed a one-off 1800 s cap.

The unsplit SQLite/Chromium producer was also the sole source of server-function
flow coverage ([ADR-0081](../0081-empirical-server-fn-flow-coverage.md)). Its
consumer accepted only that producer's registered immutable `/nix/store` output
and ran in the producer's job
([ADR-0238](../0238-just-e2e-workflow-ownership.md); ADR-0203's
producer/consumer-within-one-job rule).

## Decision

Chromium uses the same lane topology as Firefox. Each backend runs three
isolated Chromium lanes:

- ordinary shard 1 of 2 and ordinary shard 2 of 2 over `chromium-ordinary`; and
- one serial-special lane that runs `chromium-special-visual`, then
  `chromium-special-global-configuration`, then `chromium-special-invite`.

Those projects use the same spec filters and in-lane ordering as the unsplit
Chromium graph. The visual project reuses the reviewed `chromium-visual`
baselines.

Every enabled lane now follows one catalog rule per backend/browser pair. There
are twelve enabled lanes. Unsplit catalog lanes are disabled diagnostic and
measurement controls. Each split lane runs its browser's independent census.
Every pair gets the same `always()` reconciliation, and every lane uses the
shared 1500 s Playwright cap. A lane's `traceSeed` is one or two non-zero hex
digits, repeated to fill the trace and span ids. Seeds must expand to distinct
ids.

Server-function flow coverage comes from the reconciled SQLite/Chromium pair:

1. The coverage consumer first reconciles the pair from already-lifted
   `e2e-<lane>` artifacts. That requires the exact enabled lane set, one
   complete census equal to the lanes' disjoint report union, and a successful
   phase sidecar for every lane.
2. Each lane's capture is then extracted on its own, because span ids are unique
   only within one lane.
3. The resulting coverage is unioned and compared byte-for-byte with the
   committed snapshot.

In CI this check runs in the SQLite/Chromium reconciliation job. Locally,
`dev e2e` and `dev e2e-regenerate-coverage` run the pair's producers before
reading their artifacts.

This decision narrowly supersedes three clauses. The rest of each ADR stays
current.

- ADR-0203: "Chromium remains one unsplit lane per backend", and
  "producer/consumer pairs remain within one job" as applied to flow coverage.
- ADR-0238: the flow-coverage consumer verifies a selected successful immutable
  producer output.
- ADR-0081: the hit set comes from a single `sqlite × chromium` run. It now
  comes from the union of the reconciled SQLite/Chromium lanes.

## Consequences

- Good: No Chromium lane carries the serial tail together with the whole
  parallel suite. Every lane has headroom under the shared cap, and the
  PostgreSQL/Chromium timeout exception is gone.
- Good: The catalog, census, reconciliation and CI rules no longer special-case
  a browser.
- Cost: CI runs twelve producer VMs instead of eight and four reconciliation
  jobs instead of two. Each extra producer repeats roughly 1.5–2 minutes of
  runner setup.
- Cost: Coverage no longer checks that its capture came from a registered
  `/nix/store` output. CI's reconciliation job only has downloaded artifacts, so
  it cannot run that check. Coverage now relies on the same reconciliation proof
  that already backs every Firefox verdict.
- Constraint: Twelve enabled lanes plus four disabled controls use most of the
  single-digit seed space. Two-digit seeds cover future lanes.
- Constraint: Chromium lane filters, project dependencies, census and
  reconciliation are one contract, as they are for Firefox.
