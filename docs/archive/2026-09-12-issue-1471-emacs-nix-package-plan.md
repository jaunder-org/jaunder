# Emacs Nix Package Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` when delegation is
> useful. This outline exists because issue #1471 adds a public flake output.

## Scope

In:

- Build the production Emacs Protocol Client as a Nix Emacs package.
- Export and document `emacsPackages.${system}.jaunder`.
- Prove package composition, dependency metadata, loading, and platform
  exposure.

Out:

- Overlays, wrapped Emacs applications, `packages` aliases, and package-archive
  publication.
- Protocol Client, server, NixOS module, or transport behavior changes.

## Task outline

- [x] Task 1: Export the installable Emacs package
  - Contract: `nix/packages.nix` owns one `jaunder` Emacs derivation built from
    production modules, version `0.1.0`, with packaged `plz` and pinned `cmark`;
    `flake.nix` exposes it as `emacsPackages.${system}.jaunder` on every default
    system.
  - Verification: build the current-system output; inspect its installed files,
    version, propagated dependencies, and patched `plz` curl path; evaluate the
    output attribute on every default system.
- [x] Task 2: Prove consumption and document the public surface
  - Contract: a wrapped Emacs consuming the exported derivation loads
    `(require 'jaunder)` without repository load paths; documentation uses the
    exact public attribute and Home Manager `extraPackages` form.
  - Verification: run the isolated load smoke scenario,
    `devtool run -- cargo xtask check` before commit, and the final
    `devtool run -- cargo xtask validate --no-e2e` gate.

## Risk checks

- Keep the exported derivation in the same Nixpkgs Emacs package set as its
  `plz` and `cmark` dependencies.
- Exclude `elisp/test/`, `elisp/scripts/`, and documentation from the installed
  package without omitting any production module required by `jaunder.el`.
- Reuse Nixpkgs's immutable `plz` curl binding; do not add redundant curl PATH
  propagation.
- Update `docs/ARCHITECTURE.md` and `elisp/README.md` for the new public output.
- Preserve existing `packages.jaunder`, `packages.site`, and
  `nixosModules.jaunder` behavior.
