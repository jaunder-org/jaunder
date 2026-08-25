# Issue #793: WebSub Publish Ping E2E boundary

## Outcome

The publish-and-edit browser scenario observes two distinct, complete WebSub
Publish Ping waves without a fixed settle sleep. Capture-backed e2e runs use a
short feed-worker cadence, materially reducing the scenario's wall time while
production cadence remains unchanged.

## Load-bearing decisions

- Production feed-worker cadence remains 10 seconds; capture mode uses 250 ms.
- The composition root owns and injects feed-worker cadence. Existing WebSub
  capture configuration identifies e2e capture mode; no new configuration
  surface is introduced.
- Both ping waits retain the `wait.websub_ping` trace attribution supplied by
  the polling boundary.
- A complete mutation wave contains one ping for each exact User Syndication
  Feed URL: RSS, Atom, and JSON Feed, all after that mutation's line cursor.
- Feed fan-out emits each affected URL once per queued mutation. Unrelated Site
  Syndication Feed pings and duplicate requested or captured URLs neither
  complete nor corrupt a wave.
- Publish completion establishes the cursor for the subsequent edit wave;
  elapsed time and line-count stability are not mutation boundaries.
- No fixed-duration settle remains.

## Acceptance

- Publishing one untagged Post yields a complete ordered RSS/Atom/JSON User
  Syndication Feed ping wave after the publish cursor.
- Editing that Post yields a second complete ordered RSS/Atom/JSON User
  Syndication Feed ping wave after a cursor captured only after publish
  completes.
- The focused WebSub browser scenario passes without `waitForTimeout`; its
  Playwright-reported test duration is below 10 seconds, excluding one-time
  build and server startup before the test begins, versus the approximately
  19.5-second measured baseline.
- Capture-backed e2e startup selects the 250 ms cadence; ordinary server startup
  retains the 10-second production cadence.
- Each wait is recorded once as `wait.websub_ping`.

## Boundaries

- Preserve the first-matching-ping helper for scenarios that need it.
- Do not add environment variables, site configuration, Nix settings, or a
  production protocol surface.
- Do not change feed fan-out, WebSub payload semantics, or non-e2e cadence.
- No implementation outline or ADR: this is routine test-infrastructure work.
