# ADR-0118: leptosfmt is pinned past its last release

- Status: accepted
- Date: 2026-08-11

## Context

leptosfmt 0.1.33 (2025-01-30) mangles a generic component tag whenever the tag
has to wrap: `<ValidatedInput<Username>` becomes a three-line stanza with broken
indentation. It is cosmetic (it compiles, and leptosfmt is idempotent on its own
output) but it recurs at every generic-component adoption (#420). Upstream fixed
it in PR #167 ("fix: don't break generic params into mulitple lines"), merged
2025-02-02 — three days AFTER 0.1.33 shipped — and nothing has been released
since.

## Decision

The flake overrides `pkgs.leptosfmt` to the post-fix upstream rev. Mechanics
that are easy to get wrong:

- **`src` swap, not `applyPatches`**: PR #167 also bumps a `prettyplease`
  submodule, and a patch cannot move a submodule pointer — the submodule's
  contents are not in the tree the patch would apply to. Replacing `src`
  wholesale drops nixpkgs' `fetchSubmodules`, so it is restated in the override.
- **`cargoDeps` must be overridden too**: nixpkgs passes `cargoHash`, which
  `buildRustPackage` consumes _before_ `overrideAttrs` applies, so the 0.1.33
  vendor tree would survive a bare `src` swap.
- **`importCargoLock`, not `fetchCargoVendor`**: the latter's
  `fetch-cargo-vendor-util` downloads through crates.io's API endpoint, which
  answers **403** here — reproducibly, on a different crate each run (`either`,
  `crop`, `anstyle-query`), so it is the requester being rejected, not any one
  crate. `importCargoLock` uses nix's own `fetchurl` per crate, the same path
  crane already vendors this repo's dependencies through.
- **`version` deliberately stays nixpkgs' "0.1.33"**: the package runs
  `versionCheckHook` against `leptosfmt --version`, and upstream never bumped
  the version after the release. Consequence: the pinned binary is
  indistinguishable from the stock one by `--version`; only behaviour proves
  which is in use.

## 2026-08-14 implementation note

The Rust 1.97.1 pin invalidated the surrounding shell derivation, and clean
GitHub Actions runners then rebuilt the separately pinned `wasm-bindgen-cli`
through `fetchCargoVendor`. Every matrix job reproduced the crates.io API 403
described above, on different crates across attempts (#995). The flake now
shares one adapter for both overrides: Crane fetches each lockfile's packages
from `static.crates.io`, then the adapter flattens Crane's registry-grouped
output and adds the `Cargo.lock` expected by `buildRustPackage`. For
`leptosfmt`, this replaced the original `importCargoLock` mechanism while
preserving its intent: deterministic per-crate fetching without the rejected API
endpoint.

## Consequences

- REMOVE the override once a leptosfmt release later than 0.1.33 exists: drop
  the binding and take `pkgs.leptosfmt` again.
- Anyone formatting with a non-devShell leptosfmt (e.g. `npx`-style
  re-resolution) can produce diffs the gate rejects.
