# Issue #39 — Archive shipped planning docs + add a docs index (design)

> Spec for [issue #39](https://github.com/jaunder-org/jaunder/issues/39).
> Branch: `issue-39-docs-archive-and-index`.

## Goal

Two focused documentation improvements:

1. Move three shipped/stale planning docs out of the live tree into the
   established `docs/archive/`, so the live docs only describe how the system
   works **now**.
2. Add the missing `docs/README.md` index — there is currently no docs map, ADR
   index, or archive pointer anywhere; a newcomer's only path is
   `README.md` → `CONTRIBUTING.md`.

This is a docs-only change. No Rust, no code, no behavior change.

## Background

A documentation audit (Diátaxis lens + durable-vs-ephemeral classification) found
the durable core solid (README → CONTRIBUTING → ARCHITECTURE + 21 ADRs), but:

- Three completed/stale planning docs still live outside `docs/archive/`.
- No index exists (`docs/README.md`, ADR index, archive index all absent).

Note: the git history rewrite (~2026-06-22/23) reset most docs' commit dates to
06-23, so git dates are **not** a usable freshness signal. Classification and
archive dating below use content/ADR dates, not commit dates.

## Part 1 — Archive three planning docs

Each file moves with `git mv` into `docs/archive/`, following the existing
`YYYY-MM-DD-<topic>.md` convention, and gains a top-of-file `> **Status:**`
blockquote (the same convention as
`docs/archive/2026-06-24-coverage-pipeline-rust-migration-plan.md`).

| From | To | Date rationale |
|------|----|----------------|
| `docs/code-analysis-2026-06-12.md` | `docs/archive/2026-06-12-code-analysis.md` | date already in the name |
| `docs/server-submodule-refactor-plan.md` | `docs/archive/2026-05-23-server-submodule-refactor-plan.md` | ADR-0013 decision date (the pattern this plan delivered) |
| `HISTORY-REWRITE-SURVEY.md` | `docs/archive/2026-06-22-history-rewrite-survey.md` | groups with the existing `2026-06-22-history-rebuild-*` archive entries |

Status notes to prepend (exact wording finalized during implementation):

- **code-analysis** — "Status: ARCHIVED — point-in-time codebase analysis
  snapshot (2026-06-12); retained as historical record, not a live to-do list."
- **server-submodule-refactor-plan** — "Status: COMPLETE — shipped; the pattern
  is codified in ADR-0013 (Server Submodule Pattern, accepted 2026-05-23)."
- **history-rewrite-survey** — "Status: COMPLETE — phase-1 scratch survey for the
  git history rewrite, which was executed and verified (see the
  `2026-06-22-history-rebuild-*` archive entries). Originally self-labeled
  'delete when done'; archived instead to preserve the record."

### Out of scope (explicit)

- `.superpowers/sdd/*` ledger — gitignored scratch for already-shipped work; left
  untouched (not tracked, not ours to move).
- `docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md` and
  `docs/superpowers/specs/2026-06-19-content-visibility-layer-c-design.md` — live
  drafts for unshipped work; archive when their work lands, not now.
- Tracking decision for untracked durable docs (`AGENTS.md`,
  `end2end/CLAUDE.md`) — deferred to a separate pass.

## Part 2 — Add `docs/README.md`

A hand-maintained index (matches existing convention; no generation tooling),
with three sections:

1. **Durable docs** — a table linking the live docs (README, CONTRIBUTING,
   CONTEXT, ARCHITECTURE, DESIGN, ROADMAP, observability, web-style-guide,
   atompub-marsedit-acceptance), each with a one-line "what it's for / when to
   read it."
2. **ADR index** — a table of all 21 ADRs (number · title · status), each linking
   its file. This is the single highest-value addition: 21 ADRs with no map.
3. **Archive** — a short explainer of the archive convention (per ADR-0000) and
   the `YYYY-MM-DD-<topic>.md` naming scheme, plus a link to the `docs/archive/`
   directory. **No per-file listing** — a full enumeration of 70+ files would rot
   on every future archive; the directory + naming convention is the durable
   pointer.

The index links to durable docs and the archive directory; it does not duplicate
their content (single-source-of-truth — CONTRIBUTING remains the working hub).

## Testing / verification

Docs-only; no code touched. The gates are:

1. **Link integrity** — every relative link in `docs/README.md` resolves to an
   existing file/dir; the three moved files leave no dangling inbound links
   (grep the tree for references to the old paths and fix any).
2. **`cargo xtask validate --no-e2e`** stays green (sanity: the change does not
   perturb the build/coverage gate).

## Out of scope / non-goals

- No content rewrites of the archived docs (only the status-note prepend).
- No new ADR.
- No changes to `CONTRIBUTING.md` beyond, at most, a one-line pointer to the new
  `docs/README.md` index if a natural spot exists (optional, decided during
  implementation).
- No generated/automated index tooling.
