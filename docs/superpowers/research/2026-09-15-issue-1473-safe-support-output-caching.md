# Issue #1473: Safe support-output caching decision

## Decision

Retain the broad Cachix exclusion, `jaunder-coverage|jaunder-e2e`.

The structural candidate is safe: the generated inventory partitions every
coverage and end-to-end output, final derivations are non-substitutable, and
admitted support closures contain no exact final output or derivation identity.
The candidate nevertheless does not satisfy the approved measurement rule.
Jaunder has one shared Cachix namespace, while `pushFilter` controls uploads but
not downloads. A treatment run can therefore populate support paths that a later
baseline run will consume. The four required matched cache-state cells cannot be
isolated with the repository's current CI and cache capabilities. Missing cells
disqualify narrowing, so no performance claim is made.

The machine policy records `cacheBoundary: "broad"`. The cache-safety probe
continues to prove the candidate closures and source boundaries, and also checks
that the selected broad filter excludes every cataloged support and final
identity.

## Safety evidence

The staged implementation passed:

- `devtool run -- cargo xtask cache-safety probe` in 746,022 ms;
- `devtool run -- cargo xtask check --no-test` in 222,627 ms;
- 19 focused `cache_safety` tests;
- the repository pre-commit gate when commit `3af6f6fb` was created.

The probe freezes the Git index once and uses that immutable detached snapshot
for the policy, generated inventory, output and derivation identities, support
builds, closure checks, and every source mutation. It inspects actual final
`nix show-derivation` metadata for `allowSubstitutes = false` and
`preferLocalBuild = true`. Name matching is used only to reconcile the real
Cachix upload filter; closure safety uses exact store identities.

## Measurement feasibility

The approved protocol requires two matched baseline/treatment pairs in each
cell, with a possible third pair, and excludes any observation without proven
cache state, support substitution, and fresh final execution. The current
workflow has these constraints:

- [CI](../../../.github/workflows/ci.yml) runs on pull requests, `main` pushes,
  and merge groups; it has no controlled measurement dispatch.
- [Setup CI](../../../.github/actions/setup-ci/action.yml) configures the single
  `jaunder-org` Cachix namespace.
- The Cachix `pushFilter` determines which realized paths are uploaded. It does
  not create an isolated download namespace for a baseline arm.
- Existing #1463 Actions observations predate the structural candidate and lack
  per-support-path substitution evidence. Its
  [research report](2026-09-11-issue-1463-ci-wall-clock.md) already classifies
  them as diagnostics rather than matched threshold evidence.

A valid experiment would require separately provisioned Cachix namespaces (or an
equivalently isolated production-representative cache) for every arm, plus a
temporary non-required workflow that records support-path realization state,
transfer/setup cost, final execution, complete required-check wall-clock, and
aggregate runner time. Those external cache namespaces and credentials are not
part of this repository. Reusing the shared namespace would silently contaminate
the baseline and is therefore rejected rather than reported as a measurement.

## Required cells

| Change class    | Candidate prerequisites | Required pairs | Valid pairs | Outcome                                                     |
| --------------- | ----------------------- | -------------: | ----------: | ----------------------------------------------------------- |
| Source-changing | Cold                    |              2 |           0 | Invalid: no isolated baseline/treatment cache namespaces    |
| Source-changing | Warmed                  |              2 |           0 | Invalid: treatment uploads are downloadable by the baseline |
| Narrow          | Cold                    |              2 |           0 | Invalid: no isolated baseline/treatment cache namespaces    |
| Narrow          | Warmed                  |              2 |           0 | Invalid: treatment uploads are downloadable by the baseline |

No cell has a wall-clock or aggregate-runner-time median. Treating missing data
as zero improvement would be misleading, so percentages are **not available**.
Because all four cells are required, the adaptive third-pair rule is never
reached and the 10% retention rule cannot pass. There is likewise no basis for a
no-regression claim.

## Durable outcome

The broad filter remains the production boundary. The generated inventory,
non-substitution flags, exact-closure checks, source-family contracts, and
positive/negative regression tests remain checked in so a future experiment can
start from a proved candidate rather than repeating the safety work.

Reconsider narrowing only when cache-isolated matched Actions arms can produce
all four cells and a separate pull-request plus merge-group observation proves
fresh coverage and all four backend/browser final verdicts. Until then, no ADR
is warranted because the accepted architecture has not changed.
