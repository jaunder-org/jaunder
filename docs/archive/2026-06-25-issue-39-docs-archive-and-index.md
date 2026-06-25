# Docs: Archive Shipped Planning Docs + Add Docs Index — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move three shipped/stale planning docs into `docs/archive/`, and add the missing `docs/README.md` index (durable-docs map + 21-ADR index + archive pointer).

**Architecture:** Pure documentation change — no Rust, no code, no behavior. Files are relocated with `git mv` and gain a `> **Status:**` blockquote; one new hand-maintained index file is created. Spec: `docs/superpowers/specs/2026-06-25-issue-39-docs-archive-and-index-design.md`.

**Tech Stack:** Markdown only. `git mv`. Verification via `rg` (link integrity) + `cargo xtask validate --no-e2e`.

## Global Constraints

- **No `Co-Authored-By` trailers** in any commit (overrides global default).
- **Never commit on `main`.** All work lands on the worktree branch `issue-39-docs-archive-and-index`.
- **Run all commands from the worktree root** `/home/mdorman/src/jaunder/.claude/worktrees/issue-39-docs-archive-and-index`. Bash cwd is already there; do not `cd` into the main repo.
- **Archive convention** (per ADR-0000): `docs/archive/YYYY-MM-DD-<topic>.md`, with a top-of-file `> **Status:**` blockquote — model on `docs/archive/2026-06-24-coverage-pipeline-rust-migration-plan.md`.
- **Do NOT rewrite references inside `docs/archive/`.** Existing archived docs (the `2026-06-22-history-rebuild-*` set and `2026-06-15-storage-backend-dedup-dialect-design.md`) reference the old root paths as *historical record* of where files were at the time. Those are frozen; leave them byte-unchanged.
- **Single source of truth:** the new index links to durable docs; it does not duplicate their content. `CONTRIBUTING.md` remains the working hub.

---

### Task 1: Archive the three planning docs

**Files:**
- Move: `docs/code-analysis-2026-06-12.md` → `docs/archive/2026-06-12-code-analysis.md`
- Move: `docs/server-submodule-refactor-plan.md` → `docs/archive/2026-05-23-server-submodule-refactor-plan.md`
- Move: `HISTORY-REWRITE-SURVEY.md` → `docs/archive/2026-06-22-history-rewrite-survey.md`

**Interfaces:**
- Produces: three new paths under `docs/archive/`; three old paths cease to exist. Task 2's index links the `docs/archive/` directory (not these individual files), so no cross-task path coupling.

- [x] **Step 1: Move the three files with `git mv`**

```bash
git mv docs/code-analysis-2026-06-12.md docs/archive/2026-06-12-code-analysis.md
git mv docs/server-submodule-refactor-plan.md docs/archive/2026-05-23-server-submodule-refactor-plan.md
git mv HISTORY-REWRITE-SURVEY.md docs/archive/2026-06-22-history-rewrite-survey.md
```

- [x] **Step 2: Prepend the status blockquote to `docs/archive/2026-06-12-code-analysis.md`**

Insert immediately after the H1 title line (blank line above and below the blockquote):

```markdown
> **Status: ARCHIVED** — point-in-time codebase analysis snapshot (2026-06-12).
> Retained as a historical record, not a live to-do list; individual findings may
> already be addressed. Archived under issue #39.
```

- [x] **Step 3: Prepend the status blockquote to `docs/archive/2026-05-23-server-submodule-refactor-plan.md`**

Insert immediately after the H1 title line:

```markdown
> **Status: COMPLETE** — shipped. The pattern this plan delivered is codified in
> [ADR-0013: Server Submodule Pattern](adr/0013-server-submodule-pattern.md)
> (accepted 2026-05-23). Archived under issue #39.
```

(Note: from inside `docs/archive/`, the relative link to the ADR is `../adr/0013-server-submodule-pattern.md`. Use that exact form.)

- [x] **Step 4: Prepend the status blockquote to `docs/archive/2026-06-22-history-rewrite-survey.md`**

Insert immediately after the H1 title line:

```markdown
> **Status: COMPLETE** — phase-1 scratch survey for the git history rewrite, which
> was executed and verified (see the `2026-06-22-history-rebuild-*` entries in this
> directory). Originally self-labeled "delete when done"; archived instead to
> preserve the record. Archived under issue #39.
```

- [x] **Step 5: Verify no live (non-archive) inbound link breaks**

Run:
```bash
rg -n --no-heading -g '!docs/archive' -g '!docs/superpowers' -g '!target' \
  'code-analysis-2026-06-12|server-submodule-refactor-plan|HISTORY-REWRITE-SURVEY' .
```
Expected: **no output** (the only references are inside `docs/archive/` — frozen historical record, intentionally left — and `docs/superpowers/` spec/plan, which describe the move). If any live durable doc (README, CONTRIBUTING, docs/*.md, ADRs) appears, update that reference to the new `docs/archive/...` path.

- [x] **Step 6: Confirm the moves and that working tree is clean of strays**

Run:
```bash
git status --short
ls docs/archive/2026-06-12-code-analysis.md docs/archive/2026-05-23-server-submodule-refactor-plan.md docs/archive/2026-06-22-history-rewrite-survey.md
```
Expected: the three renames staged (`R`), three content edits, no untracked strays, all three target files exist.

- [x] **Step 7: Commit**

```bash
git add -A
git commit -m "docs: archive three shipped/stale planning docs (#39)"
```

---

### Task 2: Add `docs/README.md` index

**Files:**
- Create: `docs/README.md`

**Interfaces:**
- Consumes: the `docs/archive/` directory produced/relocated by Task 1 (linked as a directory, so robust to Task 1's exact filenames).
- Produces: the docs entry-point index. No later task depends on it.

- [x] **Step 1: Create `docs/README.md` with three sections**

Content (build the ADR rows from `docs/adr/*.md` — numbers, titles, and `Status:` line; all 21 are currently `accepted`). Structure:

````markdown
# Documentation Index

Entry point and map for Jaunder's documentation. New here? Start with the
[root README](../README.md), then [CONTRIBUTING](../CONTRIBUTING.md) — the
working hub for humans and agents.

## Durable docs

| Doc | What it's for |
|-----|---------------|
| [CONTRIBUTING](../CONTRIBUTING.md) | Definitive working guide: setup, hooks, workflow, testing, invariants. Read first. |
| [CONTEXT](../CONTEXT.md) | Domain glossary / ubiquitous language. |
| [ARCHITECTURE](ARCHITECTURE.md) | System architecture (Leptos, single-binary, storage); links the ADRs. |
| [DESIGN](DESIGN.md) | High-level design / how an instance runs. |
| [ROADMAP](ROADMAP.md) | Completed-milestone ledger and direction. |
| [observability](observability.md) | OpenTelemetry tracing for backend + e2e. |
| [web-style-guide](web-style-guide.md) | Conventions for `web/src/pages/` components and widgets. |
| [atompub-marsedit-acceptance](atompub-marsedit-acceptance.md) | Manual MarsEdit/AtomPub (RFC 5023) acceptance checklist. |

## Architecture Decision Records

Architecture decisions live in [`adr/`](adr/), one file per decision (per
[ADR-0000](adr/0000-documentation-strategy.md)).

| # | Title | Status |
|---|-------|--------|
| [0000](adr/0000-documentation-strategy.md) | Documentation Strategy | accepted |
| [0001](adr/0001-pluggable-storage-backends.md) | Pluggable Storage Backends | accepted |
| ... (one row per ADR file 0000–0020, exact titles from each file's H1, exact filename from `ls docs/adr/`) |

## Archive

Superseded planning docs, design specs, and dated snapshots are kept in
[`archive/`](archive/) rather than deleted, named `YYYY-MM-DD-<topic>.md` (the
date the work happened or shipped). They are frozen historical records — read
them for "why we did X," not for current behavior.
````

When building the ADR table, the ADR slugs are (verify against `ls docs/adr/`): 0000 documentation-strategy, 0001 pluggable-storage-backends, 0002 frontend-framework, 0003 asset-management, 0004 pagination, 0005 unified-content-model, 0006 storage-isolation, 0007 dual-path-auth, 0008 single-binary-deployment, 0009 high-fidelity-retention, 0010 multi-protocol-integration, 0011 unified-observability, 0012 env-aware-e2e-timeouts, 0013 server-submodule-pattern, 0014 atompub-app-specific-passwords, 0015 serialization-surfaces, 0016 di-appstate-composition-root, 0017 error-handling-boundary, 0018 timing-equalized-auth, 0019 generic-backends-via-dialect, 0020 content-visibility-and-subscription. Titles must be copied from each file's H1 verbatim; statuses from each file's `* Status:` line.

- [x] **Step 2: Verify every link in the index resolves**

Run (extracts every relative markdown link target from the index and checks each exists, resolved relative to `docs/`):
```bash
cd docs && rg -o '\]\(([^)]+)\)' -r '$1' README.md | while read -r l; do t="${l%%#*}"; [ -e "$t" ] || echo "BROKEN: $l"; done; cd ..
```
Expected: **no `BROKEN:` lines.**

- [x] **Step 3: Commit**

```bash
git add docs/README.md
git commit -m "docs: add docs/README.md index (durable docs, ADR index, archive pointer) (#39)"
```

---

### Task 3: Final verification gate

**Files:** none (verification only).

- [x] **Step 1: Re-confirm whole-tree link integrity for the moved docs**

Run:
```bash
rg -n --no-heading -g '!docs/archive' -g '!docs/superpowers' -g '!target' \
  'code-analysis-2026-06-12|server-submodule-refactor-plan|HISTORY-REWRITE-SURVEY' .
```
Expected: no output.

- [x] **Step 2: Run the standing gate**

Run (bare command, from the worktree root, so context-mode/`isError` is meaningful — but this is docs-only so it should be a fast no-op rebuild):
```bash
cargo xtask validate --no-e2e
```
Expected: exit 0 (`xtask-done: ... ok=true`). Read `.xtask/last-result.json` `steps[]` only if it fails.

- [x] **Step 3: Confirm clean tree**

Run:
```bash
git status --short
```
Expected: empty (everything committed).

---

## Self-Review

- **Spec coverage:** Part 1 (archive three docs) → Task 1. Part 2 (`docs/README.md` with durable table + ADR index + archive pointer) → Task 2. Testing (link integrity + `validate --no-e2e`) → Tasks 1 Step 5, 2 Step 2, and Task 3. Out-of-scope items (SDD scratch, live spec drafts, untracked-docs decision) → not touched by any task. ✓
- **Placeholder scan:** The ADR table is the one spot that says "one row per ADR file" — the exact slugs and the build rule (H1 verbatim + Status line) are given, so it is mechanical, not a placeholder. No "TBD/TODO/handle edge cases." ✓
- **Type/path consistency:** Archive target paths in Task 1 match the spec table exactly; Task 2 links the `archive/` directory, decoupled from those filenames; relative-link forms (`../README.md` from `docs/`, `../adr/...` from `docs/archive/`) are specified. ✓
