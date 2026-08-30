# ADR-0161: devtool owns ast-grep rule enforcement

- Status: accepted
- Date: 2026-08-29
- Issue: [#893](https://github.com/jaunder-org/jaunder/issues/893)

## Context

[ADR-0076](0076-no-full-load-spa-navigation.md) forbids in-app full document
loads and originally enforced that navigation policy with an xtask-owned source
scanner. That scanner matched source lines rather than Rust syntax, so a
forbidden call chain could evade it when formatting split the chain across
lines. The replacement is a native ast-grep rule with committed behavior
fixtures.

The rule is a non-compiling static check: its policy is exactly the four
navigation methods `replace`, `assign`, `reload`, and `set_href` when chained
from the result of `.location()` in Rust under `web/src` and `client/src`.
Location inspection, `use_location()`, and the pre-paint JavaScript redirect
remain outside the rule. A representative repository scan measured about 32 ms,
so running the native rule tests alongside the scan adds negligible gate cost.

[ADR-0052](0052-devtool-unifies-static-checks.md) and
[ADR-0146](0146-devtool-owns-compiling-static-check-definitions.md) establish
that devtool owns static-check command definitions shared by host xtask and
hermetic Nix lanes. Leaving ast-grep's native rule tests as fixtures without a
caller would leave the rule's behavior unvalidated, while recreating the command
in xtask or Nix would reintroduce the ownership drift those decisions prevent.

## Decision

`devtool check` owns both the `no-full-reload` repository scan and an
`ast-grep-tests` check that runs `ast-grep test --config sgconfig.yml`. The
repository scan remains the canonical `no-full-reload` check and invokes
ast-grep with `--filter ^no-full-reload$`; the rule-test check does not
duplicate or redefine that matching policy.

`devtool check --all` includes each check once. Host xtask delegates both
through its source-consistency inventory, and the Nix `static-checks` derivation
reaches the same definitions through `devtool check --all --sandbox-cargo`. Thus
host and Nix share command definitions while retaining their established
execution environments.

This **amends [ADR-0076](0076-no-full-load-spa-navigation.md)** only to replace
its enforcement mechanism: a devtool-owned native ast-grep rule and its native
rule tests supersede the xtask source scanner. ADR-0076's navigation policy,
namespace decision, scope, and recovery guidance remain unchanged.

## Consequences

- Formatting cannot evade the four-method navigation policy, and committed
  fixtures are validated by ast-grep itself in every `--all` lane.
- The approximately 32 ms repository scan and native rule tests add a small,
  accepted non-compiling check cost in exchange for syntax-aware enforcement.
- Future changes to this policy update the one ast-grep rule and its fixtures;
  callers continue to invoke devtool rather than copying arguments or scanners.
- `no-full-reload` remains the actionable repository diagnostic, while
  `ast-grep-tests` reports rule-fixture failures.
