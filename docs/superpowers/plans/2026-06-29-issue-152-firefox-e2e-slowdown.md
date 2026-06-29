# Issue #152 — Firefox e2e slowdown — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make per-test e2e timing a durable CI/local artifact, use it to attribute the Firefox +76% runtime, then apply a clear-win speedup or file a precise follow-up.

**Architecture:** Measurement-first. Phase 1 adds a Playwright JSON report (per-test durations/status) emitted in the nix VM and surfaced into the uploaded diagnostics alongside the already-captured OTEL per-action spans. Phase 2 analyzes firefox-vs-chromium per-test deltas; Phase 3 decides.

**Tech Stack:** Playwright (`@playwright/test`), Nix `nixosTest` VM (flake.nix), Rust xtask diagnostics pipeline, OTEL traces, nextest.

## Global Constraints

- Per-task gate: full `cargo xtask check` (clippy + fmt + coverage + heal); final gate `cargo xtask validate` (incl. e2e). (Memory: per-task gate = full check.)
- No `Co-Authored-By` trailers. Never commit on `main`; the merge is a halt point.
- CI uses the inline `nixPlaywrightConfig` (flake.nix:430), **not** `end2end/playwright.config.ts`. The CI config: `reporter:'line'`, `workers:1`, flat 30s timeout, 0 retries. Do not "fix" the config drift here — that is **#153**.
- CI uploads the whole `.xtask/diagnostics/` tree (ci.yml:98); a file placed in `.xtask/diagnostics/<check>/` is uploaded with no workflow change.
- Measurement-first: no speed change until per-test timing is a retrievable artifact and the delta is attributed.

---

### Task 1: Emit a Playwright JSON report from the nix VM and copy it out

**Files:**
- Modify: `flake.nix` — `nixPlaywrightConfig` (reporter, ~line 437) and both `testScript`s (sqlite ~line 640-642, postgres ~line 772-774).

**Interfaces:**
- Produces: `$out/playwright-report-<backend>.json` in each combo check's realized output (`.xtask/gcroots/e2e-<backend>-<browser>/`). Consumed by Task 2.

- [ ] **Step 1: Switch the reporter to line + json.** In `nixPlaywrightConfig` (flake.nix), replace `reporter: 'line',` with:

```js
            reporter: [
              ['line'],
              ['json', { outputFile: '/tmp/e2e/playwright-report.json' }],
            ],
```

- [ ] **Step 2: Copy the report out of the VM (sqlite).** In the sqlite `testScript`, immediately after the otel-traces copy-out (the `machine.copy_from_vm(".../otel-traces.jsonl", "otel-traces-sqlite.jsonl")` line), add:

```python
              machine.succeed("test -s /tmp/e2e/playwright-report.json")
              machine.copy_from_vm("/tmp/e2e/playwright-report.json", "playwright-report-sqlite.json")
```

- [ ] **Step 3: Copy the report out of the VM (postgres).** In the postgres `testScript`, after its `machine.copy_from_vm(".../otel-traces.jsonl", "otel-traces-postgres.jsonl")` line, add:

```python
              machine.succeed("test -s /tmp/e2e/playwright-report.json")
              machine.copy_from_vm("/tmp/e2e/playwright-report.json", "playwright-report-postgres.json")
```

- [ ] **Step 4: Build one combo and verify the report lands in `$out`.**

Run: `cargo xtask e2e sqlite chromium`
Expected: exit 0, and `.xtask/gcroots/e2e-sqlite-chromium/playwright-report-sqlite.json` exists and is non-empty (`test -s` passed in-VM; the symlinked `$out` now contains it).

- [ ] **Step 5: Commit.**

```bash
git add flake.nix
git commit -m "test(e2e): emit a Playwright JSON per-test report and copy it out of the VM (#152)"
```

---

### Task 2: Surface the JSON report (and OTEL traces) into the uploaded diagnostics

The diagnostics copy currently filters to `jaunder-journal-*.log` only (`copy_journals_between`, `xtask/src/steps/nix.rs`), which is why OTEL traces never reached the CI artifact. Broaden it to also copy the OTEL traces and the new Playwright report.

**Files:**
- Modify/Test: `xtask/src/steps/nix.rs` (`copy_journals_between` + its two callers + the unit test at ~line 336).

**Interfaces:**
- Consumes: `$out` files from Task 1 (`playwright-report-<backend>.json`) plus existing `jaunder-journal-*.log` / `otel-traces-*.jsonl`.
- Produces: those files in `.xtask/diagnostics/<check>/`, which CI uploads (ci.yml:98).

- [ ] **Step 1: Rewrite the unit test to assert the broadened set (TDD).** Replace `copy_journals_between_copies_only_journal_logs` (nix.rs ~336) with:

```rust
    #[test]
    fn copy_e2e_diagnostics_between_copies_journal_otel_and_playwright() {
        let tmp = std::env::temp_dir().join(format!("xtask-j-{}", std::process::id()));
        let src = tmp.join("src");
        let dest = tmp.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("jaunder-journal-sqlite.log"), b"j").unwrap();
        std::fs::write(src.join("otel-traces-sqlite.jsonl"), b"o").unwrap();
        std::fs::write(src.join("playwright-report-sqlite.json"), b"p").unwrap();
        std::fs::write(src.join("unrelated.txt"), b"x").unwrap();

        let n = super::copy_e2e_diagnostics_between(&src, &dest);

        assert_eq!(n, 3, "journal + otel + playwright report are copied; unrelated is not");
        assert!(dest.join("jaunder-journal-sqlite.log").exists());
        assert!(dest.join("otel-traces-sqlite.jsonl").exists());
        assert!(dest.join("playwright-report-sqlite.json").exists());
        assert!(!dest.join("unrelated.txt").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
```

- [ ] **Step 2: Run the test to confirm it fails (function not yet renamed).**

Run: `cargo nextest run -p xtask copy_e2e_diagnostics_between_copies_journal_otel_and_playwright`
Expected: FAIL to compile (`copy_e2e_diagnostics_between` undefined).

- [ ] **Step 3: Broaden + rename the copy function.** Replace `copy_journals_between` (nix.rs ~108-125) with:

```rust
/// Copy e2e diagnostic files — server journals (`jaunder-journal-*.log`), OTEL
/// traces (`otel-traces-*.jsonl`), and the Playwright per-test JSON report
/// (`playwright-report-*.json`) — from `src_dir` into `dest_dir` (created if
/// needed). Returns the count copied. Pure path logic so it is unit-testable.
fn copy_e2e_diagnostics_between(src_dir: &std::path::Path, dest_dir: &std::path::Path) -> usize {
    let wanted = |name: &str| {
        (name.starts_with("jaunder-journal-") && name.ends_with(".log"))
            || (name.starts_with("otel-traces-") && name.ends_with(".jsonl"))
            || (name.starts_with("playwright-report-") && name.ends_with(".json"))
    };
    let Ok(entries) = std::fs::read_dir(src_dir) else {
        return 0;
    };
    let _ = std::fs::create_dir_all(dest_dir);
    let mut copied = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if wanted(name) && std::fs::copy(entry.path(), dest_dir.join(name)).is_ok() {
            copied += 1;
        }
    }
    copied
}
```

- [ ] **Step 4: Update the two callers.** In `e2e_combo` (nix.rs ~91) and `copy_e2e_journals` (~100), change the call from `copy_journals_between(` to `copy_e2e_diagnostics_between(`. Update the `copy_e2e_journals` doc-comment to say "journals, OTEL traces, and Playwright report".

- [ ] **Step 5: Run the unit test — expect PASS.**

Run: `cargo nextest run -p xtask copy_e2e_diagnostics_between_copies_journal_otel_and_playwright`
Expected: PASS.

- [ ] **Step 6: End-to-end check — the report reaches diagnostics.**

Run: `cargo xtask e2e sqlite chromium`
Expected: exit 0; `.xtask/diagnostics/e2e-sqlite-chromium/playwright-report-sqlite.json` and `otel-traces-sqlite.jsonl` both present (uploaded in CI).

- [ ] **Step 7: Commit.**

```bash
git add xtask/src/steps/nix.rs
git commit -m "feat(xtask): surface OTEL traces + Playwright report in e2e diagnostics (#152)"
```

---

### Task 3: Measure & root-cause the Firefox/Chromium delta (Phase 2)

**Files:** none (analysis). Produces a written attribution recorded on #152 and in the spec's working notes.

**Interfaces:** Consumes the JSON reports from Tasks 1-2.

- [ ] **Step 1: Produce both same-backend reports.** Run `cargo xtask e2e sqlite firefox` then `cargo xtask e2e sqlite chromium`. The reports are at `.xtask/diagnostics/e2e-sqlite-firefox/playwright-report-sqlite.json` and `.xtask/diagnostics/e2e-sqlite-chromium/playwright-report-sqlite.json`. (If a VM boot flake occurs, retry — boot flakes are infra.)

- [ ] **Step 2: Join per-test durations and rank the deltas.** Analyze both JSON reports (via `ctx_execute`/`jq`): for each test `title`, compute `firefox.duration - chromium.duration`; print the total delta, the per-test deltas sorted desc, and the ratio (firefox/chromium) per test. Classify: **uniform** (delta roughly proportional across most tests) vs **concentrated** (a few tests dominate the +250s).

- [ ] **Step 3: Attribute to an action class via OTEL.** From `otel-traces-sqlite.jsonl` (both combos), aggregate `withTimedAction` span durations by action name (`page.goto`, `wait.selector`, `ui.click`) and by `waitForHydration`. Determine whether the firefox delta concentrates in navigation/hydration vs interaction. Net out the fixed `setTimeout` sleeps (websub.ts, feeds.spec.ts:56, visibility.spec.ts:341) — they are browser-independent constants.

- [ ] **Step 4: Record the attribution.** Post a comment on #152 with: total delta, uniform-vs-concentrated verdict, the top contributing tests, and the dominant action class. This is the Phase-2 deliverable.

---

### Task 4: Decide & act (Phase 3)

**Files:** depends on Task 3 outcome.

- [ ] **Step 1: Branch on the finding.**
  - **Concentrated in a few tests** → optimize those (e.g. remove a removable hard `setTimeout` sleep, tighten an over-broad wait, or split/skip a redundant heavy case). Re-run the two combos; confirm the firefox total drops. Commit per fix.
  - **A removable harness cost** (e.g. an unnecessary per-nav wait under firefox) → fix it; re-measure.
  - **Uniform hydration tax with no clear win** → do **not** force a speculative change. Document the conclusion on #152 and file a precise follow-up issue (`jaunder-issues`), e.g. "investigate Leptos Firefox WASM hydration time" with the measured per-nav cost. Close this cycle on the visibility + attribution deliverable.

- [ ] **Step 2 (if a fix was applied): Commit.**

```bash
git add <files>
git commit -m "perf(e2e): <specific firefox speedup> (#152)"
```

---

### Task 5: Docs + full gate

**Files:**
- Modify: `docs/observability.md` (it already documents the e2e OTEL traces; add the new `playwright-report-<backend>.json` diagnostics artifact, one paragraph).

- [ ] **Step 1: Document the new artifact.** In `docs/observability.md`, near the existing e2e-traces description, note that each e2e combo now also emits `playwright-report-<backend>.json` (per-test durations/status) into the uploaded diagnostics, and how to read it.

- [ ] **Step 2: Full local gate.**

Run: `cargo xtask validate`
Expected: green (static + coverage + e2e matrix; the new copy-out and reporter change do not affect pass/fail). Retry once on an e2e VM boot flake.

- [ ] **Step 3: Commit.**

```bash
git add docs/observability.md
git commit -m "docs(observability): document the e2e Playwright report artifact (#152)"
```

---

## Self-Review

- **Spec coverage:** Phase 1 (JSON report emitted + copied to `$out`) → Task 1; (surfaced into uploaded diagnostics + OTEL traces uploaded) → Task 2. Phase 2 (measure/attribute) → Task 3. Phase 3 (decide/act) → Task 4. Acceptance: durable per-test artifact → Tasks 1-2; OTEL uploaded → Task 2; delta attributed + written up → Task 3; clear-win-or-follow-up → Task 4; `validate` green → Task 5.
- **Placeholders:** none — Tasks 1-2 carry exact code; Tasks 3-4 are inherently data-dependent and specify exact inputs, commands, and the decision tree (the only legitimately deferred content is the Phase-3 fix, gated on Task 3's measurement, exactly as the spec requires).
- **Type consistency:** the renamed `copy_e2e_diagnostics_between` is updated at both call sites (`e2e_combo`, `copy_e2e_journals`) and in the test; artifact names (`playwright-report-<backend>.json`, `otel-traces-<backend>.jsonl`) are consistent across flake copy-out, the xtask filter, and the analysis tasks.
- **No ADR:** this cycle adds a diagnostics artifact (a convention, not a novel architectural decision); no ADR row needed. If Phase 3 lands a structural change, reassess.
