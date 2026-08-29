# Issue #819 — Reduce Firefox context-mint cost without reuse

## Outcome

Jaunder either removes one Firefox launch preference that materially reduces
fresh-context cost and SQLite suite wall clock, or closes #819 with certified
evidence that none of the four tested preference omissions is an
isolation-preserving lever. Every test continues to receive a fresh
BrowserContext.

## Load-bearing decisions

- The deciding environment is the SQLite gate configuration: Playwright 1.61.1,
  workers=2, 2-vCPU/3-GB VM. Firefox is decisive; Chromium is an unchanged
  host/run-order control.
- Measurements run only on a reserved, quiescent host. Before each browser pair,
  record two `/proc/loadavg` samples 60 seconds apart: both one-minute values
  must be ≤1.0. This numeric load rule is the deciding quiescence evidence. A
  failed precheck delays the pair; it does not create a result. Record one
  post-pair one-minute value diagnostically, but never discard a completed pair
  because its result or self-load is inconvenient.
- **Approved post-capture amendment:** the harness also delayed on the QEMU,
  `nix build|develop`, and Playwright process patterns it recognized, but did
  not implement an exhaustive standalone-browser or agent-command census. After
  conformance review identified that mismatch, the user explicitly approved the
  load values as the deciding rule and withdrew the broader process-absence
  requirement.
- Freeze five arms before capture:
  - **A — baseline:** all current Firefox preferences.
  - **B — Fission:** omit only `fission.autostart = false`.
  - **C — process cap:** omit only `dom.ipc.processCount = 1`.
  - **D — history viewers:** omit only
    `browser.sessionhistory.max_total_viewers = 0`.
  - **E — memory cache:** omit only `browser.cache.memory.capacity = 51200`.
- Device descriptors, viewport, locale/timezone, permissions, service-worker and
  HTTPS behavior, trace headers, browser args, VM resources, retries, test
  corpus, and every BrowserContext call remain unchanged.
- Every arm receives five full SQLite suite runs per browser: 50 combo runs. A
  pair attempt uses one unique salt `issue819-<arm>-r<round>-a<attempt>`;
  discarded and retained attempts never reuse a salt.
- Within each Latin-square slot, the two browsers run adjacently:
  - round 1: `A-F,C B-F,C C-F,C D-F,C E-F,C`
  - round 2: `B-C,F C-C,F D-C,F E-C,F A-C,F`
  - round 3: `C-F,C D-F,C E-F,C A-F,C B-F,C`
  - round 4: `D-C,F E-C,F A-C,F B-C,F C-C,F`
  - round 5: `E-F,C A-F,C B-F,C C-F,C D-F,C` Runs are sequential and the
    realized sequence is recorded.
- One shared baseline population serves all four comparisons. No post-hoc
  exclusion, result-based replacement, arm regrouping, or candidate combination
  is allowed.
- **Approved protocol amendment:** A2 and B1 completed under a temporary,
  user-directed five-minute quiescence rule. When the user restored the
  one-minute rule, those pairs were excluded before their metrics were
  interpreted and replaced by A3 and B2 with new salts. After conformance review
  identified the conflict with the no-replacement rule above, the user
  explicitly approved this one exception. It applies only to A2/B1; no other
  completed pair may be excluded or replaced.
- A pair may be retried in the same sequence slot, immediately and in the same
  browser order, only when failure occurs before Playwright starts: evaluation
  or build failure, VM boot failure, or a proven cached/version mismatch. Both
  browsers are rerun with the next attempt suffix. Once Playwright starts, any
  unexpected/flaky/timeout result, missing report/trace, census mismatch, or
  capture-integrity failure is retained as an arm veto, never retaken. A veto in
  baseline arm A makes the experiment inconclusive and stops it.
- Suite duration is Playwright report `.stats.duration` in milliseconds,
  including every reported attempt. Context-mint sum is the per-run sum of
  `(endTimeUnixNano - startTimeUnixNano) / 1e6` for every `e2e.context_mint`
  span from every attempt. Individual-span p50/p90 and other lifecycle phases
  are diagnostics, not deciding metrics.
- For each candidate and deciding metric, the statistical floor is three times
  `sqrt(baseline_variance / 5 + candidate_variance / 5)` over run-level means.
  The practical floor is 10% of baseline mean context-mint sum and 5,000 ms of
  suite duration. Firefox must improve beyond the larger statistical/practical
  floor in both metrics.
- The four one-sided comparisons retain the preregistered 3×SE threshold: the
  normal-approximation union bound is below 0.6% before also requiring two
  deciding metrics and practical floors. No post-result adjustment is allowed.
- Chromium vetoes an arm when either context-mint sum or suite duration
  regresses beyond that metric's 3×SE floor. Its p50/p90 and lifecycle residuals
  remain diagnostic.
- Before any B–E run, A1 freezes numeric project/test/attempt, context,
  default-page navigation, and secondary-page navigation counts in the corpus
  manifest. Every later run must equal those values. A1 itself remains in the
  baseline population; changing its census after seeing candidates is forbidden.
- Missing/dropped trace data, visual/accessibility drift, trace-attribution
  drift, or one-boot/fresh-context violations also veto an arm regardless of
  speed.
- If several candidates qualify, land only the arm with the largest Firefox
  suite-duration reduction. Do not combine preferences in this issue.
- Failed arms are reverted and recorded. If none qualifies, the certified
  negative result completes #819; context reuse remains out of scope.

## Acceptance

- The corpus manifest records the exact commit, Nix/Playwright/browser versions,
  store paths, unique salts/attempts, arm definitions, configured and realized
  sequence, quiescence samples, and every delayed, retained, vetoed, or retried
  pair with reason.
- A1 freezes the exact expected project/test/attempt, context, default-page
  navigation, and secondary-page navigation counts before any candidate runs.
  Every retained combo reports equality to that reference.
- Every combo has complete lifecycle/context-mint data, no dropped records, and
  matching Playwright report and trace artifacts; any violation follows the
  preregistered retry-or-veto rule.
- Per arm/browser, the record reports run-level `.stats.duration`, context-mint
  sum, individual-span p50/p90, test-span sum, teardown, attempt population, and
  relevant lifecycle residuals.
- Each candidate comparison publishes Firefox and Chromium deltas, sample
  variances, statistical and practical floors, census/integrity vetoes, and
  disposition before any implementation is retained.
- No candidate code or temporary salt/arm machinery lands unless that candidate
  satisfies both Firefox metrics and every control/integrity requirement.
- A retained preference change preserves the existing full behavioral, visual,
  accessibility, trace-attribution, one-boot, and fresh-context contracts across
  the final backend/browser matrix.
- `docs/observability.md` records the protocol, certified corpus, results,
  limitations, winning or negative disposition, and relationship to #792.
- Focused feedback and the repository's final implementation/ship gates pass.

## Boundaries

- No BrowserContext reuse, per-worker shared context, storage-state reuse, test
  regrouping, worker/VM sizing change, timeout increase, or warmup.
- No changes to traced-context enforcement, traceparent injection, page capture,
  device/context options, visual masking/tolerance, or test corpus.
- No single-worker result substitutes for the gate-setting decision; diagnostic
  runs may explain a finding but cannot satisfy acceptance.
- No combined preference arm or follow-on optimization beyond the frozen list.
