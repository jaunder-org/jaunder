# #818 — why is firefox ~1.5× chromium? Fix the boot-phase instrument, then attribute

Issue: [#818](https://github.com/jaunder-org/jaunder/issues/818). Milestone:
Test infrastructure & E2E. Provenance: #788 (the observation), #792 (the
corpus), #794 (the instrumentation).

## Summary

#818 assumed the question was answerable from #792's corpus, because #794's boot
marks are app-side and therefore browser-agnostic. **The marks are; the harvest
is not.** Firefox recorded zero boot phases on every navigation of every run in
the preserved corpus, so the per-phase comparison the issue asks for cannot be
computed from existing data at all.

A second, independent defect surfaced while specifying the fix: the quantity
#794 set out to decompose is in a **different clock frame** from every part it
is decomposed into (D8). Both must be corrected before the attribution question
can be asked honestly.

This cycle therefore has two parts, landing as two PRs: **fix the instrument**,
then **run a fresh measurement session and attribute the gap**.

## Evidence — the instrument is dark on firefox

Measured over the preserved corpus
(`~/measurements/jaunder/issue-792-warmup-ab/traces/`), all 210 navigations per
combo, read from `e2e.boot_marks_json` (uncapped) and `e2e.navigation_top_json`
(`e2e.navigation_top_dropped = 0` throughout, so also complete):

| combo (sqlite)           | marks-per-nav histogram | navs with `wasmFetchMs` |
| ------------------------ | ----------------------- | ----------------------- |
| b1 chromium              | `{0: 138, 4: 72}`       | 78 / 210                |
| b2 chromium              | `{0: 135, 4: 75}`       | 85 / 210                |
| b3 chromium              | `{0: 139, 4: 71}`       | 81 / 210                |
| a1 chromium (warmup arm) | `{0: 151, 4: 59}`       | 85 / 210                |
| b1 firefox               | `{0: 210}`              | 48 / 210                |
| b2 firefox               | `{0: 210}`              | 41 / 210                |
| b3 firefox               | `{0: 210}`              | 38 / 210                |
| a1 firefox (warmup arm)  | `{0: 210}`              | 26 / 210                |

Chromium is strictly all-or-nothing — 0 or all 4, never partial, last mark
always `mount_done`. Firefox is **0 on 210/210, in both arms, across every
run**: not a sampling shortfall, a systematic blackout.

**Read the denominator carefully.** 210 is _all_ navigations, including those
where `load` never fired and no harvest ran at all. The trace data cannot
separate "no harvest" from "harvested, nothing to see" — a navigation with
neither marks nor wasm timing looks identical either way. So these columns bound
the defect without isolating it, and no claim below rests on the wasm-fetch
column's exact value.

## Root cause — two defects, one fix

`capture-trace.ts:414` harvests a document's marks on the `load` event. Marks
persist for the document's lifetime, so a _late_ harvest is not the problem.
Rather:

1. **`load` frequently never fires.** `goto` waits only for `domcontentloaded`,
   so the test navigates away or ends first and the harvest never runs. This
   caps chromium at ~34% of all navigations and is already documented as a known
   gap in `docs/observability.md` §"What the boot marks do and do not cover".
2. **On firefox, when `load` does fire, it lands before boot reaches
   `jaunder.boot.entry`.** `csr/index.html:14-17` loads the wasm from a module
   script that calls `init(...)` and never awaits the returned promise, so the
   module script completes and `load` can fire with fetch/compile/instantiate
   still in flight — the wasm is structurally not `load`-blocking. Firefox loses
   this race **every time**: 0 marks across ≥38 navigations per run that
   demonstrably _were_ harvested (they carry wasm timing). Chromium wins it
   often enough to look merely lossy.

Defect 2 is what made firefox undiagnosable, and it is invisible to defect 1's
framing — which is why the corpus README recorded it as "often null" rather than
as a bug.

**The fix.** `csr/src/lib.rs:51-61` emits all four marks synchronously _before_
`mark_ready()` sets `data-mounted`:

```rust
mark(BOOT_ENTRY);
mount();            // marks SEED_PARSED, then RENDER_START
mark(BOOT_MOUNT_DONE);
mark_ready();       // sets data-mounted
```

So a harvest triggered by mount-ready catches the full mark set **by
construction, on any engine**. `capture-trace.ts:222` already exposes a
Node-side binding (`__jaunderRecordMount`) driven by a `MutationObserver` on
that attribute, and Playwright's `exposeBinding` source is
`{context, page, frame}` — so `source.page` is a usable handle and is a _more
direct_ identification than the current href match.

## Decisions

- **D1 — Harvest at mount-ready, and keep the `load` harvest.** Keeping both
  means the change cannot _reduce_ coverage on any navigation, including
  navigations that never mount, which still yield wasm resource timing from the
  `load` path.
- **D2 — Merge harvests by keeping the larger mark set, not by write order.**
  Marks persist, so a later harvest is a superset — but `documentTimings.set()`
  is last-_resolution_-wins, and that only coincides with issue order because
  two `page.evaluate`s on one page happen to serialize over a single connection.
  Firefox is exactly the case where `load` fires _first_ and its empty snapshot
  must not win, so the invariant must not rest on undocumented transport
  ordering. Keep whichever snapshot has more marks (ties → the one with wasm
  timing). One comparison, local invariant.
- **D3 — `traces analyze` reports boot-decomposition coverage.** Per project:
  navigations, mounted count, and full-mark-set count. Load-bearing for AC11,
  not a nicety: the corpus must be certified before any conclusion may be drawn
  from it, and no such figure exists today.
- **D4 — A non-thresholded in-suite assertion ships with the fix; the
  fail-closed _gate_ is a separable follow-up.** A spec asserting the full mark
  set is present after mount needs no distributional knowledge and closes the
  regression hole immediately. A per-combo coverage _gate_ needs a threshold,
  and the honest way to pick one is from the post-fix distribution, which does
  not yet exist — so that is filed as its own issue rather than guessed at here.
- **D5 — The single-worker packages are the deciding data; gate settings
  confirm.** #818 is a per-navigation question, and the suspect phases
  (compile/instantiate, first render) are exactly the CPU-bound ones two workers
  on a 2-core VM inflate — possibly asymmetrically between engines, which would
  manufacture the finding we are hunting. **Which set decides is fixed here,
  before collection.**
- **D6 — Cold and warm navigations are analyzed separately, never pooled.**
  `fixtures.ts:606` labels navigation 1 of each test `cold`, the rest `warm`;
  #792's arm B split 113/97 per combo with ~200 ms of `commitToMountMs` between
  them. The two differ most in the wasm fetch — precisely a phase under
  suspicion — and the cold/warm mix need not be identical between engines, so
  pooling would be a confound.
- **D7 — The mount-ready harvest's observer effect is measured, not assumed.**
  It fires at the busiest instant of the document's life, with shell and route
  fetches still in flight — the window `mountToSettledMs` covers — via a
  cross-process round trip whose cost is plausibly engine-asymmetric.
- **D8 — The analysis target is the document-frame boot total, not
  `commit_to_mount`.** `commitToMountMs` is `mountedMs - committedMs`, both
  Node-side `Date.now()` (`fixtures.ts:581-584`), while every phase it would be
  decomposed into is document-relative (`performance.timeOrigin`).
  `capture-trace.ts:161-170` states the rule outright — _"comparable to each
  other but NOT to the Node-side `Date.now()` fields … The two are never mixed"_
  — yet `timingFor`'s own doc comment (line 120) describes the goal as
  decomposing `commit_to_mount`. **#794 shipped that frame mix.** Decomposing
  across it would silently charge CDP/juggler event-delivery latency and the
  mount→binding round trip — both cross-process and plausibly engine-asymmetric
  — to the app's boot phases, fabricating exactly the kind of finding this issue
  is hunting.

  The analysis target is therefore **`bootTotalMs` := `mount_done.startTime`**,
  wholly document-relative, decomposed into six consecutive segments that sum to
  it _exactly_, by construction:

  | #   | segment                                  | source              |
  | --- | ---------------------------------------- | ------------------- |
  | 1   | document start → wasm fetch start        | `wasmFetchStartMs`  |
  | 2   | wasm fetch                               | `wasmFetchMs`       |
  | 3   | compile + instantiate                    | `wasmInstantiateMs` |
  | 4   | `boot.entry` → `boot.seed_parsed`        | `bootPhases`        |
  | 5   | `boot.seed_parsed` → `boot.render_start` | `bootPhases`        |
  | 6   | `boot.render_start` → `boot.mount_done`  | `bootPhases`        |

  `commitToMountMs` is still reported, as the bridge to the gate's wall-clock.
  The **difference** between it and `bootTotalMs` is the frame skew —
  commit→`timeOrigin` plus `mount_done`→binding-notify — and is itself reported
  per engine (AC14). It is a real cost of measuring firefox and a plausible
  contributor to the gap, but it is harness overhead, not app boot, and
  conflating the two is the error D8 exists to prevent.

- **D9 — The decision rule is pre-registered (AC12–AC13).** This area has twice
  produced conclusions that later proved confounded (#788's warm-vs-cold,
  comparing 2-worker checks against 1-worker packages; #155's "irreducible",
  asserted with no phase decomposition because none existed). A phase table is
  easy to narrate after the fact.
- **D10 — Two PRs, both referencing #818.** The fix is verifiable on coverage
  alone, which is robust to host load; the findings need a quiesced host.
  Landing the fix first gets it exercised by CI's full `{backend}×{browser}`
  matrix on independent hardware _before_ any conclusion rests on it — the
  precise check whose absence caused this issue. It also unblocks #801, which
  needs the same decomposition.

## Acceptance criteria

### PR 1 — fix the instrument

- **AC1.** A document's `jaunder.*` marks and wasm resource timing are harvested
  when `data-mounted` is observed, in addition to the existing `load` harvest.
  The mount-ready harvest is registered on `pendingHarvests` so `settle()`
  awaits it — without this it races span construction and yields
  intermittently-empty coverage, which would look like a partial fix.
- **AC2.** Merging follows D2: the snapshot with more marks wins regardless of
  completion order; ties break toward the one carrying wasm timing. Covered by a
  unit test of the merge rule, so the invariant does not depend on running a
  browser.
- **AC3.** In a single local `cargo xtask e2e sqlite firefox` run, **every
  navigation with a non-null `mountedMs` reports a full four-mark set** —
  `bootPhases` non-null with three intervals, and `wasmInstantiateMs` non-null.
  Coverage assertion only; makes no timing claim, so it is valid on a
  non-quiescent host. (Extract the JSONL from
  `.xtask/diagnostics/e2e-sqlite-firefox/capture-sqlite.tar.gz` before
  analyzing.)
- **AC4.** The same holds for `cargo xtask e2e sqlite chromium`.
- **AC5.** Both figures are reported on **both** denominators — full-mark-set
  count over _mounted_ navigations, and over _all_ navigations — because the two
  differ by design (unmounted navigations still legitimately record no marks)
  and quoting one against the other's baseline would misstate the improvement.
  The pre-fix all-navigations baseline is ~34% chromium, 0% firefox.
- **AC6.** `cargo xtask traces analyze` prints, per project, a
  boot-decomposition coverage section: navigation count, mounted count,
  full-mark-set count.
- **AC7.** An e2e spec asserts that after mount the document exposes the full
  `jaunder.*` mark set **and** that the harness captured it (D4). No threshold,
  so it needs no distributional knowledge; it reddens on regression in every
  combo of the gate.
- **AC8.** Documentation that describes the old harvest point is superseded —
  not merely the figure, which was true of the run it described, but the
  mechanism:
  - `docs/observability.md` §"What the boot marks do and do not cover";
  - `capture-trace.ts:102` (`DocumentTiming`, "at its `load`") and `:114-121`
    (`timingFor`, "harvested at that document's `load`", plus its
    `commit_to_mount` framing, which D8 corrects);
  - `~/measurements/jaunder/issue-792-warmup-ab/README.md` "Known limits", which
    must record that firefox has **no** decomposition in that corpus — it is not
    re-collectable, so a future reader must not mistake the blackout for
    sparseness.
- **AC9.** A follow-up issue exists for D4's fail-closed per-combo coverage
  gate, filed before the plan's implementation tasks begin.
- **AC10.** `cargo xtask validate` is green.

### PR 2 — measure and attribute

- **AC11.** A fresh corpus is collected on a quiesced host: sqlite × {chromium,
  firefox}, three runs of the single-worker packages (deciding) and three at
  gate settings (confirming), the two settings sets interleaved run-by-run, each
  run distinctly salted so nix cannot serve a cached suite. `/proc/loadavg` is
  sampled before and after every run and recorded; **any run whose 1-minute
  figure exceeds 3.0 at either sample is discarded and re-taken** (#792's band
  was 2.10–2.60 against a 0.75 baseline, so 3.0 admits that session's conditions
  and little more).
- **AC12.** The corpus is certified before analysis: per browser, pooled across
  that settings set's three runs, the full-mark-set count is **≥99% of mounted
  navigations**, and the mounted count is **≥95% of all navigations**. The
  second half matters because AC3's denominator is partly self-selecting — a
  navigation that fails to match the mount binding drops out of both numerator
  and denominator silently. Certification is required of the deciding set; a
  shortfall in the confirming set is reported, not fatal. **Below either floor
  the analysis does not proceed**: the corpus is re-collected once, and if it
  fails again that is a PR 1 regression and #818 returns to the fix.
- **AC13.** For each of cold and warm separately (D6), and for each settings
  set, the write-up states median `bootTotalMs` per engine, the firefox−chromium
  gap, and each of D8's six segments' **signed** share of that gap
  (`ff_median(segment) − chr_median(segment)`, over the gap). Shares sum to 100%
  by construction; a segment where firefox is _faster_ contributes negatively
  and must be shown as such rather than dropped. **Any residual is a data
  defect, not a finding** — the segments close exactly — so a residual above 1%
  of the gap halts the analysis and is investigated, not reported as
  unexplained.
- **AC14.** The frame skew is reported per engine: median
  `commitToMountMs − bootTotalMs`, cold and warm. This is harness overhead, not
  app boot; it is stated separately and never folded into a segment share.
- **AC15.** The verdict has two independent parts, both stated against these
  pre-registered rules.

  **Diagnosis**, from the deciding set, required to agree across cold and warm:
  - **"segment S dominates"** — S's share is **≥40% of the gap and ≥1.5× the
    next largest segment's share**;
  - **"distributed"** — no segment meets that bar.

  **Disposition:**
  - **"actionable"** — diagnosis is a dominant segment _and_ a concrete lever is
    named (bundle size, streaming compilation, deferred init), filed as an
    issue;
  - **"intrinsic"** — diagnosis is "distributed" _and_ the decomposition is
    uniform: every segment holding ≥5% of chromium's `bootTotalMs` has a
    firefox/chromium ratio within ±20% of the overall `bootTotalMs` ratio. That
    ratio is **computed from this corpus**, not imported: the familiar ~1.47× is
    #792's _suite wall-clock_ figure, and arm B's per-navigation
    `commitToMountMs` ratios were ~1.40× warm and ~1.15× cold, so anchoring to
    1.47× would read "non-uniform" on uniform data;
  - **"unresolved"** — anything else: a dominant segment with no lever, cold and
    warm disagreeing, the confirming set contradicting the deciding one, or
    "distributed" but non-uniform. This is a real outcome, not a gap in the
    rules; it is reported as such with the specific disagreement named and a
    next step proposed.

- **AC16.** D7's observer effect is quantified: post-fix chromium
  `mountToSettledMs` from the **gate-settings** subset (matching #792 arm B's
  2-worker collection) is compared against that baseline, and the delta stated.
  Materiality threshold: **>10% of the baseline median** is called out as
  affected. The comparison also states which intervening changes to the mount
  path, if any, landed between 2026-08-04 and collection — the baseline is a
  pre-_harvest_-change baseline only if nothing else moved those fetches. If
  affected, that is a follow-up issue, not a blocker for #818.
- **AC17.** Findings land in `docs/observability.md` as a `#818` section
  following the shape of the existing `#792` and `#155` sections, stating
  conditions, the reproduction recipe, and the path to the aggregation script.
- **AC18.** The aggregation is a committed script, not ad-hoc shell.
  `traces analyze` computes maxima and averages but **not medians or
  percentiles** (the #792 write-up's p50s were aggregated by hand over the
  JSONL), and AC13 is entirely medians and shares. Committing it is what makes
  AC17's reproduction recipe real.
- **AC19.** The new corpus is preserved outside the repo at
  `~/measurements/jaunder/issue-818-firefox-boot-phases/` with a README matching
  the #792 convention (runs, salts, conditions, store paths, known limits,
  consumers).
- **AC20.** If the disposition is "intrinsic", it is recorded so the question is
  not reopened — including what would have to change for re-investigation to be
  worthwhile.

## Non-goals

- **Reducing** the firefox gap. #818 asks where it lives; acting on it is #801's
  territory or a new issue, per AC15.
- The postgres axis. #792 established sqlite ≈ postgres across all six runs.
- `e2e.context_mint` (firefox ~5× chromium) — that is #819, and it sits outside
  `commit_to_mount` entirely; this cycle neither measures nor unblocks it.
- Long-task comparison. Gecko implements no `longtask` PerformanceObserver, so
  that lens is chromium-only and would silently read firefox as having none.
- Re-deriving #794's published per-phase numbers. They were chromium-only and
  frame-mixed; they are superseded, not corrected.

## Risks

- **The fix could perturb what it measures** (D7) — mitigated by AC16, against a
  real pre-change baseline rather than a guess.
- **A quiesced 70–80 minute window is required**, uninterrupted, since
  interleaving the settings sets is what makes them comparable. Two short
  windows will not substitute.
- **AC12 can halt the cycle.** That is deliberate — proceeding on a partly-dark
  corpus is the exact failure this issue exists to correct — but it means PR 2
  has a real chance of bouncing back to PR 1.
- **The answer may be boring.** "Distributed, intrinsic" is a live outcome. The
  issue explicitly accepts it, and an intrinsic finding with a real
  decomposition behind it is worth considerably more than #155's undecomposed
  assertion of the same conclusion.
