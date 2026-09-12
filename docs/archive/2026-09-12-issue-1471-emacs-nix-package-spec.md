# Issue #1471: Referenceable Nix package for the Emacs client

## Outcome

Jaunder's flake exposes the Emacs Protocol Client as
`emacsPackages.${system}.jaunder`, a standalone Emacs package derivation that a
caller can place directly in an installed-package list such as Home Manager's
`programs.emacs.extraPackages`.

## Load-bearing decisions

- `emacsPackages` is a per-system public flake output, separate from the
  server's `packages.jaunder` output.
- The `jaunder` derivation is exported for every system produced by
  `flake-utils.lib.eachDefaultSystem`; it is not restricted to Linux.
- The derivation contains the production Lisp modules rooted at
  `elisp/jaunder.el`, not the test suite, test runners, or project
  documentation.
- The package version matches the version declared by `elisp/jaunder.el`.
- The package carries `plz` and the pinned `cmark` package as transitive Emacs
  package dependencies.
- The derivation uses Nixpkgs's packaged `plz`, whose override resolves `curl`
  to an immutable Nix store executable. Jaunder does not duplicate that runtime
  dependency with a second PATH-based mechanism.
- User-facing documentation shows the exact flake reference and a Home Manager
  `extraPackages` example.

## Acceptance

- `nix build .#emacsPackages.${system}.jaunder` succeeds on the current system.
- An Emacs environment built with the exported derivation can load
  `(require 'jaunder)` without adding the repository source tree to `load-path`.
- The installed package includes every production Jaunder Lisp module and
  excludes `elisp/test/` and `elisp/scripts/`.
- The derivation version equals the `Version` header in `elisp/jaunder.el`.
- The derivation metadata carries packaged `plz` and pinned `cmark`
  transitively; its `plz` resolves `curl` to a Nix store path, and Jaunder adds
  no independent `curl` PATH propagation.
- The flake output evaluates on each default system, and
  `cargo xtask validate --no-e2e` remains green.
- Architecture and Emacs client documentation identify the output and its
  intended consumption form.

## Boundaries

- This work does not export an overlay, a complete wrapped Emacs application, or
  a second alias under `packages`.
- It does not publish Jaunder to GNU ELPA, NonGNU ELPA, or MELPA.
- It does not change Protocol Client behavior, transport semantics, or the
  server and NixOS package outputs.
