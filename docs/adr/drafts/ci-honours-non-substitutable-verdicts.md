# ADR-DRAFT: CI honours non-substitutable verdicts, and dev checks they ran

- Status: proposed
- Date: 2026-11-25
- Issue: [#2103](https://github.com/jaunder-org/jaunder/issues/2103)

## Context

[ADR-0192](../0192-split-ci-non-e2e-validation-lanes.md) requires every tested
commit to execute its Rust coverage and e2e verdicts.
[ADR-0252](../0252-substitutable-static-verdicts.md) let static verdicts be
substituted and kept coverage and e2e unsubstitutable. `nonSubstitutable`
(`nix/checks.nix`) marks those derivations `allowSubstitutes = false`. But
`cachix/install-nix-action` writes `always-allow-substitutes = true`, which
overrides the marking, so in CI only setup-ci's Cachix `pushFilter`
(`jaunder-coverage|jaunder-e2e`) kept the results out of the cache.

ADR-0252 rejected `always-allow-substitutes = false` as costing about 15
runner-minutes. That was measured while the static verdicts still carried the
marking. The `source-diagnostics cache-safety-probe` proved structurally that
the marking and the `pushFilter` lined up. No gate or CI job ran it. Run locally
on 2026-11-25 it passed in 60 minutes, almost all of it in about 233 `nix eval`s
of source-arm mutations. Those arms checked support outputs' source boundaries,
but every support output already matched the `pushFilter`, so they protected
nothing.

The setting was re-measured on draft PR #2118 (same Nix inputs as `main`):
control run 45701826384 against flag run 45705007629.

| Job         | Control   | Flag      |
| ----------- | --------- | --------- |
| Hermetic    | 2:16      | 1:34      |
| Test checks | 1:37      | 1:47      |
| Coverage    | 13:45     | 13:57     |
| e2e lanes   | 7:12–8:51 | 6:54–9:00 |
| Whole run   | 14:18     | 14:22     |

The daemon honoured the flag: the flag run's sqlite-chromium-1-of-3 lane built
`nodejs` and `playwright-browsers` locally, which the control run fetched. The
host lane's 6:40 against 8:24 came from runner hardware (Xeon 6975P against EPYC
7763, about 1.4× on every unrelated step). Neither host job built anything
locally. `tools-doctests` sets no `allowSubstitutes`, so contrary to ADR-0252 it
stays substitutable.

## Decision

This supersedes ADR-0252 and carries forward its static-verdict rule.

Static verdicts run in the Nix sandbox and are input-addressed. A passing stamp
fetched for that exact derivation is acceptable evidence, and static verdicts
carry no substitution guard. This clarifies ADR-0077's "never trust a cached
green check" as ADR-0252 did: it applies to the coverage and e2e verdicts
ADR-0192 requires per tested commit.

Rust coverage and e2e verdicts may never be substituted, locally or in CI, and
two things enforce it:

- setup-ci sets `always-allow-substitutes = false`, so Nix honours
  `nonSubstitutable` in CI as it does locally.
- After a verdict step builds, `dev` reads the output's `nix path-info` entry
  and fails the step, naming the verdict, unless `ultimate` is true: this
  machine's store built it, in this run or an earlier one. False, missing or
  unreadable fails. The verdict steps are the Rust coverage producer and gate
  and each e2e lane producer. Doctests share the producer-and-gate step but are
  static and unchecked.

The `pushFilter` stays as a second layer that keeps every `jaunder-coverage*`
and `jaunder-e2e*` output, support included, out of Cachix. The cache-safety
probe, `nix/cache-policy.json` and the inventory it read are retired.

## Consequences

- A lost `nonSubstitutable` marking or setting turns CI red at the verdict step
  that fetched its output, instead of passing silently.
- Nixpkgs trivial builders that set `allowSubstitutes = false` build locally in
  CI, at no measured cost.
- Verdicts must build on the machine running `dev`. One built on a configured
  remote builder arrives with `ultimate` false and fails.
- Nothing checks structurally that every e2e final (`checks.e2e`,
  `packages.e2e-checks`, the worker-control and single-worker packages) carries
  the marking. Lanes from `mkE2eCombo` stay marked automatically, and a final
  built anywhere else is guarded only by its marking and the `pushFilter` name.
- Source-boundary checks of support outputs return only if a future decision
  narrows the `pushFilter` to push them.
- Elisp coverage (`elisp-coverage-producer`), which ADR-0192 does not cover,
  stays substitutable.
