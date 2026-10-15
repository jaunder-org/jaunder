# Issue #1778: documented uppercase issue Priority values

## Outcome

`cargo xtask issue create` accepts the documented `P1`–`P4` Priority spellings
while preserving existing lowercase input compatibility and the uppercase
Jaunder Backlog project-field values.

## Load-bearing decisions

- Uppercase `P1`, `P2`, `P3`, and `P4` and existing lowercase `p1`, `p2`, `p3`,
  and `p4` select the same respective Priority.
- Existing project option mappings remain unchanged; Priority remains an
  explicit required project field, not a label or inferred default.
- Unsupported Priority input is rejected during CLI parsing, before issue
  creation or project mutation is reachable.
- The help example, accepted native CLI arguments, and maintained issue skill
  agree on uppercase spelling; the external skill already uses `P2`.

## Acceptance

- Parser regressions exercise all four uppercase and lowercase spellings and
  verify their respective Priority selections.
- A parser regression uses the complete issue-create help-example arguments,
  including its exact `--priority P2` spelling, without executing the command.
- Invalid Priority values (including `P0`, `P5`, and an unrelated word) fail
  parsing with an invalid-value diagnostic; no GitHub mutation is performed.
- Deterministic tests verify all four unchanged uppercase project values and
  option mappings, using existing host-test boundaries without live GitHub.
- Focused xtask host tests and integrated review provide proof; ordinary
  enforced commit/push hooks and current-head CI remain applicable later.
  Execution logs are review evidence, not tracked deliverables.

## Boundaries

No live issue creation/project mutation tests, Priority rubric or field-value
changes, labels/defaults, external skill/configuration edits, framework, or
coverage/backend/browser/CI/isolation weakening. This is a bounded parser
compatibility fix, not a new architecture decision; no outline is required.
