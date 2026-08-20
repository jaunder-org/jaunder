# #753 — scope ADR draft heading promotion to line 1

Issue: [#753](https://github.com/jaunder-org/jaunder/issues/753). Milestone:
Developer tooling & DX. Relevant decisions:
[ADR-0048](../../adr/0048-adr-out-of-git-draft-workflow.md),
[ADR-0088](../../adr/0088-promotion-is-the-acceptance-event.md).

## Summary

`cargo xtask adr promote` currently rewrites `ADR-DRAFT` with a whole-body
`str::replace` in `xtask/src/adr.rs::run_promote`. That correctly changes the
draft heading, but it also corrupts prose or code spans that discuss the literal
token. The bug is silent because the promoted file remains valid Markdown and
still passes the ADR gates.

The fix is to make heading promotion line-scoped: only the required first-line
prefix `# ADR-DRAFT: ` becomes `# ADR-NNNN: `. The rest of the draft body must
remain byte-identical except for existing, separately-owned promotion
transforms: relative-link stripping and `proposed` -> `accepted` status
promotion.

## Decisions

| ID  | Decision                                                                                                                                                       |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Replace the whole-body `ADR-DRAFT` replacement in `run_promote` with a helper that rewrites only line 1.                                                       |
| D2  | Require promoted drafts to start with `# ADR-DRAFT: ` and fail `adr promote` before writing, removing, or staging files when the heading is malformed.         |
| D3  | Keep existing promotion ordering: heading rewrite, own-file relative-link stripping, status promotion, write/remove/stage, then path-form reference rewriting. |
| D4  | Do not change draft status behavior from ADR-0088; only `proposed` on the status line becomes `accepted`.                                                      |
| D5  | Do not add an ADR. This fixes the existing ADR promotion implementation; it does not change the ADR workflow.                                                  |

## Acceptance criteria

- **AC1 — prose token preserved.** A draft whose body mentions `ADR-DRAFT` in
  prose or a code span promotes with those body mentions unchanged; only the
  line-1 heading token changes.
- **AC2 — discriminating regression test.** Add a test with a heading plus a
  body mention of `ADR-DRAFT` that would fail under whole-body replacement.
- **AC3 — malformed heading fails before any mutation.** If any draft's first
  line is not `# ADR-DRAFT: <Title>`, `adr promote` returns an error before
  writing any numbered ADR, removing any draft, syncing README, rewriting
  references, or staging any path.
- **AC4 — multi-draft preflight regression.** Add a test with one valid draft
  and one malformed draft that proves the tree and index are unchanged when
  heading validation fails.
- **AC5 — existing behavior preserved.** Existing `run_promote` tests still
  pass, including relative-link rewrites, cross-draft reference rewrites, and
  ADR-0088 status promotion.
- **AC6 — targeted test proof.** Run the xtask ADR promotion test subset that
  covers the new regression and existing `run_promote` behavior.
- **AC7 — gate proof.** `devtool run -- cargo xtask check --no-test` passes.

## Out of scope

- Changing `docs/adr/drafts/` authoring rules.
- Changing ADR numbering, path-form reference rewriting, README sync, or
  architecture-view parity.
- Changing deliberate non-`proposed` draft status handling.
