# ADR-0031: Elisp as a Separately-Tested Subproject

- Status: accepted
- Deciders: mdorman, Claude
- Date: 2026-06-27

## Context and Problem Statement

The Emacs blogging front-end (milestone #4, units C/#74 and D/#75) introduces
the repository's first Emacs Lisp. `CONTRIBUTING.md` makes "green → you may move
on" an invariant, but the verify ladder and the coverage gate are Rust-shaped:
`cargo xtask check` / `validate` drive Rust static checks + nix coverage/e2e,
and cargo-llvm-cov instruments Rust only. With no emacs in `flake.nix` and no
elisp runner anywhere, elisp would ship **untested by policy**. We need a home
for elisp in the verify ladder before C/D land (the Infra unit, #73), and a
decision on how elisp is tested, formatted, and counted (or not) toward
coverage.

## Decision Drivers

- "Green → you may move on" must mean something for elisp, not just Rust.
- Reuse the existing precedent for a non-Rust subproject rather than invent a
  new mechanism — `end2end/` (JS/Playwright) is already gated by a host
  `prettier` StepSpec plus a hermetic `prettier-check` nix derivation.
- No new heavyweight dependencies; the toolchain must be cachix-pulled and
  reproducible.
- Don't distort the Rust coverage gate with files it cannot instrument.

## Decision Outcome

**Treat elisp as a first-class but separately-tested subproject, mirroring the
`end2end/` precedent.**

1. **Location.** A top-level `elisp/` subproject holds one `jaunder.el` package,
   its ERT suite (`test/`), CWD-independent batch driver scripts (`scripts/`),
   and a README — the same shape as `end2end/`.
2. **Tests run in the verify ladder.** A host `ert` `StepSpec`
   (`xtask/src/steps/static_checks.rs`) runs the pure ERT unit tests under
   `emacs --batch`; because both `check` and `validate` call
   `static_checks::run`, the suite runs in **both** gates automatically. A
   hermetic `ert-check` nix derivation (sibling of `prettier-check`) mirrors it
   for `nix flake check`.
3. **Formatting is enforced too.** prettier does **not** support Emacs Lisp, so
   a host `elisp-fmt` StepSpec uses built-in `emacs-lisp-mode` indentation +
   `delete-trailing-whitespace` — fixing in `check` (`Mode::Fix`), verifying in
   `validate` (`Mode::Check`) — paralleling prettier's `-w` / `--check` duality.
   A hermetic `elisp-fmt-check` derivation mirrors the verify side.
4. **One emacs everywhere.**
   `emacsForCi = pkgs.emacs.pkgs.withPackages (epkgs: [ ])` is added to
   `ciInputs` and reused in both checks' `nativeBuildInputs`, so the host gate
   and the hermetic checks use the identical toolchain. `withPackages` (vs bare
   `pkgs.emacs`) is the extension point for C/D to add elisp packages via nix in
   one line; the list starts empty because the skeleton needs only built-in
   libraries.
5. **Coverage exemption.** elisp is interim-exempt from the Rust coverage gate
   (cargo-llvm-cov is Rust-only). The expectation is stated directly instead: a
   unit test for every pure mapping / transform function. `elisp/` is added to
   the coverage check's source denylist so elisp-only changes don't retrigger
   Rust coverage rebuilds. Bringing elisp under coverage is p4 follow-on
   **#82**.

Rejected: a bare `pkgs.emacs` (loses the nix package extension point C/D will
need); a heavier reformatter such as `elisp-autofmt` (adds a packaged dependency
and is more opinionated than the canonical indentation the built-in mode already
enforces); folding elisp into the Rust coverage gate now (cargo-llvm-cov cannot
see it — tracked as #82 instead).

## Consequences

- Good: elisp is tested and formatted by the same `cargo xtask check` /
  `validate` invocation developers already run, and by `nix flake check`;
  "green" now covers elisp.
- Good: the `end2end/` precedent is reused wholesale — no new gate mechanism.
- Good: C/D extend a proven harness; adding an elisp package is a one-line
  `withPackages` edit.
- Bad: elisp correctness rests on ERT discipline (a test per pure function)
  rather than an enforced coverage number until #82 lands.
- Bad: built-in indentation enforces canonical whitespace/indentation but not
  full reformatting (line wrapping/alignment); acceptable as the elisp norm,
  revisitable if it proves insufficient.
