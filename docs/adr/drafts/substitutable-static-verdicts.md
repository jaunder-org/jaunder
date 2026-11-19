# ADR-DRAFT: Static verdicts may be substituted; coverage and e2e may not

- Status: proposed
- Date: 2026-11-19
- Issue: [#2004](https://github.com/jaunder-org/jaunder/issues/2004)

## Context

`nix/checks.nix` marked the static verdicts as non-substitutable: TypeScript,
rust-format, static-code, Elisp, ERT, the e2e census, dev-python, the
node-modules setup test, the six Clippy leaves and their aggregate. CI never
honoured that marking. `cachix/install-nix-action` always writes
`always-allow-substitutes = true`, which overrides a derivation's
`allowSubstitutes = false`. `cachix-action` pushes everything a job builds
except the `jaunder-coverage|jaunder-e2e` `pushFilter`. So the static stamps
reached Cachix, and run 44383337174 fetched seven of them instead of running
them.

[ADR-0077](../0077-adopt-github-merge-queue.md) kept "never trust a cached green
check" and credited that property to the `pushFilter`.
[ADR-0192](../0192-split-ci-non-e2e-validation-lanes.md) bars only the final
coverage and e2e verdicts from Cachix. Enforcing execution for the static
verdicts as well was measured. The hermetic lane takes 108 s with the stamps
substituted (run 44383337174) and 1333 s with them built (run 44357084032).
`static-code`, whose six Clippy leaves compile without cached artifacts, took
707 s of that, and `dev-python` 84 s. Enforcement would add about 15
runner-minutes to every run whose inputs are unchanged: post-merge push runs,
ADR-promoter PRs, docs-only PRs and reruns.

## Decision

A static verdict is a deterministic function of its inputs. Its derivation is
input-addressed, so the same derivation hash means the same sources, toolchain
and configuration. A passing stamp fetched for that exact derivation is
acceptable evidence. Static verdicts carry no substitution guard.

Coverage and e2e verdicts may never be substituted: they must execute for every
ref. Their `nonSubstitutable` wrapper guards local builds. In CI the setup-ci
`pushFilter` enforces the rule, because the installer's
`always-allow-substitutes = true` overrides the wrapper there. Removing the
`pushFilter` exclusion, or adding a static verdict to it, needs a new decision.

This clarifies ADR-0077's "never trust a cached green check": it applies to
coverage and e2e, whose results depend on runtime behaviour, not to static
verdicts for identical inputs.

## Consequences

- A nondeterministic static check that passed once stays green for those exact
  inputs, and reruns as soon as any input changes.
- The runner keeps install-nix-action's `always-allow-substitutes = true`.
  Trivial nixpkgs builders stay substitutable, and no extra CI cost is incurred.
- The cache-safety probe keeps checking the wrapper on coverage and e2e finals
  and their exclusion by `pushFilter`.
- A `jaunder.cache = substituted` label on a static step is expected, not a
  policy violation.
