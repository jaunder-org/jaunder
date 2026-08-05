# ADR-0096: E2e perf capture is separate from trace attribution, under a lifecycle envelope

- Status: accepted
- Date: 2026-08-01
- Issue: [#794](https://github.com/jaunder-org/jaunder/issues/794)

## Context

Two independent things happen when the e2e harness sets a test up, and until now
they were fused into one step in `_autoPerfSpan`:

1. **Attribution** — applying the per-test `traceparent` so the server's request
   spans carry this test's span id. This is the structural join the `#[server]`
   flow-coverage gate walks (#681, ADR-0011). It must happen _after_
   `warmupPageContext`, because warmup traffic is deliberately **not**
   attributed to any test; it lands in the orphan bucket by design, and
   `docs/observability.md` documents the exact reason set expected there.

2. **Client-side perf capture** — `addInitScript`, `exposeBinding`, and the
   request/navigation/lifecycle listeners that populate the `e2e.test` span's
   navigation, resource and long-task attributes.

Because both were done in the single "attach after the warmup" step, capture
inherited attribution's timing constraint for no reason of its own. The
consequence, measured in the #788 investigation: **28–31 % of per-test
wall-clock is invisible** — warmup, context mint, fixture setup and teardown all
fall outside any span. The warmup's own duration is measured nowhere, which is
what blocks #792 from deciding whether the warmup buys anything.

A second constraint shaped the fix. `e2e.test`'s span id is the attribution
join, and its time range is what every existing analysis — including all of
#788's numbers — means by "in-span time". Widening it to cover the fixture
lifecycle would have been the smallest change and would have silently redefined
every one of those numbers.

## Decision

**Capture and attribution are decoupled.** Perf capture attaches when a browser
context is created; the per-test traceparent is still applied only after warmup.
#681's contract is unchanged — warmup traffic still carries the run-wide
traceparent and still orphans for the same recorded reason.

**Capture attaches at context level, in exactly one place.** One
`attachTraceCapture(context, …)` uses Playwright's context-scoped
`addInitScript` / `exposeBinding` / `on("request")` plus `on("page")` for
page-scoped lifecycle events. It is called from `_autoPerfSpan` and from the
`tracedContext` fixture, so every page in every context is instrumented through
one code path — including pages a spec opens later via `context.newPage()`.
There is no per-page opt-in to forget, which is the failure mode that left extra
`tracedContext` pages uninstrumented.

**Capture is phase-aware, so decoupling does not move `e2e.test`'s numbers.**
Warmup is on in the gate (`flake.nix:947`), so attaching capture earlier means
warmup's navigation and its ~10 requests would otherwise flow into the same
arrays that produce `e2e.request_count`, `e2e.navigation_count` and the top-N
JSON attributes — shifting them by roughly one navigation and ten requests per
test. `attachTraceCapture` therefore writes into a swappable sink, and
`_autoPerfSpan` swaps it from the warmup sink to the test sink at the same
moment it applies the traceparent. Without this, the "every #788 number stays
comparable" property below would be false, and a keys-only check would not have
caught it.

**`e2e.test` is never widened.** Instead a lifecycle envelope nests around it:

```
e2e.test.lifecycle
├── e2e.warmup            (only when JAUNDER_E2E_WARMUP is on)
├── e2e.context_mint
├── e2e.test              (unchanged span id, range, and attributes)
│   └── … server request spans
├── e2e.page              (one per instrumented page beyond the default)
└── e2e.teardown
```

Phase children are properly time-contained by the envelope, so interval-union
analysis works on them unchanged.

**The unmeasurable residual is named.** Playwright tears fixtures down in
reverse setup order, so the span build and OTLP POST run before
`context.close()`. Our own export and the context teardown cannot be measured
from inside the thing doing the measuring. `docs/observability.md` states this
as the known floor rather than absorbing it into a rounded number.

## Consequences

- The invisible 28–31 % becomes attributable, which is what #792 (warmup) and
  the mount-cost lever needed to be sizable from data rather than argued.
- Multi-context tests stop under-reporting client cost. `traces analyze`
  sections that key on the default page alone gain `e2e.page` siblings.
- Every `#788` number stays comparable across this change, because `e2e.test`
  kept its meaning. That was the point of the envelope over the cheaper
  widening.
- More spans per test — an envelope plus three to four children, plus one per
  extra page: roughly 4–6 where there was 1, so order 400–600 additional spans
  per combo across ~100 tests, ~2 000 across the four-combo matrix. Accepted:
  they are small and attribute-light, and the alternative was attribute blobs
  that interval-union tooling cannot consume without a bespoke join. Note the
  trade against the residual above — OTLP export is itself part of what cannot
  be measured, so span volume and the floor push in the same direction.
- A permanent floor remains on how much per-test time can ever be attributed. It
  is documented and bounded; closing it would require measurement machinery
  outside the fixture being measured, which is not worth its cost.
- Anything added to the capture path must go through `attachTraceCapture`.
  Adding page-level instrumentation directly to `_autoPerfSpan` would silently
  recreate the gap this ADR closes for `tracedContext` pages.

## Addendum (#792, 2026-08-05): the warmup this ADR measured is gone

This ADR's decision — capture is separate from attribution, under a lifecycle
envelope — is unchanged and still in force. But the per-test warmup it uses
throughout as its motivating example **no longer exists**: #792 measured it,
found it cost 113–139 s per combo to buy back at most ~12 s, and removed it.
Read the warmup references above (the Context's "attach after the warmup", the
`e2e.warmup` node in the span tree, and the `flake.nix:947` citation) as the
historical state this ADR was written against.

What that changes, and what it does not:

- **`e2e.warmup` is no longer emitted.** The envelope is now
  `lifecycle → {context_mint, test, page…, teardown}`.
- **The phase-tagged sink survives, renamed.** Its `warmup` phase is now
  `pretest` — the window between context creation and the traceparent switch.
  The guarantee it provides (nothing a fixture does before the test body lands
  in `e2e.test`'s arrays) is structural and does not depend on what occupies
  that window.
- **#681's orphan bucket is now empty, and the mechanism stays anyway.** The
  warmup's `/` load was its only source, so removing the warmup emptied it: the
  regenerated `server-fn-coverage` snapshot has `orphans: {}` where it had four
  app-shell fns. The bucket is not thereby dead — the pre-test window it guards
  (context created, traceparent not yet applied) is structural, and an empty
  bucket is the correct steady state rather than an unused branch.
- **This ADR's own premise was vindicated.** "The warmup's own duration is
  measured nowhere, which is what blocks #792 from deciding whether the warmup
  buys anything" — the envelope this ADR introduced is exactly what let #792
  decide, and the deciding number (`e2e.warmup` summed per combo) came from the
  span it added.

See [ADR-0099](./0099-e2e-does-not-pre-warm.md) for the removal decision, and
`docs/observability.md` §"#792 — the per-test warmup A/B" for the data.
