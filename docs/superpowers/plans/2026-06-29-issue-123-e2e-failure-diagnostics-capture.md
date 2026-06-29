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

- [ ] **Step 1: Add trace/screenshot/outputDir to the `use` block**

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

- [ ] **Step 2: Confirm the flake still evaluates**

Run (from the worktree root):

```bash
nix eval --raw .#checks.x86_64-linux.e2e-sqlite-chromium.drvPath
```

Expected: prints a `/nix/store/….drv` path (config is valid JS, flake evaluates).

- [ ] **Step 3: Commit**

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

- [ ] **Step 1: Define the `e2eRunAndCapture` helper**

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

- [ ] **Step 2: Replace the sqlite tail with the helper call**

In `mkE2eSqliteCheck`'s `testScript`, replace lines ~629–654 (from `machine.succeed("cd /tmp/e2e"` … through `${e2ePanicGate "sqlite"}`) with:

```nix
              seed_db()
              ${e2eRunAndCapture {
                backend = "sqlite";
                inherit browser traceId traceParent warmupEnv;
              }}
```

(The `seed_db()` line already precedes this region — keep it; replace only the run+copy+gate tail.)

- [ ] **Step 3: Replace the postgres tail with the helper call**

In `mkE2ePostgresCheck`'s `testScript`, replace lines ~780–804 (the `machine.succeed("cd /tmp/e2e"` … `${e2ePanicGate "postgres"}` tail) with:

```nix
              seed_db()
              ${e2eRunAndCapture {
                backend = "postgres";
                inherit browser traceId traceParent warmupEnv;
              }}
```

- [ ] **Step 4: Confirm both checks still evaluate**

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

---

### Task 3: Verify the failure path end-to-end + discover the kept-dir layout

This task runs ONE real failing e2e combo to (a) prove the in-VM capture works, (b) prove the failing test reaches `build.log`, and (c) record where the copied artifacts live in the `--keep-failed` build dir, which Task 4's rescue consumes. No commit (the temp spec is removed before Task 4).

**Files:**
- Create (throwaway): `end2end/tests/zzz-force-fail.spec.ts`

**Interfaces:**
- Produces: the kept-dir artifact path/layout (recorded in this task's notes), consumed by Task 4.

- [ ] **Step 1: Add a throwaway always-failing spec**

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

- [ ] **Step 2: Run one real e2e combo and let it fail**

```bash
cargo xtask e2e sqlite chromium
```

Expected: the combo FAILS (the forced spec). Confirm:
- the step result names `build.log`;
- `rg -n "force fail to exercise|zzz-force-fail|expect.*toBe" .xtask/diagnostics/e2e-sqlite-chromium/build.log` finds the failing test + assertion (proves the streamed `line` reporter reached the log).

- [ ] **Step 3: Record the kept-dir artifact layout**

```bash
find /tmp -maxdepth 4 -path '*nix-build-jaunder-e2e-sqlite-chromium*' \( -name 'playwright-report-*.json' -o -name 'playwright-artifacts-*.tar.gz' -o -name 'system-journal-*.log' -o -name 'jaunder-journal-*.log' -o -name 'otel-traces-*.jsonl' \) 2>/dev/null
```

Expected: lists the copied artifacts inside the retained build dir. **Record the common ancestor path pattern** — Task 4 globs the kept `/tmp/nix-build-jaunder-<check>-*` tree by these filenames, so the exact depth does not need hardcoding, but confirm the files are present somewhere under that prefix. If NOTHING is found, the in-VM copies did not run before the abort — revisit Task 2 Step 1 (the `assert` must be last) before proceeding.

---

### Task 4: Extend `rescue_diagnostics` to recover e2e artifacts + unit test

**Files:**
- Modify: `xtask/src/steps/nix.rs` — `rescue_diagnostics` (~line 273) and the artifact-name predicate.
- Test: `xtask/src/steps/nix.rs` `#[cfg(test)]` mod (alongside `copy_e2e_diagnostics_between_copies_journal_otel_and_playwright`, ~line 385).
- Delete (throwaway from Task 3): `end2end/tests/zzz-force-fail.spec.ts`.

**Interfaces:**
- Consumes: the kept `/tmp/nix-build-jaunder-<check>-*` build dir (Task 3's layout).
- Produces: e2e artifacts in `.xtask/diagnostics/<check>/` on a failed build.

- [ ] **Step 1: Write the failing unit test**

Add to the test module. The new helper `rescue_e2e_artifacts(kept_dir, dest)` recursively copies files whose names match the e2e artifact patterns. Test it over a fixture tree:

```rust
    #[test]
    fn rescue_e2e_artifacts_copies_named_files_recursively() {
        let tmp = tempfile::tempdir().unwrap();
        let kept = tmp.path().join("nix-build-jaunder-e2e-sqlite-chromium-1/deep/out");
        std::fs::create_dir_all(&kept).unwrap();
        std::fs::write(kept.join("playwright-report-sqlite.json"), b"r").unwrap();
        std::fs::write(kept.join("playwright-artifacts-sqlite.tar.gz"), b"t").unwrap();
        std::fs::write(kept.join("system-journal-sqlite.log"), b"s").unwrap();
        std::fs::write(kept.join("unrelated.txt"), b"x").unwrap();

        let dest = tmp.path().join("diag");
        let n = super::rescue_e2e_artifacts(
            tmp.path().join("nix-build-jaunder-e2e-sqlite-chromium-1").as_path(),
            &dest,
        );

        assert_eq!(n, 3);
        assert!(dest.join("playwright-report-sqlite.json").exists());
        assert!(dest.join("playwright-artifacts-sqlite.tar.gz").exists());
        assert!(dest.join("system-journal-sqlite.log").exists());
        assert!(!dest.join("unrelated.txt").exists());
    }
```

- [ ] **Step 2: Run the test, verify it fails**

```bash
cargo nextest run -p xtask rescue_e2e_artifacts_copies_named_files_recursively
```

Expected: FAIL (`rescue_e2e_artifacts` not defined).

- [ ] **Step 3: Implement `rescue_e2e_artifacts` and wire it into `rescue_diagnostics`**

Add the recursive name-matched copy and an e2e-artifact predicate, and call it from `rescue_diagnostics` for any kept `nix-build-jaunder-<check>-*` dir (it is a no-op when no matching files exist, so it is safe for the coverage check too):

```rust
/// True for the flat e2e diagnostic artifacts the testScript copies out of the VM
/// (Playwright report, the trace/screenshot tarball, the system + app journals,
/// and the OTEL trace). Distinct from `copy_e2e_diagnostics_between`'s success-path
/// predicate because the failure path also carries the `.tar.gz` and system journal.
fn is_e2e_artifact(name: &str) -> bool {
    (name.starts_with("playwright-report-") && name.ends_with(".json"))
        || (name.starts_with("playwright-artifacts-") && name.ends_with(".tar.gz"))
        || (name.starts_with("system-journal-") && name.ends_with(".log"))
        || (name.starts_with("jaunder-journal-") && name.ends_with(".log"))
        || (name.starts_with("otel-traces-") && name.ends_with(".jsonl"))
}

/// Recursively copy every `is_e2e_artifact` file under `kept_dir` (a retained
/// `--keep-failed` build dir) flat into `dest_dir`. A failed `nixosTest` produces
/// no `$out`, so this is the only way to recover the in-VM `copy_from_vm` output.
/// Returns the count copied. Pure std I/O so it is unit-testable.
fn rescue_e2e_artifacts(kept_dir: &std::path::Path, dest_dir: &std::path::Path) -> usize {
    let mut copied = 0;
    let mut stack = vec![kept_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(path);
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !is_e2e_artifact(name) {
                continue;
            }
            let _ = std::fs::create_dir_all(dest_dir);
            if std::fs::copy(&path, dest_dir.join(name)).is_ok() {
                copied += 1;
            }
        }
    }
    copied
}
```

Then, in `rescue_diagnostics`, after the existing `emit-out/diagnostics` block, also rescue e2e artifacts from the same kept dir (`entry.path()` is the `nix-build-jaunder-<check>-*` dir already in scope):

```rust
        // #123/#49: a failed e2e VM check leaves its copied-out diagnostics in the
        // kept build dir (no $out). Recover them by name, recursively.
        let _ = rescue_e2e_artifacts(&entry.path(), std::path::Path::new(&dest));
```

- [ ] **Step 4: Run the test, verify it passes**

```bash
cargo nextest run -p xtask rescue_e2e_artifacts_copies_named_files_recursively
```

Expected: PASS.

- [ ] **Step 5: Remove the throwaway spec and run static checks**

```bash
git rm -f end2end/tests/zzz-force-fail.spec.ts 2>/dev/null || rm -f end2end/tests/zzz-force-fail.spec.ts
cargo xtask check --no-test
```

Expected: clippy + fmt green; the temp spec is gone.

- [ ] **Step 6: Commit**

```bash
git add xtask/src/steps/nix.rs
git commit -m "test(e2e): rescue e2e VM diagnostics from the keep-failed dir on failure (#123, #49)"
```

---

### Task 5: Confirm end-to-end recovery into `.xtask/diagnostics`

Re-exercise the failure path with the rescue now implemented, confirming artifacts reach the canonical diagnostics dir. Reuses Task 3's mechanism (re-add the temp spec transiently — do NOT commit it).

**Files:**
- Temporarily re-create then delete: `end2end/tests/zzz-force-fail.spec.ts` (same content as Task 3 Step 1).

- [ ] **Step 1: Re-add the temp failing spec and run the combo**

```bash
cargo xtask e2e sqlite chromium
```

(Re-create `end2end/tests/zzz-force-fail.spec.ts` first if Task 4 removed it.) Expected: combo FAILS.

- [ ] **Step 2: Confirm the rescued artifacts**

```bash
ls -1 .xtask/diagnostics/e2e-sqlite-chromium/
```

Expected: contains `playwright-report-sqlite.json`, `playwright-artifacts-sqlite.tar.gz`, `system-journal-sqlite.log`, `jaunder-journal-sqlite.log`, `otel-traces-sqlite.jsonl`, and `build.log` (with the failing test). If an artifact is missing, reconcile its name between Task 2's copy and Task 4's `is_e2e_artifact`.

- [ ] **Step 3: Remove the temp spec (final)**

```bash
rm -f end2end/tests/zzz-force-fail.spec.ts
git status --porcelain   # expect: no zzz-force-fail entry; tree clean
```

---

### Task 6: ADR + docs, then the full gate

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
