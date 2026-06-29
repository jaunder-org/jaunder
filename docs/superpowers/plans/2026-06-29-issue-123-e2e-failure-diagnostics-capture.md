# Persist e2e VM diagnostics on failure (#123 + #49) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On a failing e2e combo, durably capture the Playwright report + trace + screenshots, the app journal, the system/serial journal, and the OTel trace into `.xtask/diagnostics/<check>/` (the always-uploaded `validate-diagnostics` artifact). Closes #123 and #49.

**Architecture:** Three layers. (1) Playwright config gains `trace`/`screenshot`/`outputDir` so artifacts exist. (2) The in-VM testScript switches from `machine.succeed("…playwright…")` to **capture-exit → stream output → copy-all-artifacts → assert**, factored into one shared Nix helper across both backends. (3) xtask's `rescue_diagnostics` is extended to recover the e2e artifacts from the `--keep-failed` build dir on a failed derivation (which has no `$out`).

**Tech Stack:** Nix flake (`flake.nix`, `pkgs.testers.nixosTest`, Playwright config), the NixOS test-driver Python `testScript`, Rust (`xtask/src/steps/nix.rs`), `cargo xtask validate`/`check`.

## Global Constraints

- Both **sqlite and postgres** variants must be covered — via the shared helper, not copy-paste.
- A healthy (green) run must produce **no new artifacts** and stay fast: `trace: 'retain-on-failure'`, `screenshot: 'only-on-failure'` (artifacts written only for failed tests). No video.
- Artifacts are copied **unconditionally, before** the pass/fail `assert` — a Playwright non-zero exit must not abort the copies.
- The failing-test name + assertion must be recoverable from `build.log` **alone** (streamed `line` reporter), independent of `--keep-failed`.
- Artifact filenames are flat and per-backend: `playwright-report-<backend>.json`, `playwright-artifacts-<backend>.tar.gz`, `system-journal-<backend>.log`, `jaunder-journal-<backend>.log`, `otel-traces-<backend>.jsonl`.
- jaunder conventions: no Co-Authored-By; fix lints don't silence them; commit only on a green gate at the issue boundary.

---

### Task 1: Playwright config — generate trace/screenshots on failure

**Files:**
- Modify: `flake.nix` — `nixPlaywrightConfig` `use:` block (~line 441–444).

**Interfaces:**
- Produces: trace zips + screenshots under `/tmp/e2e/test-results/` for failed tests; consumed by Task 2's tarball copy.

- [x] **Step 1: Add trace/screenshot/outputDir to the `use` block**

In `nixPlaywrightConfig`, change the `use:` block from:

```js
            use: {
              actionTimeout: 0,
              ...(traceParent ? { extraHTTPHeaders: { traceparent: traceParent } } : {}),
            },
```

to:

```js
            use: {
              actionTimeout: 0,
              // Capture forensics only for failed tests, so a green run (the
              // common case) writes nothing extra and pays negligible overhead.
              // Recovered from the validate-diagnostics artifact on a red e2e
              // (#123/#49). No video — the trace already carries DOM snapshots.
              trace: 'retain-on-failure',
              screenshot: 'only-on-failure',
              ...(traceParent ? { extraHTTPHeaders: { traceparent: traceParent } } : {}),
            },
            // Artifact root for traces/screenshots; copied out by the testScript.
            outputDir: '/tmp/e2e/test-results',
```

- [x] **Step 2: Confirm the flake still evaluates**

Run (from the worktree root):

```bash
nix eval --raw .#checks.x86_64-linux.e2e-sqlite-chromium.drvPath
```

Expected: prints a `/nix/store/….drv` path (config is valid JS, flake evaluates).

- [x] **Step 3: Commit**

```bash
git add flake.nix
git commit -m "test(e2e): record Playwright trace+screenshots on failure (#49)"
```

---

### Task 2: Shared `e2eRunAndCapture` helper + testScript restructure (both backends)

**Files:**
- Modify: `flake.nix` — add `e2eRunAndCapture` helper near `e2ePanicGate` (~line 541); replace the run+copy tail in `mkE2eSqliteCheck` (~lines 629–654) and `mkE2ePostgresCheck` (~lines 780–804).

**Interfaces:**
- Consumes: `e2ePanicGate` (existing, unchanged); the Playwright `outputDir` from Task 1.
- Produces: per-backend flat artifact files copied to the driver out-dir (`$out` on success; the `--keep-failed` build dir on failure) — consumed by Task 4's rescue. The build log gains the streamed `line` reporter output.

- [x] **Step 1: Define the `e2eRunAndCapture` helper**

Immediately after the `e2ePanicGate` binding (after flake.nix ~line 550), add a helper that emits the full run-capture-copy-assert tail. It takes the per-combo env the two checks currently inline:

```nix
        # #123/#49: run Playwright capturing its exit (NOT machine.succeed, which
        # would abort before we copy diagnostics), stream its line-reporter output
        # to the build log, copy ALL artifacts out of the VM unconditionally, then
        # fail the check only after the copies are safe. On success the copies land
        # in $out; on failure they live in the --keep-failed build dir for xtask's
        # rescue_diagnostics to recover. Shared by both backends so they can't drift.
        e2eRunAndCapture =
          {
            backend,
            browser,
            traceId,
            traceParent,
            warmupEnv ? "",
          }:
          ''
            pw_status, pw_out = machine.execute(
              "cd /tmp/e2e"
              + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
              + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
              + "${warmupEnv}"
              + " JAUNDER_MAIL_CAPTURE_FILE=/var/lib/jaunder/mail.jsonl"
              + " JAUNDER_WEBSUB_CAPTURE_FILE=/var/lib/jaunder/websub.jsonl"
              + " JAUNDER_E2E_TRACE_ID=${traceId}"
              + " JAUNDER_E2E_TRACEPARENT=${traceParent}"
              + " JAUNDER_E2E_OTLP_HTTP_ENDPOINT=http://127.0.0.1:4318/v1/traces"
              + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
              + " --config playwright.nix.config.js --project ${browser}"
            )
            # Stream the Playwright line-reporter output into the build log (-L),
            # so the failing test + assertion are recoverable from build.log alone,
            # even on failure and without --keep-failed.
            print(pw_out)

            # Stop otel so its trace flushes; ignore status (best-effort capture).
            machine.execute("systemctl stop otel-collector.service")

            # Copy every diagnostic UNCONDITIONALLY, each guarded so a missing file
            # (e.g. an early crash) never aborts the remaining copies. copy_from_vm's
            # 2nd arg is a target *dir*; "" lands the file flat under the per-backend
            # name carried by the source.
            def _grab(path):
                if machine.execute("test -e " + path)[0] == 0:
                    machine.copy_from_vm(path, "")

            machine.execute("test -s /var/lib/jaunder/otel-traces.jsonl && cp /var/lib/jaunder/otel-traces.jsonl /tmp/otel-traces-${backend}.jsonl")
            _grab("/tmp/otel-traces-${backend}.jsonl")

            machine.execute("test -s /tmp/e2e/playwright-report.json && cp /tmp/e2e/playwright-report.json /tmp/playwright-report-${backend}.json")
            _grab("/tmp/playwright-report-${backend}.json")

            machine.execute("tar czf /tmp/playwright-artifacts-${backend}.tar.gz -C /tmp/e2e test-results 2>/dev/null || true")
            _grab("/tmp/playwright-artifacts-${backend}.tar.gz")

            machine.execute("journalctl --no-pager -o short-precise > /tmp/system-journal-${backend}.log")
            _grab("/tmp/system-journal-${backend}.log")

            ${e2ePanicGate backend}

            # Fail the check now — after all artifacts are safely copied out.
            assert pw_status == 0, "e2e Playwright failed (exit %d) for ${backend}/${browser}; see playwright-report-${backend}.json + playwright-artifacts-${backend}.tar.gz + build.log" % pw_status
          '';
```

Note: the OTel artifact name changes from a directory copy (`otel-traces-<backend>.jsonl` as the `copy_from_vm` *target* dir, the old layout) to a **flat file** `otel-traces-<backend>.jsonl`. The trace-analysis tooling reads `otel-traces-<backend>.jsonl` — keep verifying it still resolves in Task 3's run (the file content is identical; only the success-path copy mechanism changes). If the tooling requires the directory layout, fall back to the old `copy_from_vm("/var/lib/jaunder/otel-traces.jsonl", "otel-traces-${backend}.jsonl")` form inside `_grab` semantics.

- [x] **Step 2: Replace the sqlite tail with the helper call**

In `mkE2eSqliteCheck`'s `testScript`, replace lines ~629–654 (from `machine.succeed("cd /tmp/e2e"` … through `${e2ePanicGate "sqlite"}`) with:

```nix
              seed_db()
              ${e2eRunAndCapture {
                backend = "sqlite";
                inherit browser traceId traceParent warmupEnv;
              }}
```

(The `seed_db()` line already precedes this region — keep it; replace only the run+copy+gate tail.)

- [x] **Step 3: Replace the postgres tail with the helper call**

In `mkE2ePostgresCheck`'s `testScript`, replace lines ~780–804 (the `machine.succeed("cd /tmp/e2e"` … `${e2ePanicGate "postgres"}` tail) with:

```nix
              seed_db()
              ${e2eRunAndCapture {
                backend = "postgres";
                inherit browser traceId traceParent warmupEnv;
              }}
```

- [x] **Step 4: Confirm both checks still evaluate**

```bash
nix eval --raw .#checks.x86_64-linux.e2e-sqlite-chromium.drvPath
nix eval --raw .#checks.x86_64-linux.e2e-postgres-firefox.drvPath
```

Expected: both print `.drv` paths (the Python testScript is assembled correctly; Nix string interpolation resolves).

- [ ] **Step 5: Commit**

```bash
git add flake.nix
git commit -m "test(e2e): copy all VM diagnostics before failing the check (#49)"
```
<!-- done -->


---

### Task 3: Verify the failure path end-to-end + discover the kept-dir layout

This task runs ONE real failing e2e combo to (a) prove the in-VM capture works, (b) prove the failing test reaches `build.log`, and (c) record where the copied artifacts live in the `--keep-failed` build dir, which Task 4's rescue consumes. No commit (the temp spec is removed before Task 4).

**Files:**
- Create (throwaway): `end2end/tests/zzz-force-fail.spec.ts`

**Interfaces:**
- Produces: the kept-dir artifact path/layout (recorded in this task's notes), consumed by Task 4.

- [x] **Step 1: Add a throwaway always-failing spec**

Create `end2end/tests/zzz-force-fail.spec.ts`:

```ts
import { test, expect } from '@playwright/test';

// TEMPORARY (#123/#49 verification): forces an e2e failure so the failure-path
// artifact capture can be exercised. REMOVE before committing Task 4.
test('force fail to exercise failure-path capture', async ({ page }) => {
  await page.goto('/');
  expect(1, 'intentional failure for #123/#49 verification').toBe(2);
});
```

- [x] **Step 2: Run one real e2e combo and let it fail**

```bash
cargo xtask e2e sqlite chromium
```

DONE (2026-06-29): combo FAILED on the forced spec. `build.log` carried the full Playwright `line` output (every test, the failures with assertion + error-context + `trace.zip` paths) and the final `AssertionError: e2e Playwright failed (exit 1) … see playwright-report-sqlite.json + playwright-artifacts-sqlite.tar.gz` — proving the streamed reporter reaches the log. All five artifacts were copied unconditionally before the abort.

- [x] **Step 3: Record where the artifacts land on failure**

DONE — **key finding:** with `--keep-failed`, nix keeps the failed derivation's **output store path**, and it is **world-readable**. `nix eval --raw .#checks.x86_64-linux.e2e-sqlite-chromium.outPath` returns exactly that kept path (`/nix/store/…-vm-test-run-jaunder-e2e-sqlite-chromium`), which contains `jaunder-journal-sqlite.log`, `system-journal-sqlite.log`, `playwright-report-sqlite.json`, `playwright-artifacts-sqlite.tar.gz` (6 MB), and `otel-traces-sqlite.jsonl/` (directory). The `/tmp/nix-build-jaunder-*` build dir is nixbld-owned and NOT readable, so reading the **outPath** is both simpler and more robust. Task 4 is revised to this approach.

---

### Task 4: Recover failure artifacts from the kept outPath + unit test

**Files:**
- Modify: `xtask/src/steps/nix.rs` — `copy_e2e_diagnostics_between`'s `wanted()` predicate (~line 126); add an `eval_out_path` helper; call the copier from `rescue_diagnostics` (~line 273) with the eval'd outPath.
- Test: `xtask/src/steps/nix.rs` `#[cfg(test)]` mod — extend `copy_e2e_diagnostics_between_copies_journal_otel_and_playwright` (~line 385) to cover the two new artifact types.
- Delete (throwaway from Task 3): `end2end/tests/zzz-force-fail.spec.ts`.

**Interfaces:**
- Consumes: the failed check's deterministic outPath (`nix eval --raw .#checks.<system>.<check>.outPath`), kept world-readable by `--keep-failed`.
- Produces: e2e artifacts in `.xtask/diagnostics/<check>/` on a failed build, via the existing `copy_e2e_diagnostics_between` (which already handles the otel directory).

- [ ] **Step 1: Extend the existing unit test to cover the two new artifact types**

In `copy_e2e_diagnostics_between_copies_journal_otel_and_playwright`, also write a `playwright-artifacts-<backend>.tar.gz` and a `system-journal-<backend>.log` into the fixture `src`, and assert both are copied to `dest`:

```rust
        std::fs::write(src.join("playwright-artifacts-sqlite.tar.gz"), b"a").unwrap();
        std::fs::write(src.join("system-journal-sqlite.log"), b"s").unwrap();
```
```rust
        assert!(dest.join("playwright-artifacts-sqlite.tar.gz").exists());
        assert!(dest.join("system-journal-sqlite.log").exists());
```

- [ ] **Step 2: Run the test, verify it fails**

```bash
cargo nextest run -p xtask copy_e2e_diagnostics_between_copies_journal_otel_and_playwright
```

Expected: FAIL (the two new files are not yet matched by `wanted()`).

- [ ] **Step 3: Extend `wanted()`, add `eval_out_path`, and wire the failure rescue**

Add the two patterns to `copy_e2e_diagnostics_between`'s `wanted` closure:

```rust
    let wanted = |name: &str| {
        (name.starts_with("jaunder-journal-") && name.ends_with(".log"))
            || (name.starts_with("system-journal-") && name.ends_with(".log"))
            || (name.starts_with("otel-traces-") && name.ends_with(".jsonl"))
            || (name.starts_with("playwright-report-") && name.ends_with(".json"))
            || (name.starts_with("playwright-artifacts-") && name.ends_with(".tar.gz"))
    };
```

Add a helper that evaluates the check's deterministic output path (the `--keep-failed` store path on a failed build):

```rust
/// The check's evaluated output store path. On a failed build `--keep-failed`
/// leaves this path on disk, world-readable, even though it is unregistered — so
/// the e2e diagnostics the VM copied into `$out` are recoverable from it (#123/#49).
/// `None` if the eval fails (e.g. an eval-time error unrelated to the build).
fn eval_out_path(check: &str) -> Option<String> {
    let installable = format!(".#checks.{SYSTEM}.{check}.outPath");
    let out = Command::new("nix")
        .args(["eval", "--raw", "--accept-flake-config", &installable])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8(out.stdout).ok()?;
    let path = path.trim();
    (!path.is_empty()).then(|| path.to_owned())
}
```

Then, in `rescue_diagnostics`, after the existing `emit-out/diagnostics` loop, recover the e2e artifacts from the kept outPath (a no-op for non-e2e checks — their outPath has no matching files):

```rust
    // #123/#49: a failed e2e VM check leaves its $out store path on disk
    // (--keep-failed, world-readable) though unregistered. Its deterministic path
    // is the evaluated outPath; recover the copied-out diagnostics from it, reusing
    // the success-path copier (which handles the otel directory layout).
    if let Some(out_path) = eval_out_path(check) {
        copy_e2e_diagnostics_between(std::path::Path::new(&out_path), std::path::Path::new(&dest));
    }
```

- [ ] **Step 4: Run the test, verify it passes**

```bash
cargo nextest run -p xtask copy_e2e_diagnostics_between_copies_journal_otel_and_playwright
```

Expected: PASS.

- [ ] **Step 5: Verify the rescue end-to-end against the live kept outPath**

The Task-3 failing run's outPath is still on disk. Confirm the implemented rescue actually recovers the artifacts from it (no second 10-min e2e run needed):

```bash
rm -rf /tmp/rescue-check && mkdir -p /tmp/rescue-check
OUT=$(nix eval --raw .#checks.x86_64-linux.e2e-sqlite-chromium.outPath 2>/dev/null)
ls -1 "$OUT"
```

Expected: `$OUT` lists `jaunder-journal-sqlite.log`, `system-journal-sqlite.log`, `playwright-report-sqlite.json`, `playwright-artifacts-sqlite.tar.gz`, `otel-traces-sqlite.jsonl/` — i.e. exactly what `copy_e2e_diagnostics_between` (with the extended `wanted`) will copy into `.xtask/diagnostics/<check>/`. (The `build_check` failure arm calls this on a real red run; the wiring is a single added call verified by reading.)

- [ ] **Step 6: Remove the throwaway spec, unstage it, run static checks**

```bash
git rm -f --cached end2end/tests/zzz-force-fail.spec.ts 2>/dev/null || true
rm -f end2end/tests/zzz-force-fail.spec.ts
cargo xtask check --no-test
```

Expected: clippy + fmt green; the temp spec is gone and unstaged.

- [ ] **Step 7: Commit**

```bash
git add xtask/src/steps/nix.rs
git commit -m "test(e2e): recover e2e VM diagnostics from the kept outPath on failure (#123, #49)"
```

---

### Task 5: ADR + docs, then the full gate

**Files:**
- Create: `docs/adr/00NN-e2e-failure-diagnostics-capture.md` (NN = next after the current highest).
- Modify: `docs/README.md` — add the ADR table row.

**Interfaces:**
- Consumes: nothing. Produces: the recorded convention.

- [ ] **Step 1: Determine the next ADR number**

```bash
ls docs/adr/ | rg '^[0-9]{4}-' | sort | tail -1
```

Use the next integer (the current highest is 0035 per the elisp-integration ADR; confirm and use 0036 unless a higher one landed).

- [ ] **Step 2: Write the ADR**

Create `docs/adr/0036-e2e-failure-diagnostics-capture.md` documenting: the problem (no artifacts on a failed VM check), the decision (capture-before-assert in the testScript + `--keep-failed` rescue in xtask, streamed `line` reporter for build.log recoverability), and consequences (future e2e checks follow this; healthy runs unaffected via `retain-on-failure`). Status `accepted`.

- [ ] **Step 3: Add the ADR table row to `docs/README.md`**

Add a row `| 0036 | E2E failure-diagnostics capture | accepted |` (match the table's exact column format).

- [ ] **Step 4: Commit the docs**

```bash
git add docs/adr/0036-e2e-failure-diagnostics-capture.md docs/README.md
git commit -m "docs(adr): record the e2e failure-diagnostics capture convention (#123, #49)"
```

- [ ] **Step 5: Full gate on a clean tree**

```bash
cargo xtask validate
```

Expected: green (`xtask-done: … ok=true`). All four combos pass; `retain-on-failure` means the healthy run produces no new artifacts. This is the ship gate.

---

## Self-Review

**Spec coverage:**
- Playwright trace/screenshots generated → Task 1. ✓
- testScript capture→copy→assert, both backends, shared helper → Task 2. ✓
- OTel on failure path → Task 2 helper (unconditional `_grab`). ✓
- Failing test recoverable from build.log alone → Task 2 `print(pw_out)`, verified Task 3 Step 2. ✓
- rescue_diagnostics extended → Task 4. ✓
- Both variants → Task 2 Steps 2–3 (one helper, two call sites). ✓
- Verification via real failing run → Tasks 3 & 5. ✓
- ADR + docs/README row → Task 6. ✓

**Placeholder scan:** none — exact paths, code, and commands. The one runtime-discovered value (kept-dir layout, Task 3 Step 3) is handled by a recursive name-glob in Task 4, so no hardcoded path is left as a placeholder.

**Type consistency:** `rescue_e2e_artifacts(&Path, &Path) -> usize` and `is_e2e_artifact(&str) -> bool` are used identically in the test (Task 4 Step 1) and impl (Step 3). Artifact names match between Task 2's copies (`playwright-report-<backend>.json`, `playwright-artifacts-<backend>.tar.gz`, `system-journal-<backend>.log`, `otel-traces-<backend>.jsonl`, and `jaunder-journal-<backend>.log` from `e2ePanicGate`) and Task 4's `is_e2e_artifact` predicate. ✓
