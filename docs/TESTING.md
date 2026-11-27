# Testing layers

Jaunder checks a change in six layers. Each one runs a different set of checks,
costs a different amount, and leaves some verdicts to a later layer. This page
shows how they fit together. The detailed sections of
[CONTRIBUTING](../CONTRIBUTING.md) and the linked ADRs remain the reference for
how each check works; [Local checks: `dev`](../CONTRIBUTING.md#local-checks-dev)
lists what each `dev` command runs.

| Layer               | Runs                                                                                                        | Typical cost¹                                    | Leaves to a later layer                                                                            |
| ------------------- | ----------------------------------------------------------------------------------------------------------- | ------------------------------------------------ | -------------------------------------------------------------------------------------------------- |
| 1. Focused tests    | `dev test-local <nextest args>`, `dev e2e-local <spec>`                                                     | Depends on the selection                         | Everything outside the selection                                                                   |
| 2. Pre-commit       | `dev precommit`: Markdown checks for a Markdown-only change, else `dev check-no-test`                       | Markdown only: 15–60 s. Broad: 6–15 min          | Product Rust tests, doctests, and the Nix checks other than `static-docs`                          |
| 3. Pre-push         | `dev prepush`: host checks in Verify mode, some taken from Nix; `tools` tests; product Rust tests; doctests | 10–15 min; up to 38 min when Nix outputs rebuild | Coverage/CRAP, wasm tests and budget, Elisp coverage, the platform contract, other Nix checks, E2E |
| 4. Pull-request CI  | [`ci.yml`](../.github/workflows/ci.yml): four validation lanes and the E2E matrix                           | 17 min (10–53). Markdown only: 2.5 min           | The branch combined with the current `main`                                                        |
| 5. Merge queue      | The same `ci.yml` jobs on `main` plus the entry                                                             | 14 min (11–28). Markdown only: 2.5 min           | Nothing, for the commit that lands                                                                 |
| 6. Landed and timed | `Validation host` on the push to `main`, and the scheduled workflows below                                  | Push: 5 min (5–6)                                | —                                                                                                  |

¹ Measured between 2026-11-24 and 2026-11-27 on a 16-core workstation with a
shared kache store (local) and on GitHub's `ubuntu-24.04` runners (CI). CI
figures are medians and ranges of wall-clock time over about 25 successful runs
per event, with Markdown-only runs counted separately; the push figure counts
only runs after the host lane shrank (#2144). Local pre-commit figures span
recent traced runs, the slowest being the first in a new worktree. Pre-push is
traced runs. A heavily loaded machine can push a test past nextest's 30 s
timeout and fail a local run. Re-measure rather than trust old numbers.

**What each layer is for.** Pre-commit catches formatting and static mistakes on
every commit; on `x86_64-linux` its Markdown verdict comes from the Nix
`static-docs` check, which pre-push shares. Pre-push makes sure a branch builds
and passes its host tests before it leaves the machine. Pull-request CI is the
authoritative hermetic verdict, and decides each check in one lane: the hermetic
lanes own every verdict a Nix check can decide, and the host lane runs only what
Nix does not check. `Validate (no e2e)` joins the host, hermetic, test-checks
and coverage lanes, and `e2e gate` joins sixteen E2E producers and their four
reconciliations. A Markdown-only change runs only the `Validation docs` lane,
though the joining jobs still report. The merge queue reruns the same jobs on
the commit that will land, so that commit's later push run skips everything
except `Validation host`, which saves the kache store. When an entry is ejected
by a known infrastructure failure, a running `dev pr-land` re-enqueues it once
([ADR-0220](adr/0220-explicit-one-shot-pr-reenqueue.md)).

**Workflows outside the merge path** block nobody and report what they find:
[advisories](../.github/workflows/advisories.yml) scans `main` daily and files
an issue for each failing RustSec advisory;
[mutants](../.github/workflows/mutants.yml) runs weekly (incremental) and
monthly (full) because each mutant costs a build and a test run;
[performance](../.github/workflows/performance.yml) runs weekly or on a PR that
opts in by label; [upgrade](../.github/workflows/upgrade.yml) runs weekly and on
`v*` tags; and the
[theme repository fixture](../.github/workflows/theme-repository-fixture.yml)
runs on PRs that touch its paths. All five can also be started by hand.

The local broad gates sit beside the layers rather than in them. `dev check` is
pre-commit's broad surface plus `test-local`, the wasm tests and doctests.
`dev validate-no-e2e` runs every non-E2E verdict CI runs, serially, on a clean
tree; `dev validate` adds the full E2E matrix. Run them only for a named broad
question; CI remains the authoritative integration proof.

## Which layer owns each verdict

- **Coverage and CRAP** run only in CI, `dev validate-no-e2e` and
  `dev validate`. Instrumentation and SQLite/PostgreSQL parity are part of the
  verdict
  ([Coverage and dependency policy](../CONTRIBUTING.md#coverage-and-dependency-policy)).
- **The full, reconciled E2E matrix** runs only in CI and `dev validate`
  ([ADR-0034](adr/0034-ci-e2e-matrix-distribution.md),
  [ADR-0203](adr/0203-distribute-ci-validation-and-firefox-e2e-lanes.md)). A
  focused `dev e2e-local` run is not that verdict. Only a timed-out attempt
  fails a lane; an attempt near its timeout passes with a `Duration pressure`
  warning, which `dev pr-watch` reports
  ([ADR-0272](adr/0272-e2e-duration-pressure-is-a-signal.md)).
- **Root-workspace doctests** run only as the Nix `doctests` producer and gate,
  from pre-push onward; there is no host doctest run
  ([ADR-0095](adr/0095-doctest-gate-enumerates-the-fence-population.md)). The
  `tools-doctests` check runs only in validation and CI.
- **CI's static verdicts** each come from one lane. A check a hermetic Nix
  derivation decides, such as formatting, `tsc`, Elisp, or `dev`'s lints and its
  tests that need no repository, is not repeated in the host lane
  ([CI does not repeat hermetic verdicts in the host lane](adr/drafts/ci-hermetic-verdicts-not-repeated-on-host.md)).
  The local gates still run every host check.
- **Clippy:** hooks and local gates run host Clippy; CI takes its verdict only
  from the Nix `static-code` check
  ([ADR-0255](adr/0255-ci-clippy-verdict-from-nix.md)).
- **RustSec advisories** are checked only by host cargo-deny, for both Cargo
  workspaces; the sandboxed check skips them
  ([ADR-0145](adr/0145-sandbox-cargo-deny-skips-advisories.md)). The local gates
  always check them. CI checks them only for a change to a `Cargo.toml`, a
  `Cargo.lock` or `deny.toml`, so a new advisory fails no unrelated work; the
  daily advisories scan covers `main`
  ([CI checks advisories only on a dependency change](adr/drafts/ci-checks-advisories-on-dependency-change.md),
  [cargo-deny checks every Cargo workspace](adr/drafts/cargo-deny-checks-every-workspace.md)).
- **A Markdown-only change** gets only the documentation checks, by the same
  rule in pre-commit and CI
  ([ADR-0266](adr/0266-ci-documentation-only-scope.md)).
- **A landed commit** is covered by its merge-queue run
  ([ADR-0260](adr/0260-merge-queue-verdict-covers-landed-commit.md),
  [Checks on a landed commit](ci-merge-queue.md)).

[Prepush parity by failure surface](../CONTRIBUTING.md#prepush-parity-by-failure-surface)
gives the full split between pre-push and validation.

## Where shared work comes from

- **Nix and Cachix.** Checks are Nix derivations, so a check whose inputs have
  not changed is reused, locally or from the jaunder-org Cachix cache. Static
  verdicts may be substituted that way; coverage and E2E must execute for every
  tested commit, and `dev` checks that they did
  ([ADR-0267](adr/0267-ci-honours-non-substitutable-verdicts.md)).
- **Pre-push's shared verdicts.** On `x86_64-linux`, pre-push requests the Nix
  `rust-format`, `static-docs`, `static-elisp`, `static-typescript` and
  `static-ert` checks instead of running the host leaves they cover, so a push
  that leaves their inputs untouched reuses the verdicts `main` already built
  ([ADR-0273](adr/0273-prepush-takes-shared-static-verdicts.md)).
- **Nix-built host tools.** `dev` runs host tools from their Nix package
  outputs, so they are built once per input change
  ([ADR-0225](adr/0225-nix-cached-host-tooling.md),
  [ADR-0258](adr/0258-dev-runs-nix-built-host-tools.md)).
- **kache.** Host Cargo builds share one compile cache across worktrees. In CI,
  only `Validation host` on `main` saves it; pull requests and the queue restore
  it ([ADR-0256](adr/0256-kache-compile-cache.md),
  [Git hooks](../CONTRIBUTING.md#git-hooks)).

## Choosing what to run locally

While editing, run the focused test for what you changed —
[Targeted Rust tests](../CONTRIBUTING.md#targeted-rust-tests),
[Running e2e tests locally](../CONTRIBUTING.md#running-e2e-tests-locally) — and
rerun the same one after a fix. Let the hooks do their part on commit and push.
Reach for `dev check` or `dev validate-no-e2e` only when you have a specific
broad question, such as whether a shared interface change breaks other crates
([Local checks: `dev`](../CONTRIBUTING.md#local-checks-dev)).

## Keeping this page current

No check pins this page to the code. Update it in the same change whenever
[`gates.py`](../tools/dev/dev/gates.py), [`ci.yml`](../.github/workflows/ci.yml)
or a scheduled workflow adds, removes or moves a layer or a verdict.
