# Spec — #628: elisp-integration "auth readiness" flake

**Issue:** jaunder-org/jaunder#628 · **Milestone:** Test infrastructure & E2E ·
**Priority:** P1 **Branch:** `worktree-issue-628-elisp-auth-readiness`

## Problem

`elisp-integration` intermittently fails with
`jaunder-test: timed out waiting for auth readiness`, then passes on re-run.
Failures are **partial** (e.g. 1/14, 3/14) and red the whole `e2e gate`. Only
reproducible in CI.

### Diagnosis (from the harness)

`jaunder-test--with-live-server` (in `elisp/test/jaunder-integration-helper.el`)
runs **once per test** — 14 tests ⇒ **14 server boots**, each with three
readiness gates (`runtime.json`, `server readiness`, `auth readiness`) built on
one helper:

```elisp
(defun jaunder-test--wait (predicate what)  ; poll 100× every 0.1s = 10s, then error
```

Two fragilities compound under a loaded CI VM:

1. **The 10 s budget collapses under slow connects.** The auth/reachable
   predicates use `plz` with `:connect-timeout 5`. The budget is _wall-clock_,
   not 100 guaranteed attempts — two near-5 s connect hangs (exactly what a
   contended VM produces) drain the whole budget in ~2 attempts.
2. **14 independent exposures.** Every test boots its own server, so a single
   contention spike on any one boot reds the gate — matching the observed
   _partial_ failures.

This is a too-tight, latency-coupled readiness budget multiplied across 14 boots
— not a broken server.

## Goal / non-goals

**Goal:** make `elisp-integration` reliably green by (a) making the readiness
wait tolerant of a slow VM and (b) cutting the number of exposures from 14 to 1.

**Non-goals:** root-causing server-side auth-session latency (the wait tolerance
absorbs it); the `Validate` OOM (#629); merge-queue adoption (#627). No change
to what the 14 tests assert.

## Approach — A + B + C

### A — Robustify `jaunder-test--wait` (shared by all three gates)

- Express the budget as a **wall-clock deadline**, default **30 s**, env-tunable
  via `JAUNDER_TEST_READY_TIMEOUT` (CI can raise it without a code change).
  Error message includes elapsed time for diagnosis.
- Drop the readiness predicates' `:connect-timeout` from **5 s → 2 s** so a hung
  connect can't starve the poll count.
- **Unit-testable:** a new pure-suite test (`elisp/test/jaunder-wait-test.el`,
  matches the `-test.el` glob so the host `ert` check runs it) drives
  `jaunder-test--wait` with a counter predicate — nil for the first K calls then
  t — asserting the new budget tolerates a slow start and that a never-true
  predicate errors within its timeout. No server needed; deterministic;
  host-run.

### B — VM headroom

In `flake.nix` `e2e-elisp-integration`: `virtualisation.memorySize` **2048 →
4096** and add `virtualisation.cores = 2` (currently defaulted to 1). Faster
boot ⇒ the one remaining `auth readiness` gate is met comfortably.

### C — One server for the whole suite (with an interactive fallback)

The per-test isolation the tests need is **already per-test and
server-independent** (temp dirs, org buffers, `jaunder-blogs` bindings, unique
content). All 14 tests assert on their **own** returned ids/slugs/statuses —
none assumes a clean DB or asserts collection-level state (verified across all
four `*-integration.el` files; the one stale "empty collection" phrasing is a
docstring only, not an assertion). So the server can be shared without touching
test logic.

- Split the macro's body into `jaunder-test--server-up` (init → provision
  `alice` + app-password → serve → 3 gates → `site.base_url` → bind the
  globals + suite netrc + `jaunder--active-blog`) and
  `jaunder-test--server-down` (kill proc, cleanup tmp).
- The **runner** (`run-integration-tests.el`) calls `-up` once, runs
  `ert-run-tests-batch` inside `unwind-protect`, then `-down`, then exits with
  the right code. The three gates run **once**.
- **Interactive fallback:** `jaunder-test--with-live-server` becomes: if a
  server is already bound (batch run) → just run body; else boot one for the
  body's dynamic extent and tear it down. This preserves standalone
  `M-x ert RET <one-test>` debugging.
- Update the stale "empty collection" docstring in
  `jaunder-smoke-integration.el`.

## Decisions to record

- **Shared-server-per-suite** amends the per-test isolation model of
  **ADR-0035** (elisp live-integration harness). Record as an amendment/addendum
  to ADR-0035 via `jaunder-adr` (not a new ADR — it evolves an existing
  decision).
- Readiness budget is **env-tunable with a generous default**, not a hard-coded
  bump.
- **C keeps the macro as an opt-in fallback** rather than deleting it — the plan
  must not regress interactive single-test runs.

## Verification

1. **A:** `jaunder-wait-test.el` passes in the pure `ert` host check
   (deterministic).
2. **C correctness:** run the full integration suite on the **host** —
   `emacs --batch -Q -l elisp/scripts/run-integration-tests.el` with
   `JAUNDER_TEST_BINARY` = a freshly built `jaunder` and `TZDIR` set (devShell
   `emacsForCi` has `plz`; README documents the invocation). All 14 pass against
   one shared server ⇒ the refactor is sound locally, not just in CI.
3. **Regression:** `cargo xtask elisp-integration` (the VM check) green.
4. **Statistical (secondary, post-merge):** watch `elisp-integration` flake rate
   across subsequent CI runs. Not a merge blocker — 1 and 2 close the issue; 3
   confirms in the real VM.

## Risks / tradeoffs

- **Future isolation weakens:** a new test wanting a pristine DB must stay
  collision-tolerant or opt back into its own server via the fallback macro.
  Acceptable; documented in the helper.
- **Interactive DX preserved** by the fallback — the one real cost of C is
  mitigated by design.
- elisp is coverage-exempt (ADR-0031), so no coverage-gate interaction; new
  elisp gets an ert test per convention.

## Out of scope

Server-side auth-session latency root cause; #629 (Validate OOM); #627 (merge
queue).
