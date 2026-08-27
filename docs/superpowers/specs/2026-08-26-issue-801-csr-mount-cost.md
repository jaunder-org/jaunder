# Issue #801 — Cut CSR mount cost

## Outcome

Jaunder either reduces the Firefox critical path for an owned part of CSR page
boot or post-mount settling, or closes #801 with certified evidence that no
issue-local candidate can meet the registered navigation and suite thresholds.
Neither outcome weakens authentication, delays operator safety warnings, or
regresses Chromium.

## Load-bearing decisions

- Fresh measurements decide the lever. The baseline includes both the
  document-frame boot breakdown and `mountToSettledMs`; `commitToMountMs` is
  reported but is never decomposed across clock frames (ADR-0100).
- The deciding environment is SQLite, one worker, and Firefox. Chromium is the
  no-regression control. Gate settings may be reported separately but do not
  replace the one-worker comparison.
- The baseline freezes a finite candidate list before the first experiment. A
  candidate is eligible only when it targets an issue-local, Jaunder-owned
  phase, its predicted removable ceiling exceeds the baseline uncertainty for
  both the affected-navigation phase and suite wall clock, and no separate issue
  owns or has disposed it. The record names the affected route, cache-warmth
  population, target phase, mechanism signal, and predicted ceiling. Cold and
  warm populations are reported separately.
- Each comparison uses the same test corpus and configuration, a quiescent host,
  distinct `e2eSalt` values, counterbalanced or interleaved arm order, and at
  least three runs per arm. If baseline variance requires more runs, the fixed
  count is chosen before candidate capture.
- The pass calculation is registered before capture and uses unpaired run-level
  arm means. For each browser and metric, the noise floor is three times
  `sqrt(baseline_variance / baseline_runs + candidate_variance / candidate_runs)`.
  A candidate succeeds only when both its affected-navigation phase and the
  unchanged suite wall clock improve beyond their Firefox floors. Chromium must
  not regress beyond its own phase or suite floor.
- Candidates are attempted in descending predicted ceiling. A phase already
  disposed or separately owned by #836, #864, #867, #869, or #870 is recorded
  and excluded rather than duplicated. ADR-0106's raw wasm size budget and
  ADR-0121's no-preload decision remain in force.
- The authoritative session reconcile remains ahead of the authenticated `/app`
  timeline request. The advisory marker never authorizes a speculative protected
  fetch.
- `BackupBanner` and `SiteBaseUrlBanner` remain prompt shell safety signals.
  Work may eliminate, combine, cache, or conditionally avoid their underlying
  cost only when visibility and authorization semantics remain intact; merely
  scheduling them after the measured boundary is not a performance win.
- A candidate that misses either Firefox floor or regresses Chromium is reverted
  and recorded as a negative result before the next candidate on the frozen list
  is attempted.
- If the frozen list is empty, or every candidate on it fails, the certified
  negative evidence completes #801 as no actionable win. It does not land
  measurement-only delay, an under-noise change, or work owned by another issue.

## Acceptance

- The baseline certifies complete trace populations and reports, per relevant
  navigation, the boot phase fields and `mountToSettledMs`, split by browser,
  affected route, and cache warmth.
- The frozen candidate list, each predicted mechanism and ceiling, the
  candidate-specific integrity signal, the floor calculation, run count, and
  realized arm order are recorded before results are interpreted.
- If the frozen list is non-empty, matched before/after traces show a Firefox
  improvement greater than the calculated floor for both the affected-navigation
  phase and unchanged-suite wall clock; Chromium stays within its calculated
  no-regression bounds.
- For an attempted candidate, the registered mechanism signal moves in the
  predicted direction and proves the targeted work was removed or made cheaper.
  Request counts and ordering additionally prove that any request-graph result
  was not shifted beyond the settled boundary; boot work uses its document-frame
  marks and direct wasm or resource diagnostics instead.
- If the frozen list is empty, the record demonstrates that every otherwise
  material lever is disposed, separately owned, or below the suite eligibility
  floor, and #801 closes without a behavior change.
- Any behavior change retains tests proving that `/app` waits for authoritative
  session reconciliation, rejects an invalid or stale advisory marker, and
  preserves operator-warning behavior at the shell routes where it is currently
  visible. Any changed observable contract has a focused regression test that
  fails under the prior behavior.
- `docs/observability.md` records the protocol, certified corpus, result,
  limitations, and disposition of every attempted candidate.
- The focused behavioral checks and the repository's normal implementation gate
  pass before the change is committed; the final branch passes the ship gate.

## Boundaries

- No weakening of authentication, public-projector cacheability, the one-boot
  e2e invariant, telemetry PII rules, or backend/browser parity.
- No preload revival, timeout increase, warning suppression, metric-boundary
  manipulation, or unrelated navigation-count work.
- No new performance budget, architecture boundary, public API, domain term, or
  ADR is introduced by this issue.
- Negative experiments may be documented, but their code does not land.
