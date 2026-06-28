# E2E Zero-Panic Gate + Visible-by-Default Journal — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make any server Rust panic during an e2e run fail the e2e check, and make the server journal an uploaded artifact in one fixed place on every run.

**Architecture:** Each e2e nixos `testScript` ends with a shared snippet that dumps the `jaunder.service` journal, `copy_from_vm`s it into `$out`, and asserts no `panicked at` line — so a panic fails the derivation (uncacheable-green). The xtask `e2e` step then copies the realized journals into `.xtask/diagnostics/e2e/`, which CI already uploads with `if: always()`.

**Tech Stack:** Nix (`pkgs.testers.nixosTest`, Python `testScript`), Rust (`xtask`).

## Global Constraints

- Panic marker: the literal substring `panicked at` (verified verbatim in real panic logs). Nothing else (not ERROR logs).
- Journal source: `journalctl -u jaunder.service`.
- Default-deny: an empty, commented `allowed_panics` list is the only exception seam.
- Both backends (`mkE2eSqliteCheck`, `mkE2ePostgresCheck`) get the identical shared snippet (DRY via one Nix function).
- NO `Co-Authored-By` trailers. One clean commit per task.
- Per-commit gate: `cargo xtask validate --no-e2e` (run via Bash in the worktree, or `cd <worktree> &&` context-mode). The flake change's behavioral test is the full `cargo xtask validate` (with e2e) at ship; `nix flake check --no-build` is its cheap per-task evaluation check.

## Pre-existing artifacts (written during start; commit first)

- `docs/superpowers/specs/2026-06-27-issue-93-e2e-panic-gate.md` — the spec.
- `docs/adr/0032-e2e-zero-panic-gate.md` + its `docs/README.md` ADR-table row.

Commit these before Task 1:

```bash
git add docs/superpowers/specs/2026-06-27-issue-93-e2e-panic-gate.md docs/superpowers/plans/2026-06-27-issue-93-e2e-panic-gate.md docs/adr/0032-e2e-zero-panic-gate.md docs/README.md
git commit -m "docs(issue-93): add spec, plan, and ADR-0032 (e2e zero-panic gate)"
```

(No separable concerns to file — #93 surfaced none beyond panics the gate may catch at landing, handled in-cycle.)

---

### Task 1: In-sandbox zero-panic gate (flake.nix)

**Files:**
- Modify: `flake.nix` — add the `e2ePanicGate` function near the other e2e `let` bindings (the `mkE2e*` definitions live ~533–780); append `${e2ePanicGate "sqlite"}` to the sqlite `testScript` (after line 638) and `${e2ePanicGate "postgres"}` to the postgres `testScript` (after its `otel-traces-postgres.jsonl` copy, ~line 780).

**Interfaces:**
- Produces: per-backend `jaunder-journal-<backend>.log` files in each e2e check's `$out` (and thus in the `e2e` symlinkJoin output). Task 2 consumes those names.

- [x] **Step 1: Add the shared gate function.** In `flake.nix`, in the same `let` scope as `mkE2eSqliteCheck` (just before it), add:

```nix
        # #93 / ADR-0032: shared zero-panic gate appended to each e2e testScript.
        # A server Rust panic is isolated (tests still pass), so without this it
        # gets cached green and stays invisible. Dump the service journal, copy it
        # to $out (before the assert, so a failing run is still diagnosable), then
        # fail the check on any `panicked at` line. Default-deny via `allowed_panics`.
        e2ePanicGate = backend: ''
          machine.succeed("journalctl -u jaunder.service --no-pager -o cat > /tmp/jaunder-journal.log")
          machine.copy_from_vm("/tmp/jaunder-journal.log", "jaunder-journal-${backend}.log")
          journal = machine.succeed("cat /tmp/jaunder-journal.log")
          allowed_panics = []  # default-deny; add a proven-benign substring + a comment here if one ever appears
          panics = [l for l in journal.splitlines() if "panicked at" in l and not any(a in l for a in allowed_panics)]
          assert not panics, "e2e zero-panic gate (${backend}): jaunder.service logged Rust panic(s):\n" + "\n".join(panics)
        '';
```

(`${backend}` is Nix interpolation; `\n` is literal in a `''` string, which is what the Python source needs. The snippet contains no other `${`.)

- [x] **Step 2: Append the gate to the sqlite testScript.** After the existing last line of the sqlite `testScript`:

```python
              machine.copy_from_vm("/var/lib/jaunder/otel-traces.jsonl", "otel-traces-sqlite.jsonl")
```

add, at the same 14-space indent, on its own line:

```nix
              ${e2ePanicGate "sqlite"}
```

- [x] **Step 3: Append the gate to the postgres testScript.** After the postgres `testScript`'s last line:

```python
              machine.copy_from_vm("/var/lib/jaunder/otel-traces.jsonl", "otel-traces-postgres.jsonl")
```

add, at the same indent, on its own line:

```nix
              ${e2ePanicGate "postgres"}
```

- [x] **Step 4: Verify the flake evaluates.**

Run: `nix flake check --no-build 2>&1 | rg -i 'error|warning' || echo OK`
Expected: no evaluation errors (the `testScript`s build into valid derivations). This catches Nix syntax / interpolation / indentation mistakes without the ~25-min VM build.

- [x] **Step 5: Commit.**

Run: `cargo xtask validate --no-e2e` (must be green — confirms nothing else broke).

```bash
git add flake.nix
git commit -m "test(e2e): fail the e2e check on any server panic; journal to \$out (#93)"
```

---

### Task 2: Visible-by-default journal copy (xtask)

**Files:**
- Modify: `xtask/src/steps/nix.rs` — extend `e2e()` to copy journals after the build; add `copy_e2e_journals` + a testable `copy_journals_between`; add a unit test.

**Interfaces:**
- Consumes: `jaunder-journal-<backend>.log` files at `.xtask/gcroots/e2e/` (Task 1's output, realized by the `--out-link`).
- Produces: those files copied into `.xtask/diagnostics/e2e/` (CI uploads that dir with `if: always()`).

- [x] **Step 1: Write the failing unit test.** In `xtask/src/steps/nix.rs`, inside `#[cfg(test)] mod tests`, add:

```rust
    #[test]
    fn copy_journals_between_copies_only_journal_logs() {
        let tmp = std::env::temp_dir().join(format!("xtask-j-{}", std::process::id()));
        let src = tmp.join("src");
        let dest = tmp.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("jaunder-journal-sqlite.log"), b"ok").unwrap();
        std::fs::write(src.join("otel-traces-sqlite.jsonl"), b"no").unwrap();

        let n = super::copy_journals_between(&src, &dest);

        assert_eq!(n, 1, "only the jaunder-journal-*.log file should be copied");
        assert!(dest.join("jaunder-journal-sqlite.log").exists());
        assert!(!dest.join("otel-traces-sqlite.jsonl").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
```

- [x] **Step 2: Run it; verify it FAILS.**

Run: `cargo test -p xtask copy_journals_between -- --nocapture`
Expected: FAIL to compile — `copy_journals_between` not defined.

- [x] **Step 3: Implement the copy + wire it into `e2e()`.** Add to `xtask/src/steps/nix.rs` (e.g. after `e2e`), and call it from `e2e`:

```rust
pub fn e2e(result: &mut CommandResult) {
    let step = build_check("nix-e2e", "e2e");
    // #93: surface the per-backend server journals in the one canonical, always-
    // uploaded diagnostics dir, regardless of cache-hit/pass/fail. Best-effort: a
    // failed e2e derivation produces no out-link, but its panic is already in
    // build.log (the `-L` stream + the gate's assertion message).
    copy_e2e_journals();
    result.push(step);
}

/// Copy the realized e2e check's `jaunder-journal-*.log` files into the canonical
/// diagnostics dir. Best-effort; silent on a missing out-link (e.g. a failed build).
fn copy_e2e_journals() {
    copy_journals_between(
        std::path::Path::new(".xtask/gcroots/e2e"),
        std::path::Path::new(".xtask/diagnostics/e2e"),
    );
}

/// Copy every `jaunder-journal-*.log` from `src_dir` into `dest_dir` (created if
/// needed). Returns the count copied. Pure path logic so it is unit-testable.
fn copy_journals_between(src_dir: &std::path::Path, dest_dir: &std::path::Path) -> usize {
    let Ok(entries) = std::fs::read_dir(src_dir) else {
        return 0;
    };
    let _ = std::fs::create_dir_all(dest_dir);
    let mut copied = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with("jaunder-journal-") && name.ends_with(".log")
            && std::fs::copy(entry.path(), dest_dir.join(name)).is_ok()
        {
            copied += 1;
        }
    }
    copied
}
```

- [x] **Step 4: Run the test; verify it PASSES.**

Run: `cargo test -p xtask copy_journals_between -- --nocapture`
Expected: PASS.

- [x] **Step 5: Gate and commit.**

Run: `cargo xtask validate --no-e2e` (green).

```bash
git add xtask/src/steps/nix.rs
git commit -m "test(e2e): copy server journals into .xtask/diagnostics/e2e for upload (#93)"
```

---

## Acceptance (verified at ship)

- Full `cargo xtask validate` (with e2e) green — confirms the gate passes on the current (post-#89, panic-free) tree and the flake evaluates + builds.
- After the run, `.xtask/diagnostics/e2e/jaunder-journal-sqlite.log` and `…-postgres.log` exist (Task 2's copy from the realized output).
- (If the gate surfaces a *pre-existing* panic: fix it, or add a justified `allowed_panics` entry — in-cycle, per ADR-0032.)
