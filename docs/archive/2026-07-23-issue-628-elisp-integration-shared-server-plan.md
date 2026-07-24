# Plan — #628: elisp-integration flake (readiness robustness + shared server)

**Spec:**
[`2026-07-23-issue-628-elisp-integration-shared-server.md`](../specs/2026-07-23-issue-628-elisp-integration-shared-server.md)
**For agentic workers:** drive with `jaunder-iterate`; delegate a task via
`jaunder-dispatch` if useful. Tick checkboxes live.

## Review header

**Goal:** Make `elisp-integration` reliably green — tolerate a slow CI VM (A),
give it headroom (B), and run the flaky readiness gates once per suite instead
of 14× (C), keeping standalone test runs working.

**Scope**

- **In:** `elisp/test/jaunder-integration-helper.el`,
  `elisp/scripts/run-integration-tests.el`, a new
  `elisp/test/jaunder-wait-test.el`, `elisp/test/jaunder-smoke-integration.el`
  (docstring), `flake.nix` (VM sizing), `docs/adr/0035-*` (amendment).
- **Out:** the 14 test _bodies_ (unchanged — fallback macro passes through);
  server-side latency root-cause; #629; #627.

**Tasks**

- [x] 1. Spike: prove the integration suite runs on the host in this devShell
     (linchpin of local verification). **→ PASS: 14/14 in 3.5s on host, no TZDIR
     needed. Local verification confirmed.**
- [x] 2. **A** — rewrite `jaunder-test--wait` to a wall-clock, env-tunable
     budget + tighten connect-timeouts; TDD via a new pure-suite
     `jaunder-wait-test.el`.
- [x] 3. **C** — split server lifecycle into up/down, make `with-live-server`
     reuse-if-bound (interactive fallback), boot once in the runner; amend
     ADR-0035; fix the stale docstring.
- [x] 4. **B** — bump the elisp nixosTest VM memory + cores in `flake.nix`.
- [x] 5. Regression gate — `cargo xtask elisp-integration` green in the VM;
     README touch-up if needed.

**Key risks / decisions**

- Local verification hinges on Task 1. If the suite is _not_ host-runnable here,
  C's correctness falls back to CI-only — Task 1 decides this **before** we
  build on it.
- The fallback macro means **zero test-body edits** — batch shares one server,
  interactive self-boots.
- Readiness budget is env-tunable (`JAUNDER_TEST_READY_TIMEOUT`, default 30s),
  not a hard bump.
- Shared-server-per-suite amends ADR-0035; recorded in the same change (Task 3).

## Global constraints

- No `Co-Authored-By` trailer.
- Elisp gate before every commit: `devtool check elisp-fmt`,
  `devtool check ert`, `devtool check byte-compile` (or
  `cargo xtask check --no-test`, which runs them). The pre-commit hook then runs
  full `cargo xtask check` — expect the Rust coverage/test leg to run even
  though untouched.
- Worktree-absolute paths for Read/Edit/Write. Don't edit files during a gated
  commit.
- New elisp gets an ert test (README convention); elisp is coverage-exempt
  (ADR-0031).

---

## Task 1 — Spike: host-runnable integration suite

**Why:** the spec's C-verification and A/C local loops assume the live suite
runs on the host. Prove it before relying on it.

**Steps**

1. Build the binary: `cargo build -p jaunder` → note `target/debug/jaunder`.
2. Run the live suite on the host:
   ```
   JAUNDER_TEST_BINARY=$PWD/target/debug/jaunder TZDIR=$TZDIR \
     emacs --batch -Q -l elisp/scripts/run-integration-tests.el
   ```
   (in `nix develop .#ci`; `emacsForCi` provides `plz`, `TZDIR` is set there.)
3. **Expected:** `Ran 14 tests`, all pass (baseline, current per-test-server
   code).

**Decision gate:** runs on host → proceed with local verification for Tasks 2–3.
Does **not** run (missing dep, no localhost) → record here; C/regression
verification falls back to `cargo xtask elisp-integration` (VM) only, and A's
unit test remains host-run via the pure suite. No commit (investigation).

## Task 2 — A: robust `jaunder-test--wait` (TDD)

**2a — RED.** New pure-suite test `elisp/test/jaunder-wait-test.el` (matches
`-test.el` → run by the host `ert` check):

```elisp
;;; jaunder-wait-test.el --- unit tests for the readiness poller -*- lexical-binding: t; -*-
(require 'ert)
(require 'jaunder-integration-helper)

(ert-deftest jaunder-test--wait-returns-value-after-slow-start ()
  "Succeeds once the predicate turns true, well within the budget."
  (let ((n 0))
    (should (eq 'ok (jaunder-test--wait
                     (lambda () (if (>= (cl-incf n) 3) 'ok nil)) "thing" 2)))))

(ert-deftest jaunder-test--wait-errors-with-elapsed-on-timeout ()
  "A never-true predicate errors within its (small) budget, naming what + elapsed."
  (let ((err (should-error (jaunder-test--wait (lambda () nil) "thing" 0.3))))
    (should (string-match-p "thing" (error-message-string err)))))

(ert-deftest jaunder-test--wait-honors-env-timeout ()
  "JAUNDER_TEST_READY_TIMEOUT overrides the default when no arg is passed."
  (let ((process-environment (cons "JAUNDER_TEST_READY_TIMEOUT=0.2" process-environment)))
    (should-error (jaunder-test--wait (lambda () nil) "thing"))))
```

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` → **FAIL** (old signature
has no `timeout` arg, no env override, no elapsed in message).

**2b — GREEN.** Rewrite the helper's poller + tighten connect-timeouts:

```elisp
(defun jaunder-test--wait (predicate what &optional timeout)
  "Poll PREDICATE until non-nil or TIMEOUT seconds elapse; then error WHAT.
TIMEOUT defaults to $JAUNDER_TEST_READY_TIMEOUT or 30.  Wall-clock, so a slow
per-attempt connect can't starve the poll count (#628)."
  (let* ((budget (or timeout
                     (let ((e (getenv "JAUNDER_TEST_READY_TIMEOUT")))
                       (if e (string-to-number e) 30))))
         (deadline (+ (float-time) budget)))
    (catch 'done
      (while (< (float-time) deadline)
        (let ((v (funcall predicate)))
          (when v (throw 'done v)))
        (sleep-for 0.1))
      (error "jaunder-test: timed out (%.1fs) waiting for %s" budget what))))
```

In `jaunder-test--http-reachable-p` and `jaunder-test--authed-200-p`:
`:connect-timeout 5` → `:connect-timeout 2`. Add `(require 'cl-lib)` if
`cl-incf` is used by the test (test file only).

Run pure suite → **PASS**.

**Commit** (after `cargo xtask check --no-test` clean):
`test(elisp): wall-clock, env-tunable readiness budget (#628)`.

## Task 3 — C: shared server + interactive fallback + ADR

**3a — helper.** In `jaunder-integration-helper.el`, factor the lifecycle out of
the macro:

- `jaunder-test--server-up` — everything the macro's `progn` does through the
  three `jaunder-test--wait` gates + `site.base_url`: returns a plist
  `(:proc P :tmp D :base-url U :username "alice" :token T)`. Binds nothing
  global itself.
- `jaunder-test--server-down (state)` — `delete-process`, `kill-buffer`,
  `delete-directory` from the plist.
- Rewrite the macro to reuse-if-bound, else boot for the body's extent:

```elisp
(defmacro jaunder-test--with-live-server (&rest body)
  "Run BODY against a live server.  If one is already bound (batch runner set the
globals), reuse it; otherwise boot a throwaway server for BODY's dynamic extent
(interactive/standalone use).  See ADR-0035 (#628)."
  (declare (indent 0) (debug t))
  `(if jaunder-test-base-url
       (progn ,@body)                       ; ambient shared server
     (let ((st (jaunder-test--server-up)))
       (unwind-protect
           (jaunder-test--bind-from st ,@body)  ; let-binds the globals + auth-source
         (jaunder-test--server-down st)))))
```

`jaunder-test--bind-from` is the existing `let*` that binds
`jaunder-test-base-url/username/app-password`, `jaunder--active-blog`,
`auth-sources` (temp netrc) around the body — extracted so both the fallback and
the runner use one definition.

**3b — runner.** `elisp/scripts/run-integration-tests.el`: boot once, bind
globals via `setq`, run, tear down, exit:

```elisp
(let ((st (jaunder-test--server-up)))
  (jaunder-test--set-globals st)            ; setq the defvars + auth-sources
  (unwind-protect
      (ert-run-tests-batch)                 ; NOT -and-exit (must tear down first)
    (jaunder-test--server-down st))
  (kill-emacs (if (zerop (length (ert--stats-failed-unexpected ...))) 0 1)))
```

Capture stats from `ert-run-tests-batch`'s return and exit non-zero on any
unexpected failure (mirror `-and-exit` semantics).

**3c — docstring.** `jaunder-smoke-integration.el`: drop "(empty)" from
`jaunder-smoke-authenticated-collection`'s docstring (shared server may hold
other tests' posts; the test only asserts status 200).

**3d — ADR.** Amend `docs/adr/0035-elisp-live-integration-harness.md` via
`jaunder-adr`: add an addendum recording the shift to one server per suite with
a reuse-if-bound fallback, and the isolation implication (new tests must stay
collision-tolerant or opt into their own server).

**Verify**

- Host (if Task 1 green): run the live suite → `Ran 14 tests`, all pass
  **against one server** (one boot in the log).
- Interactive fallback: load helper + one integration file,
  `(ert-run-tests-batch "jaunder-publish-scheduled-future")` with globals unset
  → boots its own server, passes.

**Commit:**
`test(elisp): share one live server per suite; keep per-test fallback (#628)`.

## Task 4 — B: VM headroom

`flake.nix` `e2e-elisp-integration.nodes.machine`:

```nix
virtualisation.memorySize = 4096;   # was 2048
virtualisation.cores = 2;           # was default 1
```

**Verify:**
`nix eval .#checks.x86_64-linux.e2e-elisp-integration.drvPath --accept-flake-config`
evaluates. **Commit:**
`ci(elisp): give the integration VM more memory + cores (#628)`.

## Task 5 — Regression gate

- `cargo xtask elisp-integration` → green (the real VM check with all changes;
  ~3 min). Confirm the log shows a single server boot / one set of readiness
  gates.
- Touch `elisp/README.md` only if the run instructions changed (they shouldn't).
- Final `cargo xtask check` clean.
- **Commit** any doc tweak; otherwise the gate is the sign-off. Ship via
  `jaunder-ship` (final review, archive spec+plan, PR, merge on approval).

## Self-review

- Tasks are commit-sized and independently verifiable; A and C each have a
  concrete pass/fail check that isn't "watch CI."
- No placeholders: the poller, macro, and runner are written out.
- Task 1 gates the approach honestly rather than assuming host-runnability.
- Ordering: A (self-contained) → C (depends on the robust poller) → B
  (independent) → regression. ADR amendment rides the C commit that makes the
  decision.
