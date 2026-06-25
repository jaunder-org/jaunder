# DX P6 — Finish Branch + GitHub Bookkeeping Runbook

> **For agentic workers:** This is a RUNBOOK of outward-facing GitHub operations, not a TDD plan. Steps use checkbox (`- [ ]`) syntax. Every step that creates/changes a GitHub object is confirmed with the user before running.

**Goal:** Land the DX program's repo docs on `main`, establish the "Developer Experience" Project as the home for DX work, close issue #5 (its asks are now delivered), and park the one deferred item (DRY/altitude).

**Context:** Most J-work is already complete and live — J1 (`jaunder-ship` skill), J2 (`jaunder-dispatch` skill), J4 (deny-list rework), J3 (folded into `feedback_ctx_run_long_scripts`), plus G1–G8. So no J-issues are filed as open work; the spec + plans (this branch) are the record. The only open future item is deferred DRY.

## Global Constraints

- **Outward-facing:** creating a GH Project, closing an issue, and opening an issue are public actions. Confirm specifics with the user before each create/close.
- **No `Co-Authored-By`.** Don't push or merge to `main` without explicit user direction (this runbook's Task 0 is exactly that direction point).
- The repo docs to land: `docs/superpowers/specs/2026-06-24-dx-program-design.md` + `docs/superpowers/plans/2026-06-24-dx-p{1..6}-*.md`.

---

## Task 0: Finish the development branch

**Decision (user):** how to land the branch `worktree-dx-program` (off `main`, docs only):
- **Merge to main** (fast, local; these are docs), or
- **Open a PR** (review trail), or
- **Leave on the branch** (do GH bookkeeping with self-contained summaries; docs not referenceable by URL).

- [ ] **Step 1: Invoke `superpowers:finishing-a-development-branch`** and execute the user's chosen option. If merging/PR, ensure the working tree is clean and the branch contains all DX commits.
- [ ] **Step 2: Exit the worktree** (`ExitWorktree`, keep) once the branch is landed, if appropriate.

## Task 1: Create the "Developer Experience" Project

- [ ] **Step 1 (confirm name):** default `Developer Experience` (parallel to `Privacy` / `Operational Support`).
- [ ] **Step 2:** `gh project create --owner jaunder-org --title "Developer Experience"` (and link to the repo, matching how the other org Projects are linked). Note the project number returned.
- [ ] **Step 3:** Confirm it lists alongside the existing org Projects (`gh project list --owner jaunder-org`).

## Task 2: Close issue #5 with a delivery summary

Issue #5 = "author project dev-skills + xtask timeout/duration guidance." Delivered by J2 (`jaunder-dispatch`, includes the e2e-timeout guidance) + the broader program.

- [ ] **Step 1:** Add it to the Developer Experience Project.
- [ ] **Step 2:** Comment summarizing what was delivered (the two skills, the guard-rail hooks, the memory gardening) and pointing to `docs/superpowers/specs/2026-06-24-dx-program-design.md` (resolves once Task 0 lands it on `main`).
- [ ] **Step 3:** `gh issue close 5` (only after user confirms #5's asks are fully met).

## Task 3: Park the deferred DRY/altitude issue

- [ ] **Step 1:** `gh issue create` titled e.g. "dx: helpers-over-boilerplate + keep AppState storage-only (altitude/DRY)", labelled `dx`, body referencing the spec's deferral and the evidence (#5 boilerplate, #42 AppState drift). Leave it OPEN (it's future work).
- [ ] **Step 2:** Add it to the Developer Experience Project, status Todo/Backlog.

---

## Self-Review

- **Spec coverage:** DX Project → Task 1. Close #5 → Task 2. Park DRY → Task 3. "Land the docs" precondition → Task 0. No open J-issues filed (work is done — recorded via spec/plans instead, a deliberate deviation from the spec's original "file J-issues," since those are now complete).
- **Placeholders:** none — exact `gh` commands and titles; the only blanks are user confirmations (branch-landing choice, project name, close-#5 go).
- **Outward-action safety:** every create/close gated on user confirmation.

## Execution note

This runbook produces GitHub objects + a `main` merge. Mark steps `- [x]` and commit this plan; the GH actions themselves are the deliverable.
