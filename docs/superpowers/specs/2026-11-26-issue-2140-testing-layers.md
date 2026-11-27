# Issue #2140 — describe how the testing layers fit together

## Outcome

A new `docs/TESTING.md`, one screen long, explains Jaunder's checks as a single
system. It says which layer runs what, what each layer costs, which layer owns
each verdict, and what each layer leaves to a later one. Contributors and agents
can then choose what to run without reconstructing that from `gates.py`,
`ci.yml` and `nix/checks.nix`.

## Approach and tradeoffs

- **Placement (decided with the user).** The overview goes in a new file,
  `docs/TESTING.md`, not in a lead section of CONTRIBUTING. CONTRIBUTING is
  2,839 lines, and `docs/README.md` indexes files, not sections. In
  CONTRIBUTING's "Testing" intro, only the layer-ordering sentence becomes a
  pointer to the new file. The rule against bypassing or suppressing tests
  without approval stays where it is, because CONTRIBUTING's lint-suppression
  bullet cites it. The new file gets a row in `docs/README.md`'s "Durable docs"
  table.
- **Content.** The file has one table of the layers, in order:
  1. focused `dev test-local` / `dev e2e-local`;
  2. pre-commit (`dev precommit`);
  3. pre-push (`dev prepush`);
  4. the pull-request CI run;
  5. the merge queue;
  6. pushes to `main` and the other workflows: mutants (weekly), performance
     (weekly, or on a PR with the opt-in label), upgrade (weekly, on `v*` tags,
     or by dispatch), and the path-filtered theme-repository fixture.

  Each row gives what the layer runs, its typical cost with the date measured,
  its purpose, and what it leaves to later layers. The optional local broad
  gates (`dev check`, `dev validate-no-e2e`, `dev validate`) are covered in a
  short note under the table, not as a layer.

  After the table come short prose sections:
  - **Verdict ownership.**
    - Coverage/CRAP runs only in CI, `dev validate-no-e2e` or `dev validate`.
    - The full, reconciled E2E matrix runs only in CI or `dev validate`. Focused
      `e2e-local` is not that verdict.
    - Root doctests run only as the Nix producer and gate, from pre-push onward.
      `tools-doctests` runs only in validation and CI.
    - CI takes its Clippy verdict from Nix (ADR-0255).
    - Only host cargo-deny checks advisories (ADR-0145).
    - Documentation-only scope (ADR-0266).
  - **Shared work.**
    - Cachix substitution: static verdicts may be substituted, but coverage and
      E2E may not (ADR-0267, which supersedes ADR-0252).
    - Nix-built host tools (ADR-0225, ADR-0258).
    - The kache compile cache (ADR-0256).
    - How a merge-queue verdict covers the landed commit (ADR-0260,
      `docs/ci-merge-queue.md`).
  - **Choosing what to run locally.** This links to CONTRIBUTING's existing
    focused-test guidance.

  Details link to the existing CONTRIBUTING sections and ADRs instead of
  restating them.

- **Cost figures.**
  - **CI wall-clock.** Take the recent successful `ci.yml` runs for each event
    (`pull_request`, `merge_group`, `push`) and drop documentation-only runs, or
    report them as their own row. Give the median and range, the sample size and
    the run-ID range.
  - **Local hooks.** Measured during iterate on this checkout with a warm kache.
    Time each route separately: precommit on staged Markdown only, precommit on
    a broad change, and prepush on an unchanged tree.
  - Every figure carries its date and commit.
- **Not pinned by a check.** No existing doc-shape or parity check compares a
  documentation table with `gates.py` or `ci.yml`; the `DOCS_SHAPE` checks cover
  ADRs, links and flow documents only. A new parity check would be new machinery
  for a one-screen table. Each layer row instead links to the code that defines
  it. The doc states that it must be updated whenever `gates.py`, `ci.yml` or
  the scheduled workflows change a layer. The reason it isn't pinned goes in the
  PR body.
- **Pending changes.** #2136 (pre-push), #2134 (advisories) and #2137 (duration
  pressure) are still open. The doc describes the current behaviour, and each of
  those issues gets a comment saying it must update `docs/TESTING.md`.

## Acceptance

- `docs/TESTING.md` exists, about one screen of table plus a page of prose. It
  is linked from CONTRIBUTING's "Testing" section and from `docs/README.md`.
- Every layer, command, CI job and verdict it names matches
  `tools/dev/dev/gates.py`, `.github/workflows/ci.yml` and `nix/checks.nix` at
  the merge head. A conformance review checks this against those files.
- Every cost figure carries its measurement date and source.
- `dev precommit`'s documentation surface passes (Prettier, `doc-links`, ADR and
  flow parity), and so does CI's documentation-only lane.
- #2136, #2134 and #2137 each have a comment asking them to update
  `docs/TESTING.md`.

## Boundaries

- Docs only: no changes to gates, workflows, checks or ADRs.
- The existing detailed CONTRIBUTING sections stay as they are; only the
  "Testing" intro paragraph changes.
- No new parity check.
