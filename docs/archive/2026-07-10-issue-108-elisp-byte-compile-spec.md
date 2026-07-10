# Spec — issue #108: elisp byte-compile with warnings-as-errors in the verify gate

- **Issue:** #108 (Milestone 4 — Emacs blogging front-end), label `dx`
- **Foundation:** #73 (elisp skeleton), ADR-0030 (elisp as a separately-tested
  subproject), ADR-0052 / #188 (devtool owns the check tool+args; xtask only
  orders them).

## Problem

The `elisp/` verify-gate steps (`ert`, `elisp-fmt`) only **load** the package;
they never **byte-compile** it. So the byte-compiler's static warnings — unused
`require`, undefined/free variables, missing docstrings, mismatched arglists,
lexical-binding issues — are invisible to the gate. This gap was what let the
dead `(require 'url)` / `(require 'cl-lib)` in #73 slip past tooling and be
caught only by eye. Now that the client is nine real modules with cross-module
`require`s, byte-compile warnings are the cheapest structural check we're not
running.

## Goal

Add a **byte-compile** step to the elisp verify gate that compiles the package
modules with **warnings promoted to errors**, so any byte-compiler warning fails
the gate. Wire it exactly like the existing `ert` / `elisp-fmt` steps so it runs
in `cargo xtask check`, `validate`, and the hermetic `static-checks` Nix check
with no separate Nix plumbing.

## Design

Three touch points, mirroring the `ert` step's wiring end-to-end.

### 1. New script: `elisp/scripts/byte-compile.el`

Follows the self-locating pattern of `elisp/scripts/run-tests.el`:

- Derive `root` = the `elisp/` directory (parent of `scripts/`) from
  `load-file-name`; `(add-to-list 'load-path root)` so every internal
  `(require 'jaunder-…)` resolves. (The only third-party require, `plz` in
  `jaunder-transport.el`, is supplied by `emacsForCi` — see §Load-path.)
- Set warnings-as-errors and full warnings:
  - `(setq byte-compile-error-on-warn t)`
  - `(setq byte-compile-warnings t)`
- Route `.elc` output into a throwaway temp directory so the compile leaves
  **zero** artifacts in `elisp/` (keeps the working tree clean; avoids the nix
  untracked-files / dirty-tree hazards). Implemented by binding
  `byte-compile-dest-file-function` to map each source basename into a
  `make-temp-file`-created directory (removed on exit).
  - **Gotcha (verified):** `(require 'bytecomp)` must run _before_ binding
    `byte-compile-dest-file-function`. Under `lexical-binding`, `let`-binding
    that variable before `bytecomp` is loaded makes emacs treat it as lexical,
    and the later `custom-declare-variable` in `bytecomp` then errors with
    "Defining as dynamic an already lexical var". Requiring `bytecomp` up front
    avoids it.
- **Scope: package modules only** — the flat `elisp/*.el` files (the nine
  `jaunder-*.el`). This is exactly `(directory-files root t "\\.el\\'")`, since
  `scripts/` and `test/` live in subdirectories. Compile each; collect any that
  warn/error; `kill-emacs` non-zero if any failed, zero otherwise (compile all
  files before exiting, so one run reports every offending module, not just the
  first).

### 2. devtool: `tools/devtool/src/check.rs`

`devtool` is the single source of truth for each check's program + args
(ADR-0052). Add `"byte-compile"` to the `ALL` list and a `spec()` arm:

- program `emacs`, args
  `["--batch", "-Q", "-l", "elisp/scripts/byte-compile.el"]`.
- The step ignores `--fix` (there is nothing to auto-fix — a warning must be
  fixed by hand), like the `ert` arm.

### 3. xtask: `xtask/src/steps/static_checks.rs`

- Register the step via the existing `devtool_check("byte-compile", mode)`
  helper, placed adjacent to the `elisp-fmt` / `ert` registrations so the elisp
  checks stay grouped.
- Update the **locked step-order test** to include `"byte-compile"` at its
  chosen position in the ordered list.

### 4. Nix — no change

The `static-checks` derivation runs `devtool check --all`, so it picks up the
new step automatically. `emacsForCi` already provides `plz` on the load-path and
`TZDIR` is already set. The step is hermetic for free.

## Load-path / dependencies

- A single load-path entry — the `elisp/` root — resolves all nine flat
  `jaunder-*.el` modules and their internal cross-`require`s.
- Built-in requires (`org`, `org-element`, `org-attach`, `cl-lib`,
  `auth-source`, `seq`, `url-parse`, `dom`) come with emacs. Byte-compiling
  `jaunder-org.el` / `jaunder-datetime.el` therefore also exercises the `org`
  load path.
- The one third-party require, `plz` (`jaunder-transport.el`), is provided by
  `emacsForCi` (`emacs.pkgs.withPackages [ epkgs.plz ]`) — the same binary the
  host gate and the Nix check both use, so they can't diverge.

## Prerequisite: the package must currently byte-compile clean

Turning warnings into errors only works if the package is already warning-free.
ADR-0042 claims `jaunder.el` byte-compiles clean, but that predates the
nine-module split. **A probe run of the full nine-module compile (all warnings
on, error-on-warn off) surfaced exactly 7 warnings across 6 modules, all the
same trivial kind and no structural ones:**

| File                   | Line | Warning                            |
| ---------------------- | ---- | ---------------------------------- |
| `jaunder-config.el`    | 41   | docstring wider than 80 characters |
| `jaunder-media.el`     | 74   | docstring wider than 80 characters |
| `jaunder-media.el`     | 89   | docstring wider than 80 characters |
| `jaunder-org.el`       | 203  | docstring wider than 80 characters |
| `jaunder-publish.el`   | 129  | docstring wider than 80 characters |
| `jaunder-transport.el` | 84   | docstring wider than 80 characters |
| `jaunder-transport.el` | 94   | docstring wider than 80 characters |

No dead-require, free-variable, or arglist warnings appeared. So the cleanup is
**re-wrapping 7 docstrings to ≤80 columns** — purely cosmetic, behaviour-
preserving — after which the step goes green. These fixes belong in this change.

A second probe confirmed the runtime behaviour the script relies on: under
`byte-compile-error-on-warn t`, `byte-compile-file` **returns nil** on a warning
(it neither signals nor aborts the loop) and the byte-compiler auto-prints the
`file:line: Error: …` diagnostic to stderr. So the script just checks each
return value and exits non-zero if any file failed — no `condition-case`, no log
parsing. (It reports the first offending line per file; fix-and-re-run surfaces
any subsequent one.)

## Non-goals

- Byte-compiling `test/*.el` or `scripts/*.el` (batch drivers) — package modules
  only, per the issue's scope.
- A separate/standalone Nix check derivation — the existing `static-checks`
  derivation already covers it via `devtool check --all`.
- Emitting or committing `.elc` artifacts — the temp-dir dest guarantees none
  are produced.
- Any runtime/behavioural change to the client.

## Verification

- `devtool check byte-compile` (and via `cargo xtask check`) **passes** on the
  clean tree, and the working tree is **unchanged afterward** (no `.elc` left).
- **Negative check:** temporarily reintroduce a warning (e.g. an unused
  `(require 'url)` in a module) and confirm the step **fails** with non-zero
  exit and names the offending file; then revert.
- The locked step-order test passes with `"byte-compile"` in the list.
- Full `cargo xtask validate` green (the `static-checks` Nix check runs the new
  step hermetically).

## Risks

- **Latent warnings** in the current modules would make the step red on first
  run — expected; fixing them is in scope (see Prerequisite).
- **Cross-module compile order:** compiling a module whose sibling `require`s
  aren't yet compiled loads the sibling's _source_ from the load-path (temp dest
  keeps `.elc` off the load-path), which is correct; no ordering constraint is
  imposed.
