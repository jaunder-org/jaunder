# Issue #815 — Refresh flake inputs and align Playwright

## Outcome

Jaunder's flake inputs are refreshed to their current upstream revisions, and
the Nix and npm Playwright runtime, types, and browser set are aligned at one
stable version at or above 1.59. Existing application and e2e behavior remains
unchanged.

## Load-bearing decisions

- Refresh every top-level flake input to its current configured upstream or
  channel revision. Review the complete `flake.lock` movement; do not narrow the
  update to nixpkgs after seeing the diff.
- The Playwright version exposed by the refreshed nixpkgs package set is the
  source of truth and must be at least 1.59.
- Pin `@playwright/test`, `playwright`, and `playwright-core` exactly to that
  Nix version in the npm manifest/lock. Mixed Playwright versions are invalid.
- `pkgs.playwright-test` and `pkgs.playwright-driver.browsers` continue to come
  from the same nixpkgs package set. No overlay, independent browser package, or
  second version-selection mechanism is introduced.
- Leave unrelated npm dependency versions unchanged. The npm update is only the
  Playwright alignment required by the refreshed Nix package set.
- Preserve `PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD` and `PLAYWRIGHT_BROWSERS_PATH`;
  npm must not download or select browsers outside Nix.
- Minimal source or configuration compatibility fixes caused directly by the
  refreshed inputs are in scope. Feature work, opportunistic refactors, and
  broad or architectural compatibility changes are not. Broad fallout stops the
  cycle for a new revision decision; the implementation does not choose an
  unspecified older pin or expand scope.
- This issue does not change seeded-auth behavior. #1233 owns Disposable-based
  init-script replacement and remains blocked until this dependency update
  lands.
- The refresh changes no domain term or architectural decision and needs no ADR.

## Acceptance

- The lock diff accounts for every refreshed input node and retains the intended
  configured sources/follows relationships.
- Evaluating the refreshed flake reports a `playwright-test` version ≥1.59.
- `end2end/package.json` and the `@playwright/test`, `playwright`, and
  `playwright-core` entries in `end2end/package-lock.json` exactly match that
  evaluated version.
- The Nix e2e environment still uses the matched `playwright-test` modules and
  `playwright-driver.browsers`; npm browser download remains disabled.
- Unrelated npm manifest and lock entries are byte-unchanged except for metadata
  that the lockfile format necessarily derives from the Playwright update.
- `flake.nix`'s `e2ePackage.npmDepsHash` is recomputed from the updated npm
  lock; it is a required derived dependency change, not compatibility fallout.
- Any source/config diff beyond lock, package, and derived hash files is traced
  to a concrete refreshed-input compatibility failure and is the smallest
  correction for it.
- Final `cargo xtask precommit` passes before commit.
- Full `cargo xtask validate` passes, including host/static checks, coverage,
  doctests, and all SQLite/PostgreSQL × Chromium/Firefox e2e combinations.

## Boundaries

- No seeded-auth helper, companion cookie, tombstone, init-script lifecycle, or
  ADR-0098 holdout change; those belong to #1233.
- No npm-wide dependency refresh, Playwright overlay, browser download, or
  package-manager substitution outside the pinned devShell.
- No feature or behavior change justified only by an opportunity exposed during
  the dependency refresh.
- If current upstream inputs require broad or architectural migration, stop for
  an explicit revision decision rather than selecting an older pin or silently
  widening this issue.
