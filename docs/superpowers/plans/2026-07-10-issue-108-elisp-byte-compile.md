# Plan — issue #108: elisp byte-compile with warnings-as-errors in the verify gate

**Spec:**
[`docs/superpowers/specs/2026-07-10-issue-108-elisp-byte-compile.md`](../specs/2026-07-10-issue-108-elisp-byte-compile.md)
**For agentic workers:** drive with **`jaunder-iterate`** (delegate a task to a
subagent via **`jaunder-dispatch`** when useful); commit with
**`jaunder-commit`**.

## Review header

**Goal:** add a `byte-compile` step to the elisp verify gate that compiles the
nine package modules with byte-compiler warnings promoted to errors, wired like
the existing `ert`/`elisp-fmt` steps. First clear the 7 latent docstring-width
warnings so the tree compiles clean.

**Scope**

- _In:_ re-wrap 7 over-wide docstrings; add `elisp/scripts/byte-compile.el`;
  register `byte-compile` in `devtool` (`check.rs`) and `xtask`
  (`static_checks.rs`, incl. the locked step-order test); update the "7 → 8
  checks" doc comments.
- _Out:_ compiling `test/`/`scripts/`; any new Nix derivation (the existing
  `static-checks` picks the step up via `devtool check --all`); any runtime
  behaviour change.

**Tasks**

1. Re-wrap the 7 over-wide docstrings so the package byte-compiles clean.
2. Add `elisp/scripts/byte-compile.el` (self-locating; error-on-warn; temp
   dest).
3. Register `byte-compile` in `devtool::check` and `xtask::steps::static_checks`
   (+ locked-order test + doc-comment counts), then verify the full gate.

**Key risks / decisions**

- **Docstring wrapping must stay behaviour-preserving:** an elisp docstring is a
  literal multi-line string, so continuation lines must be **flush to column 0**
  — indenting them injects spaces into the string. `indent-region` (the
  `elisp-fmt` engine) does not reindent inside string literals, so flush-left
  continuations survive formatting. Don't change any words.
- **`elisp-fmt` globs `elisp/` recursively** (`directory-files-recursively` in
  `format.el`), so both the edited modules _and_ the new
  `scripts/byte-compile.el` must be canonically formatted or the `elisp-fmt`
  step goes red.
- **No `.elc` in the tree:** the script routes output to a `make-temp-file` dir
  and deletes it; `git status` must be clean after a run.
- **`(require 'bytecomp)` before binding `byte-compile-dest-file-function`** —
  otherwise a lexical/dynamic conflict errors out (spec §Design gotcha).

## Global constraints

- **No `Co-Authored-By` trailer** on commits.
- Every commit must pass the pre-commit gate: run `cargo xtask check` clean
  first (**`jaunder-commit`**). Serialize edit → gate → commit (no edits while a
  gated commit runs).
- Rust edits obey `CONTRIBUTING.md` (fmt, clippy `-D warnings`). devtool lives
  in the `tools/` virtual workspace; xtask in the `xtask/` root-package
  workspace — test each via `--manifest-path`, not `-p` from the main workspace.

---

## Task 1 — Re-wrap the 7 over-wide docstrings

**Files (edit):**

- `elisp/jaunder-config.el` — docstring at line 41 (`jaunder--blog-entry-for`)
- `elisp/jaunder-media.el` — lines 74 (`jaunder--collect-media-links`) and 89
  (`jaunder--substitute-media`)
- `elisp/jaunder-org.el` — line 203 (`jaunder--org-substitute-links`)
- `elisp/jaunder-publish.el` — line 129 (`jaunder-new-post`)
- `elisp/jaunder-transport.el` — lines 84 (`jaunder--curl-header-value`) and 94
  (`jaunder--http-request`)

**Change:** for each, wrap the docstring so every physical line is ≤ 80 columns.
Break at word boundaries; continuation lines start at **column 0** (flush left);
preserve the exact wording. First line stays a complete sentence (elisp
convention). Read each function's current docstring before editing — line
numbers shift as earlier edits land, so re-locate by function name, not by
number.

**Verify (each expected to PASS):**

- Package compiles clean (no artifacts left):
  ```
  emacs --batch -Q --eval '(progn (require (quote bytecomp)) \
    (let* ((root (expand-file-name "elisp")) \
           (tmp (make-temp-file "bc" t)) \
           (byte-compile-dest-file-function \
            (lambda (s) (expand-file-name (concat (file-name-nondirectory s) "c") tmp))) \
           (byte-compile-error-on-warn t) (byte-compile-warnings t) (ok t)) \
      (add-to-list (quote load-path) root) \
      (dolist (f (directory-files root t "\\.el\\x27")) (unless (byte-compile-file f) (setq ok nil))) \
      (delete-directory tmp t) (unless ok (kill-emacs 1))))'
  ```
  (exit 0; this is the exact logic Task 2 packages into a script.)
- Existing elisp gate still green:
  `cargo run --quiet --manifest-path tools/Cargo.toml -p devtool -- check elisp-fmt`
  and `... check ert`.
- `git status` shows only the six edited `.el` files, no `.elc`.

**Commit:**
`fix(elisp): wrap over-wide docstrings so the package byte-compiles clean (#108)`

---

## Task 2 — Add `elisp/scripts/byte-compile.el`

**Files (new):** `elisp/scripts/byte-compile.el`

Mirror `elisp/scripts/run-tests.el`'s self-location. Final content:

```elisp
;;; byte-compile.el --- byte-compile the jaunder package, warnings-as-errors -*- lexical-binding: t; -*-

;;; Commentary:
;; Byte-compiles every package module (the flat elisp/*.el files) with all
;; byte-compiler warnings promoted to errors, so any warning fails the gate.
;; Output goes to a throwaway temp dir — no .elc is left in the tree.
;; Self-locating via `load-file-name' so it works from the repo root (the
;; `byte-compile' step, via `devtool check byte-compile') and from the nix
;; store (the `static-checks' derivation, via `devtool check --all').

;;; Code:

;; Require bytecomp before let-binding its options: under lexical-binding,
;; binding `byte-compile-dest-file-function' before the library defines it as a
;; special variable errors with "Defining as dynamic an already lexical var".
(require 'bytecomp)

(let* ((this (file-name-directory
              (or load-file-name buffer-file-name default-directory)))
       (root (file-name-directory (directory-file-name this)))
       (dest (make-temp-file "jaunder-bytecomp" t))
       (byte-compile-dest-file-function
        (lambda (src)
          (expand-file-name (concat (file-name-nondirectory src) "c") dest)))
       (byte-compile-error-on-warn t)
       (byte-compile-warnings t)
       (ok t))
  (add-to-list 'load-path root)
  ;; Package modules only: the flat elisp/*.el files (scripts/ and test/ are
  ;; subdirectories, so a non-recursive listing excludes them).
  (dolist (f (directory-files root t "\\.el\\'"))
    (unless (byte-compile-file f)
      (setq ok nil)))
  (delete-directory dest t)
  (unless ok (kill-emacs 1)))

;;; byte-compile.el ends here
```

Notes: `byte-compile-file` returns nil on any warning (because
`byte-compile-error-on-warn`), and the compiler auto-prints the offending
`file:line` to stderr — so no explicit reporting is needed. Compiling all files
before exiting means one run flags every offending module.

**Verify:**

- Runs clean on the (now-fixed) tree:
  `emacs --batch -Q -l elisp/scripts/byte-compile.el` → exit 0.
- Leaves no artifact: `git status` shows only the new script, no `.elc`.
- Canonical formatting (the recursive `elisp-fmt` will check this file):
  `cargo run --quiet --manifest-path tools/Cargo.toml -p devtool -- check elisp-fmt`
  → PASS.
- **Negative check:** temporarily append a stray char to a docstring to make it
  > 80 cols; rerun the script → **exit 1** naming that file; revert.

**Commit:**
`feat(elisp): byte-compile.el — compile the package with warnings-as-errors (#108)`

---

## Task 3 — Register the `byte-compile` step (devtool + xtask)

**Files (edit):**

### `tools/devtool/src/check.rs`

- `ALL` (lines 17–25): insert `"byte-compile"` immediately after `"ert"`.
- `spec()` (after the `"ert"` arm, ~line 86): add
  ```rust
  "byte-compile" => (
      "emacs",
      owned(&["--batch", "-Q", "-l", "elisp/scripts/byte-compile.el"]),
  ),
  ```
- Doc comments: bump "The 7 non-compiling static checks" (line 1) and "The 7
  non-compiling checks devtool owns" (line 12) to **8**. In the `spec()` doc
  (lines 27–30), note that `byte-compile` (like `ert`/`tsc`) has no autofix.
- Tests: extend `ert_and_tsc_ignore_fix` (or add a sibling) to assert
  `spec("byte-compile", true) == spec("byte-compile", false)`.
  `all_names_have_specs` covers the new name automatically.

### `xtask/src/steps/static_checks.rs`

- `specs()`: add `devtool_check("byte-compile", mode)` immediately after the
  `devtool_check("ert", mode)` line (~line 44), keeping the elisp checks
  grouped.
- `step_order_is_locked` test (lines 252–272): insert `"byte-compile"` after
  `"ert"` in the `expected` array.
- Doc comment (lines 17–23): bump the "7 non-compiling checks" count to 8 and
  add `byte-compile` to the parenthetical list.

**Verify (each PASS):**

- `cargo nextest run --manifest-path tools/Cargo.toml -p devtool` (spec +
  order).
- `cargo nextest run --manifest-path xtask/Cargo.toml` (locked order).
- `cargo run --quiet --manifest-path tools/Cargo.toml -p devtool -- check byte-compile`
  → exit 0.
- **Negative check:** reintroduce an over-wide docstring; `cargo xtask check`
  fails at the `byte-compile` step naming the file; revert.
- Full host gate: `cargo xtask check` green (the new step runs between `ert` and
  `cargo-deny`).

**Commit:** `feat(xtask): add elisp byte-compile step to the verify gate (#108)`

---

## Ship (via `jaunder-ship`, after the loop)

- Final two-axis review (Standards + Spec) + a cold blind review of the diff.
- `cargo xtask validate` green (confirms the `static-checks` Nix check runs the
  new step hermetically via `emacsForCi`, which already carries `plz`).
- Archive this plan + the spec, push, open the PR — PR body uses a **per-issue
  `Closes #108`** line (not a comma list), push, merge; release the project item
  to **Done**.
