# Testing layers

Jaunder checks a change in six layers. Each one runs a different set of checks,
costs a different amount, and leaves some verdicts to a later layer. This page
shows how they fit together. The detailed sections of
[CONTRIBUTING](../CONTRIBUTING.md) and the linked ADRs remain the reference for
how each check works.

| Layer                                                        | Runs                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Typical cost¹                                                          | For                                                                         | Leaves to a later layer                                                                                             |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- | --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| 1. Focused tests                                             | `dev test-local <nextest args>`, `dev e2e-local <spec>`                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Depends on the selection; not measured                                 | Feedback on the code you are editing                                        | Everything outside the selection                                                                                    |
| 2. Pre-commit ([`dev precommit`](../tools/dev/dev/gates.py)) | Markdown-only staged change: Prettier, the docs-shape checks and the Nix `static-docs` check. Anything else: `dev check-no-test` in Fix mode — the host format, docs-shape, source-contract, `tsc`, cargo-deny, Clippy and Elisp checks, plus the `tools` tests and `tools/dev` lint and tests                                                                                                                                                                                                          | Markdown only: 53 s. Broad: 14.7 min, first run in a new worktree      | Formatting and static mistakes, caught on every commit                      | Product Rust tests, doctests, all Nix and VM checks                                                                 |
| 3. Pre-push ([`dev prepush`](../tools/dev/dev/gates.py))     | Clean-tree refusal, the same host surface in Verify mode, the `tools` tests, the whole product Rust suite (`test-local`), and the Nix doctest producer and gate                                                                                                                                                                                                                                                                                                                                         | 10.5–14.6 min; 37.6 min when the doctests and CSR bundle rebuild (n=4) | A branch that builds and passes its host tests before it leaves the machine | Coverage/CRAP, wasm browser tests, the wasm budget, Elisp coverage, other Nix static checks, platform contract, E2E |
| 4. Pull-request CI ([`ci.yml`](../.github/workflows/ci.yml)) | Four non-E2E lanes (`dev ci-validate-host`, `ci-validate-hermetic`, `ci-validate-test-checks`, `coverage`) joined by `Validate (no e2e)`; sixteen E2E producers and four reconciliations joined by `e2e gate`. A Markdown-only change runs only `Validation docs`                                                                                                                                                                                                                                       | Full: median 16.8 min (10.6–52.7, n=28). Markdown only: 2.4 min (n=3)  | The authoritative hermetic verdict on the branch                            | The branch combined with the current `main`                                                                         |
| 5. Merge queue (`merge_group` run of `ci.yml`)               | The same jobs on the queue's combination of `main` and the entry                                                                                                                                                                                                                                                                                                                                                                                                                                        | Full: median 13.8 min (10.6–27.8, n=24). Markdown only: 2.6 min (n=2)  | The verdict for the exact commit that lands                                 | Nothing for that commit                                                                                             |
| 6. After landing and on a schedule                           | Push to `main`: only `Validation host`, which saves the kache store, when the queue already passed the commit; any other push validates in full. [Mutants](../.github/workflows/mutants.yml) weekly (incremental) and monthly (full); [performance](../.github/workflows/performance.yml) weekly or on an opted-in PR; [upgrade](../.github/workflows/upgrade.yml) weekly and on `v*` tags; the [theme repository fixture](../.github/workflows/theme-repository-fixture.yml) on PRs touching its paths | Push: median 8.7 min (6.8–10.9, n=23). Mutants: about 42 s per mutant  | Cache upkeep, and checks too slow or too unpredictable to gate a merge      | —                                                                                                                   |

¹ Local figures come from `dev trace summary` on a 16-core workstation with the
shared kache store. Pre-commit: one run of each route on 2026-11-26 at commit
`7668185b4`. Pre-push: four successful traced runs from 2026-11-24 to
2026-11-26. Under heavy contention from other worktrees, nextest's 30 s per-test
timeout can fail a pre-push run (#2143). CI figures: the successful `ci.yml`
runs 45529722684–45924894020 (2026-11-25 to 2026-11-27, wall-clock from creation
to completion), split by event and by whether `Validation docs` ran. Re-measure
rather than trust old numbers.

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
  focused `dev e2e-local` run is not that verdict.
- **Root-workspace doctests** run only as the Nix `doctests` producer and gate,
  from pre-push onward; there is no host doctest run
  ([ADR-0095](adr/0095-doctest-gate-enumerates-the-fence-population.md)). The
  `tools-doctests` check runs only in validation and CI.
- **Clippy:** hooks and local gates run host Clippy; CI takes its verdict only
  from the Nix `static-code` check
  ([ADR-0255](adr/0255-ci-clippy-verdict-from-nix.md)).
- **RustSec advisories** are checked only by host cargo-deny; the sandboxed
  check skips them
  ([ADR-0145](adr/0145-sandbox-cargo-deny-skips-advisories.md)).
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
