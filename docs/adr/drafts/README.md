# ADR drafts

New ADRs are authored here, **out of git**, and are numbered only at ship. This
is the holding pen for the draft-out-of-git flow (issue #219); the mechanics
live in the `jaunder-adr` skill and `CONTRIBUTING.md`.

## Why drafts live outside git

An ADR number is a shared monotonic sequence, and the only moment the correct
number is knowable is at integration. Assigning it earlier — and committing it —
means a rebase can reveal a collision, forcing a rename that churns git history.
So a draft carries **no number** until the moment it ships.

Everything in this directory except this `README.md` is gitignored, so a draft
**cannot** be committed with a premature number.

## Authoring a draft

1. Copy [`../template.md`](../template.md) to `docs/adr/drafts/<slug>.md`.
2. Keep the draft heading exactly `# ADR-DRAFT: <Title>` — `promote` swaps the
   `DRAFT` token for the assigned number.
3. Reference the draft **by path** (`docs/adr/drafts/<slug>.md`) from any code
   or prose that needs it. There is no bare `ADR-DRAFT` token — use the path so
   `promote` can rewrite it to the real number.

## Numbering at ship

At ship, after the final rebase onto `main`, run:

```console
$ cargo xtask adr promote
```

For each draft this assigns the next free number, moves it to
`docs/adr/NNNN-<slug>.md`, rewrites its path-form references, syncs the README
table, and stages the result. The ADR's first appearance in git history is
already correctly numbered.

If a collision still surfaces between your ship commit and your merge,
re-rebase, re-run, and **amend the commit that introduced the ADR** — never add
a fixup commit. `cargo xtask adr renumber` remains the tool for an
already-committed ADR.

## Gate invisibility

The `identifier-collisions`, `adr-format`, and `adr-readme-parity` gates share
one enumeration rule — `is_file` → `.md` → leading number, applied by a
non-recursive `read_dir` over `docs/adr/`. A numberless draft in this
subdirectory is excluded twice over, so drafts never trip a gate.
