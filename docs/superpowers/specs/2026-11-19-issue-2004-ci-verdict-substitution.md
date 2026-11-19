# Issue #2004: CI substitutes "non-substitutable" verdicts

## Outcome

Policy, code and docs agree on one rule. Coverage and e2e verdicts always run on
the CI runner. A static verdict may be fetched from Cachix when its derivation,
and so every input, is identical to one that already passed. The static verdicts
lose the `nonSubstitutable` wrapper that CI never honoured.

## Cause (established)

- `cachix/install-nix-action@v31` (`install-nix.sh:38-40`) always writes
  `always-allow-substitutes = true` into the runner's `nix.conf`. That setting
  makes Nix ignore a derivation's `allowSubstitutes = false`.
- `cachix-action` pushes every path the job builds except
  `pushFilter: "jaunder-coverage|jaunder-e2e"`. The static verdict stamps
  therefore reach Cachix and are fetched on the next run with the same inputs.
- Evidence from run 44383337174 (`validate-hermetic-diagnostics` trace): these
  steps were `substituted`: `static-typescript`, `rust-format`, `static-code`,
  `static-elisp`, `static-e2e-census`, `static-ert`, `dev-python`,
  `tools-doctests`, `csr-bundle`. The first seven are wrapped.
- In CI today, coverage and e2e stay fresh only because of `pushFilter`.

## Decision and tradeoff

**Decision (human, 2026-11-19): accept substitution for static verdicts.** An
input-addressed static check is a deterministic function of its inputs: the same
derivation hash means the same sources, toolchain and configuration. Fetching
its passing stamp is not trusting a stale result. Coverage and e2e stay barred,
as ADR-0192 states and ADR-0077's `pushFilter` rationale requires.

**Rejected alternative, enforcing execution:** set
`always-allow-substitutes = false` on the runner. Measured cost: the hermetic
lane takes 108 s when the stamps are substituted (run 44383337174) and 1333 s
when they are built (run 44357084032). Of the 1333 s, `static-code` with its six
uncached Clippy leaves took 707 s, `dev-python` 84 s and the unwrapped
`tools-doctests` 420 s. That is about 15 extra runner-minutes on every run whose
inputs are unchanged (post-merge push, ADR promoter, docs-only PRs, reruns),
plus an unmeasured cost for nixpkgs trivial builders. It buys nothing that
identical inputs do not already guarantee.

**Accepted cost:** a nondeterministic static check that passed once stays green
for those exact inputs. It reruns as soon as any input changes.

## Changes

- `nix/checks.nix`: remove `nonSubstitutable` from `static-typescript`,
  `rust-format`, `static-code`, `static-elisp`, `static-e2e-census`,
  `static-ert`, `dev-python`, `node-modules-setup`, `clippy-aggregate` and the
  `mkClippyLeaf` leaves. Keep it on the e2e VM tests, `e2e-checks`, `coverage`
  and `coverage-gate`. Rewrite the `nonSubstitutable` comment: it is a local
  guard for coverage and e2e, and in CI `pushFilter` enforces the rule, because
  the installer sets `always-allow-substitutes = true`.
- `.github/actions/setup-ci/action.yml`: in the `pushFilter` comment, state that
  `pushFilter` is the CI enforcement and explain why.
- `CONTRIBUTING.md` (around lines 2166-2240) and `docs/ARCHITECTURE.md` (around
  4833 and 4904): replace each "non-substitutable" claim about a static leaf
  with the rule above.
- An ADR draft (`jaunder-adr`) records the rule, the measurement and why
  enforcement was rejected, and clarifies ADR-0077 lines 70-73. Project it into
  `ARCHITECTURE.md` (`jaunder-adr-projection`).
- `tools/host-proof/src/cache_safety.rs` is unchanged. It checks the wrapper
  only on coverage and e2e finals, which keep it.

## Acceptance

1. `nix show-derivation` for each unwrapped static check shows no
   `allowSubstitutes` override. The coverage and e2e finals still show it, and
   `dev`'s cache-safety probe passes.
2. On the PR's CI run, the coverage trace and an e2e lane trace record `built`
   for coverage and e2e. Static steps may record `built`, `reused` or
   `substituted`.
3. The docs, the comments and the ADR draft state one rule, with no remaining
   "non-substitutable" claim about a static leaf.

## Boundaries

- No change to the runner's Nix configuration, `pushFilter`, `dev`'s cache-label
  logic or the CI topology.
- #1966 (skipping the post-merge push run) stays separate.
