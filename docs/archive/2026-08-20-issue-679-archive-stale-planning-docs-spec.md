# #679 — archive stale planning docs and document live spec exceptions

Issue: [#679](https://github.com/jaunder-org/jaunder/issues/679). Milestone:
Developer tooling & DX. Relevant record:
[`docs/archive/2026-06-25-issue-39-docs-archive-and-index-design.md`](../../archive/2026-06-25-issue-39-docs-archive-and-index-design.md).

## Summary

`docs/superpowers/specs/` and `docs/superpowers/plans/` still contain seven
issue-cycle artifacts for closed issues #303, #315, #400, and #433. They are
stale in the active cycle directories and can mislead `jaunder-develop`, which
derives cycle state by globbing for `*issue-<N>*` in those directories.

Two no-issue specs are different: `2026-06-16-emacs-blogging-frontend-design.md`
and `2026-06-19-content-visibility-layer-c-design.md` were explicitly left as
live drafts for unshipped work by the #39 archive design. This issue should not
silently archive them. Their home should be made explicit in
`docs/superpowers/specs/` so future cleanup does not rediscover the same
ambiguity.

## Decisions

| ID  | Decision                                                                                                                                                                                                                                                        |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Move the seven issue-numbered stale cycle artifacts to `docs/archive/` with the original date and topic preserved.                                                                                                                                              |
| D2  | Use `-spec` / `-plan` suffixes for archived spec/plan pairs that would otherwise collide in the flat archive directory. The #303 spec also gets `-spec` for consistency even though it has no matching plan.                                                    |
| D3  | Leave the two no-issue design specs in `docs/superpowers/specs/` because #39 classified them as live drafts for unshipped work.                                                                                                                                 |
| D4  | Add a short `docs/superpowers/specs/README.md` explaining that the directory normally holds in-flight issue-cycle specs, plus explicitly listed live design drafts; shipped issue-cycle specs belong in `docs/archive/`, including umbrella specs with no plan. |
| D5  | Do not add an ADR. This is repository hygiene and documentation placement, not an architectural decision.                                                                                                                                                       |
| D6  | Prepend each moved file with a top-of-file `> **Status:**` blockquote marking it archived/completed and naming #679 as the cleanup that moved it.                                                                                                               |

## Required moves

| From                                                                            | To                                                                         |
| ------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| `docs/superpowers/specs/2026-07-07-issue-303-web-canonical-colocated-leptos.md` | `docs/archive/2026-07-07-issue-303-web-canonical-colocated-leptos-spec.md` |
| `docs/superpowers/specs/2026-07-13-issue-400-invite-code-newtype.md`            | `docs/archive/2026-07-13-issue-400-invite-code-newtype-spec.md`            |
| `docs/superpowers/plans/2026-07-13-issue-400-invite-code-newtype.md`            | `docs/archive/2026-07-13-issue-400-invite-code-newtype-plan.md`            |
| `docs/superpowers/specs/2026-07-14-issue-433-invitation-process.md`             | `docs/archive/2026-07-14-issue-433-invitation-process-spec.md`             |
| `docs/superpowers/plans/2026-07-14-issue-433-invitation-process.md`             | `docs/archive/2026-07-14-issue-433-invitation-process-plan.md`             |
| `docs/superpowers/specs/2026-07-17-issue-315-web-auth-colocate.md`              | `docs/archive/2026-07-17-issue-315-web-auth-colocate-spec.md`              |
| `docs/superpowers/plans/2026-07-17-issue-315-web-auth-colocate.md`              | `docs/archive/2026-07-17-issue-315-web-auth-colocate-plan.md`              |

## Acceptance criteria

- **AC1 — stale cycle artifacts archived.** None of the seven source paths in
  Required moves remain under `docs/superpowers/specs/` or
  `docs/superpowers/plans/`; each appears at its target path under
  `docs/archive/`.
- **AC2 — archive status notes added.** Each moved file starts with a
  `> **Status:**` blockquote that marks the source artifact archived/completed
  and names issue #679 as the archival cleanup.
- **AC3 — active-cycle globs stop seeing closed issues.**
  `docs/superpowers/specs/*issue-{303,315,400,433}*` and
  `docs/superpowers/plans/*issue-{303,315,400,433}*` return no matches.
- **AC4 — live design docs explicitly stay live.**
  `docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md` and
  `docs/superpowers/specs/2026-06-19-content-visibility-layer-c-design.md`
  remain in `docs/superpowers/specs/`, and `docs/superpowers/specs/README.md`
  names both as live design drafts for unshipped work.
- **AC5 — recurrence note added.** `docs/superpowers/specs/README.md` states
  that shipped issue-cycle specs should be archived even when there is no
  matching plan, because `jaunder-develop` derives state from issue-numbered
  files in the active specs/plans directories.
- **AC6 — old-path references reconciled.** A tracked Markdown search for each
  old source path finds no stale reference, or every remaining reference is
  intentionally historical and still resolves by pointing to the new archive
  path or by naming the old path as past state.
- **AC7 — docs-only gate proof.** `devtool run -- cargo xtask check --no-test`
  passes.

## Out of scope

- Archiving or moving the two no-issue live design drafts.
- Changing `jaunder-develop` state-derivation behavior.
- Adding generated archive indexes or archive automation.
- Adding or changing ADRs.
