# ADR-DRAFT: Split hermetic static checks at the documentation boundary

- Status: proposed
- Date: 2026-09-04
- Issue: [#1289](https://github.com/jaunder-org/jaunder/issues/1289)

## Context

One broad hermetic `static-checks` derivation causes a Markdown-only change to
stage and execute every static definition, including Rust compilation and the
end-to-end, TypeScript, and Elisp inputs. The controlled warm-baseline
measurement for #1289 recorded that otherwise isolated documentation change as a
292.4-second realization. The command definitions already belong solely to
`devtool`; the unnecessary coupling is the singular Nix derivation boundary.

[ADR-0052](../0052-devtool-unifies-static-checks.md) requires one Nix
`static-checks` runCommand, and
[ADR-0146](../0146-devtool-owns-compiling-static-check-definitions.md) requires
its singular installable and `nix-static-checks` validation step. Those clauses
now prevent the measured boundary from being expressed.

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

## Consequences

Markdown-only changes invalidate the documentation boundary without invalidating
the code-static derivation. The two groups remain an ordered, duplicate-free
partition of the existing static inventory, so host and hermetic callers still
share each command and its arguments exactly once. The code boundary remains
broad by design: a non-Markdown static input continues to invalidate every
static definition that may inspect or compile it.
