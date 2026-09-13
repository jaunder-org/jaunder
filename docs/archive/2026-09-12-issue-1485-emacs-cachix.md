# Publish the Emacs package to Cachix

## Outcome

Jaunder CI realizes the supported Linux Emacs Protocol Client package while the
Cachix upload hook is active. Downstream `x86_64-linux` consumers can therefore
substitute the package instead of rebuilding it after garbage collection.

## Load-bearing decisions

- The existing public interface remains `emacsPackages.${system}.jaunder`; no
  alias is added under `packages` solely to make CI discover it.
- The authoritative `Validate (no e2e)` job owns publication because it already
  installs authenticated Cachix and is the single successful validation lane.
- Build the package only after `cargo xtask validate --no-e2e` succeeds. A
  revision that fails validation does not deliberately publish this final
  consumer package.
- Use the job runner's system (`x86_64-linux`) explicitly. The existing CI fleet
  and Jaunder cache are Linux-only; this change does not claim to publish Darwin
  outputs.
- Keep the existing Cachix push filter unchanged. The Emacs package is a
  cacheable product, not a test-result derivation.
- No ADR is required: this completes distribution of the already-approved flake
  output without changing its interface or package contents.

## Acceptance

- CI explicitly builds `emacsPackages.x86_64-linux.jaunder` with no output link.
- The build executes after successful validation under the authenticated
  `jaunder-org` Cachix action.
- The package still builds and loads through its public flake output locally.
- Existing validation and E2E required-check topology remains unchanged.
- After CI completes, the package output is queryable from
  `https://jaunder-org.cachix.org`.

## Boundaries

- Do not cache coverage, E2E, or other test-result derivations.
- Do not add a macOS runner or claim Darwin cache coverage.
- Do not change the Emacs package derivation or its public interface.
