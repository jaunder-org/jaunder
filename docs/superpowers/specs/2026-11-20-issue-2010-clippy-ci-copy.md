# Issue #2010: CI lints with Clippy once, in Nix

## Outcome

CI runs the six Clippy units once per run: as the hermetic lane's Nix
`clippy-aggregate`, not again in the host lane. Every run saves about 10
runner-minutes of host-lane work. Merge-queue latency drops by at most about 2
min. `main`-push and PR wall clock do not change, because those runs wait on the
e2e chain. Developer hooks and local `dev validate-no-e2e` keep the Clippy
coverage they have today.

## Current state (measured, job time)

| Run (2026-11-20)          | Host lane | Host Clippy steps | Hermetic lane | Nix `static-code` | e2e gate done | Wall clock |
| ------------------------- | --------- | ----------------- | ------------- | ----------------- | ------------- | ---------- |
| PR #2005 `44837736998`    | 17.5 min  | 621 s of 935 s    | 17.4 min      | 715 s, built      | 20.6 min      | 20.6 min   |
| merge group `44840288640` | 21.6 min  | (not split out)   | 3.8 min       | substituted       | 19.3 min      | 21.9 min   |
| `main` push `44842863313` | 17.6 min  | (not split out)   | 3.4 min       | substituted       | 21.5 min      | 21.5 min   |

Clippy step times come from the lanes' `dev trace summary`. Lane and gate times
come from `gh run view --json jobs`, measured from run creation.

Both lanes run the same `static.LEAVES` arguments for each unit,
`dev static <unit>`, with the same pinned toolchain and lock file. The host runs
them on the runner from a git checkout. Nix runs them with `--sandbox` (offline,
with a vendored Cargo home) over the `clippySource` filtered copy, which has no
`.git`.

## Approach and tradeoffs

- `dev ci-validate-host` runs the host check groups itself, minus Clippy: the
  format, docs-shape and source-contract groups, then `tsc` and `cargo-deny`,
  then the Elisp group. `host()`, the public `dev compile-checks` group and
  every other gate stay as they are. No flag is added.
- CI's Clippy verdict comes only from `checks.x86_64-linux.static-code`, which
  requires `clippy-aggregate`, which requires all six unit derivations.
  `Validate (no e2e)` already requires the hermetic lane, so the required checks
  still carry a verdict for every unit. ADR-0252 already accepts a substituted
  static verdict as evidence.
- No change to `dev check`, `check-no-test`, `precommit`, `prepush`,
  `validate-no-e2e` or any Nix derivation. Locally, `validate-no-e2e` keeps both
  copies (its host copy reuses the developer's sccache), so it becomes a
  superset of the CI lanes, not the same surfaces.

**Why reopening #1864 is allowed.** #1864 decided which Clippy populations to
keep and kept host Clippy required. Its sharing decisions are deferred "unless a
concrete maintenance defect, consumer need or attributable workflow cost
justifies a separately scoped decision" (CONTRIBUTING.md, verification-family
decisions). About 10 min of duplicate Clippy on every run is that attributable
workflow cost. Host Clippy stays required in the hooks.

**Alternative: keep host, drop Nix.** Deleting `mkClippyLeaf`, the six leaves,
`clippy-aggregate` and `clippySource` would remove about 100 lines of Nix. That
is the simpler final system. It saves runner time only on cache-miss runs,
because the Nix verdict is substituted on merge-group and `main` runs. The human
chose to keep Nix (2026-11-20).

**Accepted costs.**

- **CI no longer runs `cargo` on the product workspace outside Nix.** This does
  not affect caching: host Clippy builds only lint metadata in the runner's
  discarded `target/`, and never reaches Cachix. Application outputs
  (`csrBundle`, the Nix-built server) are still built and pushed by Nix. The
  narrow loss is CI coverage of host-only build paths. An example is the
  git-aware branch of `test-support/build.rs`, which Nix skips because it has no
  `.git`; the sccache/host-environment handling is another. The pre-commit and
  pre-push hooks still exercise those paths, though `--no-verify` bypasses them.
- **Filtered source.** A file the `clippySource` filter omits but a crate needs
  (for example an `include_str!` target) fails that Nix build. It cannot pass
  silently.
- **Cache-miss PRs.** A PR that changes Rust still spends about 12 min on Nix
  Clippy in the hermetic lane.
- **Where failures show.** A CI Clippy failure appears in the hermetic lane's
  log.

## Decision record

- A tracked `jaunder-adr` draft. It records the decision (CI takes its Clippy
  verdict from Nix; hooks and local gates keep host Clippy), the reopening
  clause above, ADR-0252 and the accepted costs.
- Update the statements this change makes false:
  - CONTRIBUTING.md:966-967: the four gates no longer share all five host
    groups.
  - CONTRIBUTING.md:1039-1043: the "Root/auxiliary Rust tests and Clippy" family
    bullet.
  - CONTRIBUTING.md:1158-1162: "from the host ladder".
  - docs/ARCHITECTURE.md:4731-4749: the CI-lane paragraph, including the claim
    that "local validate-no-e2e composes the same required surfaces".
  - docs/ARCHITECTURE.md:5017-5018: "Native host formatting and Clippy remain
    required".
  - docs/ARCHITECTURE.md:5023-5028: the family paragraph's Clippy authority.

## Acceptance

At PR time:

- In `tools/dev/tests/test_gates.py`, `ci-validate-host`'s expected command list
  has no Clippy commands. Every other gate's list is unchanged, including
  `validate-no-e2e`, `prepush` and `check-no-test`. `pytest tools/dev` passes.
- The PR's CI run is green. Its host-lane trace has no `*-clippy` spans, and its
  hermetic-lane trace shows `static-code` realized.
- A comment on #2010 records before and after job times (host, hermetic, e2e
  gate, wall clock) for the PR run.
- The ADR draft and the doc updates are in the PR.

After merge (follow-up, not a merge condition): a comment on #2010 adds the same
times for the merge-group run.

## Boundaries

- No change to what Nix Clippy builds or caches, and no speedup of the Nix
  Clippy leaves.
- No change to hooks or to local gates.
- No restructuring of the other CI lanes.
