# #679 Archive Stale Planning Docs Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move stale closed-issue cycle artifacts out of active
`docs/superpowers/` directories and document the live-design exceptions.

**Architecture:** This is a docs-only repository hygiene change. It uses
`git mv` for seven issue-cycle artifacts, prepends archive status notes, adds a
small `docs/superpowers/specs/README.md`, and updates references that otherwise
keep pointing at the old active-cycle paths.

**Tech Stack:** Markdown docs; verification via `grep`, `glob`, `git diff`, and
`devtool run -- cargo xtask check --no-test`.

**Scope:** In: the seven required file moves, status notes, old-path reference
reconciliation, and `docs/superpowers/specs/README.md`. Out: moving the two
no-issue live design drafts, changing `jaunder-develop`, generated indexes,
archive automation, ADR changes.

**Task list:**

1. Move the seven stale issue-cycle artifacts and add archive status notes.
2. Document active specs semantics and reconcile old-path references.
3. Verify and commit the docs-only change.

**Key risks/decisions:**

- Risk: archive path collisions in flat `docs/archive/`; use required `-spec` /
  `-plan` suffixes.
- Risk: stale links remain in moved plans; update spec links to the
  corresponding archive files.
- Decision: leave the two no-issue live drafts in `docs/superpowers/specs/`,
  because #39 explicitly classified them as live drafts for unshipped work.

## Global Constraints

- Follow `CONTRIBUTING.md`: structured edits, focused commits, no generated
  archive index.
- Keep the load-bearing `issue-679` token in this plan path until ship archives
  it.
- Spec: do not move
  `docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md`.
- Spec: do not move
  `docs/superpowers/specs/2026-06-19-content-visibility-layer-c-design.md`.
- Spec: every moved file starts with a `> **Status:**` blockquote naming #679.
- Spec: `docs/superpowers/specs/README.md` must state the active-cycle rule and
  name both live design drafts.
- Gate command: `devtool run -- cargo xtask check --no-test`.

---

### Task 1: Move stale cycle artifacts into archive

**Files:**

- Move:
  `docs/superpowers/specs/2026-07-07-issue-303-web-canonical-colocated-leptos.md`
  -> `docs/archive/2026-07-07-issue-303-web-canonical-colocated-leptos-spec.md`
- Move: `docs/superpowers/specs/2026-07-13-issue-400-invite-code-newtype.md` ->
  `docs/archive/2026-07-13-issue-400-invite-code-newtype-spec.md`
- Move: `docs/superpowers/plans/2026-07-13-issue-400-invite-code-newtype.md` ->
  `docs/archive/2026-07-13-issue-400-invite-code-newtype-plan.md`
- Move: `docs/superpowers/specs/2026-07-14-issue-433-invitation-process.md` ->
  `docs/archive/2026-07-14-issue-433-invitation-process-spec.md`
- Move: `docs/superpowers/plans/2026-07-14-issue-433-invitation-process.md` ->
  `docs/archive/2026-07-14-issue-433-invitation-process-plan.md`
- Move: `docs/superpowers/specs/2026-07-17-issue-315-web-auth-colocate.md` ->
  `docs/archive/2026-07-17-issue-315-web-auth-colocate-spec.md`
- Move: `docs/superpowers/plans/2026-07-17-issue-315-web-auth-colocate.md` ->
  `docs/archive/2026-07-17-issue-315-web-auth-colocate-plan.md`

**Interfaces:**

- Consumes: the Required moves table in the approved spec.
- Produces: archived files with preserved dates/topics and no active-cycle copy.

- [x] **Step 1: Move the files with git**

  Run these exact moves:

  ```bash
  git mv docs/superpowers/specs/2026-07-07-issue-303-web-canonical-colocated-leptos.md docs/archive/2026-07-07-issue-303-web-canonical-colocated-leptos-spec.md
  git mv docs/superpowers/specs/2026-07-13-issue-400-invite-code-newtype.md docs/archive/2026-07-13-issue-400-invite-code-newtype-spec.md
  git mv docs/superpowers/plans/2026-07-13-issue-400-invite-code-newtype.md docs/archive/2026-07-13-issue-400-invite-code-newtype-plan.md
  git mv docs/superpowers/specs/2026-07-14-issue-433-invitation-process.md docs/archive/2026-07-14-issue-433-invitation-process-spec.md
  git mv docs/superpowers/plans/2026-07-14-issue-433-invitation-process.md docs/archive/2026-07-14-issue-433-invitation-process-plan.md
  git mv docs/superpowers/specs/2026-07-17-issue-315-web-auth-colocate.md docs/archive/2026-07-17-issue-315-web-auth-colocate-spec.md
  git mv docs/superpowers/plans/2026-07-17-issue-315-web-auth-colocate.md docs/archive/2026-07-17-issue-315-web-auth-colocate-plan.md
  ```

- [x] **Step 2: Add archive status notes**

  Prepend each moved file with this shape, customized to spec/plan and issue
  number:

  ```markdown
  > **Status:** ARCHIVED — shipped issue-cycle spec/plan for issue #<N>; moved
  > out of active `docs/superpowers/` cycle directories by #679.
  ```

  For #303 use `spec` and mention it was an umbrella spec with no matching plan:

  ```markdown
  > **Status:** ARCHIVED — shipped umbrella spec for issue #303; moved out of
  > active `docs/superpowers/` cycle directories by #679. No matching plan
  > existed.
  ```

- [x] **Step 3: Verify active-cycle paths are gone**

  Run equivalent `glob` checks for these patterns:
  - `docs/superpowers/specs/*issue-303*`
  - `docs/superpowers/specs/*issue-315*`
  - `docs/superpowers/plans/*issue-315*`
  - `docs/superpowers/specs/*issue-400*`
  - `docs/superpowers/plans/*issue-400*`
  - `docs/superpowers/specs/*issue-433*`
  - `docs/superpowers/plans/*issue-433*`

  Expected: no matches.

### Task 2: Document active specs and reconcile references

**Files:**

- Create: `docs/superpowers/specs/README.md`
- Modify: `docs/archive/2026-07-13-issue-400-invite-code-newtype-plan.md`
- Modify: `docs/archive/2026-07-14-issue-433-invitation-process-plan.md`
- Modify: `docs/archive/2026-07-17-issue-315-web-auth-colocate-plan.md`
- Reference: `docs/archive/2026-06-25-issue-39-docs-archive-and-index-design.md`

**Interfaces:**

- Consumes: archived filenames from Task 1.
- Produces: a README documenting active spec semantics and updated links from
  archived plans to archived specs.

- [x] **Step 1: Add `docs/superpowers/specs/README.md`**

  Create a short Markdown file with these sections and exact commitments:
  - `# Superpowers specs`
  - State: this directory normally holds in-flight issue-cycle specs used by
    `jaunder-develop` state derivation.
  - State: when an issue ships, its spec belongs in `docs/archive/`; this
    applies even when an umbrella issue has no matching plan.
  - State: two explicitly live design drafts remain here for unshipped work:
    - `2026-06-16-emacs-blogging-frontend-design.md` — Emacs blogging frontend
      epic design.
    - `2026-06-19-content-visibility-layer-c-design.md` — Content Visibility
      Layer C design.
  - State: do not archive those two until their work lands or a later issue
    chooses a new home.

- [x] **Step 2: Update plan-to-spec links after archive moves**

  Update references found during spec drafting:
  - In `docs/archive/2026-07-13-issue-400-invite-code-newtype-plan.md`, replace
    `../specs/2026-07-13-issue-400-invite-code-newtype.md` with
    `2026-07-13-issue-400-invite-code-newtype-spec.md`; update displayed path
    text to the archive path if present.
  - In `docs/archive/2026-07-14-issue-433-invitation-process-plan.md`, replace
    `../specs/2026-07-14-issue-433-invitation-process.md` with
    `2026-07-14-issue-433-invitation-process-spec.md`; update displayed path
    text to the archive path if present.
  - In `docs/archive/2026-07-17-issue-315-web-auth-colocate-plan.md`, replace
    the old `docs/superpowers/specs/2026-07-17-issue-315-web-auth-colocate.md`
    text with `docs/archive/2026-07-17-issue-315-web-auth-colocate-spec.md`.

- [x] **Step 3: Search for old paths**

  Use the Grep tool over tracked Markdown for the old source paths:

  ```regex
  docs/superpowers/(?:specs|plans)/(?:2026-07-07-issue-303-web-canonical-colocated-leptos|2026-07-13-issue-400-invite-code-newtype|2026-07-14-issue-433-invitation-process|2026-07-17-issue-315-web-auth-colocate)\.md
  ```

  Expected: only the approved spec/plan for #679 may name old paths as the
  historical Required moves contract. All other references are updated to
  archive paths.

### Task 3: Verify and commit docs-only change

**Files:**

- All files from Tasks 1 and 2.
- Modify:
  `docs/superpowers/plans/2026-08-20-issue-679-archive-stale-planning-docs.md`
  checkbox state before commit.
- Include:
  `docs/superpowers/specs/2026-08-20-issue-679-archive-stale-planning-docs.md`

**Interfaces:**

- Consumes: completed Tasks 1 and 2.
- Produces: one checked commit for issue #679.

- [x] **Step 1: Inspect diff**

  Run: `git diff --stat` and inspect the full staged/unstaged diff.

  Expected: seven renames into `docs/archive/`, one new
  `docs/superpowers/specs/README.md`, old reference updates, and the lifecycle
  spec/plan files. No Rust/source changes.

- [x] **Step 2: Run the fast gate**

  Run: `devtool run -- cargo xtask check --no-test`

  Expected: PASS — JSON summary has `ok: true` and `exit_code: 0`.

- [x] **Step 3: Commit**

  Before committing, tick all completed task checkboxes in this plan.

  Stage exactly the moved archive files, `docs/superpowers/specs/README.md`,
  this plan, and the approved spec:

  ```bash
  git add docs/archive/2026-07-07-issue-303-web-canonical-colocated-leptos-spec.md docs/archive/2026-07-13-issue-400-invite-code-newtype-spec.md docs/archive/2026-07-13-issue-400-invite-code-newtype-plan.md docs/archive/2026-07-14-issue-433-invitation-process-spec.md docs/archive/2026-07-14-issue-433-invitation-process-plan.md docs/archive/2026-07-17-issue-315-web-auth-colocate-spec.md docs/archive/2026-07-17-issue-315-web-auth-colocate-plan.md docs/superpowers/specs/README.md docs/superpowers/specs/2026-08-20-issue-679-archive-stale-planning-docs.md docs/superpowers/plans/2026-08-20-issue-679-archive-stale-planning-docs.md
  ```

  Commit:

  ```bash
  git commit -m "docs: archive stale planning docs"
  ```

  No `Co-Authored-By` trailer.
