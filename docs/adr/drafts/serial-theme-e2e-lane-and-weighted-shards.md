# ADR-DRAFT: Give the Theme specs their own serial E2E lane and weight ordinary shards

- Status: proposed
- Date: 2026-11-23
- Issue: [#2065](https://github.com/jaunder-org/jaunder/issues/2065)

## Context

[ADR-0203](../0203-distribute-ci-validation-and-firefox-e2e-lanes.md) and
[ADR-0253](../0253-split-chromium-e2e-lanes.md) gave every backend/browser pair
three isolated lanes: ordinary shards 1 and 2 of 2, and one serial-special lane.
That serial-special lane ran visual, then global configuration, then invite.

By November 2026 the Firefox serial-special lane was the CI critical path, with
a 14.0 min job median over five merge-queue runs on both backends:

- **The serial project dominated.** Its global-configuration project runs at one
  worker, because its specs mutate site-wide state that per-author identities
  cannot isolate. The Theme specs (`theme-management`, `theme-presentation`,
  `theme`) accounted for about 360 s of its roughly 710 s Playwright time.
- **The ordinary shards were unbalanced.** Playwright splits shards by test
  count over alphabetically ordered files, so `posts.spec.ts` (84 tests, about
  660 s on Firefox) always landed in shard 2. That shard's job took 12.3–12.7
  min against 9.3 min for shard 1. Splitting the serial work alone would
  therefore have saved only about 1.3 min.
- **Per-test setup is not the cost.** A local Firefox run of `theme-management`
  measured 15 s of hooks and fixtures out of 294 s; app mounts and in-page
  evaluation dominated the test bodies. Making those tests cheaper would mean a
  test-by-test rewrite.

## Decision

Every backend/browser pair runs **four** isolated lanes:

- ordinary shard 1 of 2 and ordinary shard 2 of 2 over `<browser>-ordinary`;
- `serial-special`, which runs `<browser>-special-visual`, then
  `<browser>-special-global-configuration`, then `<browser>-special-invite`; and
- `serial-theme`, which runs only `<browser>-special-theme`: the Theme specs,
  serial at one worker.

The Theme specs form their own Playwright role in `end2end/e2eProjects.ts`.

- In the unsplit graph, `<browser>-theme` runs after site settings and before
  invite.
- Helpers that require exclusive ownership of site configuration accept both
  serial configuration roles.
- No test leaves serial execution. Each serial group now owns its own VM.

Ordinary shards are sized by measured duration, not test count. The catalog
holds one weight string per browser in `ordinaryShardWeights` (for example
`"56:44"`). Nix exports it to ordinary lanes as Playwright's
`PWTEST_SHARD_WEIGHTS`, which sets the share of tests in each `--shard` slice.

- **Deriving the weights:** order the two shards' tests as Playwright does, take
  each test's median duration from recent lane reports, and choose the split
  point that balances summed duration.
- **Validation:** `scripts/e2e-matrix.jq` checks only the weight string's shape.
  Reconciliation already fails any gap or overlap between shards.

There are 16 enabled lanes. The new lanes take trace seeds `13`–`16`.

This narrowly supersedes the clauses of ADR-0203 and ADR-0253 that give each
pair three lanes, a single serial-special lane, and twelve enabled lanes. Their
census, reconciliation, coverage and cap rules stay current, and apply to the
new lane unchanged.

## Consequences

- **Good:** the projected slowest lane drops from about 14 to about 11 min. That
  lane is now Firefox ordinary, with Validation coverage (10.3 min median) close
  behind.
- **Good:** the Theme specs no longer share a VM with other global-state specs.
- **Cost:** CI runs four more producer VMs, about +9 runner-minutes per run of
  fixed setup. The serial test work only moves between lanes.
- **Cost:** one run's concurrent-job peak rises from about 16 to about 20, which
  is the org's free-plan cap. Overlapping runs queue jobs, which can eat part of
  the saving.
- **Cost:** local pair mode, used by `dev e2e-regenerate-coverage`, now starts
  four VMs together. `all` mode keeps its three-job cap.
- **Constraint:** `PWTEST_SHARD_WEIGHTS` is an undocumented Playwright runner
  variable.
  - A Playwright upgrade that drops it fails loudly in reconciliation.
  - Weights drift as specs change. That costs balance, never correctness.
  - Re-derive the weights when the ordinary shards' durations diverge.
- **Follow-up opportunity:** Theme tests that touch only one author's own themes
  could join the ordinary shards, once each one is shown independent of site
  state.
