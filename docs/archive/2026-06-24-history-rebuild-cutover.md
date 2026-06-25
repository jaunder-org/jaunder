# History-Rebuild Cutover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `jaunder-org/jaunder` on GitHub with the rebuilt legible history (branch `spine`) by delete-and-recreate, preserving settings, with every destructive step gated behind a verified non-destructive rehearsal and a recoverable rollback bundle.

**Architecture:** Three movements — (1) **non-destructive prep & rehearsal** (rollback bundle, settings dump, full §8 gate C settings round-trip on a throwaway repo, GitHub-side eyeball of the spine); (2) **the destructive cutover** (delete → recreate → push `spine`→`main` → restore settings), which runs only on an explicit human "go"; (3) **post-cutover** (verification, §9 backlog migration, deferred teardown). The rebuilt history is sourced from the canonical durable bundle, not the volatile tmpfs clone.

**Tech Stack:** `git`, `gh` CLI (GitHub REST via `gh api`), `jq`, `bd` (beads).

## Global Constraints

- **Canonical source of the new history:** `/home/mdorman/jaunder-rebuild/jaunder-spine.bundle` (branch `spine`). The tmpfs clone `/tmp/jaunder-rebuild` is volatile — never the source of truth for the push.
- **Only `spine`→`main` is pushed** to the new repo. None of the scaffolding refs (`linear-clean`, `orig-baseline`, `spine-topic`, `spine-nextest-green`, `pre-rebuild`, `L0`/`L-cleaned`/`L3-woven`, `prebatch*`, `prepass-done`, `post-*`) go to GitHub.
- **Target identity (verbatim, from the live repo 2026-06-24):** owner `jaunder-org`, name `jaunder`, `visibility: public`, `default_branch: main`, `has_issues/has_projects/has_wiki: true`, `description` = `` `Jaunder` is an easily-hosted, multi-protocol social media application written in Rust. ``, no homepage. **Customized merge flags (differ from fresh-repo defaults, must be restored):** `allow_squash_merge: false`, `allow_merge_commit: true`, `allow_rebase_merge: false`, `allow_auto_merge: true`, `delete_branch_on_merge: false`.
- **Settings surface to restore:** core settings + **10 labels** + **1 ruleset**. Confirmed empty (nothing to re-enter): branch protection, webhooks, environments, autolinks, deploy keys, custom properties, Actions/Dependabot secrets, Actions variables; all security-analysis flags disabled.
- **Content surface to preserve:** **1 open GitHub issue** (#39 "Coverage gate: surface test failures distinctly from coverage regressions", label `tooling`) — erased on delete; must be dumped (Task 2) and recreated (Task 8). Closed issues and all PR threads are intentionally not preserved (§7). The new repo's recreated issue numbers will differ from the old ones.
- **Destructive steps (Task 5 onward) require an explicit human "go"** in the session, immediately before execution. Never self-authorize.
- **Rollback net:** the old-repo `--mirror` bundle from Task 1 plus the existing local checkout. It is removed only in the deferred teardown (Task 9), never during cutover.
- All work for this plan lives under `/home/mdorman/jaunder-rebuild/cutover/` (durable, on `/`).

---

### Task 0: Workspace, auth, and source-of-truth verification

**Files:**
- Create: `/home/mdorman/jaunder-rebuild/cutover/` (working dir)
- Create: `/home/mdorman/jaunder-rebuild/cutover/jaunder-new/` (fresh clone of the spine bundle)

**Interfaces:**
- Produces: `jaunder-new/` with branch `spine` checked out — the push source for Tasks 4 and 5.

- [ ] **Step 1: Create the durable working dir**

```bash
mkdir -p /home/mdorman/jaunder-rebuild/cutover
```

- [ ] **Step 2: Clone the canonical spine bundle (not the tmpfs clone)**

```bash
git clone /home/mdorman/jaunder-rebuild/jaunder-spine.bundle \
  /home/mdorman/jaunder-rebuild/cutover/jaunder-new
```

- [ ] **Step 3: Verify the spine in the fresh clone matches Phase-B invariants**

```bash
git -C /home/mdorman/jaunder-rebuild/cutover/jaunder-new checkout spine
# 49 --no-ff merges on the first-parent spine (51 first-parent entries incl. tip + root):
git -C /home/mdorman/jaunder-rebuild/cutover/jaunder-new \
  log --first-parent --merges --oneline spine | wc -l
# Lossless vs the reviewed linear line — must list ONLY the 2 retired docs:
git -C /home/mdorman/jaunder-rebuild/cutover/jaunder-new \
  diff --name-only spine linear-clean
```

Expected: merge count `= 49`; diff lists exactly `docs/superpowers/plans/2026-06-22-history-rebuild-phase-a.md` and `docs/superpowers/specs/2026-06-22-history-rebuild-design.md`.

- [ ] **Step 4: Confirm spine tip subject reads as the intended changelog top**

```bash
git -C /home/mdorman/jaunder-rebuild/cutover/jaunder-new \
  log --first-parent -1 --format='%h %s' spine
```

Expected: the emacs-frontend-plan commit (`docs(docs): plan emacs blogging front-end over AtomPub`).

- [ ] **Step 5: Acquire `delete_repo` scope**

```bash
gh auth refresh -s delete_repo
gh auth status
```

Expected: token scopes now include `delete_repo`. (Follow the device-flow prompt; if running non-interactively, hand this to the user with `!gh auth refresh -s delete_repo`.)

- [ ] **Step 6: Confirm org-owner rights to delete an org repo**

```bash
gh api orgs/jaunder-org/memberships/mdorman --jq '.role'
```

Expected: `admin`. If not `admin`, STOP — deletion will fail; resolve org permissions first.

---

### Task 1: Old-repo rollback bundle (the safety net)

**Files:**
- Create: `/home/mdorman/jaunder-rebuild/cutover/jaunder-old-mirror.git/` (mirror clone)
- Create: `/home/mdorman/jaunder-rebuild/cutover/jaunder-old-rollback.bundle`

**Interfaces:**
- Produces: a verified all-refs bundle of the **current** GitHub repo, the rollback target for the cutover.

- [ ] **Step 1: Mirror-clone the current live repo**

```bash
git clone --mirror git@github.com:jaunder-org/jaunder.git \
  /home/mdorman/jaunder-rebuild/cutover/jaunder-old-mirror.git
```

- [ ] **Step 2: Bundle all refs**

```bash
git -C /home/mdorman/jaunder-rebuild/cutover/jaunder-old-mirror.git \
  bundle create /home/mdorman/jaunder-rebuild/cutover/jaunder-old-rollback.bundle --all
```

- [ ] **Step 3: Verify the bundle by re-clone and ref diff**

```bash
git bundle verify /home/mdorman/jaunder-rebuild/cutover/jaunder-old-rollback.bundle
git clone --mirror /home/mdorman/jaunder-rebuild/cutover/jaunder-old-rollback.bundle \
  /tmp/rollback-verify.git
# Ref sets must be identical:
diff \
  <(git -C /home/mdorman/jaunder-rebuild/cutover/jaunder-old-mirror.git for-each-ref --format='%(refname) %(objectname)' | sort) \
  <(git -C /tmp/rollback-verify.git for-each-ref --format='%(refname) %(objectname)' | sort) \
  && echo ROLLBACK_BUNDLE_OK
rm -rf /tmp/rollback-verify.git
```

Expected: `git bundle verify` reports OK and prints `ROLLBACK_BUNDLE_OK` (empty diff).

> Note: `git bundle --all` captures `refs/heads/*` and `refs/tags/*`, not `refs/pull/*`. PR discussion threads are **permanently lost** on delete (intended per spec §7); the bundle restores code history only.

---

### Task 2: Settings dump → manifest

**Files:**
- Create: `/home/mdorman/jaunder-rebuild/cutover/manifest/core.json`
- Create: `/home/mdorman/jaunder-rebuild/cutover/manifest/labels.json`
- Create: `/home/mdorman/jaunder-rebuild/cutover/manifest/ruleset.json`
- Create: `/home/mdorman/jaunder-rebuild/cutover/manifest/full-repo.json` (whole repo object, audit reference)
- Create: `/home/mdorman/jaunder-rebuild/cutover/manifest/github-issues.json` (open GitHub issues to recreate)

**Interfaces:**
- Produces: the manifest consumed by Task 3 (rehearsal restore), Task 6 (live restore), and Task 8 (issue recreation).

- [ ] **Step 1: Create the manifest dir**

```bash
mkdir -p /home/mdorman/jaunder-rebuild/cutover/manifest
```

- [ ] **Step 2: Dump the full repo object (audit reference) and core settings subset**

```bash
M=/home/mdorman/jaunder-rebuild/cutover/manifest
gh api repos/jaunder-org/jaunder > "$M/full-repo.json"
jq '{description, homepage, has_issues, has_projects, has_wiki,
     allow_squash_merge, allow_merge_commit, allow_rebase_merge,
     allow_auto_merge, delete_branch_on_merge, default_branch, visibility}' \
  "$M/full-repo.json" > "$M/core.json"
cat "$M/core.json"
```

Expected: `core.json` shows `description` set, `visibility: "public"`, `default_branch: "main"`, the three `has_*` true.

- [ ] **Step 3: Dump labels**

```bash
M=/home/mdorman/jaunder-rebuild/cutover/manifest
gh api --paginate repos/jaunder-org/jaunder/labels \
  --jq '[.[] | {name, color, description}]' > "$M/labels.json"
jq 'length' "$M/labels.json"
```

Expected: `10`.

- [ ] **Step 4: Dump the ruleset (full body, stripped of server-assigned fields)**

```bash
M=/home/mdorman/jaunder-rebuild/cutover/manifest
RID=$(gh api repos/jaunder-org/jaunder/rulesets --jq '.[0].id')
gh api "repos/jaunder-org/jaunder/rulesets/$RID" \
  | jq 'del(.id, .created_at, .updated_at, .source, .source_type, .node_id, ._links, .current_user_can_bypass)' \
  > "$M/ruleset.json"
jq '{name, target, enforcement, rules: (.rules | length)}' "$M/ruleset.json"
```

Expected: a named ruleset prints with its `enforcement` and rule count; file written.

- [ ] **Step 5: Confirm the empty surfaces (so restore can legitimately skip them)**

```bash
for ep in hooks environments autolinks keys actions/secrets dependabot/secrets actions/variables; do
  printf '%s=' "$ep"
  gh api "repos/jaunder-org/jaunder/$ep" --jq 'if type=="array" then length else (.total_count // (.secrets|length) // (.variables|length)) end' 2>/dev/null || echo "n/a"
done
```

Expected: every line `=0` (or `n/a` for a 404). Any non-zero means a new surface appeared since planning — STOP and extend the manifest before proceeding.

- [ ] **Step 6: Dump open GitHub issues (excluding PRs)**

```bash
M=/home/mdorman/jaunder-rebuild/cutover/manifest
gh api --paginate 'repos/jaunder-org/jaunder/issues?state=open' \
  --jq '[.[] | select(.pull_request==null)
         | {number, title, body, labels: [.labels[].name], milestone: .milestone.title}]' \
  > "$M/github-issues.json"
jq 'length' "$M/github-issues.json"
jq -r '.[] | "#\(.number) \(.title)"' "$M/github-issues.json"
```

Expected: `1` — `#39 Coverage gate: surface test failures distinctly from coverage regressions`. If the count changed, capture the new set; bodies are preserved verbatim for recreation.

---

### Task 3: Settings round-trip on a throwaway repo (§8 gate C)

**Files:**
- Uses: `manifest/` from Task 2.

**Interfaces:**
- Consumes: `core.json`, `labels.json`, `ruleset.json`.
- Produces: proof that restore reproduces the dump (gate C green). No artifact survives — the throwaway repo is deleted at the end.

- [ ] **Step 1: Create the throwaway repo (in-org, for settings fidelity)**

```bash
gh repo create jaunder-org/jaunder-rehearsal --public \
  --description "THROWAWAY — settings round-trip rehearsal, delete me"
```

- [ ] **Step 2: Seed a `main` so a branch ruleset has a target**

```bash
D=/tmp/rehearsal-seed
rm -rf "$D"; mkdir -p "$D"; git -C "$D" init -q -b main
git -C "$D" commit -q --allow-empty -m "seed"
git -C "$D" remote add origin git@github.com:jaunder-org/jaunder-rehearsal.git
git -C "$D" push -q origin main
```

- [ ] **Step 3: Restore core settings from the manifest**

```bash
M=/home/mdorman/jaunder-rebuild/cutover/manifest
gh api -X PATCH repos/jaunder-org/jaunder-rehearsal \
  --input <(jq '{description, homepage, has_issues, has_projects, has_wiki, allow_squash_merge, allow_merge_commit, allow_rebase_merge, allow_auto_merge, delete_branch_on_merge}' "$M/core.json") \
  --jq '.description'
```

Expected: prints the restored description.

- [ ] **Step 4: Restore labels (delete the auto-created defaults, then recreate from manifest)**

```bash
M=/home/mdorman/jaunder-rebuild/cutover/manifest
gh api --paginate repos/jaunder-org/jaunder-rehearsal/labels --jq '.[].name' \
  | while IFS= read -r n; do
      gh api -X DELETE "repos/jaunder-org/jaunder-rehearsal/labels/$(jq -rn --arg x "$n" '$x|@uri')" >/dev/null
    done
jq -c '.[]' "$M/labels.json" | while IFS= read -r lbl; do
  gh api -X POST repos/jaunder-org/jaunder-rehearsal/labels --input - <<<"$lbl" >/dev/null
done
gh api --paginate repos/jaunder-org/jaunder-rehearsal/labels --jq 'length'
```

Expected: `10`.

- [ ] **Step 5: Restore the ruleset**

```bash
M=/home/mdorman/jaunder-rebuild/cutover/manifest
gh api -X POST repos/jaunder-org/jaunder-rehearsal/rulesets \
  --input "$M/ruleset.json" --jq '.name + " / " + .enforcement'
```

Expected: ruleset created (name + enforcement printed).

- [ ] **Step 6: Round-trip diff — re-dump the rehearsal repo and compare to the manifest**

```bash
M=/home/mdorman/jaunder-rebuild/cutover/manifest
# core (incl. customized merge-button flags):
CORE_FIELDS='{description, homepage, has_issues, has_projects, has_wiki, allow_squash_merge, allow_merge_commit, allow_rebase_merge, allow_auto_merge, delete_branch_on_merge}'
diff <(jq -S "$CORE_FIELDS" "$M/core.json") \
     <(gh api repos/jaunder-org/jaunder-rehearsal | jq -S "$CORE_FIELDS") \
  && echo CORE_OK
# labels (name+color+description, order-independent):
diff <(jq -S 'sort_by(.name)' "$M/labels.json") \
     <(gh api --paginate repos/jaunder-org/jaunder-rehearsal/labels --jq '[.[]|{name,color,description}]' | jq -S 'sort_by(.name)') \
  && echo LABELS_OK
# ruleset (name + enforcement + rule set):
diff <(jq -S '{name,target,enforcement,rules:(.rules|sort_by(.type))}' "$M/ruleset.json") \
     <(gh api "repos/jaunder-org/jaunder-rehearsal/rulesets/$(gh api repos/jaunder-org/jaunder-rehearsal/rulesets --jq '.[0].id')" | jq -S '{name,target,enforcement,rules:(.rules|sort_by(.type))}') \
  && echo RULESET_OK
```

Expected: `CORE_OK`, `LABELS_OK`, `RULESET_OK` (all diffs empty). Any mismatch → fix the restore command in Task 6 to match before going live.

- [ ] **Step 7: Tear down the throwaway repo**

```bash
gh repo delete jaunder-org/jaunder-rehearsal --yes
rm -rf /tmp/rehearsal-seed
```

Expected: rehearsal repo gone.

---

### Task 4: GitHub-side eyeball of the spine (§8 gate B-on-GitHub) + GO/NO-GO

**Files:**
- Uses: `jaunder-new/` (spine) from Task 0.

**Interfaces:**
- Produces: visual confirmation the rebuilt history renders correctly on GitHub, and the consolidated GO/NO-GO decision.

- [ ] **Step 1: Create a second throwaway repo and push the spine as `main`**

```bash
gh repo create jaunder-org/jaunder-spine-preview --public \
  --description "THROWAWAY — spine render preview, delete me"
git -C /home/mdorman/jaunder-rebuild/cutover/jaunder-new \
  push git@github.com:jaunder-org/jaunder-spine-preview.git spine:main
```

- [ ] **Step 2: Confirm GitHub reads the intended story**

```bash
# First-parent changelog as GitHub will show it on the default branch:
gh api 'repos/jaunder-org/jaunder-spine-preview/commits?sha=main&per_page=10' \
  --jq '.[].commit.message | split("\n")[0]'
# PR tab MUST be empty (local --no-ff merges, no PRs). REST, not `gh pr list`:
# this token's GraphQL path 401s, so use the REST pulls endpoint.
echo -n 'pr_count='; gh api 'repos/jaunder-org/jaunder-spine-preview/pulls?state=all' --jq 'length'
```

Expected: the first line is the emacs-plan subject, followed by clean Conventional-Commits merge subjects; PR count `0`. Also open the repo in a browser to eyeball README + `docs/` rendering and the commit graph.

- [ ] **Step 3: Tear down the preview repo**

```bash
gh repo delete jaunder-org/jaunder-spine-preview --yes
```

- [ ] **Step 4: GO/NO-GO checkpoint**

Confirm all green before any destructive step:
- §8 A (machinery) — done in Phase B.
- §8 B (full local dress + greenness at §6 tiers) — done in Phase B (`final-validate-sidecar.json`).
- §8 C (settings round-trip) — Task 3 (`CORE_OK`/`LABELS_OK`/`RULESET_OK`).
- GitHub eyeball — Task 4 (changelog + empty PR tab).
- Rollback bundle verified — Task 1 (`ROLLBACK_BUNDLE_OK`).

Expected: every item checked. **STOP HERE and obtain an explicit human "go" before Task 5.**

---

### Task 5: [DESTRUCTIVE — explicit "go" required] Delete, recreate, push

**Files:**
- Uses: `jaunder-new/` (spine), `manifest/`, `jaunder-old-rollback.bundle`.

**Interfaces:**
- Produces: the new `jaunder-org/jaunder` with `main` = rebuilt history.

- [ ] **Step 1: Final pre-flight re-verification (abort on any surprise)**

```bash
# (a) No drift on the live repo since planning — tip must still be the retired pos-648 merge:
gh api 'repos/jaunder-org/jaunder/commits?sha=main&per_page=1' \
  --jq '.[0].commit.message | split("\n")[0]'
# (b) Freeze holding — confirm with the user that no merges have landed and none will during the window.
# (c) Rollback bundle present and valid:
git bundle verify /home/mdorman/jaunder-rebuild/cutover/jaunder-old-rollback.bundle
# (d) Spine source intact:
git -C /home/mdorman/jaunder-rebuild/cutover/jaunder-new \
  log --first-parent --merges --oneline spine | wc -l
```

Expected: (a) `Merge pull request #42 from jaunder-org/history-rebuild-docs`; if anything else, STOP — new work landed, replay it onto `spine` first. (c) bundle OK. (d) `49`.

- [ ] **Step 2: Delete the repo**

```bash
gh repo delete jaunder-org/jaunder --yes
```

Expected: deletion confirmed.

- [ ] **Step 3: Recreate the repo (empty, same identity)**

```bash
gh repo create jaunder-org/jaunder --public \
  --description '`Jaunder` is an easily-hosted, multi-protocol social media application written in Rust.'
```

Expected: repo created, empty.

- [ ] **Step 4: Push the rebuilt history as `main`**

```bash
git -C /home/mdorman/jaunder-rebuild/cutover/jaunder-new \
  push git@github.com:jaunder-org/jaunder.git spine:main
```

Expected: push succeeds; `main` created.

- [ ] **Step 5: Set the default branch to `main`**

```bash
gh api -X PATCH repos/jaunder-org/jaunder -f default_branch=main --jq '.default_branch'
```

Expected: `main` (usually already the default after first push; this makes it explicit).

---

### Task 6: Restore settings to the new repo

**Files:**
- Uses: `manifest/` from Task 2 (with any fixes proven in Task 3).

**Interfaces:**
- Consumes: `core.json`, `labels.json`, `ruleset.json`.
- Produces: the new repo's settings matching the manifest.

- [ ] **Step 1: Restore core settings**

```bash
M=/home/mdorman/jaunder-rebuild/cutover/manifest
gh api -X PATCH repos/jaunder-org/jaunder \
  --input <(jq '{description, homepage, has_issues, has_projects, has_wiki, allow_squash_merge, allow_merge_commit, allow_rebase_merge, allow_auto_merge, delete_branch_on_merge}' "$M/core.json") \
  --jq '.description'
```

Expected: restored description printed.

- [ ] **Step 2: Restore labels (delete defaults, recreate from manifest)**

```bash
M=/home/mdorman/jaunder-rebuild/cutover/manifest
gh api --paginate repos/jaunder-org/jaunder/labels --jq '.[].name' \
  | while IFS= read -r n; do
      gh api -X DELETE "repos/jaunder-org/jaunder/labels/$(jq -rn --arg x "$n" '$x|@uri')" >/dev/null
    done
jq -c '.[]' "$M/labels.json" | while IFS= read -r lbl; do
  gh api -X POST repos/jaunder-org/jaunder/labels --input - <<<"$lbl" >/dev/null
done
gh api --paginate repos/jaunder-org/jaunder/labels --jq 'length'
```

Expected: `10`.

- [ ] **Step 3: Restore the ruleset**

```bash
M=/home/mdorman/jaunder-rebuild/cutover/manifest
gh api -X POST repos/jaunder-org/jaunder/rulesets \
  --input "$M/ruleset.json" --jq '.name + " / " + .enforcement'
```

Expected: ruleset created.

---

### Task 7: Post-cutover verification

**Files:** none (read-only checks).

- [ ] **Step 1: Fresh clone reads the intended history**

```bash
rm -rf /tmp/jaunder-verify
git clone git@github.com:jaunder-org/jaunder.git /tmp/jaunder-verify
git -C /tmp/jaunder-verify log --first-parent --merges --oneline | wc -l
git -C /tmp/jaunder-verify log --first-parent -1 --format='%s'
```

Expected: `49` merges; tip subject is the emacs-plan commit.

- [ ] **Step 2: Tree integrity — new `main` tip tree == spine tip tree**

```bash
A=$(git -C /tmp/jaunder-verify rev-parse main^{tree})
B=$(git -C /home/mdorman/jaunder-rebuild/cutover/jaunder-new rev-parse spine^{tree})
[ "$A" = "$B" ] && echo TREE_OK
```

Expected: `TREE_OK`.

- [ ] **Step 3: Settings match manifest, PR tab empty, CI triggered**

```bash
M=/home/mdorman/jaunder-rebuild/cutover/manifest
diff <(jq -S '{description,homepage,has_issues,has_projects,has_wiki,allow_squash_merge,allow_merge_commit,allow_rebase_merge,allow_auto_merge,delete_branch_on_merge}' "$M/core.json") \
     <(gh api repos/jaunder-org/jaunder | jq -S '{description,homepage,has_issues,has_projects,has_wiki,allow_squash_merge,allow_merge_commit,allow_rebase_merge,allow_auto_merge,delete_branch_on_merge}') && echo CORE_OK
gh api --paginate repos/jaunder-org/jaunder/labels --jq 'length'
gh api repos/jaunder-org/jaunder/rulesets --jq 'length'
echo -n 'pr_count='; gh api 'repos/jaunder-org/jaunder/pulls?state=all' --jq 'length'   # REST (GraphQL 401s)
gh run list --repo jaunder-org/jaunder --limit 5 --json status,name,conclusion
```

Expected: `CORE_OK`; labels `10`; rulesets `1`; PR count `0`; CI runs listed (workflows triggered by the push). Investigate any CI failure as ordinary repo work — not a rebuild defect (the spine tip passed `cargo xtask validate` in Phase B).

- [ ] **Step 4: Re-point the local working repo at the new history (optional, recommended)**

```bash
git -C /home/mdorman/src/jaunder fetch origin
git -C /home/mdorman/src/jaunder log --oneline -1 origin/main
```

Expected: `origin/main` now shows the rebuilt tip. (A local reset/re-clone of `/home/mdorman/src/jaunder` onto the new `main` is a follow-up; the old local `main` and the rollback bundle remain the recovery path until teardown.)

> **Rollback (if any verification fails and you choose to abort):** `gh repo delete jaunder-org/jaunder --yes`, recreate, then `git -C jaunder-old-mirror.git push --mirror git@github.com:jaunder-org/jaunder.git` (or push from the rollback bundle), and re-run Task 6 against the old manifest. The old world is recoverable until Task 9.

---

### Task 8: Content migration — GitHub issues (§7) + open backlog (§9) — after cutover

**Files:**
- Uses: `manifest/github-issues.json` from Task 2.
- Create: `/home/mdorman/jaunder-rebuild/cutover/open-beads.json`
- Create: `/home/mdorman/jaunder-rebuild/cutover/backlog-triage.md`

**Interfaces:**
- Consumes: the dumped GitHub issues, and the live `bd` database in `/home/mdorman/src/jaunder`.
- Produces: GitHub issues in the new repo for the preserved issue(s) plus the surviving beads backlog.

- [ ] **Step 0: Recreate the preserved GitHub issue(s) verbatim**

REST throughout (`gh issue create`/`list` use the GraphQL path, which 401s on this token).

```bash
M=/home/mdorman/jaunder-rebuild/cutover/manifest
jq -c '.[]' "$M/github-issues.json" | while IFS= read -r iss; do
  gh api repos/jaunder-org/jaunder/issues -X POST --input <(
    jq '{title,
         body: (.body + "\n\n_Migrated from old #\(.number) during the 2026-06-24 history rebuild._"),
         labels}' <<<"$iss"
  ) --jq '"created #\(.number): \(.title)"'
done
# Verify (REST — excludes PRs):
gh api 'repos/jaunder-org/jaunder/issues?state=open' --jq '[.[]|select(.pull_request==null)]|length'
```

Expected: `#39`'s content recreated as a new issue (label `tooling`); open-issue count `1`. Note its new number — it feeds Step 2 dedup. (Recreate runs before the beads pass so the beads triage can skip any duplicate.)

- [ ] **Step 1: Export the open backlog**

```bash
( cd /home/mdorman/src/jaunder && bd list --status open --json ) \
  > /home/mdorman/jaunder-rebuild/cutover/open-beads.json
jq 'length' /home/mdorman/jaunder-rebuild/cutover/open-beads.json
```

Expected: ~31 (the live count at execution time).

- [ ] **Step 2: Triage (human pass) into keep / resolve / stale**

Write `/home/mdorman/jaunder-rebuild/cutover/backlog-triage.md` with one line per bead: `id — title — KEEP|RESOLVE|STALE — note`. Per §9: visibility-related items are already resolved (visibility merged); revisit stale items; only genuine backlog becomes issues. **Mark any bead that duplicates the already-recreated GitHub issue from Step 0 (e.g. a coverage-gate / test-failure-vs-coverage-regression item ↔ old #39) as `RESOLVE — dup of recreated issue` so it isn't created twice.** This step gates Step 3 — do not auto-create issues for every open bead.

- [ ] **Step 3: Create GitHub issues for the KEEP set**

For each KEEP bead, map per §9: `issue_type`→label, `priority`→one of `P1`–`P4` label, epics→milestone, deps→a `Blocked by #N` line in the body; link design context to `docs/`, don't duplicate it.

```bash
# Example for one bead (repeat per KEEP item, substituting fields from open-beads.json).
# REST (GraphQL 401s on this token):
gh api repos/jaunder-org/jaunder/issues -X POST \
  -f title="<bead title>" \
  -f body=$'<bead description>\n\nBlocked by: <#N or none>\nContext: docs/<path>' \
  -f 'labels[]=<issue_type>' -f 'labels[]=P2' \
  --jq '"created #\(.number)"'
```

Expected: one issue per KEEP bead; verify with `gh api 'repos/jaunder-org/jaunder/issues?state=open' --jq '[.[]|select(.pull_request==null)]|length'`.

---

### Task 9: Teardown (DEFERRED — only after the new repo has been healthy for a grace period)

**Files:** removes the rebuild scaffolding.

**Interfaces:** terminal cleanup; do not run until the new repo is proven healthy and you accept losing the rollback net.

- [ ] **Step 1: Remove volatile + build-cache scaffolding**

```bash
rm -rf /tmp/jaunder-rebuild /tmp/jaunder-rebuild-driver /tmp/jaunder-*.jsonl
rm -rf /home/mdorman/jb-target
nix-collect-garbage -d
```

- [ ] **Step 2: Remove the working-artifact dir, bundles LAST**

Per PHASE-B-DONE order: remove the Phase-A files, handoffs, `phaseB-driver-artifacts.tar.gz`, the `cutover/` working dir — and **last of all** `jaunder-spine.bundle` and `jaunder-old-rollback.bundle` (the only durable copies of, respectively, the new and old history). Keep both bundles until you are certain you will never need to re-push or audit.

- [ ] **Step 3: Drop stale agent memory**

Delete the file-memory `project_history_rebuild_phaseB_done.md` and its `MEMORY.md` pointer so future sessions don't chase the old repo or pre-cutover state. Add a short closing memory noting the cutover completed (date + new-repo tip).

---

## Self-Review

- **Spec coverage (§7 sequence):** dump settings → Task 2; bundle + verify → Task 1; push rehearsal-verified history → Tasks 3+4; delete + recreate + push → Task 5; restore settings → Task 6; re-enter write-only secrets → **N/A, surface is empty** (verified Task 2 Step 5); preserve the 1 open GitHub issue → dump Task 2 Step 6, recreate Task 8 Step 0; migrate backlog → Task 8. §8 gates A/B (Phase B) + C (Task 3) + eyeball (Task 4) → Task 4 GO/NO-GO. §9 → Task 8. Teardown → Task 9. All covered.
- **Decisions encoded:** full §8 gate (Tasks 3+4), `gh auth refresh -s delete_repo` (Task 0 Step 5), backlog after cutover (Task 8 follows Task 7).
- **Placeholders:** the only intentionally human-driven steps are the backlog triage (Task 8 Step 2) and the per-item issue creation (Task 8 Step 3) — these require judgment per §9 and are marked as such, not left as vague code.
- **Type/identity consistency:** repo identity, manifest paths, and the `jaunder-new/spine` source are referenced identically across Tasks 0–7.
- **Destructive isolation:** all irreversible actions are in Task 5 onward, behind the Task 4 GO/NO-GO and an explicit human "go"; the rollback path is documented in Task 7 and preserved until Task 9.
