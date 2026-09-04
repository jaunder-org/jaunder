# ADR-0099: The e2e suite does not pre-warm

- Status: accepted
- Date: 2026-08-05
- Issue: [#792](https://github.com/jaunder-org/jaunder/issues/792)

## Context

From 2026-04 (`7774d540`) every e2e test began with a warmup: before the test
body, the default page's context navigated to `/` and waited for
`body[data-mounted]`, so the test's own first navigation would hit a warm HTTP
cache. It arrived as one half of a diagnostic A/B pair — warm checks and cold
packages — and the warm half then became the gate default, where it stayed for
four months without its cost ever being measured.

#788 suspected it protected nothing, on the grounds that
`navigation.commit_to_mount` looked no faster warm than cold. #792 measured it
properly: six runs of `cargo xtask traces run`, interleaved A/B/A/B/A/B on a
quiescent host, arms differing in exactly one token at otherwise gate-identical
settings, each run cache-busted so nix could not serve a cached suite.

The result (full data in `docs/observability.md` §"#792 — the per-test warmup
A/B", verdict at
<https://github.com/jaunder-org/jaunder/issues/792#issuecomment-5186123216>):

| sqlite median suite duration | with warmup | without | Δ           |
| ---------------------------- | ----------- | ------- | ----------- |
| chromium                     | 226.8 s     | 174.0 s | **−23.3 %** |
| firefox                      | 323.7 s     | 256.0 s | **−20.9 %** |

Span sums say why: the warmup costs **113 s/combo on chromium and 139 s on
firefox**, and removing it adds back **11.7 s** of test time on chromium and
nothing measurable on firefox. It buys roughly a tenth of what it costs.

Two of #788's premises did not survive the measurement, and the difference
matters for anyone tempted to reintroduce this:

- **Warm genuinely is faster than cold** — about 200 ms of `commitToMountMs` and
  6–15 ms of `requestMs`. #788's contrary finding compared the warm checks (2
  workers) against the cold packages (1 worker) and was confounded. The warmup's
  problem is not that warming does nothing; it is that it pays a **full mount
  per test** to make ~1.6 navigations per test slightly cheaper.
- **The "28–31 % of per-test time outside the test span" was mostly the warmup
  itself.** Removing it collapses that envelope to ~7.5 % on chromium.

Flakiness — the reason to be cautious, given `RETRIES=1` hides fail-then-pass —
did not argue for keeping it: summed `flaky + unexpected` across each browser's
three sqlite runs was **0 in both arms**, and the session's only flake occurred
in the warmup arm.

## Decision

**The e2e suite does not pre-warm.** There is no per-test warmup navigation, no
`JAUNDER_E2E_WARMUP*` configuration, and no `e2e.warmup` span. Every test's
first navigation is a genuine cold load, and the traces show it as one.

Do not reintroduce a warmup — at any scope — without a measurement of the same
shape as #792's: matched arms at gate-identical settings, cache-busted so each
run really executes, with suite wall-clock as the primary metric and retry
counts as a guardrail. The specific reasoning that must be rebutted is the
ratio: a warmup costs one full mount per test, so it has to save more than that
per test to pay for itself.

Note that a **once-per-worker** warmup is not an available shortcut around this.
Playwright mints a fresh browser context per test and HTTP cache is not shared
across `browser.newContext()`, so a worker-scoped warmup would prime a context
no test uses.

## Consequences

- **The gate is ~21–23 % faster per combo**, and correspondingly cheaper in CI
  where each combo is its own job.
- **Cold-start cost is now visible instead of hidden.** Under the warmup, traces
  recorded _zero_ cold navigations — every measured navigation was warm because
  the fixture had already paid the cold one outside the test span. Post-removal
  a little over half of navigations report `cacheWarmth: "cold"`, which is what
  the suite actually does and what #801 needs in order to size mount cost
  honestly.
- **The `-cold` package family lost its distinguishing feature** and is renamed
  to say what it now uniquely provides — a single worker, for per-navigation
  timings free of contention (`traces run --single-worker`). It is kept, not
  deleted: #801 needs that isolation.
- **`e2e.warmup` disappears from the span tree** (#794/ADR-0096). Analyses must
  tolerate its absence; traces captured before this change still contain it.
- The measurement apparatus that produced this decision — `nix/checks.nix`'s
  `e2eSalt` and the `e2e-scaffold` guard — **stays**, so the next e2e
  performance question can be answered the same way rather than argued from
  stale numbers.
