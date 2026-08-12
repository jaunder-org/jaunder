# ADR-0116: The coverage probe dirties an excluded file to defeat shallow-clone eval failures

- Status: accepted
- Date: 2026-08-11

## Context

The coverage-filter probe evaluates the coverage derivation's `drvPath` in a
temporary worktree. On a CLEAN worktree, nix's flake git-fetcher walks history
(`revCount`) to resolve the rev. CI checkouts are shallow, and a PR head is a
merge commit whose parents are grafted away, so the walk fails with "getting Git
object <parent>: object not found" (verified against a shallow clone).

## Decision

The probe dirties an EXCLUDED tracked file (`README.md`, filter-excluded as
`.md`) before every eval, so nix copies the working directory and reads only
HEAD (present), skipping the parent walk entirely. Because the dirtied file is
filter-excluded, the dirtying is constant and never perturbs the coverage
`drvPath` — which is the very thing the probe measures. (This is orthogonal to
the probe's `git add` staging, which exists so the _new_ probe files are visible
at all.)

## Consequences

- Probe evals behave identically on full and shallow clones.
- If the flake's source filter ever stops excluding `README.md`, the probe's
  base state changes and its assertions will fail loudly.
