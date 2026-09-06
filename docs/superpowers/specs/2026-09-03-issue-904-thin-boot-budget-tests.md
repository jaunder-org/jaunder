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

## Measurement result

The interleaved campaign used baseline
`728885a2f682d297095f59ef1421723e1f1f8988` and treatment
`f18215dd8d9507571b97564a26ca53107ea8dd65`. The retained corpus is under
`~/measurements/jaunder/issues-869-904/`; its analyzer reports a complete
30-slot shared campaign with no validation errors. Seventy-eight attempts were
rejected by the pre-registered host-load checks before the retained runs below.
The load column records first precheck, second precheck, run-start, and run-end
samples.

### Single-worker deciding runs

| Round | Order | Arm       | Browser  | Salt                                        | Load                | Documents | Body ms | Wall ms |
| ----: | ----: | --------- | -------- | ------------------------------------------- | ------------------- | --------: | ------: | ------: |
|     1 |     1 | baseline  | Chromium | `i869-904-d-01-baseline-chromium-attempt-1` | 0.93/0.69/0.69/1.44 |       270 |  511607 |  525662 |
|     1 |     3 | treatment | Chromium | `i869-904-d-03-904-chromium-attempt-3`      | 0.84/0.65/0.65/1.64 |       251 |  505517 |  519261 |
|     1 |     4 | baseline  | Firefox  | `i869-904-d-04-baseline-firefox-attempt-4`  | 0.70/0.78/0.78/1.50 |       270 |  863308 |  885141 |
|     1 |     6 | treatment | Firefox  | `i869-904-d-06-904-firefox-attempt-3`       | 1.00/0.65/0.65/1.45 |       251 |  832275 |  854618 |
|     2 |     1 | treatment | Firefox  | `i869-904-d-07-904-firefox-attempt-5`       | 0.74/0.94/0.94/1.35 |       251 |  844254 |  865715 |
|     2 |     3 | baseline  | Firefox  | `i869-904-d-09-baseline-firefox-attempt-3`  | 0.95/0.85/0.85/1.94 |       270 |  904789 |  927738 |
|     2 |     4 | treatment | Chromium | `i869-904-d-10-904-chromium-attempt-10`     | 0.96/0.54/0.54/1.33 |       251 |  498755 |  512536 |
|     2 |     6 | baseline  | Chromium | `i869-904-d-12-baseline-chromium-attempt-6` | 0.62/0.61/0.61/1.61 |       270 |  520550 |  534746 |
|     3 |     2 | baseline  | Chromium | `i869-904-d-14-baseline-chromium-attempt-2` | 0.86/0.68/0.68/2.04 |       270 |  526376 |  540522 |
|     3 |     3 | treatment | Firefox  | `i869-904-d-15-904-firefox-attempt-4`       | 0.50/0.40/0.40/1.55 |       251 |  828446 |  849586 |
|     3 |     5 | baseline  | Firefox  | `i869-904-d-17-baseline-firefox-attempt-2`  | 0.78/0.65/0.65/1.71 |       270 |  836906 |  858608 |
|     3 |     6 | treatment | Chromium | `i869-904-d-18-904-chromium-attempt-3`      | 0.86/0.59/0.59/1.82 |       251 |  495367 |  508831 |

| Browser  | Arm       | Mean documents | Mean body ms | Mean wall ms |
| -------- | --------- | -------------: | -----------: | -----------: |
| Chromium | baseline  |            270 |       519511 |       533643 |
| Chromium | treatment |            251 |       499880 |       513543 |
| Firefox  | baseline  |            270 |       868334 |       890496 |
| Firefox  | treatment |            251 |       834992 |       856639 |

| Browser  | Round | Document delta | Body delta ms | Wall delta ms |
| -------- | ----: | -------------: | ------------: | ------------: |
| Chromium |     1 |            -19 |         -6090 |         -6401 |
| Chromium |     2 |            -19 |        -21795 |        -22210 |
| Chromium |     3 |            -19 |        -31009 |        -31691 |
| Firefox  |     1 |            -19 |        -31033 |        -30523 |
| Firefox  |     2 |            -19 |        -60535 |        -62023 |
| Firefox  |     3 |            -19 |         -8460 |         -9022 |

### Gate-settings confirming runs

| Round | Order | Arm       | Browser  | Salt                                        | Load                | Documents | Body ms | Wall ms |
| ----: | ----: | --------- | -------- | ------------------------------------------- | ------------------- | --------: | ------: | ------: |
|     1 |     1 | baseline  | Chromium | `i869-904-c-19-baseline-chromium-attempt-3` | 0.90/0.52/0.52/2.07 |       270 |  498992 |  289955 |
|     1 |     2 | treatment | Firefox  | `i869-904-c-20-904-firefox-attempt-3`       | 0.88/0.69/0.69/2.35 |       251 |  828357 |  479669 |
|     1 |     3 | baseline  | Firefox  | `i869-904-c-21-baseline-firefox-attempt-3`  | 0.84/0.84/0.84/2.64 |       270 |  873903 |  503354 |
|     1 |     4 | treatment | Chromium | `i869-904-c-22-904-chromium-attempt-3`      | 0.81/0.58/0.58/2.06 |       251 |  472500 |  276357 |
|     2 |     1 | treatment | Chromium | `i869-904-c-23-904-chromium-attempt-2`      | 1.00/0.80/0.80/1.96 |       251 |  475431 |  278288 |
|     2 |     2 | baseline  | Firefox  | `i869-904-c-24-baseline-firefox-attempt-4`  | 0.60/0.97/0.97/2.27 |       270 |  873700 |  502463 |
|     2 |     3 | treatment | Firefox  | `i869-904-c-25-904-firefox-attempt-3`       | 0.72/0.45/0.45/2.67 |       251 |  849869 |  492746 |
|     2 |     4 | baseline  | Chromium | `i869-904-c-26-baseline-chromium-attempt-4` | 0.75/0.56/0.56/2.26 |       270 |  496156 |  288643 |
|     3 |     1 | baseline  | Firefox  | `i869-904-c-27-baseline-firefox-attempt-3`  | 0.76/0.52/0.52/2.17 |       270 |  880385 |  506861 |
|     3 |     2 | treatment | Chromium | `i869-904-c-28-904-chromium-attempt-3`      | 0.77/0.64/0.64/2.16 |       251 |  480812 |  281878 |
|     3 |     3 | baseline  | Chromium | `i869-904-c-29-baseline-chromium-attempt-5` | 0.69/0.50/0.50/2.32 |       270 |  502358 |  292423 |
|     3 |     4 | treatment | Firefox  | `i869-904-c-30-904-firefox-attempt-4`       | 0.88/0.97/0.97/2.55 |       251 |  844074 |  487930 |

| Browser  | Arm       | Mean documents | Mean body ms | Mean wall ms |
| -------- | --------- | -------------: | -----------: | -----------: |
| Chromium | baseline  |            270 |       499169 |       290340 |
| Chromium | treatment |            251 |       476248 |       278841 |
| Firefox  | baseline  |            270 |       875996 |       504226 |
| Firefox  | treatment |            251 |       840767 |       486782 |

| Browser  | Round | Document delta | Body delta ms | Wall delta ms |
| -------- | ----: | -------------: | ------------: | ------------: |
| Chromium |     1 |            -19 |        -26492 |        -13598 |
| Chromium |     2 |            -19 |        -20725 |        -10355 |
| Chromium |     3 |            -19 |        -21546 |        -10545 |
| Firefox  |     1 |            -19 |        -45546 |        -23685 |
| Firefox  |     2 |            -19 |        -23831 |         -9717 |
| Firefox  |     3 |            -19 |        -36311 |        -18931 |

The change passes both pre-registered decisions. Every treatment run removed 19
document loads. All three paired Firefox deciding runs were faster, and Firefox
mean summed body time fell by 33342 ms (3.84%). At gate settings, mean wall time
fell by 11499 ms (3.96%) in Chromium and 17444 ms (3.46%) in Firefox.

## Boundaries

- No weakening or removal of the one-boot-per-page enforcement policy.
- No deletion of coverage solely to improve timing.
- No change to application navigation behavior or production Rust/WASM code.
- No broad Playwright fixture refactor beyond the seam required to test the
  existing budget implementation without a browser.
- No arbitrary timing target or claim that Chromium and Firefox costs are
  interchangeable.
- No permanent benchmark gate or new timing threshold in CI.
