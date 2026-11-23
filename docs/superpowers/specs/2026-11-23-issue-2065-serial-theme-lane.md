# Issue #2065: split the Theme specs into their own serial lane and balance ordinary shards

## Outcome

The slowest E2E lane in CI is the Firefox serial-special lane, with a 14.0 min
job median over five merge-queue runs. After this change the slowest lane should
be about 11 min. That lane will be Firefox ordinary, and Validation coverage, at
a 10.3 min median, becomes nearly co-critical. Every test still runs exactly
once. The browser × backend matrix stays the same, and every test that runs
serially today still runs serially.

## Measurement behind the choice

Data comes from merge-queue runs 45101740019, 45099172574, 45098058184,
45096609470 and 45088369794, on both backends. Medians:

- **Serial-special lanes.** Playwright time is the whole lane: VM startup is
  16–25 s. The `*-special-global-configuration` project runs at one worker.
  - In Firefox, `theme-management` (61 tests) takes 315–324 s.
    `theme-presentation` takes 36–38 s and `theme` 8–9 s. The other
    global-configuration specs take about 253 s, invite 54 s and visual 27 s.
  - Chromium has the same shape, at roughly 0.75× those times.
- **Ordinary shards.** These are split by test _count_ over files in
  alphabetical order, so `posts.spec.ts` (84 tests, about 660 s on Firefox)
  always lands in shard 2. Firefox summed time is about 820 s against 1200 s;
  shard 2's job (12.3–12.7 min) is the second-longest lane. Splitting serial
  alone would therefore save only about 1.3 min.
- **Theme-management per-test time** (a 3.6 s median and 8.3 s p90 in CI): CI's
  JSON report has no hook or fixture steps, so a local Firefox run's HTML report
  supplied them. 61 tests took 294 s in all.
  - Before-hooks and fixtures took 15 s, mostly the `page` fixture at 0.15 s per
    test.
  - After-hooks took 14 s.
  - Test bodies took 265 s. Their two largest costs were the app mounting
    (`body[data-mounted]` waits: 117 s over 92 boots, about 1.3 s each) and
    `page.evaluate` calls (104 s over 497 calls).

  Per-test setup is therefore not the cost. Making the tests cheaper would mean
  fewer app boots per test, which is a test-by-test rewrite rather than a
  fixture change. That is why the lane split was chosen over that option.

## Approach and tradeoffs

1. **A new Theme role.** `e2eProjects.ts` gains a serial role holding the Theme
   specs: `theme-management`, `theme-presentation` and `theme`. Those specs
   leave the site-settings role.
   - Split projects are `<browser>-special-theme`, with `workers: 1` and
     `fullyParallel: false`, like site settings.
   - The unsplit graph (a disabled control, also used by `dev e2e-local`) gains
     `<browser>-theme`, chained after site settings and before invite.
   - Helpers that require an exclusive configuration project, such as
     `withSiteThemeSelection`, accept both roles.
   - In `dev e2e-local`, every filtered browser project list gains
     `<browser>-theme`, so a `theme-management.spec.ts:N` filter still finds its
     test. The pinned Firefox theme-filter test moves accordingly.
2. **A fourth lane per backend/browser pair.** The lane catalog gets one enabled
   lane per pair, with partition `serial-theme`. It runs only the Theme project,
   in its own NixOS VM, database and storage.
   - The existing serial-special lane keeps visual, then site settings, then
     invite.
   - This makes 16 enabled lanes. The new trace seeds are `13`–`16`. The catalog
     adds `"<browser>-special-theme": []` to `experimentalProjectDependencies`.
   - These move together, as ADR-0203 requires:
     - `scripts/e2e-matrix.jq` (16 lanes, four per pair, one `serial-theme`)
     - `nix/cache-policy.json`
     - the host-proof topology and ownership tests
     - `pr-observer`'s expected `e2e-lane` count
     - the `dev` workflow tests (`LANES`)
     - the census/reconciliation contract
   - Local pair mode (`scripts/e2e-workflow`, which
     `dev e2e-regenerate-coverage` uses) starts a pair's lanes together, so it
     now starts four VMs instead of three. That is accepted; `all` mode keeps
     its three-job cap.
   - Projected job times: about 8.2 min for the Firefox Theme lane and about 7.8
     min for the remaining serial lane.
3. **Duration-weighted ordinary shards.** The catalog carries one weight string
   per browser:
   `"ordinaryShardWeights": {"firefox": "56:44", "chromium": "59:41"}`. The
   measured balance point is the same on both backends.
   - `mkE2eCombo` and `e2eRunAndCapture` in `nix/checks.nix` take a new optional
     argument. They export it as `PWTEST_SHARD_WEIGHTS` on ordinary lanes only.
     That Playwright 1.61.1 runner variable (`a:b`, parsed with `parseInt`)
     sizes `--shard` slices by weighted test count.
   - A wrong weight cannot drop or duplicate a test. Both shards use the same
     string, and reconciliation already fails any gap or overlap. jq checks only
     the string's shape.
   - Checked against the five CI reports: Firefox comes out at about 1016/1040 s
     summed, about 8.9 min of Playwright and about 11 min of job time. Chromium
     comes out at about 540–670 s per shard.

**Alternatives rejected:**

- **Splitting `theme-management` alone.** The user chose to move the whole Theme
  family together.
- **Three ordinary shards.** That adds four more lanes and still splits by
  count.
- **Sharding the serial-special lane 2-way with weights.** This needs no new
  role, but the cut lands on whole files and is count-based over 142 tests, 61
  of them in theme-management. It would drift, and it hides the isolation
  domain.
- **Making serial tests cheaper.** There is no evidence for a single dominant
  cost, and it would be a larger rewrite.

**Accepted costs:**

- **More runner time.** CI runs four more producer VMs, at roughly 2–2.5
  runner-minutes of fixed setup each, so about +9 runner-min per run. The serial
  test work only moves between lanes.
- **More concurrent jobs.** The org's free plan allows 20 concurrent jobs. One
  run's peak rises from about 16 jobs (12 lanes plus 4 validation) to about 20.
  An overlapping run, such as a second queue entry or a PR push, will then queue
  jobs. That queueing can eat part of the saving, which is why the measurement
  below includes whole-run wall clock.
- **Undocumented weights.** `PWTEST_SHARD_WEIGHTS` isn't in Playwright's
  documentation. A Playwright upgrade that drops it fails loudly: the census
  still requires the lanes' union to be complete. The weights drift as specs
  change, which costs balance, not correctness. The ADR records how to re-derive
  them from lane reports.

**Recorded through `jaunder-adr`:** a draft ADR narrowly superseding the "three
lanes per pair" and "twelve enabled lanes" clauses of ADR-0203 and ADR-0253.
`docs/ARCHITECTURE.md`, `CONTRIBUTING.md` and `docs/observability.md` lane
counts are updated in the same change.

## Acceptance

- **Breakdown confirmed.** The per-spec breakdown above is confirmed over five
  runs on both backends. The PR records theme-management's setup-versus-body
  split from a local Firefox run.
- **Local proof.**
  - A Firefox `dev e2e-local … theme-management.spec.ts:N` filter runs in
    `firefox-theme`.
  - The Chromium default chain includes `chromium-theme`.
  - The jq catalog check accepts the 16-lane catalog and rejects a malformed
    weight string.
  - `dev check` passes.
- **CI proof.** On the PR, all 16 producers and 4 reconciliations pass, and each
  census accounts for every test exactly once. Only these producers prove the
  Theme specs pass on a fresh, isolated site.
- **Measured outcome.** After merge, record on the issue, for at least five
  merge-queue runs:
  - the slowest E2E lane's median job time, against the 14.0 min baseline;
  - whole-run wall clock, against the 18.5 min baseline;
  - the added runner-minutes.

## Boundaries

- No test leaves serial isolation, and no test bodies are rewritten.
- There are no changes to VM resources, workers or retries.
- WebKit (`webkit-theme`) and the unsplit controls' enabled state are unchanged.
