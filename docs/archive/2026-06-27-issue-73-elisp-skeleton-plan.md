# Elisp Package Skeleton + ERT Harness + Verify-Gate Wiring — Implementation Plan (issue #73)

**Status:** Executed 2026-06-27 — all 7 tasks landed (commits `b0c1eb1`..`9dcf4f4`), one gate-verified commit each, plus a code-review fixup. Full `cargo xtask validate --no-e2e` green; ERT 10/10. Deferred follow-on filed: #108 (byte-compile in the gate).

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up a tested, formatted, gate-wired `elisp/` subproject so units C (#74) and D (#75) have a verified home in the verify ladder.

**Architecture:** A new top-level `elisp/` subproject (mirroring `end2end/`) holds one `jaunder.el` package with pure helpers + C/D seams, an ERT suite, and two CWD-independent batch driver scripts. Emacs is added to the flake once (`emacsForCi`) and reused by two host xtask `StepSpec`s (`elisp-fmt`, `ert`) and two hermetic nix checks (`ert-check`, `elisp-fmt-check`). elisp is coverage-exempt.

**Tech Stack:** Emacs Lisp (built-in `url`, `auth-source`, `ert`, `cl-lib`), ERT, Nix flake, Rust `xtask`.

**Spec:** `docs/superpowers/specs/2026-06-27-issue-73-elisp-skeleton.md`. **ADR:** `docs/adr/0031-elisp-separately-tested-subproject.md` (already written + README row added this cycle).

## Global Constraints

- **Emacs floor:** `Package-Requires: ((emacs "27.1"))`; only built-in libraries.
- **Naming:** all public symbols `jaunder-`, private helpers `jaunder--`. `lexical-binding: t` in every `.el`.
- **Comments:** use `;;` / `;;;` (never single `;` standalone/trailing comments) so built-in `indent-region` doesn't realign them.
- **Testing:** an ERT test for every *pure* mapping/transform function; seams (stubs that `error`) are not unit-tested. elisp is interim-exempt from the Rust coverage gate (#82 follow-on).
- **No `Co-Authored-By` trailers** in any commit (overrides the global default).
- **Worktree only / never `main`:** all commits land on `worktree-issue-73-emacs-elisp-skeleton`. Review against the fork point: `git diff wt-base-issue-73..HEAD`.
- **Gate invocation:** anything needing `emacs` (the ERT runner, `cargo xtask check`/`validate` after Task 4) must run with emacs on PATH — run it as `nix develop .#ci -c <cmd>` (or from inside `nix develop .#ci`). Per-task gate while iterating: `cargo xtask check --no-test`; final gate: `cargo xtask validate --no-e2e`.
- **No separable concerns to file:** the coverage (#82), e2e-coverage (#83), and app-password self-provision (#76) follow-ons already exist; this plan files no new issues.

---

### Task 1: Add emacs to the flake (devShell + shared binding)

Adds the single `emacsForCi` binding and `emacsSrc`, and puts emacs on the CI/host PATH so later tasks' `emacs` invocations resolve. (`emacsSrc` is defined now but first consumed in Task 5.)

**Files:**
- Modify: `flake.nix` — add `emacsSrc` + `emacsForCi` near `end2endSrc` (`flake.nix:502-505`); add `emacsForCi` to `ciInputs` (`flake.nix:956-975`).

**Interfaces:**
- Produces: a `nix develop .#ci` shell with `emacs` on PATH; the `emacsForCi` and `emacsSrc` let-bindings (consumed by Task 5).

- [ ] **Step 1: Add the bindings after `end2endSrc`.** Insert immediately after `flake.nix:505` (the closing `};` of `end2endSrc`):

```nix
        emacsSrc = pkgs.lib.cleanSourceWith {
          src = ./elisp;
        };

        # One emacs for both the host verify gate (the xtask StepSpecs) and the
        # hermetic nix checks, so they cannot diverge. withPackages (vs bare
        # pkgs.emacs) is the extension point for units C/D to add elisp packages
        # via nix; the skeleton needs only built-in libraries, so the list is empty.
        emacsForCi = pkgs.emacs.pkgs.withPackages (epkgs: [ ]);
```

- [ ] **Step 2: Verify the `withPackages` attr path resolves on the pinned nixpkgs.**

Run: `nix eval --raw .#devShells.x86_64-linux.ci.name` is not a useful probe; instead build the binding directly:
Run: `nix build --no-link --print-out-paths '.#devShells.x86_64-linux.ci'`
Expected: builds without `error: attribute 'withPackages' missing`. If that attr path is wrong on this nixpkgs pin, substitute the correct one (`pkgs.emacs.pkgs.withPackages`, `pkgs.emacsPackagesFor pkgs.emacs |> …`, or `pkgs.emacsWithPackages`) until it evaluates; keep the empty package list.

- [ ] **Step 3: Add `emacsForCi` to `ciInputs`.** Insert the line after `pkgs.dart-sass` (`flake.nix:964`):

```nix
              pkgs.dart-sass
              emacsForCi
              pkgs.jq
```

- [ ] **Step 4: Verify emacs is on the CI shell PATH.**

Run: `nix develop .#ci -c emacs --version`
Expected: prints `GNU Emacs <version>` (≥ 27.1), exit 0.

- [ ] **Step 5: Commit.**

```bash
git add flake.nix
git commit -m "build(flake): add emacs to the CI devshell for the elisp gate"
```

---

### Task 2: Scaffold `jaunder.el` + ERT harness + pure helpers (TDD)

The package, the test file, the runner, the three pure helpers (each test-first), and the C/D seams.

**Files:**
- Create: `elisp/jaunder.el`
- Create: `elisp/test/jaunder-test.el`
- Create: `elisp/scripts/run-tests.el`

**Interfaces:**
- Produces:
  - `(jaunder--build-url BASE &rest SEGMENTS) → string` — normalized URL; errors on nil/empty BASE.
  - `(jaunder--basic-auth-header USER PASSWORD) → (cons "Authorization" "Basic <base64>")`.
  - `(jaunder--auth-source-spec BASE-URL USER) → (:host H :user USER :max 1)`.
  - Seams (error when called): `jaunder--http-request`, `jaunder--auth-secret`, `jaunder--org->atom`, `jaunder--atom->org`.
  - Customs: `jaunder-base-url`, `jaunder-username`.
- Consumes: emacs on PATH (Task 1).

- [ ] **Step 1: Write the runner `elisp/scripts/run-tests.el`** (no test of its own; it's the harness):

```elisp
;;; run-tests.el --- ERT batch runner for jaunder -*- lexical-binding: t; -*-

;;; Commentary:
;; Loads the jaunder package and every test/*-test.el, then runs ERT in
;; batch mode.  Self-locating via `load-file-name' so it works both from the
;; repo root (xtask StepSpec) and from the nix store (hermetic ert-check).

;;; Code:

(require 'ert)

(let* ((this (file-name-directory
              (or load-file-name buffer-file-name default-directory)))
       (root (file-name-directory (directory-file-name this)))
       (test-dir (expand-file-name "test" root)))
  (add-to-list 'load-path root)
  (require 'jaunder)
  (dolist (f (directory-files test-dir t "-test\\.el\\'"))
    (load f nil t)))

(ert-run-tests-batch-and-exit)

;;; run-tests.el ends here
```

- [ ] **Step 2: Write the failing test file `elisp/test/jaunder-test.el`:**

```elisp
;;; jaunder-test.el --- ERT suite for jaunder -*- lexical-binding: t; -*-

;;; Commentary:
;; Unit tests for the pure helpers in jaunder.el.

;;; Code:

(require 'ert)
(require 'jaunder)

(ert-deftest jaunder-build-url-bare ()
  (should (equal (jaunder--build-url "https://x.example") "https://x.example")))

(ert-deftest jaunder-build-url-strips-trailing-slash ()
  (should (equal (jaunder--build-url "https://x.example/") "https://x.example")))

(ert-deftest jaunder-build-url-joins-segments ()
  (should (equal (jaunder--build-url "https://x.example" "atom" "feed")
                 "https://x.example/atom/feed")))

(ert-deftest jaunder-build-url-collapses-inner-slashes ()
  (should (equal (jaunder--build-url "https://x.example/" "/atom/" "feed")
                 "https://x.example/atom/feed")))

(ert-deftest jaunder-build-url-drops-empty-segments ()
  (should (equal (jaunder--build-url "https://x.example" nil "" "feed")
                 "https://x.example/feed")))

(ert-deftest jaunder-build-url-errors-on-empty-base ()
  (should-error (jaunder--build-url nil))
  (should-error (jaunder--build-url "")))

(ert-deftest jaunder-basic-auth-header ()
  (should (equal (jaunder--basic-auth-header "alice" "secret")
                 (cons "Authorization" "Basic YWxpY2U6c2VjcmV0"))))

(ert-deftest jaunder-auth-source-spec-derives-host ()
  (should (equal (jaunder--auth-source-spec "https://blog.example.com/path" "alice")
                 '(:host "blog.example.com" :user "alice" :max 1))))

(ert-deftest jaunder-auth-source-spec-ignores-port ()
  (should (equal (plist-get (jaunder--auth-source-spec "https://blog.example.com:8443" "bob")
                            :host)
                 "blog.example.com")))

;;; jaunder-test.el ends here
```

- [ ] **Step 3: Write a minimal `elisp/jaunder.el` with the helpers UNDEFINED so the suite fails.** Create the package with customs + seams but **without** the three pure helpers:

```elisp
;;; jaunder.el --- Jaunder blogging client (AtomPub) -*- lexical-binding: t; -*-

;; Author: Jaunder contributors
;; Version: 0.1.0
;; Package-Requires: ((emacs "27.1"))
;; Keywords: hypermedia, comm, outlines
;; URL: https://jaunder.org

;;; Commentary:
;; Shared plumbing for the Jaunder Emacs blogging front-end over AtomPub.
;; This is the Infra-unit skeleton (issue #73): pure helpers plus seams that
;; units C (#74, authoring/publish) and D (#75, management/reconcile) extend.

;;; Code:

(require 'url)
(require 'url-parse)
(require 'auth-source)
(require 'cl-lib)

(defgroup jaunder nil
  "Emacs blogging front-end for Jaunder over AtomPub."
  :group 'comm
  :prefix "jaunder-")

(defcustom jaunder-base-url nil
  "Base URL of the Jaunder instance, e.g. \"https://blog.example.com\"."
  :type '(choice (const :tag "Unset" nil) string)
  :group 'jaunder)

(defcustom jaunder-username nil
  "Username used for AtomPub authentication."
  :type '(choice (const :tag "Unset" nil) string)
  :group 'jaunder)

;;; Seams — implemented by later units; calling them now is a programmer error.

(defun jaunder--http-request (&rest _args)
  "HTTP transport seam.  Implemented by unit C (issue #74)."
  (error "jaunder: HTTP layer not yet implemented (unit C, issue #74)"))

(defun jaunder--org->atom (&rest _args)
  "Org->Atom mapping seam.  Implemented by unit C (issue #74)."
  (error "jaunder: org->atom mapping not yet implemented (unit C, issue #74)"))

(defun jaunder--atom->org (&rest _args)
  "Atom->Org mapping seam.  Implemented by units C/D (issues #74/#75)."
  (error "jaunder: atom->org mapping not yet implemented (units C/D, issues #74/#75)"))

(provide 'jaunder)
;;; jaunder.el ends here
```

- [ ] **Step 4: Run the suite to confirm it FAILS.**

Run: `nix develop .#ci -c emacs --batch -Q -l elisp/scripts/run-tests.el`
Expected: FAIL — `jaunder--build-url`, `jaunder--basic-auth-header`, `jaunder--auth-source-spec` are void-function; non-zero exit.

- [ ] **Step 5: Implement the three pure helpers.** Insert before the `;;; Seams` comment in `elisp/jaunder.el`:

```elisp
;;; Pure helpers

(defun jaunder--build-url (base &rest segments)
  "Join BASE and path SEGMENTS into a normalized URL.
Trailing slashes on BASE and surrounding slashes on each segment are
collapsed to single separators; nil or empty segments are dropped.
Signals an error when BASE is nil or empty."
  (when (or (null base) (string= base ""))
    (error "jaunder--build-url: BASE must be non-empty"))
  (let ((head (replace-regexp-in-string "/+\\'" "" base))
        (tail (delq nil
                    (mapcar (lambda (s)
                              (when (and s (not (string= s "")))
                                (replace-regexp-in-string "\\`/+\\|/+\\'" "" s)))
                            segments))))
    (mapconcat #'identity (cons head (delq "" tail)) "/")))

(defun jaunder--basic-auth-header (user password)
  "Return the HTTP Basic Authorization header cons for USER and PASSWORD.
The value is \"Basic <base64(user:password)>\" with no line breaks."
  (cons "Authorization"
        (concat "Basic "
                (base64-encode-string (concat user ":" password) t))))

(defun jaunder--auth-source-spec (base-url user)
  "Return the `auth-source-search' plist for BASE-URL and USER.
:host is the URL host of BASE-URL (port excluded); at most one match."
  (list :host (url-host (url-generic-parse-url base-url))
        :user user
        :max 1))
```

- [ ] **Step 6: Add the `jaunder--auth-secret` I/O wrapper** (a real seam over `auth-source`, not unit-tested). Insert in the `;;; Seams` block of `elisp/jaunder.el`:

```elisp
(defun jaunder--auth-secret ()
  "Retrieve the app password for `jaunder-username' via auth-source.
Thin I/O wrapper over `auth-source-search' using `jaunder--auth-source-spec'."
  (let* ((match (car (apply #'auth-source-search
                            (jaunder--auth-source-spec jaunder-base-url
                                                       jaunder-username))))
         (secret (and match (plist-get match :secret))))
    (cond ((functionp secret) (funcall secret))
          (secret secret)
          (t (error "jaunder: no auth-source entry for %s@%s"
                    jaunder-username jaunder-base-url)))))
```

- [ ] **Step 7: Run the suite to confirm it PASSES.**

Run: `nix develop .#ci -c emacs --batch -Q -l elisp/scripts/run-tests.el`
Expected: PASS — `Ran 9 tests, 9 results as expected`, exit 0.

- [ ] **Step 8: Commit.**

```bash
git add elisp/jaunder.el elisp/test/jaunder-test.el elisp/scripts/run-tests.el
git commit -m "feat(elisp): scaffold jaunder.el package, ERT harness, and pure helpers"
```

---

### Task 3: Elisp formatting driver

A batch driver that reindents (`jaunder-fmt-fix`) or verifies (`jaunder-fmt-check`) every `.el` under `elisp/`, using built-in `emacs-lisp-mode` indentation. prettier cannot format elisp, so this is the elisp-native equivalent.

**Files:**
- Create: `elisp/scripts/format.el`

**Interfaces:**
- Produces: interactive-callable `jaunder-fmt-fix` (writes) and `jaunder-fmt-check` (`kill-emacs 1` on any non-canonical file).
- Consumes: emacs on PATH (Task 1); the `.el` files from Task 2.

- [ ] **Step 1: Write `elisp/scripts/format.el`:**

```elisp
;;; format.el --- elisp formatting driver for jaunder -*- lexical-binding: t; -*-

;;; Commentary:
;; `jaunder-fmt-fix' reindents every .el under elisp/ in place;
;; `jaunder-fmt-check' exits non-zero if any is not canonically formatted.
;; Uses built-in emacs-lisp-mode indentation + trailing-whitespace removal.
;; Self-locating via `load-file-name' (repo root under xtask, nix store under
;; the hermetic check).

;;; Code:

(defun jaunder-fmt--files ()
  "Return all .el files under the elisp/ subproject."
  (let* ((this (file-name-directory
                (or load-file-name buffer-file-name default-directory)))
         (root (file-name-directory (directory-file-name this))))
    (directory-files-recursively root "\\.el\\'")))

(defun jaunder-fmt--canonical (file)
  "Return the canonically-formatted contents of FILE as a string."
  (with-temp-buffer
    (insert-file-contents file)
    (delay-mode-hooks (emacs-lisp-mode))
    (let ((indent-tabs-mode nil))
      (indent-region (point-min) (point-max)))
    (delete-trailing-whitespace)
    (buffer-string)))

(defun jaunder-fmt--raw (file)
  "Return the on-disk contents of FILE as a string."
  (with-temp-buffer
    (insert-file-contents file)
    (buffer-string)))

(defun jaunder-fmt-fix ()
  "Reindent every elisp file in place."
  (dolist (f (jaunder-fmt--files))
    (let ((formatted (jaunder-fmt--canonical f)))
      (unless (string= formatted (jaunder-fmt--raw f))
        (with-temp-file f (insert formatted))))))

(defun jaunder-fmt-check ()
  "Exit non-zero if any elisp file is not canonically formatted."
  (let ((bad '()))
    (dolist (f (jaunder-fmt--files))
      (unless (string= (jaunder-fmt--canonical f) (jaunder-fmt--raw f))
        (push f bad)))
    (when bad
      (message "elisp-fmt: not canonically formatted:\n%s"
               (mapconcat #'identity (nreverse bad) "\n"))
      (kill-emacs 1))))

;;; format.el ends here
```

- [ ] **Step 2: Normalize the existing files, then verify check passes.** First fix (in case any committed `.el` drifted), then check:

Run: `nix develop .#ci -c emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-fix`
Then run: `nix develop .#ci -c emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-check`
Expected: the check command prints nothing and exits 0.

- [ ] **Step 3: Confirm the formatter actually catches drift (negative test).** Temporarily break indentation and confirm a non-zero exit:

Run: `printf '\n  (defun bad () nil)\n' >> elisp/jaunder.el && nix develop .#ci -c emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-check; echo "exit=$?"`
Expected: lists `elisp/jaunder.el` and `exit=1`.
Then restore: `git checkout -- elisp/jaunder.el`

- [ ] **Step 4: Re-confirm clean.**

Run: `nix develop .#ci -c emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-check`
Expected: exit 0.

- [ ] **Step 5: Commit.**

```bash
git add elisp/scripts/format.el elisp/jaunder.el elisp/test/jaunder-test.el elisp/scripts/run-tests.el
git commit -m "feat(elisp): add emacs-batch indentation formatter (fix/check)"
```

---

### Task 4: Wire `elisp-fmt` + `ert` host StepSpecs into xtask

Add both steps to the static-check suite so they run in `check` (fix) and `validate` (check), and update the locked-order + per-step arg tests.

**Files:**
- Modify: `xtask/src/steps/static_checks.rs` — `specs()` (after the `prettier` StepSpec, `static_checks.rs:68-72`), the `step_order_is_locked` expected array (`static_checks.rs:183-193`), and add two arg-assertion tests.

**Interfaces:**
- Consumes: `elisp/scripts/run-tests.el` (Task 2), `elisp/scripts/format.el` (Task 3), emacs on PATH (Task 1).
- Produces: `elisp-fmt` + `ert` steps in both gate modes.

- [ ] **Step 1: Add the `elisp-fmt` mode-dependent args.** Insert after the `prettier_args` block (`static_checks.rs:37`):

```rust
    // elisp-fmt — emacs-batch indentation; prettier cannot format Emacs Lisp.
    let elisp_fmt_args = match mode {
        Mode::Check => vec![
            "--batch", "-Q", "-l", "elisp/scripts/format.el", "-f", "jaunder-fmt-check",
        ],
        Mode::Fix => vec![
            "--batch", "-Q", "-l", "elisp/scripts/format.el", "-f", "jaunder-fmt-fix",
        ],
    };
```

- [ ] **Step 2: Add both StepSpecs after the `prettier` spec.** Insert after the `prettier` `StepSpec { … }` (`static_checks.rs:72`, before the `cargo-deny` spec):

```rust
        StepSpec {
            name: "elisp-fmt",
            program: "emacs",
            args: elisp_fmt_args,
        },
        StepSpec {
            name: "ert",
            program: "emacs",
            args: vec![
                "--batch", "-Q", "-l", "elisp/scripts/run-tests.el",
            ],
        },
```

- [ ] **Step 3: Update the `step_order_is_locked` expected array.** Replace the array in the test (`static_checks.rs:183-193`) with the two new names inserted after `"prettier"`:

```rust
        let expected = [
            "fmt",
            "leptosfmt",
            "prettier",
            "elisp-fmt",
            "ert",
            "cargo-deny",
            "clippy",
            "tools-fmt",
            "tools-clippy",
            "xtask-fmt",
            "xtask-clippy",
        ];
```

- [ ] **Step 4: Add arg-assertion tests** for the new steps. Insert in the `tests` module (after `xtask_clippy_denies_warnings_in_both_modes`, before `step_order_is_locked`):

```rust
    #[test]
    fn elisp_fmt_checks_in_check_writes_in_fix() {
        let check = find(&specs(Mode::Check), "elisp-fmt").args.clone();
        assert_eq!(
            check,
            ["--batch", "-Q", "-l", "elisp/scripts/format.el", "-f", "jaunder-fmt-check"]
        );
        let fix = find(&specs(Mode::Fix), "elisp-fmt").args.clone();
        assert_eq!(
            fix,
            ["--batch", "-Q", "-l", "elisp/scripts/format.el", "-f", "jaunder-fmt-fix"]
        );
    }

    #[test]
    fn ert_runs_the_batch_runner_in_both_modes() {
        for mode in [Mode::Check, Mode::Fix] {
            let ert = find(&specs(mode), "ert");
            assert_eq!(ert.program, "emacs");
            assert_eq!(
                ert.args,
                ["--batch", "-Q", "-l", "elisp/scripts/run-tests.el"]
            );
        }
    }
```

- [ ] **Step 5: Run the xtask unit tests.**

Run: `cargo test --manifest-path xtask/Cargo.toml`
Expected: PASS, including the two new tests and the updated `step_order_is_locked`.

- [ ] **Step 6: Run the gate (fix mode) to confirm both steps execute green.**

Run: `nix develop .#ci -c cargo xtask check --no-test`
Expected: exit 0; `elisp-fmt` and `ert` appear as steps. Confirm via the sidecar: `jq '.steps[] | select(.name=="ert" or .name=="elisp-fmt")' .xtask/last-result.json` shows `"ok": true` for both.

- [ ] **Step 7: Commit.**

```bash
git add xtask/src/steps/static_checks.rs
git commit -m "build(xtask): run elisp-fmt and ert in the verify gate"
```

---

### Task 5: Hermetic nix checks + coverage exemption

Mirror the prettier precedent in `nix flake check`, and exclude `elisp/` from the coverage source so elisp-only changes don't retrigger Rust coverage rebuilds.

**Files:**
- Modify: `flake.nix` — add `ert-check` + `elisp-fmt-check` after `prettier-check` (`flake.nix:948`); add `/elisp/` to the coverage source filter (`flake.nix:871-877`).

**Interfaces:**
- Consumes: `emacsForCi` + `emacsSrc` (Task 1), the driver scripts (Tasks 2–3).
- Produces: `checks.<system>.ert-check`, `checks.<system>.elisp-fmt-check`.

- [ ] **Step 1: Add the two checks after `prettier-check`.** Insert before the closing `};` of the checks attrset (`flake.nix:949`):

```nix
            ert-check =
              pkgs.runCommand "ert-check"
                {
                  nativeBuildInputs = [ emacsForCi ];
                }
                ''
                  emacs --batch -Q -l ${emacsSrc}/scripts/run-tests.el
                  touch $out
                '';
            elisp-fmt-check =
              pkgs.runCommand "elisp-fmt-check"
                {
                  nativeBuildInputs = [ emacsForCi ];
                }
                ''
                  emacs --batch -Q -l ${emacsSrc}/scripts/format.el -f jaunder-fmt-check
                  touch $out
                '';
```

- [ ] **Step 2: Exclude `elisp/` from the coverage source.** Add one line to the coverage `cleanSourceWith` filter (after `flake.nix:876`, the `.github/` exclusion):

```nix
                    !(pkgs.lib.hasInfix "/xtask/" path)
                    && !(pkgs.lib.hasInfix "/tools/" path)
                    && !(pkgs.lib.hasInfix "/docs/" path)
                    && !(pkgs.lib.hasInfix "/.github/" path)
                    && !(pkgs.lib.hasInfix "/elisp/" path);
```

- [ ] **Step 3: Build both checks.**

Run: `nix build --no-link --print-out-paths '.#checks.x86_64-linux.ert-check' '.#checks.x86_64-linux.elisp-fmt-check'`
Expected: both build, exit 0.

- [ ] **Step 4: Verify the coverage drv is insensitive to elisp.** Capture the coverage drvPath, touch an elisp file, re-eval, confirm unchanged:

Run: `nix eval --raw '.#checks.x86_64-linux.coverage.drvPath'`
Then: `printf '\n' >> elisp/jaunder.el && nix eval --raw '.#checks.x86_64-linux.coverage.drvPath'; git checkout -- elisp/jaunder.el`
Expected: identical drvPath both times (the `/elisp/` exclusion works).

- [ ] **Step 5: Commit.**

```bash
git add flake.nix
git commit -m "build(flake): add hermetic ert-check + elisp-fmt-check; exempt elisp from coverage"
```

---

### Task 6: Docs — CONTRIBUTING subsection + elisp README

Document the subproject and how to run it. (ADR-0031, the spec, and the README ADR row are already written this cycle.)

**Files:**
- Create: `elisp/README.md`
- Modify: `CONTRIBUTING.md` — add a short "Elisp subproject" subsection.

**Interfaces:**
- Consumes: nothing (documentation only).

- [ ] **Step 1: Write `elisp/README.md`:**

```markdown
# Jaunder Emacs client (`jaunder.el`)

The Emacs blogging front-end for Jaunder over AtomPub. This is the Infra-unit
skeleton (issue #73): shared plumbing and pure helpers that units C (#74,
authoring/publish) and D (#75, management/reconcile) extend.

## Layout

- `jaunder.el` — the package: customization group, pure helpers, and the
  HTTP/auth/mapping seams later units implement.
- `test/` — the ERT suite (one test per pure helper).
- `scripts/run-tests.el` — batch ERT runner.
- `scripts/format.el` — `jaunder-fmt-fix` / `jaunder-fmt-check` (built-in
  `emacs-lisp-mode` indentation; prettier cannot format Emacs Lisp).

## Running locally

From the repo root, inside the dev shell (`nix develop .#ci`):

```sh
# tests
emacs --batch -Q -l elisp/scripts/run-tests.el
# format check / fix
emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-check
emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-fix
```

Both run automatically as `ert` and `elisp-fmt` steps in `cargo xtask check`
and `cargo xtask validate`, and as the `ert-check` / `elisp-fmt-check` nix
checks. elisp is interim-exempt from the Rust coverage gate (follow-on #82);
write an ERT test for every pure mapping/transform function. See
`docs/adr/0031-elisp-separately-tested-subproject.md`.
```

- [ ] **Step 2: Add the CONTRIBUTING subsection.** Locate the section listing the verify ladder / subproject tooling (search `CONTRIBUTING.md` for the `prettier`/`end2end` mention) and add a sibling subsection nearby:

```markdown
### Elisp subproject (`elisp/`)

The Emacs client lives in `elisp/` (see `elisp/README.md`). Its ERT suite and
formatter run in the verify ladder: `ert` and `elisp-fmt` are `cargo xtask
check`/`validate` steps, mirrored by the `ert-check` / `elisp-fmt-check` nix
checks. prettier cannot format Emacs Lisp, so `elisp-fmt` uses built-in
`emacs-lisp-mode` indentation (auto-fix under `check`, verify under
`validate`). elisp is interim-exempt from the Rust coverage gate (cargo-llvm-cov
is Rust-only; follow-on #82) — instead, write an ERT test for every pure
mapping/transform function. Rationale: `docs/adr/0031-elisp-separately-tested-subproject.md`.
```

- [ ] **Step 3: Verify the docs render and links resolve.**

Run: `nix develop .#ci -c prettier --check elisp/README.md CONTRIBUTING.md` (prettier *does* format Markdown)
Expected: exit 0, or run `prettier -w` then re-check. Confirm `docs/adr/0031-elisp-separately-tested-subproject.md` exists.

- [ ] **Step 4: Commit.**

```bash
git add elisp/README.md CONTRIBUTING.md
git commit -m "docs(elisp): document the elisp subproject in CONTRIBUTING + README"
```

---

### Task 7: Final full-gate verification

**Files:** none (verification only).

- [ ] **Step 1: Run the verify-only gate.**

Run: `nix develop .#ci -c cargo xtask validate --no-e2e`
Expected: exit 0. Confirm via sidecar that `elisp-fmt`, `ert`, and `coverage` are all `"ok": true`: `jq '.ok, (.steps[] | {name, ok})' .xtask/last-result.json`.

- [ ] **Step 2: Review the branch diff against the fork point.**

Run: `git diff wt-base-issue-73..HEAD --stat`
Expected: only `elisp/**`, `flake.nix`, `xtask/src/steps/static_checks.rs`, `CONTRIBUTING.md`, and the already-committed `docs/**` (spec, ADR-0031, README row). No stray files; main untouched.

---

## Self-Review

**Spec coverage:** layout (Task 2) · thin-but-real shared layer w/ auth-source (Task 2) · two host StepSpecs (Task 4) · emacsForCi in flake (Task 1) · two nix checks + coverage denylist (Task 5) · coverage exemption (Task 5 + docs) · ADR-0031 + README row (done in brainstorming) · CONTRIBUTING + elisp/README (Task 6) · harness proves itself with real assertions (Task 2 Step 7, Task 7). All spec sections map to a task.

**Placeholders:** none — every code/edit step carries complete content; the only deliberate stub is the *negative* drift test (Task 3 Step 3), which is restored.

**Type/name consistency:** `jaunder--build-url`, `jaunder--basic-auth-header`, `jaunder--auth-source-spec`, `jaunder--auth-secret`, `jaunder-fmt-fix`, `jaunder-fmt-check` are used identically across the elisp, the ERT tests, the xtask StepSpecs, and the nix checks. Step names `elisp-fmt`/`ert` match between `specs()`, the locked-order array, the arg tests, and the nix check names.
