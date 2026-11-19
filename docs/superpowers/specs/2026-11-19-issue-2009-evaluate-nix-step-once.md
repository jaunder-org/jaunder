# Issue #2009: evaluate each `dev` Nix step once

## Outcome

Every Nix build step `dev` runs (`steps.nix_build`, and so every `nix_check`)
evaluates its installable once instead of twice. A warm hermetic lane, which is
almost all evaluation, gets correspondingly faster. `jaunder.cache` keeps its
three values and their meaning.

## Approach and tradeoffs

Today a step first runs `nix eval --raw <installable>.outPath` and
`nix path-info` on the result, to learn whether the output was already valid
(`reused`). Then it runs `nix build` on the same installable, which evaluates it
again. Measured locally on `static-code`, each evaluation takes about 17 s;
Nix's flake evaluation cache does not absorb the second one.

The pre-build probe goes away. The step runs only
`nix build ... --print-out-paths` and then one `nix path-info --json` on the
printed output path. That entry already carries `registrationTime`, the second
at which the local store registered the path, alongside `ultimate`:

| After a successful build                     | `jaunder.cache`  |
| -------------------------------------------- | ---------------- |
| registered before the step's start second    | `reused`         |
| registered after it, `ultimate`              | `built`          |
| registered after it, not `ultimate`          | `substituted`    |
| registered in the start second itself        | nothing recorded |
| not exactly one printed path                 | nothing recorded |
| failed or unreadable query, null entry, or   | nothing recorded |
| `registrationTime` missing or not an integer |                  |

The start second is the wall-clock time taken just before `nix build` starts. A
path registered in that same second could be either side of the start, so the
step records nothing rather than guess. That is the existing honest "unknown"
fallback. It covers a path registered within the start second, either by an
earlier step or by a sub-second build or substitution in this one.

Nothing recorded never changes the step: its status, exit code, output and
return value (the printed text, as today) are the build's. A build printing
several paths simply records no `jaunder.cache`.

Because nothing is evaluated outside `nix build`, the `output` parameter of
`nix_build` (used by performance producers, whose derivation installables have
no `.outPath` attribute) is no longer needed and is removed. The performance
refusal already compares the printed path with the selection's identity.

**Alternatives considered:**

- Evaluate `drvPath` once, read its output path from the derivation, probe it,
  then `nix build <drv>^out`. That keeps the pre-build probe, but adds a
  derivation query, a separate evaluation span, and a build command line that
  names a store derivation instead of the installable.
- `nix build --json` or Nix's JSON activity log: both change the step's command
  line or output, need more parsing, and still need `path-info` to tell built
  from substituted. "No activity seen" is a weaker sign of `reused` than a
  timestamp.

**Accepted cost:** `reused` now means "registered before this step started"
rather than "valid when probed just before the build". Those agree except for a
path registered while the step itself runs, which is what `built` and
`substituted` mean. The rule assumes the registration time is local: true for
local builds and HTTP binary-cache substitution, which is what `dev` and its
hosts use. Nix keeps a source store's registration time when copying a path in,
so an output from a remote builder, an `ssh-ng://` or `local://` substituter, or
`nix copy` may read as `reused`. CONTRIBUTING.md states this limit.

## Testing seam

The classification (entry and start second in, cache value or nothing out) is
one pure function in `dev.steps`, tested in-process for every row of the table,
including the same-second and malformed-entry cases. The subprocess tests'
`FAKE_NIX_STORE` reports a fixed past `registrationTime` (0) for an output valid
from the start and a fixed far-future one for an output the fake build creates,
so they never depend on the real clock.

## Acceptance

- Subprocess tests show the build is the only `nix` evaluation: the recorded
  command order is the build, then one `path-info` (none after a failed build).
- Tests pin `reused`, `built`, `substituted`, and every "nothing recorded" row
  without changing the step's status or returned text.
- `traced-run -- dev ci-validate-hermetic`, warm (all outputs reused), timed
  three times on `origin/main` and three times on the branch, on this host. The
  branch median should be lower by roughly one evaluation per Nix step in the
  lane. Timings go in the PR description, not the tree.
- The `nix_build` docstring, CONTRIBUTING.md and ARCHITECTURE.md describe the
  new rule in place of "evaluated before the build". ADR-0251 lists only the
  values and needs no change.

## Boundaries

- No change to which steps record `jaunder.cache`, to its values, or to the
  step's command line, exit status or output.
- Recording output paths, fetched bytes or other new attributes is #2005.
- No change to `scripts/e2e-workflow`'s own builds.
