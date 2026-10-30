# #1791 corner-coverage probe — corrected design

## Recommendation (single candidate; read-only plan)

On a future, separately approved draft PR, add one ancillary probe to the
existing `home-timeline.png` assertion helper. Preserve the existing assertion
first and unchanged: its `stylePath`, `.j-post-time` mask, strict
`maxDiffPixels: 0` / `threshold: 0`, baseline, fixture and product page are not
touched. Preserve its original failure artifacts and error before any probe
operation. Probe once after either pass or failure, only for
`home-timeline.png`, project `chromium-visual`, retry 0, and verified producer
input lane `sqlite-chromium-unsplit`; unavailable/unknown producer input means
explicit skip, never fallback. This produces new controlled PNG evidence even
when the original passes; it does not fill a missing original pass PNG or repeat
CI.

The accepted pair differs at 11 raw ±1-level pixels in two right rounded-corner
clusters; only one is Playwright-classified. Post-failure Actions geometry is
fractional-y (569.265625), 72×32 with 6px radii, but not raster-time geometry. A
stable 100ms recapture failed, weakening a missing-font-wait premise. Test
whether the corner pattern is phase-sensitive and returns when fractional y is
restored—not whether a probe recapture equals the original assertion buffer.

## Exact future pseudodelta / lifecycle

```ts
// unchanged strict assertion has completed; save outcome/error and original artifacts
if (
  eligibleExactVisualTest &&
  verifiedProducerLane === "sqlite-chromium-unsplit" &&
  remainingMs() >= 850
) {
  preserveOriginalFailureArtifactsAndError();
  await boundedProbe(page); // ancillary; no assertion; no replacement error
}
throw originalMatcherError; // same object; or return original success
```

Within `boundedProbe`, read/hash exact `visual.css` bytes from `stylePath`;
apply those bytes with public `page.addStyleTag({content: css})` in the main
frame and keep it active through geometry reads/captures; pass the same CSS text
via locator screenshot option `style` (not `stylePath`). Inspect at most two
existing `button.j-post-action-trigger` locators with public
`boundingBox()`/`evaluate()`; select a bounding rect intersecting the viewport,
not a presumed `.first()` or `visible` predicate. Record selected index, rect,
scroll, computed radius/transform and time. Skip if none/ambiguous.

```ts
const originalStyle = await target.getAttribute("style");
try {
  for (const [phase, dy] of [
    ["zero-before", 0],
    ["integer-snap", Math.round(y) - y],
    ["zero-after", 0],
  ] as const) {
    await target.evaluate((el, offset) => {
      el.style.transform = `translateY(${offset}px)`;
    }, dy);
    const before = await geometryAndScroll(target, page); // CSS remains active
    const timeout = Math.min(150, remainingMs());
    if (timeout <= 0) {
      markPhaseUnavailable(phase);
      continue;
    } // timeout:0 means unlimited
    const png = await target.screenshot({
      type: "png",
      scale: "css",
      animations: "disabled",
      caret: "hide",
      style: css,
      timeout,
    });
    const after = await geometryAndScroll(target, page);
    await attachUniquePhasePngAndMetadata(phase, png, before, after); // only within remaining allowance
  }
} finally {
  // Each cleanup is independent; reserve allowance within the existing 750ms best-effort budget.
  try {
    await restoreOriginalInlineStyle(target, originalStyle);
  } catch (error) {
    markEvidenceInvalid(error);
  }
  try {
    await restoreOriginalScroll(page, initialScroll);
  } catch (error) {
    markEvidenceInvalid(error);
  }
  try {
    await removeInjectedStyleTag();
  } catch (error) {
    markEvidenceInvalid(error);
  }
}
```

Capture exactly these three phases; identical transform property/paint path each
phase. Verify and record the original computed transform; if it is not `none`,
skip as unsupported. Even `translateY(0px)` changes `none` to an explicit
transform and may create a new compositing/paint path, so the three captures
couple geometry and paint mode rather than isolating geometry. Record
rect/scroll immediately before and after each screenshot, PNG dimensions,
SHA-256, CSS hash, browser/version, DPR, viewport, font status, phase timestamps
and outcome; bound to two-node geometry records, ≤256KiB/image, ≤32KiB JSON,
strings ≤256 chars. Locator screenshots wait for actionability, scroll into view
and clip to element; verify geometry/scroll return to control after final zero
before classifying reversible. Rect brackets are not raster-time telemetry.

Admit only with ≥750ms probe budget plus 100ms original-result reserve; 750ms is
best-effort observation budget, not hard cancellation. Check remaining budget
before each API/attachment; screenshot timeout is positive and ≤150ms (if
computed timeout is ≤0, skip capture explicitly as unavailable; never pass `0`,
which means unlimited). Do not race uncancelled RPCs or claim they were
cancelled. In protected `finally`, reserve cleanup allowance within the existing
750ms budget and independently try inline-style restoration, original-scroll
restoration, and stylesheet removal so one failure cannot prevent the others.
Any cleanup failure marks evidence INVALID. Incomplete phases are unavailable;
restoration/geometry-return failure marks probe INVALID. Warn/report ancillary
errors only; never manufacture test failure or replace original matcher
error/result or original success. Existing identity 100ms and timeline
500ms/100ms reserve remain unchanged and separate.

## Reading results and route

A corner change at integer snap with restoration of the original zero-phase
pattern supports a geometry/coverage association, not root cause. Difference
between initial and restored zero, or failure to restore rect/scroll, points to
paint/capture/clipping instability or invalid probe; it does not prove font
cause. Stable images at all phases leave original cause unresolved. Compare raw
ROI pixels/colors only; no internal comparator invocation or tolerance change.
Transform, compositor, locator actionability/scroll/clip and duplicated CSS are
coupled confounds: this is not a pure geometry experiment. Main-frame style-tag
coverage is limited; screenshot `style` pierces frames/shadow while original
`stylePath` does likewise, so the target ordinary main-document Actions control
is the only intended scope. Identical CSS lifetime reduces but does not remove
insertion/duplication cascade confounding.

Use one normal draft PR on preserved current base
`f0bd4c31883b3e6860db9659e662a9ca68502553` / tree
`06cbc2a8b1e5f64f859a5a976ab94045ead2b25c` plus existing dirty patch SHA
`7bf83d9c79c0449b56df0be49f0cc30082c592a8e50bbfb469b95d31dde74478`; new
diagnostic delta gives a new head. Do not update base silently or confuse queue
source `0e927d3`. After explicit approval for implementation,
hooks/commit/push/PR creation and full-workflow resource cost, propose one
initial ordinary `pull_request` workflow only: four validation producers, eight
browser producers (two Chromium + six split Firefox), two Firefox reconciliation
jobs, two aggregates. SQLite guard means at most three probe PNGs per workflow
attempt, not all eight lanes. Cost is unknown. A 3600s observation cap is not
the CI workflow timeout or a settlement guarantee; unfinished stays
unknown/owned, no duplicate producer or autorerun. No PR1847 update/merge or
expired-campaign retry authority.

## Review findings disposition

1. **Selection, reversibility, screenshot scrolling:** fixed by bounded
   two-trigger viewport-rect selection; exact zero→integer snap→zero phases;
   per-capture rect/scroll brackets and return-to-control validity gate. Locator
   actionability/scroll/clip remains an explicit confound.
2. **Unsupported timing claim:** replaced hard-wall claim with 750ms best-effort
   budget, ≥850ms admission including reserve, per-screenshot ≤150ms timeout,
   sizes/count bounds, admission checks and explicit non-cancellation
   limitation.
3. **CSS and original-evidence order:** original strict assertion/artifacts
   first; exact CSS text/hash held through geometry reads and passed through
   public `style`; main-frame addStyleTag versus screenshot stylePath/style
   scope and duplication confound documented. Original error object is rethrown
   unchanged.
4. **Multiplicity — resolved:** exact visual/project/retry + verified
   `producerInput().lane` guard; `end2end/e2e-lanes.json` identity confirms
   SQLite unsplit Chromium; max three PNGs despite two Chromium backends.
   Unknown lane skips explicitly.
5. **Cleanup independence — final mechanical correction done:** inline-style
   restore, scroll restore, and stylesheet removal each have independent
   protected attempts; cleanup allowance stays within the existing 750ms
   best-effort budget; any cleanup failure invalidates evidence without
   replacing the original matcher error or success.
6. **Screenshot timeout — final mechanical correction done:** computed timeout
   must be positive; non-positive timeout skips capture as explicitly
   unavailable and is never passed as `0` (unlimited in the API).

### Evidence/source references

Accepted raster review: `result:new-failure-raster#2` (11 raw pixels; one
classified; stable recapture and capture-time caveats). Current helper:
`end2end/tests/visual-diagnostics.ts` (`producerInput`,
`runTimelineObservation`, `runVisualAssertion`, existing identity/timing).
Assertion: `end2end/tests/visual.ts` (`stylePath`, unchanged strict matcher);
CSS: `end2end/tests/visual.css`. Target: `end2end/tests/theme.spec.ts:100`.
Producer lane: `end2end/e2e-lanes.json:13` (SQLite Chromium unsplit). Public API
references: `end2end/node_modules/playwright-core/types/types.d.ts`
(`Locator.screenshot` actionability/scroll/clip, `style`, timeout);
`end2end/node_modules/playwright/types/test.d.ts` (`TestInfo.attach`). Workflow
topology: `.github/workflows/ci.yml` (matrix/reconciliation/gate);
`CONTRIBUTING.md` Visual snapshot workflow. No API supports observing the
original internal assertion buffers.

## Approved checkpoint-resilience supplement

Human decision `1791-probe-checkpoints`, request
`99b5339f-ae3c-466b-a92d-021a2678407c`: “Approve correction and one CI run”. The
Manager's exact grant:

> Authorized narrow correction: save compact structured checkpoint after each
> completed image/phase before next capture, retain final post-cleanup report,
> disclose precise ancillary attachment errors; test partial progress surviving
> later metadata failure while original matcher outcome/error unchanged. Keep
> original assertion/baseline/mask/strict thresholds and existing diagnostic
> time limits; no product change or framework. Normal local
> validation/review/hooks/commit/push to diagnostic draftPR1852 plus ONE
> additional full existing CI workflow, no automatic repeats or merge. Earlier
> resource hold released due1754 prerequisite STOP; coordinate if renewed timing
> window later admitted. Bind reviewed final head/tree and actual producer
> source to results.

Checkpoints record retained PNG identity/hash/dimensions, script-phase identity,
minimal geometry context, completion time and elapsed/remaining allowance. They
are progress evidence, not raster-time geometry or cleanup success. Keep at most
three compact checkpoints (at most 1KiB each); aggregate emitted probe JSON,
including original-evidence/checkpoint/final bodies, stays at most 32KiB.
Checkpoint work and cleanup/final-report admission remain inside the existing
750ms allowance, with the separate 100ms original-result reserve unchanged. Stop
further captures when admission cannot retain cleanup/checkpoint allowance.
Warnings distinguish budget, serialization/size and attachment rejection using
bounded stage/name/type/message and elapsed/remaining facts; a broken sink is
not claimed reliable. Retain the final independent-cleanup report attempt.

Use the existing helper/sink/producer testing seam to prove an accepted first
PNG/checkpoint survives later final-metadata rejection or budget exhaustion,
without changing original PASS or the identical matcher-error object. Cover
precise checkpoint errors, admission, aggregate JSON and positive capture
limits. The three zero/snap/zero transforms and locator screenshot API stay
unchanged; none/zero/none or page-screenshot experiments are not approved here.
Historical final-attachment cause remains unknown; a partial raster match is not
cause proof.

## Approved two-second allowance supplement

Human decision `1791-probe-two-seconds`, request
`cf9fbbee-9415-4b52-bdc2-c458de2db326`: “Approve allowance change and one run”.
This supersedes only the corner probe's 750ms time allowance above with 2000ms,
adding at most 1250ms of best-effort observation allowance, not a hard timing or
RPC-cancellation guarantee. No whole-test timeout extension or scaling change.

Admission requires at least 2100ms remaining test time: 2000ms probe allowance
plus the unchanged 100ms original-result reserve. All probe work remains capped
by remaining test time minus that reserve. Keep 50ms final-report time allowance
**inside** the 2000ms: capture work retains checkpoint25 + cleanup100 + final50;
checkpoint admission retains cleanup100 + final50; cleanup admission retains
final50. Final reporting uses remaining allotted time. Unavailable/INVALID
ancillary status never replaces original PASS or its identical matcher error.

Retain the same positive screenshot cap of 150ms, three zero/snap/zero locator
captures, exact guard, independent cleanup, identity100/timeline500, original
strict assertion/mask/baseline/thresholds and product/fixtures. The 32KiB
aggregate JSON, 1KiB checkpoints and 2KiB final **byte** reserve remain
unchanged and are distinct from the new 50ms final **time** reserve. No extra
capture, API, framework, retry, timer race, automatic workflow repeat, merge or
cleanup.

Use the existing helper seam and controlled clock to test 2100ms admission, the
observed 530/647/751ms progression, the absolute 2000ms envelope (not 2000ms
plus reserves), reserved final-report admission and original-result protection.
Run the complete helper regression and pinned type/format/diff checks. Actual
renderer timing remains unproven until the ONE future approved existing full
workflow, after independent review and Lead head/tree acceptance; the previous
two workflows are consumed. Preserve their evidence and distinct producer source
identities. This changes allowance only, not the diagnostic hypothesis or scope.

## Approved transform-presence supplement

Human decision `1791-transform-presence`, request
`9c1c0ae3-e131-4542-abdf-2c18333fbd3a`: “Approve transform test and one run”.
This **supersedes the phase contract only**: retain the existing guard, original
strict assertion first and identical PASS/error, locator screenshot API, CSS,
mask, scroll, crop, two-control viewport selection, independent cleanup and all
2000ms/150ms/three-image/byte limits above.

Use exactly `original-none`, `identity-zero`, `restored-none`. Both NONE arms
restore the exact saved original inline style (remove the style attribute when
null, otherwise set its original string); only the identity arm assigns
`el.style.transform = 'translateY(0px)'`. Require original computed transform
`none`, then verify computed `none → identity matrix → none`. There is no
intentional Y movement and no compositor-feature claim.

Replace endpoint-only validity with equality across all six before/after
observations: rect x/y/width/height/top/right/bottom/left, scroll x/y, viewport
width/height, DPR and enclosing clips derived from the recorded rect plus scroll
using pinned Playwright floor/ceil semantics with epsilon 0.001. Any difference,
unexpected computed transform or PNG dimensions inconsistent with recorded clips
makes interpretation INVALID. Brackets are not raster-time telemetry;
checkpoints remain progress evidence, not cleanup success. Preserve truthful
independent final cleanup reporting and valid compact JSON within existing caps.

Use the existing helper seam for faithful computed-transform mocks, exact
null/string style restoration, all-six invariance (including midphase drift that
returns at endpoints), viewport/DPR/clip drift, PNG dimensions and unchanged
original outcomes/cleanup/admission bounds. Run the complete helper regression
and pinned TypeScript/Prettier/diff checks. Real-renderer feasibility remains
unvalidated until ONE future existing full CI workflow after independent review
and Lead head/tree acceptance; no automatic repeat or scope/base expansion.

Raw registered PNG comparison is future CI analysis only, not a promise of an
original PASS buffer. Expected NONE arms surrounding the historical failed
pattern support reversible transform-associated paint within this probe, not
historical cause proof. Three failed-pattern arms show identity unnecessary
within this probe; three expected arms are inconclusive. No product fix,
tolerance change, issue closure, merge or cleanup is authorized.
