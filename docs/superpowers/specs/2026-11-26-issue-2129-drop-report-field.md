# Issue #2129: drop the unused pre-rendered `report` field

## Outcome

The host-tool JSON report (`host_tool::report::CommandResult`) no longer has a
`report` string field, and `dev`'s renderer no longer prints one. The report a
host tool emits is `command`, `ok`, `duration_ms`, `steps`, optional
`exit_override` and optional `evidence`. Docs stop describing the field.

## Approach and tradeoffs

The field's only producer was host-proof's Rust `trace-analysis`. #2100 moved
trace analysis into `dev` Python, which prints its tables directly, and #2105
deleted host-proof. No code in the tree sets the field now. `coverage-check`,
the one remaining host tool whose report `dev` renders, never sets it.

- `tools/host-tool/src/report.rs`: delete the `report` field and its `None`
  initializer.
- `tools/dev/dev/report.py`: delete the `report.get("report")` branch and update
  the module docstring, which describes the table coming first.
- `tools/dev/tests/test_report.py`: drop the `report` key and the table lines
  from the rendering fixture. Rename
  `test_a_report_renders_its_table_then_one_line_per_step` to
  `test_a_report_renders_one_line_per_step`. The step-line cases stay.
- ADR-0247: amendments to this ADR only add text, as #2066, #2100, #2101, #2103
  and #2105 did. Add a header line "- Amended: 2026-11-26 — the `report` field
  is removed (#2129)". Then, after the #2105 amendment, add a paragraph in the
  #2105 form: "_Amended by #2129:_ as of 2026-11-26, …; current state:
  ARCHITECTURE.md §Residual tooling owners after xtask retirement." It says the
  `report` field no longer exists and that a diagnostic prints its own tables
  instead of carrying them in the report. That supersedes the original mentions
  of the field at the field list, the `scripts/step-report.jq` sentence and the
  last Consequences bullet, and those mentions stay unchanged. There is no new
  draft ADR, because removing a field the ADR offered makes no new decision.
- `docs/ARCHITECTURE.md` (§Residual tooling owners) and `CONTRIBUTING.md`: drop
  "optional rendered `report` text" from the field lists. In the same
  ARCHITECTURE.md bullet, fix the stale owner: the `host_tool::report` contract
  lives in `tools/host-tool`, not `tools/repository-policy`.

A future diagnostic that wants a table will print it itself, as `dev`'s Python
diagnostics already do. That is the only cost, and it matches the current
convention. Keeping the field has no consumer, so there is no simpler option.

## Acceptance

- Searching for the field in `tools/` finds no `report` field on `CommandResult`
  and no `report.get("report")` in `report.py`.
- `tools/dev/tests/test_report.py` passes against the reduced renderer, and the
  `host-tool` crate builds and its tests pass.
- ADR-0247, ARCHITECTURE.md and CONTRIBUTING.md no longer list the field as
  current. ADR-0247 records the removal as a dated amendment for #2129.
- The commit-time gate (`dev precommit`) and the pre-push gate (`dev prepush`)
  pass.

## Boundaries

- Do not touch the `evidence` field, `exit_override`, or the stdout reservation.
- Do not edit any existing text in ADR-0247. The #2129 amendment is the only
  change to it.
