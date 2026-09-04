# Issue 1289 Nix invalidation boundaries implementation outline

> Execute with `jaunder-iterate`; delegate bounded slices with
> `jaunder-dispatch`. This outline exists because the approved specification
> changes durable Nix derivation boundaries, a public devtool selection surface,
> and two accepted ADR clauses.

## Scope

In:

- committed normalized pre/post measurement evidence;
- docs/code hermetic static groups with one devtool definition inventory;
- target-closure source filters for CSR/site and client wasm tests;
- a non-realizing host derivation-identity probe wired into CI;
- a proposed ADR and architecture/contributor projection.

Out:

- changed-path routing;
- coverage, doctest, Elisp, or e2e verdict splits;
- remote-cache attribution or compatibility aliases.

## Task outline

- [x] Task 1: Commit the controlled pre-change evidence
  - Contract: write the normalized record to
    `docs/superpowers/research/2026-09-04-issue-1289-nix-invalidation-boundaries.md`.
    Its fixed schema records baseline revision, system, exact gate command,
    warm-up/store conditions, and for each arm the marker path and exact marker
    contents, overall outcome/duration, every Nix step's name/installable/full
    derivation path/realization/duration, and any failure detail. Preserve the
    failed `common` and successful `macros` outcomes. Ignored
    `.xtask/measurements/*.json` files remain source material, not the durable
    format.
  - Verification: compare result rows and failures against the saved sidecars;
    verify revision and input metadata against the recorded commands and marker
    procedure; Prettier checks the committed report.

- [ ] Task 2: Partition devtool's static inventory without duplication
  - Contract: `devtool check --group docs` and `devtool check --group code` are
    stable selectors, mutually exclusive with positional single-check selection
    and `--all`. `docs` contains Markdown Prettier only; `code` contains every
    other existing definition and splits end2end Prettier from Markdown
    Prettier. `--all` is their ordered, duplicate-free union. Existing
    single-check selection remains available.
  - Verification: focused devtool tests prove exact membership, ordering,
    disjointness, union completeness, selector exclusivity, command arguments,
    and sandbox Cargo behavior. A direct live-source `devtool check --all` host
    run proves the public aggregate.

- [ ] Task 3: Replace the singular hermetic static derivation
  - Contract: Nix checks are `static-docs` and `static-code`; xtask result steps
    are `nix-static-docs` and `nix-static-code`, built in that order before Nix
    test checks. Each derivation invokes `devtool check --group docs` or
    `devtool check --group code --sandbox-cargo` respectively and contains only
    the source/configuration it needs. The old installable and step are removed.
  - Contract: add a tracked proposed ADR draft that narrowly supersedes the
    singular clauses in ADR-0052 and ADR-0146, and project it into
    `docs/ARCHITECTURE.md`; devtool ownership, offline Cargo, and validation
    ordering remain unchanged.
  - Verification: devtool catalog tests, xtask check-catalog/result tests, Nix
    attr evaluation, and direct builds of both new checks prove the complete
    hermetic surface.

- [ ] Task 4: Filter wasm source closures and add the drift probe
  - Contract: the CSR/site Cargo source admits workspace manifests and the
    resolved `csr` closure; the client wasm-test source admits manifests and the
    resolved `client` wasm-test closure. Both retain required build scripts and
    explicit non-Cargo inputs. `common`/`macros` stay in both; unrelated
    host/server/storage sources stay out.
  - Contract: `cargo xtask nix probe-source` creates isolated tracked
    perturbations, evaluates `.drv` identities without realizing outputs, and
    fails closed on this matrix: docs changes affect only static-docs; server
    changes affect static-code but neither wasm path; web changes affect
    static-code and site but not wasm-tests; common/macros changes affect every
    dependent checked boundary. CI invokes the probe.
  - Verification: focused pure tests cover matrix comparison and malformed or
    failed evaluation; the real probe passes against the live flake and proves
    both over-inclusion and missing fan-out fixtures.

- [ ] Task 5: Record post-change evidence and document the final graph
  - Contract: repeat the warm baseline plus isolated docs, web, server, common,
    and macros arms under Task 1's exact command, markers, and conditions;
    append post-change rows in the same fixed schema plus before/after
    conclusions. Record each arm's actual overall outcome rather than requiring
    the `common` coverage consumer to reproduce its pre-change failure. Record
    rejected candidates and their measured/structural costs.
  - Contract: update `CONTRIBUTING.md` and `docs/ARCHITECTURE.md` with both
    static groups, the target source closures, drift probe, full aggregate gate,
    and reproduction procedure. Remove stale singular-static descriptions.
  - Verification: normalized records satisfy the approved identity/reuse matrix;
    `cargo xtask validate --no-e2e` passes and full `cargo xtask validate`
    retains all four e2e combinations and server-function coverage.

## Risk checks

- The docs/code groups are disjoint and their union equals the prior devtool
  inventory; no check runs twice or disappears.
- Both static derivations keep the tools, offline Cargo homes, timezone, and
  provisioned TypeScript inputs required by their selected checks.
- Source filters include every path Cargo or a build script reads while
  excluding unrelated crate sources; the identity probe is the fail-closed
  authority.
- `common` and `macros` perturbations still fan out through every dependent Nix
  surface.
- Coverage remains one SQLite/PostgreSQL instrumented union with populated CSR
  assets and its existing producer/gate/host-consumer precedence.
- Doctests remain one `--workspace` producer with bidirectional fence
  reconciliation; Elisp remains one pure/live VM producer.
- Full validate ordering and all four e2e combinations remain unchanged.
- No lint suppression is introduced without explicit user approval.
- Each task reaches `jaunder-commit`; the commit hook owns the single precommit
  gate, and commits contain no `Co-Authored-By` trailer.
