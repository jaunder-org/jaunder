# Issue #866 — decompose the non-wasm half of `commitToMount`

**Status:** draft, awaiting approval **Issue:**
[#866](https://github.com/jaunder-org/jaunder/issues/866) **Branch:**
`worktree-issue-866-boot-phase-remainder` (fork point tagged
`wt-base-issue-866`) **Predecessors:** #788 → #792 → #794 → #818 (decomposition)
→ #840 (withdrew the streaming claim) → #836 (bundle size; found the volume
lever weak)

## Why

#836 established that page boot is **43–47% of e2e suite wall-clock** in both
engines, and that wasm explains only part of it.

**Denominator, stated once and used throughout:** each suite run performs **211
navigations**, of which **208 mount** and carry boot marks. All per-suite
seconds below are `mean × 208`. Means, never medians —
`median(a+b) ≠ median(a) + median(b)` (#818 saw a 2–7% artifact residual from
that mistake).

Arm C (what ships), three runs pooled, all six segments:

| segment                             | chromium              | firefox          |
| ----------------------------------- | --------------------- | ---------------- |
| `document_start → wasm_fetch_start` | **44.2 s** (54% boot) | **54.2 s** (36%) |
| `wasm_fetch`                        | 33.5 s                | 11.8 s           |
| `wasm_instantiate`                  | 2.9 s                 | **84.2 s** (56%) |
| `boot.entry → seed_parsed`          | 0.3 s                 | 0.3 s            |
| `seed_parsed → render_start`        | 0.1 s                 | 0.3 s            |
| `render_start → mount_done`         | 0.7 s                 | 0.4 s            |
| **boot total**                      | 81.9 s                | 151.2 s          |
| frame skew (`commitToMount` − boot) | **61.3 s**            | 38.3 s           |
| **`commitToMount`**                 | 143.2 s               | 189.5 s          |

The ~95 s/browser wasm did not explain is **two** things: the pre-fetch window
and frame skew. Neither had been looked at.

Two facts worth stating outright:

- **The Rust boot path totals ~1 s of a 300–450 s suite.** #801 was right to be
  redirected; there is nothing there.
- **`document_start → wasm_fetch_start` is the largest boot segment on
  chromium** and second largest on firefox — and it is entirely our own code.

### What is actually in that window — observed, then hypothesised

**Observed.** The shell (`csr/index.html`) boots in this order: a synchronous
inline pre-paint auth script (#181/ADR-0044) in `<head>`, then two
`<link rel="stylesheet">`, then an **inline `<script type="module">` in
`<body>`** which imports `/pkg/jaunder.js` and only then calls
`init("/pkg/jaunder.wasm")`. The wasm fetch therefore cannot begin until the 56
KiB glue has been fetched, parsed and executed. That ordering is a fact about
the file.

The window shrinks 35–39% cold→warm (chromium 257.9 → 158.5 ms, firefox 310.2 →
201.4 ms), so it is **part network, part not** — a substantial residual survives
a warm cache in both engines.

**Hypothesised, and explicitly not established.** A pending stylesheet blocks
script execution per spec, so the two stylesheets _may_ sit on the critical path
to starting the wasm fetch. **This has not been measured**, and this spec does
not rely on it. Naming it as the cause without a test is the #840 error. It is
recorded as a candidate for whoever takes the stylesheet question (deferred
below), not as a premise here.

### A second, larger lever this analysis surfaced

`server/src/site.rs` sets **no `Cache-Control` on `/pkg/*`** — only a
per-representation `ETag` and `Vary: Accept-Encoding`. Browsers therefore
heuristically cache and revalidate, and the code's own doc comment records that
**~44% of `pkg/jaunder.wasm` requests were conditional** in a measured run.

So a large share of `wasm_fetch` is a revalidation round-trip. Preload starts
that round-trip earlier; it does not remove it. Removing it needs content-hashed
asset URLs plus a long `max-age`, which the current unhashed `jaunder.wasm` name
forbids. **That is plausibly a bigger win than the fix in this spec, and it is
deliberately not attempted here** (D9).

## Scope

1. **Report** the decomposition in `docs/observability.md`.
2. **Land** the preload, taking the wasm fetch off the serial chain.
3. **Measure** it against a prediction registered beforehand, with a
   pre-committed abort rule.
4. **File** frame skew, the caching lever, and the stylesheet question.

## Decisions

### D0 — "Material" is defined

Material = **≥ 10 s per suite run** in either engine. Material segments get a
disposition: fixed here, filed, or a recorded reason. Below-threshold segments
are reported and **closed**, not deferred.

Material: pre-fetch (44.2 / 54.4 s), `wasm_fetch` (33.5 s chromium),
`wasm_instantiate` (84.5 s firefox — already #864), frame skew (61.3 / 38.4 s).
Not material: the three Rust segments (~1 s total) — closed.

### D1 — The decomposition reuses #836's corpus

Already certified: 100% mark coverage on 3744 mounted navigations,
`dropped = 0`, 0 closure violations, 72 populations. Re-reading segments it
already contains needs no new arms.

Inherited limits, restated wherever the numbers appear: n=3, host not perfectly
quiescent, arm confounded with position within a round. The last threatened
**cross-arm contrasts**; this analysis makes none.

### D2 — `topSlow` is truncated; it is not a census

`e2e.resource_top_slow_dropped` sums to **54** across 822 arm-C spans, so
per-resource timings from `e2e.resource_summary_json` are **duration-biased**
and may not carry share arithmetic. Quotable only as illustrative
single-resource timings, said to be such.

The contrast matters: `navigation_top_json` has `dropped = 0` and **is** a
census. Losing that distinction is how a biased top-N slice gets read as a
population.

### D3 — Frame skew is reported, not decomposed, and filed

61.3 s (chromium) / 38.4 s (firefox) — material, and larger on the _faster_
engine. **ADR-0100 forbids decomposing `commitToMountMs`** into document-frame
segments; different clocks. Attributing it needs a Node-frame instrument this
cycle is not building. Whether it is harness overhead or real time is precisely
what is unknown; asserting either is the #840 error.

### D4 — The fix: take the wasm fetch off the serial chain

Add to the shared `<head>`:

- `<link rel="preload" href="/pkg/jaunder.wasm" as="fetch" type="application/wasm">`
- `<link rel="modulepreload" href="/pkg/jaunder.js">`

**Ceiling, not an estimate.** The saving cannot exceed the wasm fetch time, and
only insofar as it fits inside the pre-fetch window:

| engine   | pre-fetch | `wasm_fetch` | ceiling           | of boot total |
| -------- | --------- | ------------ | ----------------- | ------------- |
| chromium | 212.5 ms  | 161.3 ms     | ≤161 ms → ≤33.5 s | ≤41%          |
| firefox  | 260.7 ms  | 56.8 ms      | ≤57 ms → ≤11.8 s  | ≤7.8%         |

**The ceiling will not be reached**, and the write-up must not imply it will.
Moved in parallel, the wasm download contends with the glue and both stylesheets
for the same connection, so its own duration grows. And on the ~44% of requests
answered `304`, the round-trip persists — preload only starts it sooner.

**Firefox is the gate's critical path and gains least**: ≤11.8 s of a 447 s
suite, ≤2.6%. This is worth doing — it is cheap and helps real users on slow
links more than it helps CI — but it is **not** a fix for the e2e gate.

### D5 — There are two shells, and `render_head` is shared

Verified, correcting the obvious guess:

| location                      | emits                                                            |
| ----------------------------- | ---------------------------------------------------------------- |
| `csr/index.html`              | the standalone SPA shell (head + boot script)                    |
| `web/src/app/render.rs`       | `render_head` — stylesheets/meta only, **no script**             |
| `server/src/projector/mod.rs` | composes `PREPAINT_SCRIPT` + `render_head` + its own boot script |

So the preload belongs in **`render_head`** (which covers every projected page)
and in **`csr/index.html`**. Adding it to the projector as well would emit it
**twice** on projected pages.

`PREPAINT_SCRIPT` already has a drift guard (`render.rs:255`) — precedent that
this duplication has bitten. The new links need equivalent coverage.

**Correcting a plausible-sounding non-issue:**
`audit_wasm::shell_boot_artifacts` scans only for `init("` and
`import init from "`; it never reads `href=`, so preload links do not affect it.
No work is needed there.

**The real gap:** `/pkg/jaunder.wasm` is hardcoded in at least four places
(`render.rs`, `projector/mod.rs`, `audit_wasm.rs`, `wasm_budget.rs`), and
nothing cross-checks a preload `href` against the `init()` target. A preload
pointing at a stale path degrades silently into a double fetch.

### D6 — A double fetch is the failure mode, and it is tested not assumed

A preload whose mode does not match the real request downloads the resource
**twice** — worse than doing nothing, and invisible unless requests are counted.

This spec deliberately **does not pre-commit** whether `crossorigin` is
required; the browsers have historically disagreed for `as="fetch"`. It is
determined by test, and the test is the deliverable: **a navigation must issue
exactly one request for `/pkg/jaunder.wasm`.**

The assertion must state its cache state (cold vs warm) and count a `304` and a
`.br` variant as the same resource — otherwise it is either flaky or vacuous.
Server-side `site.status` is the engine-independent signal (the browsers
disagree on `transferSize` for revalidated responses — #818).

### D7 — The effect is predicted before it is measured

Per #836 and #840's standing lesson.

- The decisive quantity is **`document_start → mount_done` (boot total)** —
  document-frame, ADR-0100-clean. Not the pre-fetch segment: the fix leaves that
  window untouched and collapses the _later_ `wasm_fetch` segment into it, so a
  per-segment reading would misattribute the change.
- Suite wall-clock is context only. It did not separate arms on firefox in #836
  and will not resolve ≤2.6% here.
- The prediction is registered against the **before-arm of this capture**, not
  against #836's corpus, so divergence cannot confound with host drift.
- Protocol is #818's: single-worker sqlite × both browsers, 3 runs per arm, two
  arms (before/after) **interleaved**, distinct `e2eSalt` per run. ~2 × 70 min.

**Minimum detectable effect, stated before collecting.** Firefox boot total is
~727 ms with run-level SD ~3–9 ms at n=3, giving SE ≈ 2–6 ms; the ≤57 ms ceiling
is many SE and _is_ resolvable on boot total. The same effect is **not**
resolvable on suite wall-clock, which is why D7 fixes boot total as decisive in
advance.

**A floor, so the prediction can fail.** A ceiling alone is unfalsifiable —
every outcome in `(0, ceiling]` confirms it. The registered prediction therefore
also carries a floor: the improvement must exceed **3 × SE** on boot total in
**at least one** engine. Below that, the fix is indistinguishable from noise and
D8 treats it as "no improvement".

### D8 — Abort rule, committed before the capture

If the after-arm shows **no improvement in boot total, or a regression, in
either engine**, the preload is **reverted** and the finding is written up as a
negative result. It does not land on the strength of "the reasoning is sound."

If a double fetch is detected (D6) the preload is reverted regardless of timing.

This is what stops A8 from being satisfiable by narrating any outcome.

### D9 — The caching lever is filed, not taken

`Cache-Control` on `/pkg/*` plus content-hashed asset URLs would remove the ~44%
conditional round-trips outright, rather than starting them earlier. It is
plausibly the larger win.

It is **not** attempted here: hashed URLs touch `csr_bundle.rs`, both shells,
the projector, `audit_wasm`, `wasm_budget`, and the server's embed — a cycle of
its own. Filed with the 44% figure and the measured `wasm_fetch` seconds
attached, so the next person starts from evidence rather than re-deriving it.

## Acceptance criteria

- **A1** All six segments, both engines, cold and warm, means, with the 208-
  navigation denominator stated, in `docs/observability.md`.
- **A2** Every D0-material segment has a disposition that is **either** a landed
  change **or** a filed issue number. Prose alone does not satisfy this. The
  non-material Rust segments are stated as closed.
- **A3** Frame skew's magnitude is reported and filed, naming ADR-0100 as the
  reason it was not decomposed.
- **A4** `topSlow`'s truncation (D2) is stated wherever resource timings appear.
- **A5** _(applies only if the preload lands — see D8)_ The preload is present
  in `render_head` **and** `csr/index.html`, absent from the projector's own
  markup, and a test fails if the two shells drift or if a preload `href` stops
  matching the `init()` target. **If D8 reverts the fix, A5 is satisfied by the
  preload and its now-vacuous guards being absent.**
- **A6** A test asserts exactly one request for `/pkg/jaunder.wasm` per
  navigation, with the cache state named, counting `304` and `.br` as the same
  resource. This one **survives a revert** — it guards the property regardless
  of how the wasm fetch is initiated — and its ability to detect a double fetch
  is demonstrated once by deliberate mutation, not assumed.
- **A7** The predicted-vs-observed table reports boot total per engine, and
  **states whether D8's abort rule fired**.
- **A8** The headline figure quoted for the fix is **firefox's**, not
  chromium's.
- **A9** Issues filed for: frame skew, the caching lever (D9), the render-
  blocking stylesheet question.
- **A10** `cargo xtask validate` green, e2e included.

## Deferred — filed as issues, not solved here

- Frame skew, 61.3 s chromium / 38.4 s firefox per suite (D3) —
  [#868](https://github.com/jaunder-org/jaunder/issues/868).
- `Cache-Control` + content-hashed asset URLs (D9), against `wasm_fetch` of 33.5
  s chromium / 11.8 s firefox and ~44% conditional requests — likely the larger
  lever. [#869](https://github.com/jaunder-org/jaunder/issues/869).
- Render-blocking stylesheets on the boot critical path — the untested
  hypothesis from _Why_, against a pre-fetch window of 44.2 s chromium / 54.4 s
  firefox. [#870](https://github.com/jaunder-org/jaunder/issues/870).
- Firefox's ~377 ms instantiate floor —
  [#864](https://github.com/jaunder-org/jaunder/issues/864), still the single
  largest item at 84.5 s.
- The navigation count, 211 for 137 tests —
  [#867](https://github.com/jaunder-org/jaunder/issues/867).

## Out of scope

- Changing the e2e protocol, worker count, or CI matrix shape (ADR-0034).
- Pre-warming, in any form (ADR-0099).
- Decomposing `commitToMountMs` (ADR-0100).
