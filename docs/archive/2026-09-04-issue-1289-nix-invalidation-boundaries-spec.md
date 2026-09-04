# Issue 1289: Split measured Nix invalidation boundaries

## Outcome

Narrow documentation, web, and high-stack Rust changes stop re-realizing
unrelated hermetic gate work, while low-stack `common` or `macros` changes
continue to fan out through every dependent surface. `cargo xtask validate`
remains the complete ship gate and reports each resulting Nix boundary
independently.

## Load-bearing decisions

- The split is justified by a controlled warm-baseline measurement from the
  pre-change `origin/main` graph, not by crate names alone.
- The committed evidence records the baseline and isolated docs-only, web-only,
  server-only, `common`, and `macros` perturbations, including full derivation
  identities, realization states, step durations, marker paths, command outcome,
  and the one observed `common` sample's host coverage-consumer failure.
- The successful `macros` sample is the representative low-stack acceptance arm;
  the failed `common` sample remains evidence rather than being hidden or
  relabeled.
- The pre-change measurements establish these material costs:
  - docs-only re-realized only `static-checks`, costing 292.4 seconds;
  - server-only unnecessarily re-realized `.#site`/`wasm-budget` for 115.6
    seconds and client `wasm-tests` for 22.8 seconds;
  - web-only unnecessarily re-realized client `wasm-tests` for 26.0 seconds;
  - both low-stack samples changed every Nix derivation identity, as required by
    their dependency position.
- Hermetic static checks split into exactly two semantic groups:
  - `docs` owns Markdown formatting and only the files/configuration needed for
    it;
  - `code` owns every other existing hermetic static definition, including
    end-to-end formatting, Rust and tools formatting/compilation, TypeScript,
    Elisp, cargo-deny, clippy, and repository scanners.
- `devtool` remains the single owner of every check's command and arguments. Its
  host `--all` aggregate still runs every definition exactly once; named `docs`
  and `code` groups exist only to let Nix realize them independently.
- The old Nix `static-checks` installable and xtask step are replaced cleanly by
  the two named boundaries. No compatibility aggregate or duplicate check path
  remains.
- A proposed ADR records the measurement-backed split and narrowly supersedes
  both ADR-0052's one-Nix-derivation clause and ADR-0146's singular
  `static-checks` installable/`nix-static-checks` step requirement. Their
  devtool ownership, shared host/hermetic definitions, offline Cargo, and
  validation-order decisions remain in force.
- Cargo source filtering follows resolved target reachability:
  - the CSR/site source contains the workspace manifests plus sources reachable
    from the `csr` target and its required shell inputs;
  - the client wasm-test source contains the workspace manifests plus sources
    reachable from the `client` wasm-test target;
  - unrelated `server`, `storage`, or host-only source files are absent from
    both;
  - shared `common` and `macros` sources remain present wherever the resolved
    target graph reaches them.
- Existing broad product, coverage, and workspace-doctest source sets remain
  broad where their semantics require the full product graph.
- A non-realizing host derivation-identity probe makes isolated docs, web,
  server, and low-stack marker changes and fails closed unless the expected
  changed/reused matrix holds for both static groups, `.#site`, and
  `wasm-tests`.
- The probe is wired into CI and compares evaluated `.drv` identities; it does
  not build outputs, purge the store, or infer remote-cache behavior.
- Post-change measurements repeat the controlled warm-baseline arms and record
  the resulting identities, realization states, and durations beside the
  pre-change evidence.
- Rejected candidate splits and their concrete costs are recorded:
  - per-ecosystem static fan-out beyond docs/code would add at least three more
    Nix evaluations, source-tree staging copies, and process boundaries to every
    validation. No representative Elisp-, tools-, or end2end-only arm was
    measured, so there is no observed saving to justify that duplication;
  - splitting coverage would repeat some or all of the instrumented
    SQLite/PostgreSQL compilation and test pass, measured at 216.2–354.9 seconds
    in the changed-source arms, and would require a new merged verdict despite
    the accepted single union-coverage policy;
  - splitting doctests would duplicate workspace compilation, measured at
    93.5–176.4 seconds in the changed-source arms, or lose the `--workspace`
    feature unification and bidirectional fence population;
  - splitting the Elisp producer would add a second NixOS VM boot to the
    combined 201.5–306.9 second changed-source producer and create a second
    artifact handoff for one accepted pure/live verdict;
  - e2e is not a split candidate: its four backend/browser derivations and
    aggregate already expose the accepted semantic boundary.

## Acceptance

- Committed pre-change and post-change evidence is reproducible and contains the
  exact inputs, derivation identities, realization states, durations, and
  command outcomes for each representative arm.
- On a warmed store, a docs-only marker changes and realizes the docs-static
  boundary while the code-static, site, and wasm-test derivations retain their
  baseline identities and report reuse.
- A server-only marker changes the code-static, coverage, doctest, and dependent
  Elisp paths while `.#site` and client `wasm-tests` retain their baseline
  identities and report reuse.
- A web-only marker changes the code-static and CSR/site paths while client
  `wasm-tests` retains its baseline identity and reports reuse.
- A low-stack `common` or `macros` marker changes every dependent static,
  CSR/site, wasm-test, coverage, doctest, and Elisp derivation identity.
- The derivation-identity probe detects both accidental over-inclusion and a
  missing dependency fan-out without realizing the expensive outputs.
- Host `devtool check --all`, both hermetic static groups, and their catalog
  tests prove that every prior static definition runs exactly once in each
  applicable aggregate.
- Coverage policy, SQLite/PostgreSQL parity, doctest reconciliation, wasm tests,
  wasm size enforcement, server-function coverage, Elisp coverage, all four e2e
  combinations, and failure diagnostics retain their current behavior.
- `cargo xtask validate --no-e2e` builds both static groups before Nix-backed
  test checks; full `cargo xtask validate` still adds the four e2e combinations
  and server-function coverage verification.
- Architecture and contributor documentation show the resulting derivation
  graph, the source/filter invariants, the probe, and the exact measurement
  reproduction procedure.

## Boundaries

- No changed-path command routing; issue #1123 owns that policy.
- No remote-cache-specific correctness or attribution.
- No source filtering for coverage or workspace doctests beyond their existing
  accepted filters.
- No split of coverage, doctest, Elisp coverage, or e2e verdicts.
- No weakening, skipping, or reordering of the aggregate validate semantics.
- No compatibility aliases for removed derivation or step names.
