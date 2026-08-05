# #818 — Firefox boot-phase gap: implementation plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
`docs/superpowers/specs/2026-08-05-issue-818-firefox-boot-phase-gap.md` — the
"what/why". This plan is the "how" and does not restate it.

**Goal:** Make the #794 boot decomposition work on every engine, then use it to
say which phase firefox loses `commit_to_mount` in.

**Architecture:** Harvest a document's `performance` marks when `data-mounted`
is observed rather than only at `load` — `csr` emits all four marks
synchronously before setting that attribute, so mount-ready is complete by
construction. Merge harvests by mark count so completeness never depends on
which `page.evaluate` resolves first. Then collect a fresh corpus on a quiesced
host and decompose a document-frame boot total whose segments close exactly.

**Tech Stack:** TypeScript (Playwright harness, `end2end/`), Rust (`xtask` trace
analyzer), Nix (e2e VM checks).

## Review header

**Scope — in:** the harvest fix and its merge rule; boot-coverage and boot-phase
reporting in `cargo xtask traces`; an unthresholded in-suite regression
assertion; doc supersession + an ADR for the frame rule; the measurement
session; the attribution write-up; corpus preservation.

**Scope — out:** reducing the gap (#801 or a new issue); the postgres axis;
`e2e.context_mint` (#819); long-task comparison; a _thresholded_ fail-closed
coverage gate (Task 1 files it).

**Tasks:**

1. File the follow-up issue for the fail-closed coverage gate (spec D4/AC9).
2. `mergeDocumentTiming` — pure merge rule + unit tests (AC2).
3. Wire the mount-ready harvest through the existing mount binding (AC1).
4. `bootTiming` fixture + in-suite regression spec (AC7).
5. Boot-coverage section in `cargo xtask traces analyze` (AC6).
6. Supersede the docs; ADR for the measurement-frame rule (AC8).
7. Verify coverage on both engines; `validate`; open PR 1 (AC3–AC5, AC10).
8. `cargo xtask traces boot-phases` — medians and signed shares (AC18).
9. Collect the fresh corpus; certify it (AC11, AC12).
10. Decompose, apply the pre-registered verdict rules, write up (AC13–AC17,
    AC20).
11. Preserve the corpus; open PR 2 (AC19).

**Key risks / decisions:**

- **Tasks 1–7 are PR 1; tasks 8–11 are PR 2** (spec D10). PR 2 does not start
  until PR 1 has merged and CI's full matrix has exercised it.
- **Task 9 can bounce the cycle back to PR 1.** AC12's certification is a hard
  gate; a shortfall means the fix is incomplete, not that the floor is wrong.
- **Task 9 needs an uninterrupted quiesced ~75 minute window.** It is the only
  task that cannot proceed while other work is on the box. It drives `nix build`
  per combo rather than `cargo xtask traces run`, because `traces run` **always
  builds both backends** (`traces/run.rs:21-22`) — using it would collect the
  postgres axis the spec lists as a non-goal and double the window to ~150
  minutes.
- **Backends are separated by trace-file, never by project label.**
  `projectName` is the browser and names no backend (`traces/run.rs:99-101`), so
  any per-project aggregation silently pools sqlite with postgres. Tasks 5 and 8
  key on `(source, project)`.
- **Task 6's ADR is my judgment call, not a spec requirement** — the spec
  flagged it as open. Drop the ADR step if you'd rather it live only in
  `observability.md`.

## Global Constraints

- No `Co-Authored-By` trailer on any commit.
- Call binaries the devShell provides, under `devtool run --` — never `npx`,
  `npm run`, `nix develop -c`.
- Pin the worktree on every gate run:
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- <cmd>`.
- Per-commit gate is `cargo xtask check` (see `jaunder-commit`); it auto-fixes
  formatting, so run it before staging.
- Every boot mark name lives only in Rust and is discovered by the `jaunder.`
  prefix (`client/src/perf/mod.rs`). Never enumerate mark names in TypeScript,
  and never assume the count is exactly four — `client::perf` may gain marks,
  and an equality assertion would turn that into a build failure or, worse, a
  silent coverage blackout.
- Document-relative and Node-side (`Date.now()`) values are never mixed in one
  arithmetic expression (spec D8).
- **`xtask` is its own workspace** (`Cargo.toml:14`, `exclude = ["xtask"]`), so
  `cargo nextest run -p xtask …` fails with "package ID specification 'xtask'
  did not match any packages". Use
  `cargo nextest run --manifest-path xtask/Cargo.toml …`.

---

### Task 1: File the fail-closed coverage-gate follow-up

Separable concern — capture it up front so it can be picked up concurrently.

**Files:** none (tracker only).

**Interfaces:**

- Produces: an issue number, referenced from the spec's D4 and from
  `observability.md` in Task 6.

- [x] **Step 1: File the issue** via `jaunder-issues` (GitHub MCP preferred). →
      **[#831](https://github.com/jaunder-org/jaunder/issues/831)**, type
      `Task`, labels `test-infra`/`observability`, milestone "Test
      infrastructure & E2E", added to Jaunder Backlog (#1).

Title: `e2e: gate boot-decomposition coverage per combo, fail-closed`

Body must state: that #818 found firefox recording 0 boot marks on 210/210
navigations across six preserved runs with nothing positioned to notice; that
#818 adds per-project coverage _reporting_ (`traces analyze`) and an
unthresholded in-suite assertion, but no per-combo gate; that the gate needs a
threshold derived from the post-#818 distribution, which is why it was not
guessed at in #818; and that `server-fn-coverage verify` is the structural
precedent (regenerate/verify against the lifted capture, fail closed on
missing/empty/unparseable input).

Labels: `test-infra`, `observability`. Milestone: `Test infrastructure & E2E`.

- [x] **Step 2: Record the number** in this plan's Task 6, Step 2 — replace
      `#TBD-GATE` there.

---

### Task 2: `mergeDocumentTiming` — the merge rule

Spec D2/AC2. A pure function, so the invariant is provable without a browser.

**Files:**

- Modify: `end2end/tests/capture-trace.ts` (add export after `DocumentTiming`,
  ~line 106)
- Test: `end2end/tests/boot-marks.spec.ts` (create)

**Interfaces:**

- Consumes: `DocumentTiming` (`capture-trace.ts:102-106`), `BootMark`
  (`capture-trace.ts:93`), `WasmTiming` (`capture-trace.ts:96-100`).
- Produces:
  `export function mergeDocumentTiming(existing: DocumentTiming | undefined, incoming: DocumentTiming): DocumentTiming`
  — used by Task 3.

- [x] **Step 1: Write the failing tests**

Create `end2end/tests/boot-marks.spec.ts`.

**Import `test` from `./fixtures`, not `@playwright/test`** — even though these
cases are pure. The `traced-context` gate
(`xtask/src/steps/traced_context_check.rs`) scans **every** `.ts` under
`end2end/tests` (only `fixtures.ts` is exempt) and rejects the upstream import,
because a spec opening no `e2e.test` span makes everything it drives
unattributable — silently. The blanket rule is the point. Cost: the harness
`test` carries `_autoPerfSpan`, which depends on `page`, so these launch a
browser they don't use. The assertions stay pure, so the invariant is still
proven by their logic rather than by browser behavior.

```ts
import { test, expect } from "./fixtures";
import { mergeDocumentTiming, type DocumentTiming } from "./capture-trace";

const wasm = { startTime: 10, durationMs: 5, responseEndMs: 15 };
const marks = (count: number) =>
  Array.from({ length: count }, (_, index) => ({
    name: `jaunder.boot.m${index}`,
    startTime: index,
  }));

// `toBe` (identity), not `toEqual`: the merge PICKS a snapshot, it never builds a
// blended one. Copying would be a silent behavior change, so identity is the
// contract.
test.describe("mergeDocumentTiming", () => {
  test("takes the incoming snapshot when there is no existing one", () => {
    const incoming: DocumentTiming = { marks: marks(4), wasm };
    expect(mergeDocumentTiming(undefined, incoming)).toBe(incoming);
  });

  test("prefers the snapshot with more marks when it arrives second", () => {
    // The firefox ordering: `load` harvests an empty document first, mount-ready
    // harvests the full one after.
    const existing: DocumentTiming = { marks: [], wasm: null };
    const incoming: DocumentTiming = { marks: marks(4), wasm };
    expect(mergeDocumentTiming(existing, incoming)).toBe(incoming);
  });

  test("prefers the snapshot with more marks when it arrived first", () => {
    // The clobber this rule exists to prevent: a late-resolving `load` harvest
    // must not overwrite a complete mount-ready one.
    const existing: DocumentTiming = { marks: marks(4), wasm };
    const incoming: DocumentTiming = { marks: [], wasm: null };
    expect(mergeDocumentTiming(existing, incoming)).toBe(existing);
  });

  test("breaks a mark-count tie toward the incoming snapshot's wasm timing", () => {
    const existing: DocumentTiming = { marks: marks(4), wasm: null };
    const incoming: DocumentTiming = { marks: marks(4), wasm };
    expect(mergeDocumentTiming(existing, incoming)).toBe(incoming);
  });

  test("keeps the existing snapshot's wasm timing on a tie", () => {
    const existing: DocumentTiming = { marks: marks(4), wasm };
    const incoming: DocumentTiming = { marks: marks(4), wasm: null };
    expect(mergeDocumentTiming(existing, incoming)).toBe(existing);
  });

  test("keeps the existing snapshot when a tie gives neither side more", () => {
    const existing: DocumentTiming = { marks: marks(4), wasm };
    const incoming: DocumentTiming = { marks: marks(4), wasm };
    expect(mergeDocumentTiming(existing, incoming)).toBe(existing);
  });
});
```

- [x] **Step 2: Run the tests, verify they fail**

`end2end/node_modules` must exist; `devtool check tsc` provisions it, so run the
gate once first if this is a fresh worktree.

**`--project=chromium`, with the `=`.** `--project` takes multiple values, so
`--project chromium boot-marks` parses the filter as a second project name and
errors with "Project(s) "boot-marks" not found".

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- \
  playwright test --config end2end/playwright.config.ts --project=chromium boot-marks
```

Expected: FAIL — `mergeDocumentTiming` is not exported from `./capture-trace`.
**Observed:** 6 failed, all `mergeDocumentTiming is not a function`.

- [x] **Step 3: Implement against the tests**

Add to `capture-trace.ts`, immediately after the `DocumentTiming` type.
Signature as in **Interfaces** above. Every branch is pinned by a Step 1 test —
no existing, incoming-larger, existing-larger, tie-with-incoming-wasm,
tie-with-existing-wasm, tie-with-both — so the tests determine the body.

The doc comment must carry the _reason_, since the naive alternative looks
correct:

```ts
/**
 * Pick the more complete of two harvests of the same document.
 *
 * Marks persist for a document's lifetime, so a later harvest is a superset of an
 * earlier one — but `documentTimings.set` is last-*resolution*-wins, which coincides
 * with issue order only because two `page.evaluate`s on one page serialize over a
 * single connection. That is undocumented transport behavior, and firefox is exactly
 * the case that depends on it: `load` fires BEFORE mount there, so its empty snapshot
 * would win under any rule that trusts arrival order. Comparing mark counts makes the
 * invariant local (#818).
 */
```

- [x] **Step 4: Run the tests, verify they pass**

Run: the Step 2 command. Expected: PASS (6 passed). **Observed:** 6 passed.

- [x] **Step 5: Commit**

Pass the message via `-F <file>`, never `-m` with backticks — bash
command-substitutes them, and the words vanish from the message silently (caught
here after the fact: two `` `load` `` mentions were eaten).

```bash
git add end2end/tests/capture-trace.ts end2end/tests/boot-marks.spec.ts
git commit -F /tmp/msg.txt
```

Run `cargo xtask check` first (**jaunder-commit**).

---

### Task 3: Harvest at mount-ready

Spec AC1. The bug fix proper.

**Files:**

- Modify: `end2end/tests/capture-trace.ts` — `harvestDocument` (`:171-201`,
  merge instead of overwrite), the `__jaunderRecordMount` binding (`:222-243`),
  `settle()` (`:432-434`), and the doc comments at `:102`, `:114-121`,
  `:161-170`

**Interfaces:**

- Consumes: `mergeDocumentTiming` (Task 2); Playwright's `BindingSource`
  (`{ context, page, frame }`) — the binding currently discards it as `_source`.
- Produces: no new exports. `timingFor(navigationId)` gains full coverage on
  mounted navigations, which Tasks 4, 5 and 8 rely on.

- [x] **Step 1: Change `harvestDocument` to merge**

At `capture-trace.ts:196`, replace the overwrite:

```ts
documentTimings.set(
  navigationId,
  mergeDocumentTiming(documentTimings.get(navigationId), timing),
);
```

- [x] **Step 2: Harvest from the mount binding**

In the `exposeBinding("__jaunderRecordMount", ...)` callback, take the source
rather than discarding it, and harvest the navigation the mount was just
attributed to — inside the match loop, where `navigation.id` is in hand:

```ts
navigation.mountedMs = nowMs;
// Harvest HERE, not at `load`: `csr` emits every `jaunder.*` mark synchronously
// before setting `data-mounted`, so this instant is complete by construction on
// any engine. `load` is not — it frequently never fires (goto waits only for
// domcontentloaded), and on firefox it lands before boot reaches `boot.entry` (#818).
pendingHarvests.push(harvestDocument(source.page, navigation.id));
return;
```

The callback's first parameter changes from `_source` to
`source: { page: Page }` (Playwright's `BindingSource`; only `page` is used).
Calling `page.evaluate` from a binding callback is safe — the callback is not
`async` here, so Playwright resolves the binding immediately and the evaluate
proceeds on the duplex connection without re-entering the page's JS thread.

- [x] **Step 3: Make `settle()` drain until stable**

`Promise.all(pendingHarvests)` snapshots the array synchronously, so anything
pushed _during_ the await is never awaited. Until now every harvest came from a
`load` handler that fired before teardown; a mount-ready harvest arrives via
async binding dispatch and can land after `settle()` has begun. Loop until the
array stops growing:

```ts
async settle() {
  let drained = 0;
  while (drained < pendingHarvests.length) {
    const batch = pendingHarvests.slice(drained);
    drained = pendingHarvests.length;
    await Promise.all(batch);
  }
},
```

Without this, coverage is intermittently short — which looks like a partial fix
and would survive a single-run check.

- [x] **Step 4: Correct the doc comments that name `load`**

`:102` (`DocumentTiming` — "Everything harvested from a single document, at its
`load`"), `:114-121` (`timingFor`), and `:161-170` (`harvestDocument`'s own
header). `timingFor`'s comment also frames the goal as decomposing
`commit_to_mount`; per spec D8 that is a frame error, so restate the target as
the document-relative boot total and note that `commit_to_mount` is Node-side.

- [x] **Step 5: Verify the harness typechecks**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- \
  tsc --noEmit -p end2end/tsconfig.json
```

Expected: PASS, no diagnostics. (Behavioral verification is Task 7 — it needs a
full e2e run.)

- [x] **Step 6: Commit**

```bash
git add end2end/tests/capture-trace.ts
git commit -m "fix(e2e): harvest boot marks at mount-ready, not at load (#818)"
```

---

### Task 4: In-suite regression assertion

Spec D4/AC7. Unthresholded, so it needs no distributional knowledge and reddens
in every combo of the gate.

**Files:**

- Modify: `end2end/tests/fixtures.ts` — a module-level capture registry beside
  `tracedContextRecords` (`:69-71`); registration inside `_autoPerfSpan`
  (`:510+`); the fixture type map (`:328`); the fixture body
- Modify: `end2end/tests/boot-marks.spec.ts` (append the browser test)

**Interfaces:**

- Consumes: `TraceCapture` (`capture-trace.ts:108-134`), `testSpanId` fixture
  (`fixtures.ts:365`).
- Produces: fixture `bootTiming: () => Promise<DocumentTiming | undefined>` —
  the harvested timing for the most recent navigation that reached mount-ready.

**Design note — why a registry, not a `capture` fixture.** `capture` is a local
`const` inside `_autoPerfSpan`'s body (`fixtures.ts:522`), not a fixture, and it
must stay that way: `fixtures.ts:352-357` warns that fixture registration order
is load-bearing — `_lifecycleStart` must precede `_autoPerfSpan` or
`e2e.context_mint` "silently collapses to zero width," and hoisting the capture
into its own fixture would move its setup into exactly the interval
`context_mint` measures. Instead follow the existing `tracedContextRecords`
pattern (`:69-71`): a module-level `Map` keyed by test span id, written by
`_autoPerfSpan`, read at call time. Nothing is reordered.

- [x] **Step 1: Write the failing test**

Append to `end2end/tests/boot-marks.spec.ts`:

```ts
import { test as harnessTest } from "./fixtures";
import { waitForMount } from "./mount";

harnessTest(
  "the harness captures the full boot mark set after mount",
  async ({ page, bootTiming }) => {
    await page.goto("/");
    await waitForMount(page);

    const timing = await bootTiming();
    harnessTest
      .expect(
        timing,
        "no document timing was harvested for the mounted navigation",
      )
      .toBeDefined();

    // Assert the SHAPE, never the names: mark names live only in Rust and are
    // discovered by prefix, so enumerating them here would reintroduce exactly the
    // cross-language drift `MOUNTED_ATTR` suffers (#794).
    const names = (timing?.marks ?? []).map((mark) => mark.name);
    harnessTest.expect(names.length).toBeGreaterThanOrEqual(4);
    harnessTest.expect(names.every((n) => n.startsWith("jaunder."))).toBe(true);
    harnessTest.expect(new Set(names).size).toBe(names.length);
  },
);
```

`toBeGreaterThanOrEqual(4)`, never `toBe(4)` — see Global Constraints.

- [x] **Step 2: Run it, verify it fails**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- \
  cargo xtask e2e-local boot-marks.spec.ts
```

Expected: FAIL — `bootTiming` is not a declared fixture.

- [x] **Step 3: Implement the registry and fixture**

1. Beside `tracedContextRecords`, add
   `const captureByTestSpanId = new Map<string, TraceCapture>();`
2. Inside `_autoPerfSpan`, immediately after `capture` is created, register it;
   delete the entry in the same teardown that drains `tracedContextRecords`, so
   it cannot leak across tests.
3. Declare `bootTiming: () => Promise<DocumentTiming | undefined>` in the
   fixture type map and implement it depending on `{ testSpanId }`.

Behavior, fully pinned by Step 1: look up the capture; `await capture.settle()`;
return the timing for the highest-id navigation whose `mountedMs` is non-null,
or `undefined` when there is none. It must `settle()` before reading — the
consumer-side half of Task 3 Step 3.

- [x] **Step 4: Run it, verify it passes**

Run: the Step 2 command. Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add end2end/tests/fixtures.ts end2end/tests/boot-marks.spec.ts
git commit -m "test(e2e): assert the harness captures the boot mark set after mount (#818)"
```

---

### Task 5: Boot-coverage reporting in `traces analyze`

Spec D3/AC6. Load-bearing for Task 9's certification.

**Files:**

- Modify: `xtask/src/traces/analyze.rs` (row struct beside `ByProjectRow`
  `:177`; section fn beside `navigation_sections` `:300`; field on `Analysis`
  `:97`; wire in `analyze_spans_inner` `:576`)
- Modify: `xtask/src/traces/render.rs` (`*Display` + `From` impl; `section(...)`
  call in `render` `:253`)
- Test: in-file `mod tests` in both (crate convention)

**Interfaces:**

- Consumes: `Span` (`parse.rs:26`, including its `source` field),
  `parse_json_attr` (`parse.rs:155`), `get_attr` (`parse.rs:107`),
  `project_label` (`analyze.rs:76`), `e2e_tests` (`analyze.rs:70`).
- Produces:

```rust
pub struct BootCoverageRow {
    /// The trace file this came from. **Load-bearing:** `projectName` is the browser
    /// and names no backend (`traces/run.rs:99-101`), so keying on `project` alone
    /// pools sqlite with postgres into one row.
    pub source: String,
    pub project: String,
    pub navigations: u64,
    pub mounted: u64,
    pub full_marks: u64,
    pub dropped: u64,
}

fn boot_coverage_rows(spans: &[Span]) -> Vec<BootCoverageRow>;
```

- [x] **Step 1: Write the failing tests**

In `analyze.rs`'s `mod tests`, over synthetic `e2e.test` spans carrying
`e2e.navigation_top_json` (follow the existing tests' span-construction helper):

```rust
#[test]
fn boot_coverage_counts_mounted_and_fully_marked_navigations_per_project() {
    // firefox: 2 navigations, both mounted, neither decomposed — the #818 blackout.
    // chromium: 2 navigations, 1 mounted and fully marked, 1 never mounted.
    // Asserts firefox {navigations:2, mounted:2, full_marks:0} and
    //         chromium {navigations:2, mounted:1, full_marks:1}.
}

#[test]
fn boot_coverage_separates_rows_by_source_file() {
    // Two spans, same project "firefox", different `source`. Asserts TWO rows —
    // sqlite and postgres must never be pooled (traces/run.rs:99-101).
}

#[test]
fn a_navigation_is_mounted_iff_commit_to_mount_is_present() {
    // `commitToMountMs` is non-null iff committedMs AND mountedMs are both set
    // (fixtures.ts:581-584), so it is the mounted proxy. A navigation with
    // bootPhases but no commitToMountMs counts as NOT mounted.
}

#[test]
fn full_marks_accepts_extra_boot_phases_but_not_missing_ones() {
    // >= 3 phases AND non-null wasmInstantiateMs. A FOURTH phase (a new mark in
    // client::perf) still counts as full; 2 phases does not. Equality here would
    // make adding a mark read as a total coverage blackout.
}

#[test]
fn boot_coverage_sums_navigation_top_dropped_so_truncation_is_never_silent() {
    // e2e.navigation_top_json is the top 20 BY DURATION per test — a biased sample,
    // not a census. Without the dropped count a truncated capture reads as complete.
}
```

- [x] **Step 2: Run them, verify they fail**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- \
  cargo nextest run --manifest-path xtask/Cargo.toml boot_coverage
```

Expected: FAIL — `boot_coverage_rows` / `BootCoverageRow` not defined.
**Observed:** exit 101, 7 compile errors, all "cannot find `BootCoverageRow`" /
"cannot find function `boot_coverage_rows`". No test passed before
implementation.

- [x] **Step 3: Implement**

**`bootPhases` is a JSON object, not an array** — `fixtures.ts` types it
`Record<string, number>`, keyed `"<from>-><to>"`. So "≥3 entries" is
`as_object().len() >= 3`.

`boot_coverage_rows` groups `e2e_tests(spans)` by `(source, project_label)`, and
per navigation in `e2e.navigation_top_json` counts: always `navigations`;
`mounted` when `commitToMountMs` is non-null; `full_marks` when `bootPhases` has
**≥3** entries **and** `wasmInstantiateMs` is non-null. `dropped` sums
`e2e.navigation_top_dropped`. Every branch is pinned by Step 1.

Document the mounted proxy's approximation in the fn's doc comment: a navigation
that mounted but whose `committedMs` never landed (the `state.pending.shift()`
path, `capture-trace.ts:386-395`) drops out of both numerator and denominator.
AC12's second floor (`mounted / navigations`) is what catches that class.

Add `pub boot_coverage: Vec<BootCoverageRow>` to `Analysis`, populate it in
`analyze_spans_inner`, render it as
`"Boot decomposition coverage (from e2e.navigation_top_json)"` with columns
`source | project | navigations | mounted | full marks | dropped`, following the
`ByProjectDisplay` pattern exactly.

- [x] **Step 4: Run them, verify they pass**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- \
  cargo nextest run --manifest-path xtask/Cargo.toml traces
```

Expected: PASS (including the existing render tests). **Observed:** 66 passed —
the five named tests plus
`render_shows_every_boot_coverage_row_with_its_source`, added to cover the
display path since the Files block calls for tests in both files.

- [x] **Step 5: Commit**

```bash
git add xtask/src/traces/analyze.rs xtask/src/traces/render.rs
git commit -m "feat(xtask): report boot-decomposition coverage per project (#818)"
```

---

### Task 6: Supersede the docs; record the frame rule

Spec AC8, plus the ADR flagged as my judgment call.

**Files:**

- Modify: `docs/observability.md` (§"What the boot marks do and do not cover")
- Create: `docs/adr/drafts/measurement-frames-are-not-mixed.md`
- Modify: `~/measurements/jaunder/issue-792-warmup-ab/README.md` ("Known
  limits")

- [x] **Step 1: Rewrite the observability section**

It currently describes the `load` harvest and its "73 navigations across 59 of
127 tests" figure as the steady state. **Supersede, don't delete** — the figure
was true of the run it described; the _mechanism_ is what's obsolete. The
rewrite must state: the harvest point is now mount-ready; why that is complete
by construction (marks precede `data-mounted` synchronously in `csr`); that the
`load` harvest is retained so unmounted navigations still yield wasm timing; and
that **the pre-#818 corpus contains no firefox decomposition at all** — 0 marks
on 210/210 navigations per combo, both arms.

- [x] **Step 2: Cross-reference the coverage gate**

Note that coverage is _reported_ (Task 5) but not yet gated, pointing at
**#831**.

- [x] **Step 3: Draft the ADR** →
      `docs/adr/drafts/measurement-frames-are-not-mixed.md`

`docs/adr/drafts/measurement-frames-are-not-mixed.md`, numberless (promoted at
ship by `cargo xtask adr promote`) — see **jaunder-adr**.

Decision: browser-side measurements are decomposed only in the document frame
(`performance.timeOrigin`); Node-side `Date.now()` values (`committedMs`,
`mountedMs`, and therefore `commitToMountMs`) are never used as the total for a
document-relative decomposition. Context: #794 shipped `timingFor`'s stated goal
as decomposing `commit_to_mount` while every part it harvests is
document-relative — the frames differ by CDP/juggler event-delivery latency plus
the mount→binding round trip, both cross-process and plausibly
engine-asymmetric, so decomposing across them charges harness overhead to app
boot phases. Consequence: the analysis target is `mount_done.startTime`, whose
segments close exactly; the frame skew is reported separately as a harness cost.

- [x] **Step 4: Update the preserved corpus README**

`~/measurements/jaunder/issue-792-warmup-ab/README.md` "Known limits" says boot
decomposition is "often `null`". That corpus is **not re-collectable**, so amend
it to record that firefox has _none at all_ and chromium ~34%, with the cause
and a pointer to #818. Outside the repo, so outside the commit below.

- [x] **Step 5: Format and commit**

**Check `git status` before every commit.** `observability.md` was already
partially staged when Task 5 was committed and rode along in it; both commits
were unpushed so they were re-split, but the lesson is to stage deliberately
rather than assume the index is empty.

**The ADR draft is NOT staged.** Everything under `docs/adr/drafts/` except its
`README.md` is gitignored, by design — a draft carries no number until
`cargo xtask adr promote` assigns one at ship, which is also what stages it
(`docs/adr/drafts/README.md`). Staging it here would either fail or commit a
premature number.

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- \
  prettier -w docs/observability.md docs/adr/drafts/measurement-frames-are-not-mixed.md
git add docs/observability.md
git commit -F /tmp/msg.txt
```

---

### Task 7: Verify coverage on both engines, then open PR 1

Spec AC3–AC5, AC10. **This task's numbers are coverage only** — valid on a
non-quiescent host, unlike anything in Tasks 9–10.

- [x] **Step 1: Run both sqlite combos** (Bash background mode — long/cold) Both
      exit 0. Note this also exercises `boot-marks.spec.ts` **inside** the
      firefox VM — it matches the `firefox` project's `testIgnore`-only filter —
      so the run is simultaneously the coverage measurement and the regression
      assertion's first firefox exercise.

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- \
  cargo xtask e2e sqlite firefox
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- \
  cargo xtask e2e sqlite chromium
```

- [x] **Step 2: Extract each capture and read the coverage section**

```bash
tar -xzf .xtask/diagnostics/e2e-sqlite-firefox/capture-sqlite.tar.gz capture/otel-traces.jsonl
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- \
  cargo xtask traces analyze capture/otel-traces.jsonl
```

Repeat for chromium, extracting to a distinct filename.

- [x] **Step 3: Check the acceptance condition**

**Observed (2026-08-05, `cargo xtask e2e sqlite {firefox,chromium}`):**

| project        | navigations | mounted | full marks | dropped |
| -------------- | ----------- | ------- | ---------- | ------- |
| firefox        | 199         | 196     | 196        | 0       |
| firefox-admin  | 12          | 12      | 12         | 0       |
| chromium       | 199         | 196     | 196        | 0       |
| chromium-admin | 12          | 12      | 12         | 0       |

`full marks == mounted` on both engines — AC3 and AC4 met. AC5's two
denominators, against the pre-fix baseline:

| engine   | full marks / mounted | full marks / all navigations | pre-fix (all navs) |
| -------- | -------------------- | ---------------------------- | ------------------ |
| firefox  | 208/208 = **100%**   | 208/211 = **98.6%**          | **0%** (0/210)     |
| chromium | 208/208 = **100%**   | 208/211 = **98.6%**          | ~34% (72/210)      |

`dropped = 0` everywhere, so the population is a census rather than a
duration-biased top-20 slice.

**Closure validated on real data** (load-independent, so valid despite a busy
host): all 208 fully-decomposed firefox navigations sum to
`mount_done.startTime` within 1 ms — **0 violations**. D8's decomposition closes
by construction, which is what makes AC13's residual check meaningful.

**Frame skew is real and bidirectional** — sampled at −137, +87, +119 ms on
1200–2000 ms boots. Decomposing `commitToMountMs` into document-relative parts,
as #794 intended, would have smeared that term across the app's boot phases.

**AC3/AC4:** `full marks == mounted` for both engines.

**AC5:** record BOTH denominators — `full_marks / mounted` and
`full_marks / navigations` — against the pre-fix baseline (~34% chromium, 0%
firefox, both on _all_ navigations). The denominators differ by design; quoting
one against the other's baseline would misstate the improvement.

If `full marks < mounted` on either engine, **stop and diagnose** — most likely
Task 3 Step 2's `pendingHarvests` registration or Step 3's drain loop.

- [x] **Step 4: Full gate** (background mode)

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- \
  cargo xtask validate
```

Expected: green (AC10).

- [x] **Step 5: Push and open PR 1**

Per **jaunder-ship**. The body states Step 3's figures on both denominators,
references #818, and says explicitly that it does **not** close the issue — PR 2
does.

- [x] **Step 6: Watch it home**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- \
  cargo xtask pr watch
```

**Do not merge without approval.** Once merged, confirm the CI matrix's firefox
job shows the coverage section — that independent-hardware check is the reason
for the two-PR split (spec D10).

---

### Task 8: `cargo xtask traces boot-phases`

Spec AC18. `traces analyze` computes maxima and averages, **not** medians or
percentiles; AC13 is entirely medians and signed shares.

**In xtask, not `scripts/*.mjs`.** Both existing trace commands are documented
as faithful Rust ports of retired shell scripts (`xtask/src/lib.rs:301,333`) —
the repo moved this exact class of tool into xtask deliberately. A committed
`.mjs` under `scripts/` would also sit outside every gate: prettier covers only
`end2end` and `**/*.md`, and tsc only `end2end/tsconfig.json`
(`tools/devtool/src/check.rs:53-61`).

**Files:**

- Create: `xtask/src/traces/boot_phases.rs`
- Modify: `xtask/src/traces/mod.rs` (declare), `xtask/src/lib.rs` (subcommand
  beside `traces analyze`/`traces run`)
- Test: in-file `mod tests`

**Interfaces:**

- Consumes: `Span`, `parse_json_attr`, `read_spans`, `project_label`.
- Produces: `cargo xtask traces boot-phases <files…>` → a table per
  `(source, project, cacheWarmth)`: n, median of each of spec D8's six segments,
  median `bootTotalMs`, median `commitToMountMs`, median frame skew.

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn segments_sum_to_the_boot_total_within_a_millisecond() {
    // The closure property spec D8 relies on:
    // wasmFetchStartMs + wasmFetchMs + wasmInstantiateMs + the three boot intervals
    // == mount_done.startTime.
}

#[test]
fn boot_phase_rows_split_cold_from_warm() {
    // Spec D6: never pooled. Two navigations, one cold one warm, → two rows.
}

#[test]
fn boot_phase_rows_split_by_source_so_backends_never_pool() {
    // Same reason as Task 5: projectName names no backend.
}

#[test]
fn a_navigation_failing_closure_is_reported_not_silently_included() {
    // Segments that miss by > 1ms must surface as a counted violation.
}

#[test]
fn a_population_with_no_decomposed_navigations_reports_that_explicitly() {
    // The #818 failure mode: firefox pre-fix. Must NOT render an empty table or
    // divide by zero — it must say so.
}

#[test]
fn medians_are_the_lower_of_the_two_middle_values_on_even_counts() {
    // Pin the convention; `traces analyze` has none to inherit.
}
```

- [x] **Step 2: Run them, verify they fail**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- \
  cargo nextest run --manifest-path xtask/Cargo.toml boot_phase
```

Expected: FAIL — module not defined.

- [x] **Step 3: Implement**

Segments per spec D8's table, all document-relative, in `startTime` order;
select the three intervals by `boot.` prefix rather than by position, so a new
mark in `client::perf` extends the table instead of breaking closure.
`commitToMountMs` and the skew are **reported, never decomposed** (spec D8).

- [x] **Step 4: Self-check against the #792 corpus**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- \
  cargo xtask traces boot-phases \
  ~/measurements/jaunder/issue-792-warmup-ab/traces/b1-sqlite-chromium.jsonl.gz \
  ~/measurements/jaunder/issue-792-warmup-ab/traces/b1-sqlite-firefox.jsonl.gz
```

Expected: a chromium table over ~72 fully-marked navigations with **zero**
closure violations, and an explicit "no decomposed navigations" line for
firefox. (If `read_spans` cannot read `.gz`, gunzip to a temp file first — do
not add decompression to this task.)

- [x] **Step 5: Commit**

```bash
git add xtask/src/traces/boot_phases.rs xtask/src/traces/mod.rs xtask/src/lib.rs
git commit -m "feat(xtask): median boot-phase decomposition report (#818)"
```

---

### Task 9: Collect and certify the fresh corpus

Spec AC11, AC12. **Requires an uninterrupted quiesced ~75 minute window.**

**Why not `cargo xtask traces run`.** It always builds both backends
(`traces/run.rs:21-22`) with no backend filter, which would collect the postgres
axis the spec lists as a non-goal and double the window. Drive the two sqlite
combos directly instead — the same shape as #792's own documented reproduction
recipe.

- [x] **Step 1: Confirm quiescence with the user** before starting; record the
      baseline (`cat /proc/loadavg`).

- [x] **Step 2: Collect six runs**, interleaving the settings sets run-by-run
      (single-worker, gate, single-worker, gate, …), each with a distinct
      `e2eSalt` in `flake.nix` — without it nix replays a cached derivation and
      runs 2–3 are byte-identical to run 1.

Per run, per browser (sqlite only):

```bash
# deciding set (single-worker packages)
nix build --print-out-paths --no-link \
  .#packages.x86_64-linux.e2e-sqlite-firefox-single-worker
# confirming set (gate checks)
nix build --print-out-paths --no-link \
  .#checks.x86_64-linux.e2e-sqlite-firefox
```

then lift each capture:

```bash
tar -xzf <out>/capture-sqlite.tar.gz capture/otel-traces.jsonl
```

renaming to `<set>-<run>-sqlite-<browser>.jsonl`.

Sample `/proc/loadavg` **before and after every run** and record it. **Discard
and re-take any run whose 1-minute figure exceeds 3.0 at either sample.**

- [x] **Step 3: Certify** with Task 5's coverage section, per browser, pooled
      over each settings set's three runs:
  - `full_marks / mounted ≥ 99%`
  - `mounted / navigations ≥ 95%`
  - `dropped == 0` — `navigation_top_json` is the top 20 by duration per test,
    so a non-zero drop means the certified population is a duration-biased
    sample rather than a census

  Required of the **deciding** set; a shortfall in the confirming set is
  reported, not fatal.

  **Below any floor the analysis does not proceed.** Re-collect once; if it
  fails again that is a PR 1 regression — return to Task 3, do not lower the
  floor.

---

### Task 10: Decompose, apply the verdict rules, write up

Spec AC13–AC17, AC20.

- [x] **Step 1: Produce the tables** with `cargo xtask traces boot-phases`, for
      each of cold and warm and each settings set (spec D6).

- [x] **Step 2: Compute signed shares** (AC13): per segment,
      `ff_median(segment) − chr_median(segment)`, over the firefox−chromium
      `bootTotalMs` gap. Shares sum to 100% by construction. A segment where
      firefox is _faster_ contributes negatively and is shown as such, never
      dropped.

      **A residual above 1% of the gap halts the analysis** — the segments close
      exactly, so a residual is a data defect to investigate, not a finding to report.

- [x] **Step 3: Report the frame skew** (AC14): median
      `commitToMountMs − bootTotalMs` per engine, cold and warm. Stated
      separately; never folded into a segment share.

**TWO PRE-REGISTERED RULES WERE WRONG AND WERE REPLACED. Both are recorded in
the write-up, because a rule fixed in advance is only worth something if
breaking it is visible.**

1. **AC11's loadavg discard rule (>3.0) is confounded.** Samples are taken
   between back-to-back runs, so they measure the _finishing run's own VM_, not
   ambient contention — systematically: after gate runs 2.69/3.32/2.25, after
   single-worker runs 1.40/1.52/1.42. One sample hit 3.32; re-taking would have
   re-rolled the same self-load. Replaced by within-arm consistency (<2% spread,
   #792's standard). The breaching run is not a duration outlier.
2. **AC13's 1%-residual rule conflates per-navigation closure with median
   additivity.** Closure holds per navigation exactly (0/2496 violations), but
   `median(a+b) ≠ median(a)+median(b)`, so median-based shares cannot close —
   observed 2–7% apparent residual, entirely an artifact. **Shares are computed
   on means**, which are linear and close to 0.0000%. Medians are kept as robust
   cross-checks.

- [x] **Step 4: Apply the pre-registered rules** (AC15) — **write the diagnosis
      before composing any narrative**, so the rules decide rather than the
      prose:

      *Diagnosis* (deciding set, must agree across cold and warm): a segment
      **dominates** iff its share is ≥40% of the gap **and** ≥1.5× the next largest;
      otherwise **distributed**.

      *Disposition:* **actionable** (dominant segment + a named lever → file an issue) ·
      **intrinsic** (distributed *and* every segment holding ≥5% of chromium's
      `bootTotalMs` has a ratio within ±20% of the overall ratio **computed from this
      corpus** — not #792's suite-level 1.47×) · **unresolved** (anything else, named
      explicitly with a proposed next step).

- [x] **Step 5: Quantify the observer effect** (AC16): post-fix chromium
      `mountToSettledMs` from the **gate-settings** subset vs #792 arm B (also
      2-worker — this is why the subset matters). Call out >10% of the baseline
      median as affected, and state which mount-path changes, if any, landed
      between 2026-08-04 and collection. If affected: a follow-up issue, not a
      blocker.

- [x] **Step 6: Write it up** (AC17) as a `#818` section in
      `docs/observability.md`, following the `#792`/`#155` shape: conditions,
      the runs table, the phase table, the verdict, the reproduction recipe, and
      the `traces boot-phases` invocation. If the disposition is **intrinsic**,
      state what would have to change for re-investigation to be worthwhile
      (AC20) — an intrinsic answer's whole value is that it stops the question
      being reopened.

- [x] **Step 7: Commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-818-firefox-boot-phase-gap -- \
  prettier -w docs/observability.md
git add docs/observability.md
git commit -m "docs(e2e): attribute the firefox/chromium boot gap by phase (#818)"
```

---

### Task 11: Preserve the corpus; open PR 2

Spec AC19.

- [x] **Step 1: Preserve** to
      `~/measurements/jaunder/issue-818-firefox-boot-phases/` with a README
      matching the #792 convention: runs, salts, settings set, conditions
      (`/proc/loadavg` before/after each run), `store-paths.tsv`, known limits,
      consumers. Outside the repo — 380 MB-scale, and anything inside the
      worktree risks perturbing flake evaluation.

- [x] **Step 2: Reset `e2eSalt`** in `flake.nix` to `""` and confirm no commit
      carries a salt.

- [ ] **Step 3: Push and open PR 2**, referencing #818 and stating that it
      closes the issue. Then `cargo xtask pr watch`. **Do not merge without
      approval.**

- [ ] **Step 4: On merge**, release the claim — Status → **Done** in project #1
      (**jaunder-ship**).

## Self-review

**Spec coverage.** AC1→T3 · AC2→T2 · AC3/AC4→T7S3 · AC5→T7S3 · AC6→T5 · AC7→T4 ·
AC8→T6 · AC9→T1 · AC10→T7S4 · AC11→T9S2 · AC12→T9S3 · AC13→T10S2 · AC14→T10S3 ·
AC15→T10S4 · AC16→T10S5 · AC17→T10S6 · AC18→T8 · AC19→T11S1 · AC20→T10S6.
D1/D2→T2+T3 · D3→T5 · D4→T1+T4 · D5→T9S2 · D6→T8S1+T10S1 · D7→T10S5 ·
D8→T6S3+T8S3 · D9→T10S4 · D10→T7/T11. No gaps.

**Placeholders.** One deliberate: `#TBD-GATE` in T6S2, resolved by T1S2. No
"TBD", "handle edge cases", or untested implementation steps.

**Type consistency.** `mergeDocumentTiming(existing, incoming)` — same name and
argument order in T2 (defined) and T3S1 (called). `DocumentTiming` consistent
across T2, T3, T4. `BootCoverageRow` fields (`source`, `project`, `navigations`,
`mounted`, `full_marks`, `dropped`) match between T5's Interfaces, its tests,
and T9S3's three certification ratios. `bootTiming` is a zero-arg async function
in both its T4S3 definition and its T4S1 call site. Task 5 and Task 8 both key
on `(source, …)` and both accept ≥3 boot phases, so a new mark in `client::perf`
never reads as a blackout.
