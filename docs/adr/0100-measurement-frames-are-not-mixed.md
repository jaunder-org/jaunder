# ADR-0100: Browser measurements are decomposed only in the document frame

- Status: accepted
- Date: 2026-08-05
- Issue: [#818](https://github.com/jaunder-org/jaunder/issues/818)

## Context

The e2e harness measures a navigation's client-side cost in **two different
clocks**, and they are not interchangeable.

- **The document frame.** `performance.mark` timestamps and
  `PerformanceResourceTiming` entries are relative to that document's
  `performance.timeOrigin`. Everything `capture-trace.ts`'s `harvestDocument`
  returns — the `jaunder.*` boot marks, the `.wasm` fetch timing — is in this
  frame, read from inside the page.
- **The Node frame.** `NavigationRecord`'s `committedMs`, `domContentLoadedMs`,
  `loadMs`, and `mountedMs` are `Date.now()` stamps taken in the Playwright
  driver when an event or a binding call arrives. `commitToMountMs` is
  `mountedMs - committedMs`, so it lives entirely here.

`capture-trace.ts` already stated the rule — _"comparable to each other but NOT
to the Node-side `Date.now()` fields … The two are never mixed"_ — but #794's
own `timingFor` doc comment simultaneously described the goal as decomposing
`commit_to_mount`, and `docs/observability.md` repeated that framing. The
instrumentation was therefore shipped pointed at a target it could not correctly
decompose, and #818 was specified against that target before the conflict was
noticed.

The difference between the two frames is not a constant offset that cancels. It
is:

- the lag between the driver observing a navigation commit and the new
  document's `timeOrigin` being set; plus
- the mount→binding round trip — `data-mounted` is set in the page, a
  `MutationObserver` fires, and an `exposeBinding` call crosses to Node before
  `mountedMs` is stamped.

Both are cross-process, both depend on the driver protocol (CDP for chromium,
juggler for firefox), and both are therefore **plausibly engine-asymmetric**.
That is exactly the confound #818 exists to rule out: decomposing a Node-frame
total into document-frame parts would charge harness IPC latency to the app's
boot phases, and a browser-differential study would report it as a finding about
the app.

## Decision

**A decomposition is computed entirely within one clock frame. Browser-side boot
analysis uses the document frame.**

Concretely:

- The analysis target is `bootTotalMs` — the document-relative interval from
  `timeOrigin` to the last boot mark (`jaunder.boot.mount_done`). Its parts are
  `wasmFetchStartMs`, `wasmFetchMs`, `wasmInstantiateMs`, and the `bootPhases`
  intervals. These **sum to it exactly, by construction**, so a non-zero
  residual is a data defect rather than an unexplained phase.
- `commitToMountMs` remains reported, as the bridge to suite wall-clock, but is
  **never** used as the total for that decomposition.
- The difference, `commitToMountMs - bootTotalMs`, is reported separately as
  **frame skew** — a harness cost, attributed to the harness.
- `mountToSettledMs` stays wholly in the Node frame, as it already was.

## Consequences

- #794's published per-phase numbers are **superseded, not corrected**: they
  were chromium-only (firefox recorded no marks at all — #818) and framed
  against `commit_to_mount`. They are not re-derived.
- Any future instrument that wants to decompose `commit_to_mount` must first
  measure the frame skew per engine and subtract it explicitly. Nothing does
  today, and adding one is a decision to revisit, not an oversight to fill in.
- Frame skew becomes a measurable quantity in its own right. If it turns out to
  be large and engine-asymmetric, that is a finding about the harness's own
  overhead and belongs to the e2e wall-clock conversation, not to app
  performance.
- This rules out the tempting shortcut of comparing a mark-derived phase against
  a driver-derived duration to "check" one against the other. They disagree by
  the skew, and reading that disagreement as measurement error would hide a real
  cost.
