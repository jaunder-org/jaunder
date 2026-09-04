# Split flake outputs by concern — implementation outline

> Execute with `jaunder-iterate` and `jaunder-dispatch`. This outline exists
> because four independently implemented files must preserve an explicit lazy
> fixed point and one public flake output schema.

## Scope

In:

- Extract the NixOS/VM, package/source, check/e2e, and development-shell
  concerns into four explicit argument-set functions under `nix/`.
- Reduce `flake.nix` to input declarations, imports, dependency wiring, and
  producer-owned output-fragment assembly.
- Preserve the current public outputs and normalized module/VM contracts.

Out:

- Any behavior from #802, #828, #893, or #276.
- Dependency/hash/policy/tool/CI changes, output renames, and fixed-point
  redesign.
- Generic layer machinery or additional concern files.

## Task outline

- [x] Capture the pre-extraction output and VM contracts.
  - Contract: for every `flake-utils.lib.eachDefaultSystem` system, record each
    `packages`, `checks`, and `devShells` attribute's `drvPath` and each `apps`
    attribute's `{ type, program }`; record the exact baseline failure for an
    unsupported system. Also record module option names/types/defaults and both
    VM configurations' hostnames, forwarded/firewall ports, users, services,
    environment/capture paths, package derivations, launcher commands, and
    system/VM `drvPath`s.
  - Verification: the saved `.xtask/` projections are readable, normalized,
    non-recursive, and distinguish same-name derivation/script drift, attribute
    removal, option/config drift, and baseline evaluation failure without
    becoming tracked fixtures.

- [x] Extract `nix/nixos.nix`.
  - Contract: accept exactly `{ self, nixpkgs }`; return `nixosModules`,
    `nixosConfigurations`, `appsForSystem`, and
    `internals = { captureEnv; e2eOtelCollectorEnv; }`.
    `appsForSystem { system, pkgs }` returns the two existing VM launcher apps
    only on their current Linux/system predicate. Preserve all current
    `self.packages`/`self.nixosModules` references; keep VM system constants and
    runner derivations private to this file.
  - Verification: the normalized module/configuration and `{ type, program }`
    launcher projections equal their baselines on x86_64-linux.

- [x] Extract `nix/packages.nix`.
  - Contract: accept exactly `{ system, pkgs, fenix, crane, atom-fork }`; return
    the ordinary public `packages` fragment and the exact `internals` record in
    “Layer record contracts” below. Preserve source filters, vendoring, offline
    homes, package scripts, pins, comments, and derivation arguments
    semantically; update only relative paths required by the move. Do not
    produce e2e-backed package outputs.
  - Verification: before the required live Rust source-comment migrations, every
    ordinary package `drvPath` equals its baseline and the application source
    tree remains byte-identical despite the new `nix/` directory. In the final
    tree, normalized differences are limited to those comment bytes and their
    source hashes; source-filter/pin-bearing definitions remain single-owned
    here.

- [x] Extract `nix/checks.nix`.
  - Contract: accept exactly
    `{ self, system, pkgs, nixosInternals, packageInternals }`, destructuring
    only the fields listed in “Layer record contracts”; return `checks` plus the
    producer-owned `packages` fragment for e2e aggregate and single-worker
    outputs. Preserve all existing check names, combo order, generated scripts,
    timeout values, `self.checks` and `self.packages` edges, producer/gate
    pairs, and fail-closed behavior.
  - Verification: before the required live source-comment migrations, every
    check and e2e-backed package `drvPath` equals its baseline except
    `static-checks`, whose deliberately broad repository source contains the
    extracted Nix files. In the final tree, normalize derivations to prove
    builders, environments, tools, and commands are unchanged apart from the
    expected repository/application source inputs.

- [x] Extract `nix/dev-shells.nix`.
  - Contract: accept exactly `{ system, pkgs, packageInternals }`, destructuring
    only the fields listed in “Layer record contracts”; return exactly the
    current `ci`, `mutants`, and `default` dev shells. Preserve package lists,
    environment, and shell hook semantically; update only moved relative paths.
  - Verification: all three dev-shell `drvPath`s equal their baselines.

- [x] Migrate source-location consumers to the concern owners.
  - Contract: production gates and tests that parse `flake.nix` source follow
    `e2eSalt`/e2e scripts to `nix/checks.nix` and the application source filter
    to `nix/packages.nix`; live comments and operational
    architecture/contributor citations name the same owners. Flake-installable
    consumers remain unchanged.
  - Verification: focused xtask tests prove the salt gate, source-filter census,
    and e2e capture-order assertions still enforce the same contracts.

- [x] Assemble the four layers in `flake.nix` and prove parity.
  - Contract: root imports the four files, passes named dependencies, merges the
    ordinary and e2e-backed package fragments, and publishes the NixOS-owned
    apps fragment. Root retains no concern implementation. No compatibility
    aliases.
  - Verification: compare every baseline app/module/VM projection and exact
    derivation identity where source content is unchanged. Normalize derivations
    whose source hashes reflect the extracted Nix files or required live
    source-comment migrations. Preserve unsupported-system failures; run the
    repository host/static gate. PR CI remains authoritative for hermetic builds
    and the distributed e2e matrix.

## Layer record contracts

- `nixosInternals` contains exactly `captureEnv` and `e2eOtelCollectorEnv`.
- `packageInternals` contains exactly `visualFontConfig`, `toolchain`,
  `craneLib`, `commonArgs`, `appOfflineCargoHome`, `toolsOfflineCargoHome`,
  `cargoArtifacts`, `leanTestProfile`, `leanDevAndTestProfile`, `jaunderBin`,
  `testSupportBin`, `devtoolBin`, `cargo-crap`, `wasm-bindgen-cli`,
  `wasmTestWebdriverConfig`, `leptosfmt`, `csrWasmBundle`, `e2ePackage`,
  `emacsSrc`, and `emacsForCi`.
- `checks.nix` destructures both `nixosInternals` fields and all
  `packageInternals` fields. `dev-shells.nix` destructures only
  `visualFontConfig`, `toolchain`, `cargo-crap`, `devtoolBin`, `emacsForCi`,
  `leptosfmt`, `wasm-bindgen-cli`, and `e2ePackage`.
- Public fragments are exact: `nixos.nix` returns top-level modules,
  configurations, and per-system VM apps; `packages.nix` returns ordinary
  packages; `checks.nix` returns checks plus e2e-backed packages;
  `dev-shells.nix` returns dev shells. Only `flake.nix` merges them.

## Dependency order and ownership

- NixOS and package extraction may proceed in parallel; each creates only its
  owned file and does not edit `flake.nix`.
- Checks depends on the approved NixOS/package return records. Development
  shells depend only on the approved package return record; those two files may
  then be created in parallel without editing `flake.nix`.
- Source-location consumers may migrate after the four owner files exist; they
  do not edit the owner files or root assembly.
- One integration owner alone rewrites `flake.nix` and performs parity
  verification. The reviewed layer schemas are fixed before delegation; a
  missing field is resolved explicitly rather than by passing a catch-all layer.
  Sibling agents never edit the same file.

## Risk checks

- Preserve `self` as the flake's existing lazy fixed point; do not replace it
  with a new recursive layer graph.
- Keep producer ownership despite output type: VM launch apps come from
  `nixos.nix`; e2e-backed package outputs come from `checks.nix`.
- Preserve ADR-0028 host/sandbox ownership, ADR-0034 e2e matrix/aggregate
  parity, ADR-0052 shared static-check implementation, and ADR-0118 pinned-tool
  vendoring.
- Keep generated test scripts, source filters, attr names/order, constants, and
  comments semantically unchanged; extraction alone must not refresh hashes.
- Each work item reaches `jaunder-commit` only after its iteration evidence; the
  commit hook owns the single `precommit` run. No lint suppression or
  `Co-Authored-By` trailer.
