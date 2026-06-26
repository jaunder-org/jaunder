# Capture Nix build logs on failure — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `xtask`'s Nix-check runner capture the full `nix build -L` log to a durable, gitignored, CI-uploaded path on every run, and reference it in the failure detail — so a failed coverage/e2e check is diagnosable without a rebuild, locally and from CI.

**Architecture:** Add `-L` to the `nix build` in `build_check` and fan its piped stderr out — via `std::io::copy` into a tiny `MultiWriter` `Write` adapter — to both the process's own stderr (live view) and `.xtask/diagnostics/<check>/build.log`. On failure, a pure `failure_detail` formatter embeds that path in the `StepResult.detail`. Both helpers are unit-tested; `build_check` stays the thin shell-out.

**Tech Stack:** Rust std (`std::io`, `std::fs`, `std::process`), the xtask dev-driver. Spec: `docs/superpowers/specs/2026-06-25-issue-48-xtask-nix-log-capture.md`.

## Global Constraints

- xtask runs **host-only**; never invoked from a Nix derivation. (CLAUDE.md invariant.)
- **No coverage instrumentation for xtask** — in-file `#[cfg(test)] mod tests` are fine (nix.rs already has one; xtask is coverage-exempt).
- The single chokepoint is `build_check` (`xtask/src/steps/nix.rs:73-98`); its callers are `nix-coverage`, `nix-coverage-gate`, `nix-e2e` — all benefit from one change.
- Log path convention: `.xtask/diagnostics/<check>/build.log` — already gitignored (verified via `git check-ignore`) and already inside ci.yml's `validate-diagnostics` upload glob (`.github/workflows/ci.yml:59-70`). **Do not** change ci.yml.
- `-L` is passed unconditionally. Piping stderr makes it non-TTY → plain `-L` log lines instead of the progress bar (intended).
- `rescue_diagnostics` is left unchanged (it serves coverage's structured `emit-out/diagnostics` bundle).
- No schema change to `StepResult`; the log path rides in the existing `detail: Option<String>`.
- Commit convention: conventional-commit subject, no `Co-Authored-By` trailers.
- The gate is the source of truth: `cargo xtask validate --no-e2e` green before committing.

---

### Task 1: Capture the Nix build log to a durable path + reference it on failure

**Files:**
- Modify: `xtask/src/steps/nix.rs` (imports; add `MultiWriter` + `failure_detail`; rewrite `build_check`; extend the test module)

**Interfaces:**
- Produces (private to the module, reachable by the in-file tests via `super::`):
  - `struct MultiWriter<A: Write, B: Write>(A, B)` implementing `std::io::Write`.
  - `fn failure_detail(installable: &str, status: &std::process::ExitStatus, log_path: &str) -> String`.
- `build_check(step_name, check) -> StepResult` keeps its signature; only its body changes.

- [x] **Step 1: Write the failing unit tests**

Add these tests inside the existing `#[cfg(test)] mod tests` block in `xtask/src/steps/nix.rs` (it currently holds the `sentinel_detail` tests). Append within that module:

```rust
    use super::{failure_detail, MultiWriter};

    #[test]
    fn multiwriter_fans_full_input_out_to_both_sinks() {
        // Larger than io::copy's internal buffer (8 KiB) so the input spans
        // multiple write() calls — proves we don't assume a single chunk.
        let input = vec![b'x'; 200_000];
        let mut a: Vec<u8> = Vec::new();
        let mut b: Vec<u8> = Vec::new();
        {
            let mut sink = MultiWriter(&mut a, &mut b);
            let mut reader: &[u8] = &input;
            std::io::copy(&mut reader, &mut sink).unwrap();
        }
        assert_eq!(a, input);
        assert_eq!(b, input);
    }

    #[test]
    fn failure_detail_names_installable_status_and_log_path() {
        // `false` exits non-zero, giving a real failed ExitStatus to format.
        let status = std::process::Command::new("false").status().unwrap();
        let d = failure_detail(
            ".#checks.x86_64-linux.e2e",
            &status,
            ".xtask/diagnostics/e2e/build.log",
        );
        assert!(d.contains(".#checks.x86_64-linux.e2e"));
        assert!(d.contains("exited with"));
        assert!(d.contains("full build log: .xtask/diagnostics/e2e/build.log"));
    }
```

- [x] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test --manifest-path xtask/Cargo.toml -p xtask nix::`
Expected: FAIL — `cannot find type MultiWriter` / `cannot find function failure_detail`.

- [x] **Step 3: Update imports at the top of `xtask/src/steps/nix.rs`**

Replace the first line:

```rust
use std::process::Command;
```

with:

```rust
use std::fs::File;
use std::io::{self, Write};
use std::process::{Command, Stdio};
```

(Leave the existing `use crate::result::{CommandResult, Mode, StepResult};` line as-is.)

- [x] **Step 4: Add `MultiWriter` and `failure_detail`**

Insert these two items immediately above `fn build_check` (i.e. just before the `/// `nix build …`` doc comment):

```rust
/// A `Write` that fans every write/flush out to two inner writers. Used to send
/// `nix build -L`'s stderr to both the live terminal and a saved log file in one
/// `io::copy` pass. `write` reports `buf.len()` after `write_all` to both sinks so
/// `io::copy` treats the whole chunk as consumed.
struct MultiWriter<A: Write, B: Write>(A, B);

impl<A: Write, B: Write> Write for MultiWriter<A, B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write_all(buf)?;
        self.1.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()?;
        self.1.flush()
    }
}

/// The failure `detail` for a Nix check, naming the installable, the exit status,
/// and the captured build-log path. Pure so it can be unit-tested.
fn failure_detail(installable: &str, status: &std::process::ExitStatus, log_path: &str) -> String {
    format!("nix build {installable} exited with {status}; full build log: {log_path}")
}
```

- [x] **Step 5: Rewrite `build_check`'s body**

Replace the entire current `fn build_check` (`nix.rs:73-98`) with:

```rust
/// `nix build -L --keep-failed --accept-flake-config --out-link .xtask/gcroots/<check> .#checks.<system>.<check>`,
/// fanning the `-L` build log to both the live terminal and
/// `.xtask/diagnostics/<check>/build.log` (gitignored; uploaded by ci.yml's
/// `validate-diagnostics` artifact). On failure the saved log path is named in the
/// `StepResult` detail so the failure is diagnosable without a rebuild.
/// --accept-flake-config honors the jaunder-org cachix substituter for the
/// untrusted local user; --out-link makes the closure a GC root.
fn build_check(step_name: &str, check: &str) -> StepResult {
    let _ = std::fs::create_dir_all(".xtask/gcroots");
    let out_link = format!(".xtask/gcroots/{check}");
    let installable = format!(".#checks.{SYSTEM}.{check}");

    let log_dir = format!(".xtask/diagnostics/{check}");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = format!("{log_dir}/build.log");

    let mut child = match Command::new("nix")
        .args([
            "build",
            // -L streams every (transitive) derivation's build log to stderr, so
            // the failing dependency's output is in the stream we capture below.
            "-L",
            // Retain the failed build dir so a catastrophic in-sandbox failure
            // (e.g. ENOSPC that prevented writing `$out`) still leaves first-hand
            // data; `rescue_diagnostics` then copies it out.
            "--keep-failed",
            "--accept-flake-config",
            "--out-link",
            &out_link,
            &installable,
        ])
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return StepResult::fail(step_name).detail(e.to_string()),
    };

    // Drain the piped stderr to both the live terminal and the log file. We must
    // drain it regardless (an undrained full pipe would block the child); if the
    // log file can't be opened we still copy to stderr alone.
    if let Some(mut stderr_pipe) = child.stderr.take() {
        match File::create(&log_path) {
            Ok(file) => {
                let mut sink = MultiWriter(file, io::stderr());
                let _ = io::copy(&mut stderr_pipe, &mut sink);
            }
            Err(_) => {
                let _ = io::copy(&mut stderr_pipe, &mut io::stderr());
            }
        }
    }

    match child.wait() {
        Ok(s) if s.success() => StepResult::ok(step_name),
        Ok(s) => {
            rescue_diagnostics(check);
            StepResult::fail(step_name).detail(failure_detail(&installable, &s, &log_path))
        }
        Err(e) => StepResult::fail(step_name).detail(e.to_string()),
    }
}
```

- [x] **Step 6: Run the unit tests to verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml -p xtask nix::`
Expected: PASS — `multiwriter_fans_full_input_out_to_both_sinks`, `failure_detail_names_installable_status_and_log_path`, plus the two pre-existing `sentinel_detail` tests.

- [x] **Step 7: Run the gate and confirm a real check's log is captured on the success path**

Run (from the worktree): `cargo xtask validate --no-e2e`
Expected: exit 0 (static + clippy + coverage all green; the new `xtask-fmt`/`xtask-clippy` from #41 and these changes pass).

Then confirm the real coverage check's log was captured (success path):
Run: `test -s .xtask/diagnostics/coverage/build.log`
Expected: exit 0 (the file exists and is non-empty — proves `build_check` captured an actual nix check's `-L` output, not just unit-test fixtures).

- [x] **Step 8: Acceptance — confirm the FAILURE path captures a transitive builder log and names it**

This proves the motivating scenario end-to-end. Induce a deterministic e2e failure, run the full gate **in the background** (so the long e2e build is not severed by context-mode's ~600s RPC cutoff), then inspect the saved log.

1. Add a guaranteed-failing assertion to a Playwright spec — append to the end of the first `test(...)` body in `end2end/tests/atompub.spec.ts`:
   ```ts
   expect(true, 'issue-48 deliberate failure probe').toBe(false);
   ```
2. Run the full gate in the background: `cargo xtask validate` (run_in_background).
3. After it finishes (exit non-zero), confirm the captured log holds the real failure:
   Run: `rg -c "issue-48 deliberate failure probe|database is locked|Error:|expect\\(received\\)" .xtask/diagnostics/e2e/build.log`
   Expected: ≥1 match — the failing Playwright assertion (and/or its surrounding Playwright failure output) is present in `.xtask/diagnostics/e2e/build.log`, captured from the VM derivation's `-L` stream.
   Run: `rg "full build log: .xtask/diagnostics/e2e/build.log" .xtask/last-result.json`
   Expected: ≥1 match — the e2e step's `detail` names the saved log path.
4. Revert the probe: `git checkout -- end2end/tests/atompub.spec.ts`. Confirm clean: `git status --short end2end/tests/atompub.spec.ts` prints nothing.

(CI-side exfiltration needs no test change: `.xtask/diagnostics/e2e/build.log` is inside ci.yml's `validate-diagnostics` upload glob — confirmed by inspection of `.github/workflows/ci.yml:64-65`.)

- [x] **Step 9: Commit**

Re-run the per-commit gate to be sure the probe revert left the tree green, then commit code + spec + plan:

```bash
git add xtask/src/steps/nix.rs \
        docs/superpowers/specs/2026-06-25-issue-48-xtask-nix-log-capture.md \
        docs/superpowers/plans/2026-06-25-issue-48-xtask-nix-log-capture.md
git commit -m "feat(xtask): capture Nix build logs to a durable, CI-uploaded path on failure (#48)"
```

(Commit only after Step 7's `cargo xtask validate --no-e2e` is green and the Step 8 probe is reverted.)

---

## Self-Review

**Spec coverage:**
- `-L` + fan-out via `MultiWriter`/`io::copy` to live stderr + `.xtask/diagnostics/<check>/build.log` → Steps 3–5. ✓
- Drain-regardless / file-open-failure fallback → Step 5 body. ✓
- Failure `detail` names the log path → `failure_detail` (Step 4) wired in Step 5; tested Step 1. ✓
- No `StepResult` schema change → confirmed (detail string only). ✓
- Testable seams (`MultiWriter`, `failure_detail`) + tests → Steps 1, 4. ✓
- Capture works on a real check → Step 7 (success/coverage); Step 8 (failure/e2e transitive log). ✓
- CI artifact pickup via existing glob, no ci.yml change → Step 8 note + Global Constraints. ✓
- `rescue_diagnostics` unchanged → not touched. ✓
- Gate green → Steps 7, 9. ✓

**Placeholder scan:** No TBD/TODO; all code/commands concrete with expected output.

**Type consistency:** `MultiWriter(A, B)` tuple-struct usage, `failure_detail(installable, status, log_path)` arg order, and the `io::copy(&mut reader, &mut sink)` shape are identical across Steps 1, 4, 5. The `detail` substring asserted in Step 1 (`full build log: …`) matches the `failure_detail` format string in Step 4.
