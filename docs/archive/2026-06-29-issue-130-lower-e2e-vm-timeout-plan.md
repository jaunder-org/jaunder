# Lower e2e VM driver timeout (#130) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cap each e2e NixOS VM's test-driver budget at 20 min so boot/infra flakes fail fast instead of burning the full 60-min default.

**Architecture:** The NixOS test driver's total wall budget is its `globalTimeout` attribute, which defaults to 3600 s when unset. Both e2e checks (`mkE2eSqliteCheck`, `mkE2ePostgresCheck`) build their VM via `pkgs.testers.nixosTest { … }` without setting it. Add `globalTimeout = 1200;` to each attrset.

**Tech Stack:** Nix flake (`flake.nix`), `pkgs.testers.nixosTest`, `cargo xtask validate` (the e2e gate).

## Global Constraints

- Timeout value: **1200 s (20 min)** — ~1.9× the measured ~10.6 min healthy max (slowest single-browser combo, Firefox). Copied verbatim from the approved spec.
- One uniform value across both backends; do **not** differentiate per-backend.
- Do **not** touch the per-step `wait_for_unit` (60 s) / `wait_for_open_port` (30 s) timeouts — out of scope.
- No ADR (tuning change). No automated regression test (a nix driver-timeout value is not unit-testable; the spec accepts the framework-contract + comment).
- jaunder conventions: no Co-Authored-By trailers; commit only after the gate is green and with user approval at the issue boundary.

---

### Task 1: Set `globalTimeout = 1200` on both e2e VM checks

**Files:**
- Modify: `flake.nix` — `mkE2eSqliteCheck`'s `pkgs.testers.nixosTest { … }` (the attrset opening at ~line 560, `name = checkName;` at ~561)
- Modify: `flake.nix` — `mkE2ePostgresCheck`'s `pkgs.testers.nixosTest { … }` (the attrset opening at ~line 666, `name = checkName;` at ~667)

**Interfaces:**
- Consumes: nothing (leaf change).
- Produces: nothing consumed by later tasks (single-task plan).

- [x] **Step 1: Add `globalTimeout` to the sqlite check**

In `mkE2eSqliteCheck`, insert the attribute immediately after `name = checkName;` inside the `pkgs.testers.nixosTest { … }` attrset:

```nix
          pkgs.testers.nixosTest {
            name = checkName;

            # Cap the test-driver budget at 20 min (default is 3600 s). Healthy
            # runs peak at ~10.6 min (slowest single-browser combo), so this is
            # ~1.9x headroom; a boot/infra hang now fails near 20 min instead of
            # burning the full hour. See issue #130.
            globalTimeout = 1200;

            nodes.machine =
```

- [x] **Step 2: Add `globalTimeout` to the postgres check**

In `mkE2ePostgresCheck`, make the same insertion after its `name = checkName;` (the second `pkgs.testers.nixosTest { … }` attrset, ~line 666):

```nix
          pkgs.testers.nixosTest {
            name = checkName;

            # Cap the test-driver budget at 20 min (default is 3600 s). Healthy
            # runs peak at ~10.6 min (slowest single-browser combo), so this is
            # ~1.9x headroom; a boot/infra hang now fails near 20 min instead of
            # burning the full hour. See issue #130.
            globalTimeout = 1200;

            nodes.machine =
```

- [x] **Step 3: Confirm the flake still evaluates and both checks carry the value**

Run (from the worktree root):

```bash
nix eval --raw .#checks.x86_64-linux.e2e-sqlite-chromium.drvPath
```

Expected: prints a `/nix/store/….drv` path (the flake evaluates cleanly with the new attribute; `globalTimeout` is a recognized `nixosTest` arg, so no eval error). Repeat for `e2e-postgres-firefox` to confirm both code paths evaluate.

- [x] **Step 4: Run the full e2e gate**

Run (from the worktree root — context-mode runs against the MAIN repo, so use Bash or `cd <worktree> &&`):

```bash
cargo xtask validate
```

Expected: green. All four `{sqlite,postgres}×{chromium,firefox}` combos pass, each completing well under 1200 s — proving the new budget is adequate for a healthy run (no false timeouts). The `xtask-done: … ok=true` sentinel confirms completion.

- [x] **Step 5: Commit** (after the gate is green and at the issue boundary, per jaunder workflow)

```bash
git add flake.nix docs/superpowers/specs/2026-06-29-issue-130-lower-e2e-vm-timeout.md docs/superpowers/plans/2026-06-29-issue-130-lower-e2e-vm-timeout.md
git commit -m "test(e2e): cap VM driver globalTimeout at 20 min so boot flakes fail fast (#130)"
```

---

## Self-Review

**Spec coverage:**
- Change (`globalTimeout = 1200` on both attrsets) → Task 1 Steps 1–2. ✓
- 20-min justification comment → embedded in both inserts. ✓
- Not touching the 60s/30s service waits → Global Constraints; no task modifies them. ✓
- Verification via `cargo xtask validate` → Step 4. ✓
- No ADR, no regression test → Global Constraints. ✓
- Both acceptance criteria (3600→1200 justified; hang terminates near bound by semantics) → satisfied by Steps 1–2 + the framework contract. ✓

**Placeholder scan:** none — exact lines, exact value, exact commands.

**Type consistency:** single attribute `globalTimeout = 1200;`, identical in both inserts. ✓
