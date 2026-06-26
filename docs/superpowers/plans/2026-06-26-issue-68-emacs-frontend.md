# Emacs Blogging Front-End — Epic Orchestration Plan (issue #68)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to
> work through this plan. Steps use checkbox (`- [ ]`) syntax for tracking. This is an
> **orchestration** plan: it stands up the epic's tracking (milestone + issues + ADRs)
> and back-fills identifiers. It deliberately contains **no application code** — each
> unit issue gets its own `jaunder-develop` cycle (worktree → spec confirm →
> bite-sized plan → implement) when picked up.

**Goal:** Stand up the tracking and architectural record for the Emacs blogging
front-end epic, so the six units can each be built as their own one-issue cycle.

**Architecture:** A milestone groups the epic; six unit issues (A/B/Infra/C/D/Slug)
carry the build order via native dependency links; three ADRs (0023–0025) record the
cross-cutting decisions; the reviewed spec and the ADRs are then back-filled with the
issues' final numbers and committed on the `worktree-issue-68-emacs-frontend` branch.

**Tech Stack:** `gh` CLI (issues, `gh api` for the milestone), GitHub Projects, git.

**Status:** Executed inline 2026-06-26. Milestone **#4** + issues **#70–#83** created,
dependencies/projects/priorities wired, IDs back-filled into the spec and ADRs. The
tables below show the final numbers (token names kept in parentheses for cross-ref).

## Global Constraints

- **Issue conventions (`jaunder-issues`):** every issue sets `--type Task|Bug|Feature`;
  one concern per issue; **topic labels only** (no priority/layer labels); **priority
  is a Project field**, not a label; add every issue to a Project (Jaunder Backlog #1
  by default); express ordering with native `--add-blocked-by`/`--blocking`/`--parent`,
  never "depends on #N" prose.
- **No `Co-Authored-By` trailers** in any commit (overrides the global default).
- **Worktree only / never `main`:** all commits land on
  `worktree-issue-68-emacs-frontend`. Review against the fork point:
  `git diff wt-base-issue-68..HEAD`.
- **Commit only after explicit user approval** of this plan.
- **Spec:** `docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md`
  (revised 2026-06-26). **ADRs:** `docs/adr/0023..0025` (already drafted this cycle),
  plus the `0015` status edit.

---

## Issue inventory (the canonical set to create)

The placeholder tokens (`A`, `B`, `Infra`, `C`, `D`, `Slug`) are what get replaced
with real `#NN` during back-fill (Task 3). C/D do **not** get sub-issues here — those
are created during C's and D's own cycles.

| Issue | Title | Type | Topic label | Deps | Priority (Project) |
|---|---|---|---|---|---|
| **#70** (A) | `scheduled publishing: unify visibility + restart-durable go-live (storage/web/AtomPub)` | Feature | — | — | **P1** |
| **#71** (B) | `atompub: per-entry format media types, j:slug + capability discovery, server-side org canonicalization` | Feature | — | — | **P1** |
| **#72** (Slug) | `slug: Unicode-preserving, never-fail generation (product-wide)` | Task | — | — | **P1** |
| **#73** (Infra) | `emacs: elisp package skeleton, ERT harness, flake + verify-gate wiring` | Task | dx | — | **P1** |
| **#74** (C) | `emacs: authoring / publish workflow (org → AtomPub)` | Feature | — | blocked-by #70, #71, #73 | **P2** |
| **#75** (D) | `emacs: blog management / reconcile` | Feature | — | blocked-by #70, #71, #73 | **P2** |

Follow-ons (filed now so they can be picked up concurrently; **not** on the v1 path):

| Issue | Title | Type | Priority | Blocked by |
|---|---|---|---|---|
| **#76** | `emacs: self-provision an app password (login → create_app_password)` | Task | **P3** | #74 |
| **#77** | `atompub: parse the full org header block server-side (raw-org web authoring)` | Task | **P3** | #71 |
| **#78** | `atompub: content-based ETag (removes time-based divergence false-positive)` | Task | **P3** | — (independent) |
| **#79** | `atompub: idempotency key for post create (duplicate-on-retry; mobile)` | Task | **P3** | — (independent) |
| **#80** | `emacs: download + localize media on pull (offline preview of pulled posts)` | Task | **P3** | #75 |
| **#81** | `atompub: emit WWW-Authenticate challenge on 401 (deferred pending client experiments)` | Task | **P3** | #74 |
| **#82** | `coverage: include the emacs client in coverage` | Task | **P4** | #74, #75 |
| **#83** | `coverage: include the e2e tests in coverage` | Task | **P4** | — (independent) |

Existing issues to attach to the milestone (already filed): **#15** (full
scheduled-post management UI), **#25** (broaden Emacs media upload beyond images).

---

## Task 1: Create the milestone and all issues

**Files:** none (GitHub state only).

- [x] **Step 1: Create the milestone**

```bash
gh api repos/jaunder-org/jaunder/milestones -f title="Emacs blogging front-end" \
  -f state=open \
  -f description="Authoring + reconcile over AtomPub, plus the server-side extensions it needs. Epic spec: docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md"
```

Note the returned milestone `number` (call it `$MS`).

- [x] **Step 2: Create the six unit issues** (one `gh issue create` each, capturing
  the printed URL/number). Bodies: a one-paragraph summary plus a link to the spec
  section. Example for A — repeat the pattern for B, Slug, Infra, C, D using the
  inventory table's title/type/label:

```bash
gh issue create --repo jaunder-org/jaunder --type Feature \
  --milestone "Emacs blogging front-end" \
  --title "scheduled publishing: unify visibility + restart-durable go-live (storage/web/AtomPub)" \
  --body "Unit A of the Emacs front-end epic. Three states from published_at (draft/scheduled/live); unify every public read on \`published_at IS NOT NULL AND <= now\`; restart-durable go-live (in-memory window + startup feed-relative catch-up; writes enqueue their own regen); AtomPub honors <published>. Spec: docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md (Unit A)."
```

Infra additionally takes `--label dx`. Record each number: `$A $B $SLUG $INFRA $C $D`.

- [x] **Step 3: Create the eight follow-on issues** from the follow-ons table (same
  pattern; all `--type Task`; **do not** put them on the milestone — they are off the
  v1 path). Record `$F_SELFPROV $F_BETA $F_ETAG $F_IDEM $F_PULLMEDIA $F_WWWAUTH
  $F_COVEMACS $F_COVE2E`.

- [x] **Step 4: Attach the existing deferred-tail issues to the milestone**

```bash
gh issue edit 15 25 --repo jaunder-org/jaunder --milestone "Emacs blogging front-end"
```

---

## Task 2: Wire dependencies, projects, and priorities

**Files:** none (GitHub state only).

- [x] **Step 1: Native dependency links.** Unit C and D are blocked by A, B, and
  Infra; the follow-ons are blocked per the follow-ons table:

```bash
# Unit blockers
gh issue edit $C --repo jaunder-org/jaunder --add-blocked-by $A --add-blocked-by $B --add-blocked-by $INFRA
gh issue edit $D --repo jaunder-org/jaunder --add-blocked-by $A --add-blocked-by $B --add-blocked-by $INFRA
# Follow-on blockers (emacs follow-ons need the front-end finished first)
gh issue edit $F_SELFPROV  --repo jaunder-org/jaunder --add-blocked-by $C
gh issue edit $F_BETA      --repo jaunder-org/jaunder --add-blocked-by $B
gh issue edit $F_PULLMEDIA --repo jaunder-org/jaunder --add-blocked-by $D
gh issue edit $F_WWWAUTH   --repo jaunder-org/jaunder --add-blocked-by $C
gh issue edit $F_COVEMACS  --repo jaunder-org/jaunder --add-blocked-by $C --add-blocked-by $D
# F_ETAG, F_IDEM, F_COVE2E are independent — no blockers.
```

(If the installed `gh` lacks `--add-blocked-by`, set the relationship from each issue's web
"Development"/"Relationships" panel; do **not** fall back to "depends on #N" prose.)

- [x] **Step 2: Add every new issue to the Jaunder Backlog project (#1)**

```bash
for url in <the 14 issue URLs from Task 1>; do
  gh project item-add 1 --owner jaunder-org --url "$url"
done
```

(If this errors on scope: `gh auth refresh -s project`.)

- [x] **Step 3: Set Priority = P4 on the two coverage follow-ons** inside the Project
  (Priority is a Project field, not a label). In the Jaunder Backlog project, set the
  Priority field of `$F_COVEMACS` and `$F_COVE2E` to **P4** (via the project board UI,
  or `gh project item-edit` with the project's Priority field id).

- [x] **Step 4: Verify no typeless issues slipped in**

```bash
gh api graphql -f query='{repository(owner:"jaunder-org",name:"jaunder"){issues(first:80,states:OPEN){nodes{number issueType{name}}}}}' \
  --jq '[.data.repository.issues.nodes[]|select(.issueType==null)|.number]'
```

Expected: `[]`.

---

## Task 3: Back-fill identifiers into the spec and ADRs

**Files:**
- Modify: `docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md`
- Modify: `docs/adr/0024-server-side-org-canonicalization.md` (the "β" follow-on ref)
- Modify: `docs/superpowers/plans/2026-06-26-issue-68-emacs-frontend.md` (this file)

- [x] **Step 1: Replace placeholder tokens with real numbers in the spec.** In the
  "Issue decomposition & follow-ons" table and prose, replace `A`/`B`/`Infra`/`C`/`D`/
  `Slug` and each follow-on bullet with its `#NN`. Also update the header's "Milestone"
  line and any in-text unit references (e.g. "deferred to #15" already concrete).

- [x] **Step 2: Back-fill the ADRs.** In ADR-0024, replace the "β" follow-on mention
  with `#<F_BETA>`. (ADR-0023/0025 reference each other and 0015 by number already — no
  issue numbers embedded — leave as-is.)

- [x] **Step 3: Back-fill this plan's inventory tables** — replace the token column
  values with the real `#NN` so the plan and spec agree.

- [x] **Step 4: Sanity check** — no bare placeholder tokens remain:

```bash
grep -nE '\b(F-selfprov|F-beta|F-etag|F-idem|F-pullmedia|F-wwwauth|F-covemacs|F-cove2e)\b' \
  docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md \
  docs/superpowers/plans/2026-06-26-issue-68-emacs-frontend.md docs/adr/0024-*.md
```

Expected: no matches (every token replaced by a number).

---

## Task 4: Commit the planning artifacts

**Files:** the spec, the three new ADRs, the `0015` edit, and this plan.

- [x] **Step 1: Confirm the branch and review the diff**

```bash
git -C /home/mdorman/src/jaunder/.claude/worktrees/issue-68-emacs-frontend rev-parse --abbrev-ref HEAD   # worktree-issue-68-emacs-frontend
git -C /home/mdorman/src/jaunder/.claude/worktrees/issue-68-emacs-frontend diff wt-base-issue-68..HEAD --stat
```

- [x] **Step 2: Stage and commit** (no `Co-Authored-By` trailer):

```bash
git -C /home/mdorman/src/jaunder/.claude/worktrees/issue-68-emacs-frontend add \
  docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md \
  docs/superpowers/plans/2026-06-26-issue-68-emacs-frontend.md \
  docs/adr/0015-atompub-serialization-surfaces.md \
  docs/adr/0023-atompub-jaunder-wire-extensions.md \
  docs/adr/0024-server-side-org-canonicalization.md \
  docs/adr/0025-unicode-slug-generation.md
git -C /home/mdorman/src/jaunder/.claude/worktrees/issue-68-emacs-frontend commit -m "docs: spec, ADRs, and orchestration plan for the Emacs blogging front-end epic (#68)"
```

This is a docs-only change, so no `cargo xtask` gate is required (no code touched).

---

## Handoff — per-unit cycles

After this plan is executed, each unit is started as **its own** `jaunder-develop`
cycle in **build order A → B → (Slug ∥) → Infra → (C, D)**:

1. `jaunder-start` for the unit's issue (`issue-<NN>-<slug>` worktree).
2. Confirm/refine the unit's slice of the spec; **C and D create their sub-issues**
   (the ~4 review-sized children each) as that cycle's first planning step.
3. `superpowers:writing-plans` → the unit's bite-sized TDD plan.
4. Implement via `jaunder-iterate`; ship via `jaunder-ship`.

The ADRs (0023–0025) are the binding architectural contracts those cycles build
against.
