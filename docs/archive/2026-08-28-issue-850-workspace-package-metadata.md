# Issue #850: Workspace-owned package metadata

## Outcome

Each multi-package Cargo workspace owns identical package metadata once, and its
members inherit it. The standalone xtask workspace keeps direct values, while
resolved package metadata remains byte-for-byte equivalent in meaning.

## Load-bearing decisions

- Root `Cargo.toml` owns `version = "0.1.0"`, `edition = "2024"`, and
  `license = "GPL-3.0-only"` under `[workspace.package]`; all nine product
  members inherit those three fields.
- Root publish policy is not shared. `test-support` keeps its direct
  `publish = false`; the other eight packages retain their existing unspecified
  publish state.
- `tools/Cargo.toml` owns `version = "0.1.0"`, `edition = "2024"`, and
  `publish = false`; coverage, devtool, and doctests inherit all three.
- Tools do not acquire license metadata: absence is preserved rather than
  converted into a new policy.
- `xtask/Cargo.toml` keeps direct version, edition, and publish values. Adding
  an inheritance layer to its one-package workspace would increase indirection
  without centralizing another package.
- Resolver, features, dependencies, package names, workspace membership, and
  every non-identical field remain unchanged.
- No ADR is required: this applies Cargo's existing workspace ownership model
  and ADR-0104's unit-edition decision without choosing new metadata values.

## Acceptance

- Root manifests contain one literal each for the shared version, edition, and
  license; all nine product members use the corresponding `.workspace = true`
  keys.
- Tools manifests contain one literal each for shared version, edition, and
  publish state; all three tools members inherit those keys.
- Xtask retains direct version, edition, and publish values, and root
  `test-support` retains its direct publish prohibition.
- `cargo metadata --format-version 1` for root, tools, and xtask produces
  identical complete workspace-member package records before and after the
  manifest rewrite; the focused name/version/edition/license/publish projection
  remains an explicit human-readable summary.
- `Cargo.lock`, `tools/Cargo.lock`, and `xtask/Cargo.lock` remain byte-identical
  to their captured pre-change SHA-256 values.
- The source diff changes package metadata only through the specified workspace
  ownership declarations and member inheritance keys.
- Root, tools, and xtask manifest/check lanes pass; the PR's Validate and E2E
  gates provide the repository boundary proof.
- Architecture documentation names the three ownership points and distinguishes
  inherited fields from deliberate direct exceptions.

## Boundaries

- Do not introduce metadata that is currently absent, including tools licenses
  or root-wide publish policy.
- Do not merge the three Cargo workspaces or make xtask inherit through a
  synthetic parent.
- No Rust source or lockfile behavior changes.
