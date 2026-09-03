# Issue #904 — thin browser-backed boot-budget tests

## Outcome

The end-to-end boot-budget suite spends browser time only on contracts that
require real browser navigation events. Pure budget accounting remains fully
covered through non-browser tests of the same implementation, reducing document
loads and producing a repeatable Firefox suite-time improvement without
weakening the one-boot-per-page guard.

## Load-bearing decisions

- Keep browser coverage for behavior whose subject is which events an engine
  fires or when those events can be observed. This includes real document-load
  counting, same-document navigation, raw navigation, automatic and late page
  arming, fixture integration, and multiple-page behavior.
- Exercise allowance consumption and ordering, reason validation, orphan
  accounting, path matching, and violation reporting without launching a
  browser. Those contracts are deterministic state transitions rather than
  browser behavior.
- Browser and non-browser tests must drive one shared budget implementation. Do
  not duplicate the accounting rules in a test model, fake browser helper, or
  second budget engine merely to make them cheap to test.
- Preserve the public test-helper behavior and failure messages relied on by the
  e2e suite. This is a test-placement and testability change, not a change to
  the one-boot-per-page policy.
- Preserve every observable contract currently defended by the browser-backed
  suite. Before moving tests, produce a contract ledger that maps every
  pre-change assertion and public failure-message check to its retained browser
  assertion or non-browser replacement and names the defect that still makes the
  replacement fail. Landing requires zero unmapped entries.
- Measure the resulting browser-backed set with the established #818 protocol:
  sqlite × {Chromium, Firefox}; three before/after runs of the single-worker
  packages as the deciding set and three at gate settings as the confirming set;
  arms interleaved run-by-run with distinct salts and the protocol's host load
  rejection rule. Do not pool the two settings sets.
- There is no predetermined minimum number of seconds the change must save. It
  passes the deciding set when all three after runs contain fewer document
  loads, at least two of the three paired Firefox runs are faster, and mean
  Firefox summed test-body time is lower. It passes the confirming set when each
  browser's mean gate wall time does not increase. Report every raw run, both
  means, and each paired delta.
- If no candidate seam can let page listeners and non-browser tests drive the
  same state transitions without duplicating an accounting rule or fabricating
  browser objects, stop rather than force the extraction. Record a finding in
  `docs/observability.md` naming each candidate seam, the browser loads it would
  remove, the duplicated rule or browser coupling that rejects it, and why no
  candidate satisfies the shared-implementation constraint. That discrete
  condition is the only passing no-code outcome.
- This issue changes no domain vocabulary or durable product architecture, so it
  requires neither a `CONTEXT.md` change nor an ADR.

## Acceptance

- The browser-backed boot-budget tests remaining after the change each state a
  contract that depends on actual browser navigation events, page lifecycle, or
  Playwright fixture integration.
- Non-browser tests cover allowance consumption, single-use behavior, exact
  versus path-scoped ordering, path matching and mismatch, reason validation,
  orphan collection, and recorded violation reporting.
- The non-browser tests exercise the same implementation used by the Playwright
  fixtures; there is no parallel model of the accounting rules.
- The suite still proves that a same-document router push is not a boot and that
  a declaration made after page arming cannot deadlock or miss its load.
- The suite still detects an undeclared second document load even when no later
  navigation helper raises the recorded violation.
- The contract ledger inventories every pre-change assertion and public
  failure-message check, maps each to retained or replacement coverage, names
  the discriminating defect, and has zero unmapped entries.
- A before/after #818-protocol measurement records browser, settings set, run
  order, salt, accepted host-load samples, document loads, summed test-body
  time, gate wall time, means, and paired deltas.
- Every after run has fewer document loads; at least two of three paired Firefox
  deciding runs and the Firefox deciding-set mean are faster; and neither
  browser's confirming-set mean gate wall time increases. If these conditions
  fail, the split does not land.
- Alternatively, the written-finding outcome passes only when the required
  candidate-seam analysis demonstrates that every candidate violates the
  shared-implementation constraint.

## Contract ledger

| Pre-change contract                                                       | Retained or replacement proof                                                         | Discriminating defect                                           |
| ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| One real load counts one boot                                             | Browser: `one real document load counts one boot`                                     | The page listener misses or miscounts the entry event.          |
| A router push is not a boot                                               | Browser: `a same-document router push does not count`                                 | SPA history changes are mistaken for document loads.            |
| An undeclared second load reports its route and declaration guidance      | Accounting: `an exact allowance covers one further document load only`                | A later load is unreported or its actionable message regresses. |
| One exact allowance permits exactly one further load                      | Accounting: `an exact allowance covers one further document load only`                | The allowance is ignored or becomes permanent.                  |
| Raw `page.goto` loads are counted independently of the wrapper            | Browser: `a raw page.goto is counted by the page listener`                            | Enforcement moves into `goto` and raw loads escape.             |
| Both declaration forms reject blank reasons with caller-specific messages | Accounting: `both allowance forms require a non-empty reason`                         | An unauditable reason is accepted or the named API is lost.     |
| A declaration can arm a page after its entry                              | Browser: `a declaration arms a page after its entry load`                             | Late arming misses the entry or spends the allowance on it.     |
| `registeredPage` enters the requested route                               | Browser: `registeredPage boots at the given entry`                                    | Fixture navigation stops honoring its entry.                    |
| A second `registeredPage` call reports the first route                    | Browser: `registeredPage refuses a second call`                                       | The fixture allows two entries or loses route context.          |
| The fixture automatically arms the default page                           | Browser: `the fixture arms every test page`                                           | Fixture installation stops enforcing the budget.                |
| A traced second page is armed and explicit arming is idempotent           | Browser: `a traced second page is armed and explicit arming is idempotent`            | Secondary pages escape or repeated arming double-counts.        |
| An exact orphan reports route and reason once, then clears                | Accounting: `failure collection reports route-bearing orphan reasons and clears them` | Orphans disappear, lose context, or leak across collections.    |
| A consumed allowance is not an orphan                                     | Accounting: `an exact allowance covers one further document load only`                | Consumption leaves stale pending state.                         |
| A raw second load reaches teardown without a later helper call            | Browser: `an undeclared raw second load reaches the teardown sweep`                   | Listener-recorded violations vanish before teardown.            |
| An unused engine-dependent allowance is not an orphan                     | Accounting: `an unconsumed engine-dependent allowance is not an orphan`               | Engine variance causes a false teardown failure.                |
| A scoped allowance matches by pathname across origin and query changes    | Accounting: `a scoped allowance matches its pathname before an exact allowance`       | Per-run origin or query data prevents a valid match.            |
| A scoped allowance cannot absorb a different path                         | Accounting: `a scoped allowance is inert for another pathname`                        | An undeclared load is silently accepted.                        |
| A matching scoped allowance is consumed before an exact allowance         | Accounting: `a scoped allowance matches its pathname before an exact allowance`       | Declaration order makes the later exact load fail.              |

## Boundaries

- No weakening or removal of the one-boot-per-page enforcement policy.
- No deletion of coverage solely to improve timing.
- No change to application navigation behavior or production Rust/WASM code.
- No broad Playwright fixture refactor beyond the seam required to test the
  existing budget implementation without a browser.
- No arbitrary timing target or claim that Chromium and Firefox costs are
  interchangeable.
- No permanent benchmark gate or new timing threshold in CI.
