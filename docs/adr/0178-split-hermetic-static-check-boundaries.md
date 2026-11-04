# ADR-0178: Split hermetic static checks at the documentation boundary

- Status: accepted
- Date: 2026-09-04
- Issue: [#1289](https://github.com/jaunder-org/jaunder/issues/1289)

## Context

One broad hermetic `static-checks` derivation causes a Markdown-only change to
stage and execute every static definition, including Rust compilation and the
end-to-end, TypeScript, and Elisp inputs. The controlled warm-baseline
measurement for #1289 recorded that otherwise isolated documentation change as a
292.4-second realization. The command definitions already belong solely to
`devtool`; the unnecessary coupling is the singular Nix derivation boundary.

[ADR-0052](0052-devtool-unifies-static-checks.md) requires one Nix
`static-checks` runCommand, and
[ADR-0146](0146-devtool-owns-compiling-static-check-definitions.md) requires its
singular installable and `nix-static-checks` validation step. Those clauses now
prevent the measured boundary from being expressed.

## Decision

Replace the singular hermetic static derivation with two Nix checks:

- `static-docs` runs `devtool check --group docs` over Markdown plus only
  Prettier's required configuration and ignore files.
- `static-code` runs `devtool check --group code --sandbox-cargo` over every
  non-Markdown input required by the existing static group, retaining the
  configuration, tool, source, offline Cargo, timezone, and end-to-end Node
  inputs that those checks require.

`cargo xtask validate` builds `nix-static-docs`, then `nix-static-code`, before
the existing Nix test checks. There is no aggregate installable or compatibility
step.

This narrowly supersedes ADR-0052's one-Nix-derivation clause and ADR-0146's
singular `static-checks`/`nix-static-checks` requirement. Their decisions that
`devtool` owns shared host and sandbox command definitions, that sandbox Cargo
uses workspace-specific offline homes, and that hermetic static checks precede
Nix test checks remain in force.

## Scoped Rust-format refinement (#1864)

The shared Rust-format checks now have their own `rust-format` leaf, running
`devtool check --group rust-format --sandbox-cargo` in the established order
`fmt`, `leptosfmt`, `tools-fmt`. They are removed from residual `code` exactly
once; `--all` retains its prior order. Product/tools Cargo formatting
populations overlap, so they stay in one verdict. The named Leptos formatter is
bounded to `web/src` and `csr/src` in host Fix/Verify and Nix Verify, with
checked enumeration and promotion of the pinned CLI's per-file errors. This is
not an automatic syntax selector or a placement rule.

Hermetic validation consumes docs, Rust-format, then residual code before test
checks; flake checks expose each leaf once. This narrowly refines this ADR's
original two-group partition and broad-code-only clauses. Shared devtool command
ownership, offline workspace homes, and host-only xtask execution remain intact.
The Rust-format direct source filter does not narrow its transitive
`devtoolBin`, offline-home, vendor or toolchain closure. Valid local reuse is
allowed; neither faster execution nor full cache isolation is promised.

## Consequences

Markdown-only changes invalidate the documentation boundary without invalidating
the code-static derivation. The groups remain a duplicate-free partition of the
existing static inventory, while the host `--all` order is preserved
independently of leaf boundaries. Host and hermetic callers still share each
command and its arguments exactly once. The residual code boundary remains broad
by design.
