# ADR-0034: CI distributes e2e across a {backend}×{browser} matrix; `validate` stays the local full gate

- Status: accepted
- Deciders: mdorman, Claude Opus
- Date: 2026-06-28

## Context and Problem Statement

CI ran the whole gate as a single job,
`nix develop .#ci -c cargo xtask validate`, and the project treated "CI == one
`cargo xtask validate` command" as the definition of the CI-faithful gate
(CLAUDE.md). Measurement (2026-06-28) showed e2e dominates CI at ~17.3 min of a
~24 min run, and that the cost is the two browser suites (chromium, firefox)
running **serially within each VM** — they share one jaunder instance + DB and
re-seed between runs, so they cannot run concurrently in the same VM. The two
backend VMs already run in parallel, so the single-runner model has no remaining
parallelism to exploit: 4 heavy KVM VMs on one 2–4 vCPU runner would contend,
not speed up.

Realizing the parallelism therefore requires running each `{backend}×{browser}`
combination on its own runner — which means CI can no longer be a single
`cargo xtask validate` invocation for e2e. That tension between the existing "CI
== validate" invariant and the parallelism win is the decision.

## Decision

**CI distributes e2e across a GitHub Actions matrix; `cargo xtask validate`
remains the full local gate; CI faithfulness is redefined as "the same Nix check
derivations, distributed."**

1. **Flake split.** `mkE2e*Check` is parameterized by browser, producing one VM
   derivation per `{backend}×{browser}` combo (full 2×2). Each runs a single
   `playwright test --project <browser>`. `jaunderBin`, the e2e bundle, and the
   Playwright config remain shared/cached.

2. **CI fan-out.** The CI workflow runs `cargo xtask validate --no-e2e` in one
   job plus a 4-way `{backend}×{browser}` matrix, each job building one e2e
   check derivation via `nix build` (pulling the warm app build from Cachix). A
   small `e2e-gate` job `needs:` all four and succeeds iff all pass, giving
   branch protection one stable required-check name (`validate-no-e2e` +
   `e2e-gate`) immune to matrix-value churn.

3. **Local gate unchanged in scope.** `cargo xtask validate` still builds the
   e2e aggregate (`e2e-checks`), so a local run executes all combos — now
   parallel by browser via Nix concurrency, bounded by host `max-jobs`. The
   single-command full gate survives where one machine holds the whole checkout.

## Consequences

- Good: e2e wall-clock drops from ~17.3 min toward the slowest single combo
  (~10.6 min); per-combo pass/fail is visible in CI.
- Good: each browser run is fully isolated (own VM + DB), removing the
  shared-DB-with-re-seed coupling that forced serial execution.
- Neutral: CI is no longer literally one `cargo xtask validate` command. The
  faithful-gate guarantee shifts to "same derivations, distributed," with
  `e2e-gate` as the aggregating required check. `xtask`'s host-only invariant is
  untouched — CI still invokes it (for the non-e2e job) and never the reverse.
- Cost: ~4 e2e runners instead of one job (billing-minutes up; wall-clock down,
  since parallel). Each combo re-pays VM boot + one backend bring-up (~30s),
  previously amortized across two browsers per VM.
- Cost: branch protection must be updated at landing to require
  `validate-no-e2e` + `e2e-gate`; skipping it leaves PRs ungated. Handled as an
  explicit landing step.
