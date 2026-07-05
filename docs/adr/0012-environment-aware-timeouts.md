# ADR-0012: Environment-Aware E2E Timeouts

- Status: accepted
- Deciders: mdorman, Gemini CLI
- Date: 2026-05-15

## Context and Problem Statement

End-to-end tests, particularly those involving WASM hydration, exhibit
significantly different performance profiles across browsers (Chromium vs.
Firefox vs. WebKit). Fixed timeouts lead to either flaky tests in slower
browsers or unnecessarily long wait times in faster ones.

## Decision Drivers

- Reliability: Eliminating flaky test failures due to environmental lag.
- Efficiency: Keeping timeouts as tight as possible for each browser.
- Maintainability: Avoiding hard-coded timeout values throughout the test suite.

## Decision Outcome

Chosen option: "Adaptive Timeout Budgeting", because it allows timeouts to scale
based on the observed performance characteristics of the target browser.

### Implementation Details

- Timeouts are calculated using multipliers derived from observed p90 hydration
  latency for each browser.
- Helper functions in `end2end/tests/fixtures.ts`:
  - `slowBrowserTimeoutMs(testInfo, chromiumBudgetMs)`: For whole-test budgets.
  - `slowBrowserFirstNavigationTimeoutMs(testInfo, chromiumBudgetMs)`: For the
    initial (coldest) navigation.
- Tests specify a "base" budget (for Chromium), which is then scaled for other
  projects based on known performance differentials.

## Consequences

- Good: Dramatically reduced flakiness in Firefox and WebKit E2E runs.
- Good: Explicit documentation of performance expectations per browser.
- Bad: Multipliers must be periodically tuned as the application and hydration
  logic evolve.
