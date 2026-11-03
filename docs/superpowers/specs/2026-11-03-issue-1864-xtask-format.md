# Proposed #1864 standalone xtask-format verdict

**Status: human-approved design specification; implementation is not authorized
until the separate outline is approved.** Human provenance: request
`088c5c67-cb02-4ba4-8f38-39c2473a95dd` / packet `issue1864-xtask-format-v1`,
exact answer “Approve xtask-format spec,” relayed by Manager. Approval covers
this specification's unchanged substance despite canonical relocation. Clippy
and Rust-format proposals remain unapproved and are not prerequisites.

## Outcome

Expose the repository's existing raw
`cargo fmt --manifest-path xtask/Cargo.toml --check` as its own Nix Verify
terminal verdict, usable by `nix flake check` and hermetic validation. The
derivation runs pinned Cargo/rustfmt only, never `cargo xtask` or the xtask
executable.

## Approach and tradeoffs

Use a distinct raw Cargo/rustfmt function with only xtask formatting targets and
required manifest/config/toolchain inputs. Existing command omits `--all`; Cargo
help defines `--all` as also formatting local path dependencies, so preserve the
command exactly. Locked metadata identifies a one-member xtask workspace;
metadata resolution does not compile. Include xtask Rust source,
`xtask/Cargo.toml`, applicable lock/config/toolchain files, and any
workspace/path-dependency manifests Cargo actually reads for this command. Do
not admit dependency Rust sources unless focused metadata behavior proves they
are needed. Do not compile `devtool`, provision product/tools offline Cargo
homes, or invoke xtask absent proof a specific Cargo metadata requirement needs
them.

This creates an xtask Rust source admission exception to ADR-0146's current
flake-source exclusion, while preserving ADR-0028's rule that Nix never executes
xtask. Reconcile maintained architecture/verification documentation if approved;
no new ADR is presumed. This source boundary is separate from the Leptos
selector's broader Rust discovery closure: each check has its own direct filter
and terminal verdict, even where a Rust source affects both.

## Acceptance and boundaries

After approval, prove that the terminal leaf runs precisely the raw
`cargo fmt --manifest-path xtask/Cargo.toml --check` behavior, does not execute
xtask or depend on compiled devtool/offline homes, admits xtask format targets
and required metadata but excludes path-dependency Rust source, and fails
ordinarily on a deliberate formatting violation. Include the leaf exactly once
in `nix flake check` and hermetic validation; verify relevant source/config/tool
invalidation, valid local-store reuse, and unrelated-input retention only where
the full derivation closure permits. Preserve derivation identity,
request-duration and reuse reporting plus existing execution-timing semantics;
introduce no timing protocol. Do not force rebuilds/cache purges.

Out of scope: host Fix/Git-index changes, hook/CI routing, new aggregate API,
Clippy, formatter selector implementation, generic framework, and new ADR by
default. Independent work may proceed in separate admitted checkouts; coordinate
exact conflicting edits only and rebase actual conflicts. #1872 build-csr
design/Just pin is human-approved (request
`e0d243bd-007d-40cc-b670-5e6e6fe35820`, packet `issue1859-build-csr-spec-v1`,
answer “Approve design and Just pin”); its outline remains pending and is not an
aggregate prerequisite.

## Evidence and explicit ask

The existing command is at `xtask/src/steps/static_checks.rs`; metadata evidence
is `.xtask/evidence/formatter-cargo-populations.json`. No formatter, build,
gate, or Nix work was performed for this proposal.

**Approval record:** the standalone xtask-format design was approved as
specified above. This record does not authorize implementation; the outline
remains a separate approval gate.
