# Issue #1956 — remove the unused `adr-promoter promote` CLI action

## Outcome

`nix run .#adr-promoter -- promote` stops being a valid command: the CLI accepts
only `controller` (what `.github/workflows/adr-promoter.yml` runs) and
`sync-readme` (the documented table-only recovery in `CONTRIBUTING.md`). The
in-process `promote` function that the controller calls is unchanged.

## Consumer audit (done at spec time)

No caller of the `promote` action exists in workflows, `tools/dev`, Nix,
`.githooks`, `CONTRIBUTING.md`, `docs/`, or the agent skills under
`~/.claude/skills` and `~/.agents/skills`. The one documentation mention,
`docs/adr/drafts/README.md:46`, is a prohibition rather than a caller. ADR-0242
already says feature authors must not promote and that manual promotion is never
a recovery path, so the action contradicts the accepted design. Its only users
are two tests:

- `tools/adr-promoter/tests/test_cli.py` — runs `promote` then `sync-readme`
  through `python -m adr_promoter`.
- `tools/repository-policy/tests/adr_promoter_package.rs` — the Nix package
  smoke test. It uses `promote` to prove that admission refuses an untracked
  declared input without changing the tree, and that the promoted output passes
  the Rust ADR format, README/view parity and doc-link gates.

## Approach and tradeoffs

- `__main__.py`: drop `promote` from the argparse choices, so the `else` branch
  runs `sync_readme` only.
- `test_cli.py`: the mutation test runs `sync-readme` on a README with a missing
  or stale row and asserts the complete resulting README. Add one assertion that
  `promote` exits 2 with argparse's invalid-choice error, so the removal is
  enforced by a test.
- Package smoke test: the admission refusal and the gate checks move to
  `sync-readme`. The fixture becomes accepted ADRs plus a README with a drifted
  table. `sync-readme` must first be refused for the untracked input, and the
  refusal must leave `docs/README.md` byte-identical. (`sync-readme` writes the
  working tree but does not stage, so the current `git write-tree` comparison
  would not catch a bypass.) Once the input is tracked, `sync-readme` must fix
  the table. Afterwards the fixture must pass `adr-format`, `adr-readme-parity`,
  `adr-view-parity` and `doc-links`. The test gets a new name that says what it
  now does.
- `docs/adr/drafts/README.md:45-46`: delete the parenthetical "(including the
  package's `promote` action)", because that action no longer exists.

**Accepted cost:** the package smoke test no longer exercises promotion through
the selected Nix package. It also no longer checks that promoted bytes pass the
Rust gates. The promotion output stays covered byte-for-byte by
`test_promotion.py`, and the controller's promotion wiring by
`test_controller.py`. Each real promoter PR also has to pass the Rust gates in
required CI before it can merge, so a mismatch between the two would block that
PR rather than land. Keeping a test-only CLI action just to retain the
cross-check would bring back the surface this issue removes.

No ADR change: ADR-0242 and `docs/ARCHITECTURE.md` name only `controller` and
`sync-readme`, so they already describe the result.

Required CI already runs these Rust gates. `Validate (no e2e)` runs
`dev ci-validate-host`, whose docs-shape step covers `adr-format`,
`adr-readme-parity`, `adr-view-parity` and `doc-links`. That job runs on
`pull_request` and `merge_group`.

## Acceptance

- `adr-promoter promote` exits non-zero with an invalid-choice error (enforced
  by `test_cli.py`).
- `controller` and `sync-readme` behave the same as before. The Python package
  tests pass, and so does `adr_promoter_package.rs` through the normal gate.
- No repository text invokes or documents the `promote` CLI action. The only
  exceptions are the negative test and the in-process controller fakes in
  `test_controller.py`.

## Boundaries

- Do not change `promotion.promote`, the controller, the workflow, admission or
  the Nix packaging.
- Do not add a replacement or hidden promotion entry point.
