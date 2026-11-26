# Issue #2103: enforce coverage and e2e execution directly

## Outcome

Rust coverage and e2e verdicts cannot be fetched from a cache in CI, and a
verdict step whose output was not built on the machine running `dev` fails,
naming the verdict. The Rust cache-safety probe and the Nix scaffolding only it
needed are deleted, not ported.

## Why the probe goes

ADR-0192 and ADR-0252 require every tested commit to execute its Rust coverage
and e2e verdicts. Two things enforce that today:

- **Locally:** `nonSubstitutable` (`nix/checks.nix`) marks those derivations
  `allowSubstitutes = false`, so Nix will not fetch them.
- **In CI:** `cachix/install-nix-action` writes
  `always-allow-substitutes = true`, which overrides that marking. Only
  setup-ci's Cachix `pushFilter` (`jaunder-coverage|jaunder-e2e`) keeps the
  results out of the cache.

`source-diagnostics cache-safety-probe` proves structurally that the two line
up. Nothing runs it. Run locally on 2026-11-25 it passed in 60 minutes, almost
all of it in ~233 separate `nix eval`s of source-arm mutations. Those arms check
support outputs' source boundaries. But `cacheBoundary` is `broad`: every
support output already matches the `pushFilter` and is never pushed, so the arms
protect nothing today.

Honoring the marking in CI, plus checking each verdict's outcome, enforces the
rule directly, so the structural proof is no longer needed.

_Contradicts ADR-0252_, which rejected `always-allow-substitutes = false` for
~15 runner-minutes and made the `pushFilter` CI's enforcement. Worth reopening
because that cost was measured while static verdicts still carried the marking.
Re-measured below, it is negligible.

## Approach

1. **Honor the marking in CI.** setup-ci adds `always-allow-substitutes = false`
   to `install-nix-action`'s `extra_nix_config`. Nix then refuses to substitute
   any derivation marked `allowSubstitutes = false`, including nixpkgs' trivial
   builders; everything else is substituted as before.
2. **Check the outcome.** After a verdict step's build, `dev` reads the output's
   `nix path-info` entry. It passes only when `ultimate` is true, meaning this
   machine's store built it; that holds for an earlier local build too, as
   ADR-0252 allows locally. False, missing or unreadable fails, with a message
   naming the verdict as not executed here. Registration time plays no part.
   `jaunder.cache` stays telemetry only, and `nix_build`'s other steps still
   never fail on query errors. The verdict steps are the Rust coverage producer
   and gate (`checks.coverage`, `checks.coverage-gate`) and each e2e lane
   producer (`dev e2e-producer`). Doctests share `_producer_and_gate` but are
   static verdicts and are not checked.
3. **Delete the probe and what only it used:** `cache_safety.rs`, `checkout.rs`,
   `bin/source-diagnostics.rs`, `tests/checkout_admission.rs`, the
   `dev source-diagnostics` front door and its tests, `nix/cache-policy.json`,
   `packages.cache-safety-inventory`, the `cacheSafety*` attrs, and the
   probe-only exposure of `e2eTestDriverPackages` and `e2e-npm-deps`. The
   coverage final/support split and `coveragePartitionAssertion` go only where
   nothing else reads them. **Kept:** `packages.e2e-support` (the upgrade check
   reads it) and `checks.coverage-source-probe` (`coverage-check probe-source`
   uses it). `checkout.rs`'s three tests exercise `repository_policy::checkout`,
   not the probe, so they move into `tools/repository-policy` unchanged apart
   from imports.
4. **Keep the `pushFilter`**, with its comment corrected: it keeps every
   `jaunder-coverage*`/`jaunder-e2e*` output, support included, out of Cachix.
   It is now a second layer, not the enforcement. Narrowing it is a separate
   decision.

**Alternatives.** Porting the probe to Python (this issue's original scope)
keeps a structural proof that takes minutes per run, plus the policy/inventory
scaffolding. It would fail a pushFilter-breaking PR earlier, but with step 1 the
filter no longer carries the rule. The outcome check alone would catch a
substitution only in the run that reuses a pushed result, usually that PR's
merge-queue run. Step 1 stops the fetch in the first place.

**Measured cost of step 1** (draft PR #2118, same Nix inputs as `main`): control
run 45701826384 vs flag run 45705007629. The daemon honored the flag: the flag
run's sqlite-chromium-1-of-3 lane built `nodejs-24.19.0.drv` and
`playwright-browsers.drv` locally, which the control run fetched.

| Job         | Control   | Flag      |
| ----------- | --------- | --------- |
| Hermetic    | 2:16      | 1:34      |
| Test checks | 1:37      | 1:47      |
| Coverage    | 13:45     | 13:57     |
| e2e lanes   | 7:12–8:51 | 6:54–9:00 |
| Whole run   | 14:18     | 14:22     |

The host lane's 6:40 → 8:24 is runner hardware (Xeon 6975P vs EPYC 7763, the
same ~1.4× on every unrelated step); neither host job built anything locally.
ADR-0252 also said `tools-doctests` would lose substitution. That derivation
sets no `allowSubstitutes`, so it would not.

**Accepted losses.**

- The source arms. They become relevant again only if the boundary is narrowed
  to push support outputs.
- The structural assertion that _every_ final (`checks.e2e`,
  `packages.e2e-checks`, the worker-control and single-worker packages) carries
  the marking. The outcome check covers only the verdict steps CI runs. Lanes
  from `mkE2eCombo` stay marked automatically, and a final built anywhere else
  is guarded only by its marking and the `pushFilter` name match.
- Verdicts built on a configured remote builder arrive with `ultimate` false and
  fail. Verdicts must build on the machine running `dev`.
- The probe binary's checkout refusal. The same boundary stays enforced for the
  remaining Rust tools and tested at `tools/repository-policy/tests/native.rs`.

## Acceptance

- `tools/dev` tests:
  - The Rust coverage producer, its gate and an e2e lane producer each fail with
    the naming message when the fake store reports the output as `ultimate`
    false or unreadable.
  - Each passes when the output is `ultimate`, whether built in this run or
    earlier.
  - Doctests' producer and gate still pass when substituted.
  - A test asserts that setup-ci sets `always-allow-substitutes = false`.
  - The fake store's default `path-info` reports built outputs, so existing
    tests keep passing.
- The moved admission tests pass in `tools/repository-policy`.
- `rg` finds no `source-diagnostics`, `cache-safety` or `cacheSafety` in code,
  Nix, workflows, CONTRIBUTING.md or ARCHITECTURE.md.
- The PR's CI is green. Each lane is within 10% of the flag run above, allowing
  for runner hardware.
- **ADRs:**
  - A numberless draft ADR (via `jaunder-adr`) records the new enforcement and
    these measurements, carries forward ADR-0252's static-verdict decision, and
    supersedes ADR-0252 (status → `superseded`).
  - ADR-0246, ADR-0247 and ADR-0248 get `_Amended by #2103:_` notes where they
    name the probe.
  - ADR-0260's `pushFilter` sentence points to the draft.
  - `docs/ARCHITECTURE.md` and `CONTRIBUTING.md` are updated in the same change.

## Boundaries

- No change to which verdicts may be substituted. Static verdicts and Elisp
  coverage (`elisp-coverage-producer`, not covered by ADR-0192) stay
  substitutable. Rust coverage and e2e stay non-substitutable.
- No new structural checker, policy file or CI job.
- Other host-proof binaries and the `tools/host-proof` retirement (#2105) are
  out of scope.
