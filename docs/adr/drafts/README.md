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
4. Link sibling ADRs **as if the draft already lived in `docs/adr/`** —
   `[ADR-0061](0061-web-keyed-list-reactive-store.md)`, not
   `../0061-web-keyed-list-reactive-store.md`. `promote` moves the file up one
   directory and strips one `../` level from every link target, so the bare form
   is what survives. (The `../template.md` link in step 1 above is correct
   _here_ — this README is never promoted.)
5. Link **another draft** as `[Aaa](../drafts/aaa.md)`. Promotion strips one
   level to `drafts/aaa.md`, which `promote` then rewrites to the number it
   assigned. Do **not** use the rule-3 repo-root form (`docs/adr/drafts/aaa.md`)
   in a markdown link from one draft to another: it becomes
   `docs/adr/NNNN-aaa.md`, which is dead from inside `docs/adr/` and will fail
   the `doc-links` gate. Rule 3 still applies to references from code and prose
   _outside_ `docs/adr/`.

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

`doc-links` enumerates differently — tracked files, via `git ls-files`.
Everything here except this `README.md` is gitignored, so drafts stay invisible
to it too, by a stronger rule: an uncommitted draft is not a tracked file.
