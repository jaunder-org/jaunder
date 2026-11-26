# ADR-0252: Static verdicts may be substituted; coverage and e2e may not

- Status: superseded
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

[ADR-0077](0077-adopt-github-merge-queue.md) kept "never trust a cached green
check" and credited that property to the `pushFilter`.
[ADR-0192](0192-split-ci-non-e2e-validation-lanes.md) bars only the final
coverage and e2e verdicts from Cachix.

The rejected alternative was to enforce execution by setting
`always-allow-substitutes = false` on the runner. Its cost was measured from the
hermetic lane, which takes 108 s with its stamps substituted (run 44383337174)
and 1333 s with them built (run 44357084032). Of the 1333 s, `static-code`,
whose six Clippy leaves compile without cached artifacts, took 707 s,
`tools-doctests` 420 s and `dev-python` 84 s. `tools-doctests` was never
wrapped, but the runner-wide setting would also stop its substitution, along
with that of every nixpkgs trivial builder that sets `allowSubstitutes = false`.
The trivial-builder cost was not measured. Enforcement would add about 15
runner-minutes to every run whose inputs are unchanged: post-merge push runs,
ADR-promoter PRs, docs-only PRs and reruns.

## Decision

Static verdicts run in the Nix sandbox and are input-addressed: the same
derivation hash means the same sources, toolchain and configuration. Some of
them, such as ERT and `dev-python`'s pytest, do run tests. A passing stamp
fetched for that exact derivation is still acceptable evidence, because
rerunning identical sandboxed inputs was measured as not worth its cost. Static
verdicts carry no substitution guard.

Coverage and e2e verdicts may never be substituted:
[ADR-0192](0192-split-ci-non-e2e-validation-lanes.md) requires them per ref, and
CI executes them for every ref. Their `nonSubstitutable` wrapper guards local
builds, where a valid local output for an identical derivation may still be
reused. In CI the setup-ci `pushFilter` enforces the rule, because the
installer's `always-allow-substitutes = true` overrides the wrapper there.
Removing the `pushFilter` exclusion, or adding a static verdict to it, needs a
new decision.

This clarifies ADR-0077's "never trust a cached green check": it applies to the
coverage and e2e verdicts that ADR-0192 requires per ref, not to static verdicts
for identical inputs.

## Consequences

- A nondeterministic static check that passed once stays green for those exact
  inputs, and reruns as soon as any input changes.
- The runner keeps install-nix-action's `always-allow-substitutes = true`.
  Trivial nixpkgs builders stay substitutable, and no extra CI cost is incurred.
- The cache-safety probe keeps checking the wrapper on coverage and e2e finals
  and their exclusion by `pushFilter`.
- A `jaunder.cache = substituted` label on a static step is expected, not a
  policy violation.

_Superseded by #2103:_ as of 2026-11-25, CI sets
`always-allow-substitutes = false` and `dev` fails a coverage or e2e verdict
step whose output this machine did not build. The cache-safety probe is retired.
The static-verdict rule carried forward unchanged
([CI honours non-substitutable verdicts](drafts/ci-honours-non-substitutable-verdicts.md)).
