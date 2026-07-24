# Plan — #627: adopt GitHub merge queue

**Spec:** `docs/superpowers/specs/2026-07-24-issue-627-merge-queue.md` (the
"what/why"; this plan is the "how"). **For agentic workers:** drive with
`jaunder-iterate`; delegate a task via `jaunder-dispatch` if useful.

## Review header

**Goal.** End the strict up-to-date-before-merge treadmill on `main` by adopting
a GitHub merge queue, keeping the combined-state (semantic-conflict) guarantee.
The in-PR deliverable is CI-config + docs; the live ruleset flip is applied
post-merge, maintainer-gated (spec Decision 2).

**Scope — in:**

- A `merge_group:` trigger in `.github/workflows/ci.yml` so
  `Validate (no e2e)` + `e2e gate` run on queue branches.
- A committed **runbook** (`docs/ci-merge-queue.md`) with the exact GitHub-API
  payloads to **enable** the queue and to **roll back**, derived from the live
  ruleset JSON (id `18086446`).
- An **ADR draft** recording the decision (supersedes the strict-ruleset
  choice), the #629 monitored-risk + rollback trigger, and the `--auto` enqueue
  behavior.

**Scope — out:** fixing #629 (tracked separately); applying the live ruleset
flip inside this cycle; agent-skill/memory updates (`.claude/` untracked). No
new separable concerns surfaced during the interview, so there is **no
issue-filing first task**.

**Tasks (one line each):**

1. Draft the ADR (numberless, `docs/adr/drafts/`).
2. Add the `merge_group:` trigger to `ci.yml`.
3. Write the enable+rollback runbook `docs/ci-merge-queue.md`.
4. Verify in-PR: gate green, `ci.yml` triggers correct, runbook payloads match
   the live ruleset schema (dry read).

**Key risks / decisions:**

- The ruleset flip is **not** in this PR — it's a post-merge maintainer-gated
  action. The plan's job is to make that action exact and reversible (Task 3),
  not to perform it.
- Acceptance #4 (queue actually merges a PR) is validated **after** the flip, on
  the next PR — outside this branch. The plan cannot tick it; ship/ post-merge
  does.
- No Rust changes → no TDD red/green. "Tests" here are: `ci.yml` parses and
  wires the two required contexts onto `merge_group`; runbook payloads are
  schema-valid against the live ruleset.

## Global constraints

- **No source code changes.** This is `.github/workflows/ci.yml` + `docs/`. Do
  not touch crates.
- **The gate is still `cargo xtask check`** (fmt + clippy + Nix coverage). With
  no Rust touched it should pass via cache; run it before committing
  (`jaunder-commit`) because the pre-commit hook runs it, and because a Markdown
  prettier hook may restage prose (`docs/` edits).
- **No `Co-Authored-By` trailer.**
- **The ADR draft is gitignored** (`docs/adr/drafts/` except its README) — it is
  gate-invisible until `cargo xtask adr promote` at ship. Get the draft format
  right now (line 1 `# ADR-DRAFT: …`; a single-token `- Status:` line).
- **Do not mutate the live ruleset** anywhere in this cycle's tasks. Task 3 only
  _documents_ the payloads and may _read_ the ruleset to confirm schema.

---

## Task 1 — Draft the ADR

**Files:**

- Create `docs/adr/0077-adopt-github-merge-queue.md` from
  `docs/adr/template.md`.

**Content (fill the template):**

- **Line 1 exactly:** `# ADR-DRAFT: Adopt a GitHub merge queue for main`
- `- Status: accepted`
- `- Date: 2026-07-24`
- `- Issue: [#627](https://github.com/jaunder-org/jaunder/issues/627)`
- **Context:** the strict `up-to-date-before-merge` ruleset on `main` and the
  rebase/retest treadmill (cite #624 as evidence, ~13-min cold e2e); the real
  reason strict exists (a past textually-clean/logically-incompatible semantic
  conflict broke `HEAD`); the goal to keep that guarantee without the manual
  re-sync. Reference the spec.
- **Decision:** adopt a GitHub merge queue on `main`:
  - `merge_group:` trigger added to `ci.yml`; the required contexts
    `Validate (no e2e)` + `e2e gate` run on `gh-readonly-queue/…` branches.
  - Ruleset change (applied post-merge, maintainer-gated): add a `merge_queue`
    rule (grouping `ALLGREEN`, merge method `merge`, small starting batch) and
    set `strict_required_status_checks_policy: false`. State that this
    **supersedes** the strict-ruleset decision.
  - `gh pr merge --auto` **enqueues** under a queue (does not direct-merge) —
    the ship flow is unaffected.
- **Consequences:** treadmill removed; combined-state guarantee preserved via
  `ALLGREEN` + GitHub's stacked-build/bisect (design rationale — the two-PR
  catch is argued, not observed in a serial-dev repo; spec Decision 7). **#629
  (Validate OOM) is a monitored risk, not a hard gate** (maintainer-approved
  reframe): an OOM-ejected PR is auto-requeued; **rollback trigger** — if OOM
  ejections thrash the queue, restore strict via the runbook. Points to
  `docs/ci-merge-queue.md` for the exact enable/rollback payloads.

**Verify:** file exists; line 1 is `# ADR-DRAFT:`; `- Status:` is a single token
(`accepted`). (Gate-invisible until promote — no gate run proves it.)

**Commit:** `docs(adr): draft — adopt a GitHub merge queue for main (#627)` —
note the draft is gitignored, so this commit carries only the ci.yml/runbook if
staged together; keep the ADR draft as its own logical step even though git
won't track it until promote. (If nothing stageable, fold the "authoring" into
Task 3's commit.)

## Task 2 — Add the `merge_group:` trigger to `ci.yml`

**Files:**

- Edit `.github/workflows/ci.yml` `on:` block.

**Change:** add a `merge_group:` event alongside `pull_request` and `push`:

```yaml
on:
  pull_request:
  push:
    branches: [main]
  merge_group:
```

No per-job `if:` filtering: all jobs (`validate-no-e2e`, `e2e` matrix,
`elisp-integration`, `e2e-gate`) must run on `merge_group` because both required
contexts (`Validate (no e2e)` and `e2e gate`, the latter via its `needs`) have
to report on the queue branch or the queue can't evaluate them.

**Verify:**

- `ci.yml` is valid YAML and the workflow still parses — read it back; if
  `actionlint` is available in the devShell, run
  `devtool run -- actionlint .github/workflows/ci.yml` (else a structural read
  suffices).
- Confirm the three triggers are present and the job set is unchanged (no job
  gained an `if:` that would skip it on `merge_group`).

**Commit:** `ci: run required checks on merge_group (queue branches) (#627)`

## Task 3 — Write the enable+rollback runbook

**Files:**

- Create `docs/ci-merge-queue.md`.

**Content — derive payloads from the captured live ruleset** (id `18086446`; the
current rules array is: `deletion`, `non_fast_forward`, `pull_request`
[`allowed_merge_methods:["merge"]`], `required_status_checks`
[`strict_required_status_checks_policy:true`, contexts `Validate (no e2e)` +
`e2e gate`]). The runbook must give:

1. **Enable** — the exact
   `gh api --method PUT /repos/jaunder-org/jaunder/rulesets/18086446` (or PATCH)
   call with the full `rules` array that (a) **adds** a `merge_queue` rule and
   (b) **flips** `strict_required_status_checks_policy` to `false`. The
   `merge_queue` parameters to document (start conservative):

   ```json
   {
     "type": "merge_queue",
     "parameters": {
       "merge_method": "MERGE",
       "grouping_strategy": "ALLGREEN",
       "max_entries_to_build": 5,
       "min_entries_to_merge": 1,
       "max_entries_to_merge": 5,
       "min_entries_to_merge_wait_minutes": 5,
       "check_response_timeout_minutes": 60
     }
   }
   ```

   Explain each: `ALLGREEN` = every stacked prefix (`main+A`, `main+A+B`, …)
   must pass (keeps `main` green at every intermediate landing, spec Decision
   4); `MERGE` matches the ruleset's only allowed method; small batch because
   the repo is effectively serial. Note that `grouping_strategy` / parameter
   names must be **re-confirmed against the live ruleset schema** at apply time
   (GitHub may rename fields) — the task's verify step does a dry read to check.

2. **Rollback** — the exact reverse call: remove the `merge_queue` rule and
   restore `strict_required_status_checks_policy: true`, returning the ruleset
   to the captured baseline. Include the baseline `rules` array verbatim as the
   known-good target.

3. **Post-flip validation checklist** (spec Acceptance #4): open the next PR;
   observe check runs on a `gh-readonly-queue/…` ref in Actions; confirm the PR
   merges without manual re-sync; confirm
   `gh api /repos/jaunder-org/jaunder/rulesets/18086446` shows
   `strict_required_status_checks_policy:false` + a `merge_queue` rule. Confirm
   `gh pr merge --auto` enqueues.

4. **Rollback trigger** — restate the #629 OOM condition: if OOM ejections
   thrash the queue, run the rollback call.

**Precision on Acceptance #2 ("exact, not guessed").** The runbook has two kinds
of payload, and only one is derivable read-only:

- **Exactly derived (read-only, verifiable now):** the **baseline** `rules`
  array, the **rollback** call, and the **strict-flip**
  (`strict_required_status_checks_policy` true→false) — all come field-for-field
  from the captured live ruleset.
- **Schema-sourced (confirmed at apply time):** the `merge_queue` rule's
  parameter names/enums — the baseline ruleset has **no** `merge_queue` rule, so
  these come from GitHub's published rulesets schema, not from our ruleset. The
  runbook labels this block "confirm against GitHub's rulesets schema at apply
  time," and the maintainer/agent re-reads the ruleset **immediately after**
  applying to confirm the queue rule was accepted with the intended params.

**Verify:**

- **Dry read** the live ruleset
  (`gh api /repos/jaunder-org/jaunder/rulesets/18086446`) and confirm the
  runbook's **baseline `rules` array** matches it field-for-field (no mutation)
  — this satisfies Acceptance #2 for the exactly-derived half.
- Confirm the `merge_queue` parameter block is sourced from and cited against
  GitHub's REST rulesets schema documentation (fetch the "Update a repository
  ruleset" schema at apply time), **not** from the `rule-suites`
  evaluation-history endpoint. If the exact enum/param names can't be pinned
  read-only, the block stays flagged "confirm at apply time."

**Commit:** `docs(ci): merge-queue enable/rollback runbook (#627)`

## Task 4 — Verify in-PR & wrap

**Steps:**

- Run the gate: `devtool run -- cargo xtask check` (or `validate --no-e2e`).
  Expect green — no Rust touched; confirm the Markdown prettier hook didn't
  leave the tree dirty (`git status --porcelain` after).
- Re-read `ci.yml` to confirm the `merge_group:` trigger survived formatting.
- Confirm the ADR draft is present and correctly formatted (Task 1 verify).
- Confirm the runbook's baseline matches the live ruleset (Task 3 verify).

**No commit** unless the gate auto-fixed formatting (then commit the fixup into
the owning task's commit, not a churn commit).

## Post-merge (ship + maintainer gate — not a plan task, recorded for the driver)

After this PR merges under the current strict rule, `jaunder-ship` promotes the
ADR (`cargo xtask adr promote`). Then, **on a fresh explicit "go" from the
maintainer**, apply the runbook's Enable call, and run the post-flip validation
checklist. Roll back if the OOM trigger fires. Release #627 to Done only after
the queue is confirmed working (spec Acceptance #4).
