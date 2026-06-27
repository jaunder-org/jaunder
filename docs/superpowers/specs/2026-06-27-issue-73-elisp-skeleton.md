# Issue #73 — elisp package skeleton, ERT harness, flake + verify-gate wiring

* Status: approved (design), pending implementation
* Deciders: mdorman, Claude
* Date: 2026-06-27
* Milestone: Emacs blogging front-end (#4) — **Infra unit**
* Epic spec: `docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md`
  ("Infra unit" section)
* ADR: `docs/adr/0030-elisp-separately-tested-subproject.md`

## Goal

Give the elisp units a real home in the verify ladder **before** units C (#74) and
D (#75) land. `CONTRIBUTING.md` makes "green → you may move on" an invariant; today
there is **zero elisp** in the repo and **no emacs** in `flake.nix` or CI, so elisp
would otherwise be untested by policy. This unit is pure infrastructure — the
foundation C and D build on — and contains no authoring/publish/reconcile logic of
its own.

## Scope

In scope:

* A new top-level `elisp/` subproject holding one `jaunder.el` package, its ERT
  suite, batch driver scripts, and a README.
* A **thin-but-real** shared layer: a small number of genuinely pure helpers (each
  ERT-tested) plus documented seams (stubs) for the HTTP/auth/mapping work that C
  and D implement.
* `emacs` added to the flake (devShell input as `emacsForCi`).
* Two host `StepSpec`s wired into `cargo xtask check` and `validate`: `elisp-fmt`
  (formatting) and `ert` (tests).
* Two hermetic nix checks mirroring the prettier precedent: `ert-check` and
  `elisp-fmt-check`.
* ADR-0026, a `CONTRIBUTING.md` subsection, and `elisp/README.md`.

Out of scope (each is its own issue):

* HTTP/auth/org↔atom *implementations* — units C/D (#74/#75); the skeleton only
  declares the seams.
* Live-server integration tests (publish/pull round-trip) — e2e-VM tier, separate
  issue.
* Bringing elisp under the coverage gate — p4 follow-on #82.
* Server-side app-password self-provisioning — follow-on #76.

## Design

### 1. Layout — new top-level `elisp/` subproject (mirrors `end2end/`)

```
elisp/
  jaunder.el              ; main package: header, customization group, shared plumbing + C/D seams
  test/
    jaunder-test.el       ; ERT suite covering every pure helper
  scripts/
    run-tests.el          ; CWD-independent ERT batch runner (self-locates via load-file-name)
    format.el             ; jaunder-fmt-fix / jaunder-fmt-check drivers
  README.md               ; what the package is + how to run tests/format
```

`jaunder.el` uses `;;; jaunder.el --- … -*- lexical-binding: t; -*-`, the standard
Commentary / Code / `(provide 'jaunder)` structure, and
`Package-Requires: ((emacs "27.1"))` (only built-in libraries: `url`,
`auth-source`, `ert`, `cl-lib`). All public symbols use the `jaunder-` prefix;
private helpers use `jaunder--`.

The driver scripts are **CWD-independent**: they locate the package relative to
their own file (`load-file-name` / `file-name-directory`), so the same script works
when invoked from the repo root (xtask host StepSpec) and from the nix store
(hermetic check).

### 2. Thin-but-real shared layer

**Pure helpers (each gets an ERT test):**

* `jaunder--build-url` — join a base URL + path segments into a normalized URL
  (collapses duplicate / trailing slashes). Pure.
* `jaunder--basic-auth-header` — `(user pass)` → `("Authorization" . "Basic
  <base64>")`. Pure / deterministic.
* `jaunder--auth-source-spec` — given base-url + username, return the
  `auth-source-search` plist (`:host` derived from the URL, `:user`, `:max 1`).
  Pure.

**Config (`jaunder` customization group):**

* `jaunder-base-url`, `jaunder-username` defcustoms. **No secret in a defcustom.**
* App-password **retrieval** goes through `auth-source` (the standard
  `~/.authinfo(.gpg)` store): the thin wrapper `jaunder--auth-secret` calls
  `auth-source-search` with `jaunder--auth-source-spec` and extracts the secret.
  Minting/storing the password is manual for v1 (server-side self-provisioning is
  follow-on #76); only retrieval is wired here. The I/O wrapper is a seam, not
  unit-tested; the pure spec-builder it depends on is.

**Seams for C/D (stubs that `error` with a pointer to the owning issue; NOT
ERT-tested):** `jaunder--http-request` (a `url.el` wrapper → #74), the org↔atom
mapping seams (→ #74 / #75), and the auth wrapper. These establish the interfaces C
and D extend without faking behavior.

### 3. Gate wiring — two host StepSpecs

In `xtask/src/steps/static_checks.rs`. Both `check` and `validate` call
`static_checks::run`, so a spec added to `specs()` runs in **both** gates
automatically (and is unaffected by `--no-test` / `--no-e2e`, which only gate the
nix coverage / e2e steps).

* **`elisp-fmt`** — `program: "emacs"`, mode-dependent args mirroring `prettier`:
  * `check` (`Mode::Fix`) → `… -f jaunder-fmt-fix`: reindent + strip trailing
    whitespace in place.
  * `validate` (`Mode::Check`) → `… -f jaunder-fmt-check`: fail if any `.el` is not
    canonically formatted.
  * Uses built-in `emacs-lisp-mode` indentation (`indent-region`) +
    `delete-trailing-whitespace` — zero new dependencies. (prettier does **not**
    support Emacs Lisp, so it cannot guard `.el` files; this is the elisp-native
    equivalent.)
* **`ert`** — `program: "emacs"`, same args both modes:
  `--batch -Q -l elisp/scripts/run-tests.el`. The runner loads the package + every
  `test/*-test.el` and calls `ert-run-tests-batch-and-exit` (non-zero exit on any
  failure).

Ordering: both are added to `specs()` after the existing `prettier` step (the other
non-Rust gate), with `elisp-fmt` before `ert`. The `step_order_is_locked` test and
the per-step argument assertions are updated to include both new steps in both
modes.

### 4. Flake (`flake.nix`)

* A single let-binding reused everywhere so the host gate and the hermetic checks
  use the **identical** emacs:

  ```nix
  emacsForCi = pkgs.emacs.pkgs.withPackages (epkgs: [ ]);   # empty now; the extension point is the point
  ```

  (Exact `withPackages` attr path verified against the pinned nixpkgs during
  implementation.) Using `withPackages` rather than bare `pkgs.emacs` future-proofs
  for C/D adding elisp packages via nix in one line. The skeleton needs only
  built-in libraries, so the list starts empty.
* `emacsForCi` is added to `ciInputs` (so the host StepSpecs' `emacs` resolves) and
  reused in the two new checks' `nativeBuildInputs`.
* `emacsSrc = pkgs.lib.cleanSourceWith { src = ./elisp; }`.
* Two new platform-agnostic checks (siblings of `prettier-check`, full parity),
  each a `runCommand` with `nativeBuildInputs = [ emacsForCi ]` ending in
  `touch $out`:
  * `ert-check` — runs the ERT runner over `emacsSrc`.
  * `elisp-fmt-check` — verify-only format check over `emacsSrc`.
* Add `/elisp/` to the **coverage** check's source denylist (the `cleanSourceWith`
  over `./.`) so elisp-only changes don't retrigger Rust coverage rebuilds. elisp is
  coverage-exempt anyway (cargo-llvm-cov instruments Rust only). The main `src`
  allowlist already ignores `.el`, so clippy / rustfmt / deny are unaffected.

### 5. Coverage

elisp is **interim-exempt** from the Rust coverage gate — documented, not wired
(cargo-llvm-cov can't see `.el`). The expectation is stated directly instead: a unit
test for every pure mapping / transform function. Bringing elisp under coverage is
p4 follow-on #82.

### 6. Docs

* **ADR-0030** "Elisp as a separately-tested subproject" — records the host ERT
  StepSpec + nix checks in the verify ladder, emacs-batch indentation for
  formatting, and the coverage exemption with #82 as the follow-on. Plus its row in
  `docs/README.md`.
* **`CONTRIBUTING.md`** — a short subsection: the `elisp/` subproject, how to run its
  tests / format locally, and that ERT runs in `check` / `validate`.
* **`elisp/README.md`** — package overview + run instructions.

## Edge cases / tests

* `jaunder--build-url`: trailing slash on base, leading slash on segment, empty
  segment list, multiple segments — all normalize to one canonical URL.
* `jaunder--basic-auth-header`: known user/pass → known base64; verifies the
  `Authorization` header shape.
* `jaunder--auth-source-spec`: host correctly derived from `https://host:port/...`;
  username threaded through; `:max 1`.
* The harness must prove itself: `cargo xtask validate --no-e2e` is green **with the
  ERT suite actually executing the pure-helper tests** — not a vacuous
  `(should t)` smoke test. The seam stubs are present but not asserted.

## Conventions

Per `CONTRIBUTING.md` and the epic spec's "Testing and conventions": elisp is new
code, host-run ERT wired into `check` and `validate`, interim-exempt from the Rust
coverage gate, with unit tests required for every pure mapping / transform function.
No `Co-Authored-By` trailers. All work lands on
`worktree-issue-73-emacs-elisp-skeleton`, never `main`.
