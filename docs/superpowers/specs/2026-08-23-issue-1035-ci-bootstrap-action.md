# Issue #1035 — centralize GitHub Actions bootstrap

## Outcome

The CI and mutation-testing workflows share one repository-owned bootstrap
action for checkout-adjacent Nix, Cachix, and xtask host-build cache setup. The
workflow graph, required check names, runner choices, cache semantics, and test
commands stay behaviorally unchanged.

## Load-bearing decisions

- The seam is `.github/actions/setup-ci/action.yml`, a local composite action
  used only after `actions/checkout@v7` in jobs that need Nix/Cachix/xtask
  bootstrap.
- The composite action owns the repeated `cachix/install-nix-action@v31`,
  `cachix/cachix-action@v17`, and `actions/cache@v6` steps; callers keep their
  own job names, `runs-on`, matrices, policies, commands, diagnostics uploads,
  and comments about job-specific constraints.
- The action takes explicit `github-token` and `cachix-auth-token` inputs, so
  secret flow remains visible at every caller and the action does not reach into
  workflow secrets implicitly.
- The exact Nix config, Cachix cache name, Cachix push filter, cache paths,
  cache key, and restore prefix are preserved. In particular, CI still excludes
  `jaunder-coverage` and `jaunder-e2e` check-result derivations from cache
  pushes.
- The seam is not applied to jobs that do not perform this bootstrap, such as
  `e2e-gate` and `mutants plan`; they intentionally stay lightweight and
  checkout-free.
- No ADR is needed: this is a workflow de-duplication inside existing CI
  decisions, not a new CI policy.

## Acceptance

- `.github/workflows/ci.yml` routes `validate-no-e2e`, `e2e`, and
  `elisp-integration` through `.github/actions/setup-ci/action.yml` immediately
  after checkout.
- `.github/workflows/mutants.yml` routes the `shard` job through the same action
  immediately after checkout; `plan` remains unchanged.
- The local action preserves the audited action pins:
  `cachix/install-nix-action@v31`, `cachix/cachix-action@v17`, and
  `actions/cache@v6`.
- The local action preserves the audited cache paths, key, restore prefix,
  Cachix cache name, `github_access_token`, `authToken`, and `pushFilter`
  values.
- Existing observable CI behavior is unchanged: required check names, matrix
  values, runner labels, schedule/manual triggers, workflow commands, and
  diagnostic artifact uploads do not change.
- The affected focused workflow-action validation passes, either by an existing
  focused check if one exists or by a targeted parser/smoke command that proves
  the local composite action and its workflow callers are structurally valid.
- `cargo xtask check` passes.

## Boundaries

- Do not redesign CI, the merge queue, e2e distribution, mutation-test
  scheduling, or runner sizing.
- Do not change `#629` OOM handling; the issue explicitly excludes it.
- Do not replace pinned action versions, alter cache scope, or add new secrets.
- Do not introduce an external reusable workflow; the seam is repository-local
  and private to these workflows.
