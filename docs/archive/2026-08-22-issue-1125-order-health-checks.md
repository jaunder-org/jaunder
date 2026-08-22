# Issue 1125: order xtask health checks

## Outcome

The host/local xtask gates run codebase-health checks in an explicit, documented
order: fast, likely, actionable source-shape failures appear before expensive
compilation and runtime checks where dependencies allow. Future checks have a
clear insertion policy instead of inheriting historical append order.

## Load-bearing decisions

- Ordering policy lives with the xtask gate definitions, not in prose alone. The
  code groups representative health checks by why they run there: precondition,
  formatter/generated consistency, cheap source-shape invariant, compile/type
  check, runtime test, or hermetic Nix check.
- The change is order-only. It must not remove checks, weaken command surfaces,
  alter fail-fast semantics, or move checks into `cargo test` unless the check
  is genuinely one crate/module contract.
- Local hook surfaces prioritize early actionable feedback: clean/staged
  preconditions first, then source-format/generated consistency, then cheap
  deterministic repository-shape checks, then compile/type checks, then host
  runtime tests, then Nix-backed hermetic checks where applicable.
- `cargo xtask validate` may remain exhaustive and hermetic, but its host-local
  prefix should share the same policy so CI and local runs fail early on the
  same cheap host problems before expensive realization.
- Documentation should explain why cross-file, generated-artifact, Git-state,
  multi-language, and hermetic checks remain xtask/Nix health checks rather than
  becoming `cargo test`s.

## Acceptance

- `cargo xtask check --no-test`, `cargo xtask check`, `cargo xtask precommit`,
  and `cargo xtask prepush` preserve their checked surfaces while running
  cheap/actionable health checks before expensive checks when no dependency
  requires otherwise.
- Unit tests assert the relative order of representative steps across the host
  gate and hook surfaces, including at least one precondition,
  formatter/generated-doc check, cheap repository-shape check, compile/type
  check, host runtime test, and Nix-backed check.
- The ordering policy is discoverable from the xtask code and from contributor
  guidance; a new health check has an obvious insertion point.
- Documentation states why some codebase-health checks remain xtask or Nix
  health checks instead of `cargo test`s.
- Existing `cargo xtask check`/`precommit`/`prepush` command semantics stay
  intact: auto-fix behavior, clean/staged-tree behavior, and coverage of host
  tests/Nix checks do not change.

## Boundaries

- Do not implement dependency-aware routing, receipt caching, conservative
  change classification, or fail-fast after the first cheap failure; those are
  separate issues.
- Do not add or remove health checks except for renaming/internal grouping
  needed to express the order.
- Do not change CI’s validate/e2e matrix split.
