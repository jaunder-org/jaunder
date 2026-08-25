# CI: GitHub merge queue — enable / rollback runbook

This is the operational runbook for the merge queue adopted in
`docs/adr/0077-adopt-github-merge-queue.md` for issue
[#627](https://github.com/jaunder-org/jaunder/issues/627). It gives the
**exact** GitHub-API calls to enable the queue on `main` and to roll it back,
plus the post-flip validation checklist.

> **The enable step is a repo-admin action and the point of no return.** It is
> applied **after** the #627 PR merges (under the current strict rule), and
> **only on a fresh, explicit maintainer "go"** — prior approval of the PR does
> not authorize the flip.

The target is the ruleset **"Main branch protection"**, id **`18086446`**
(`gh api /repos/jaunder-org/jaunder/rulesets/18086446`).

## Baseline (current, known-good) `rules` array

This is the live ruleset's `rules` array today — captured verbatim; it is the
rollback target. `strict_required_status_checks_policy` is `true` and there is
**no** `merge_queue` rule:

```json
[
  { "type": "deletion" },
  { "type": "non_fast_forward" },
  {
    "type": "pull_request",
    "parameters": {
      "required_approving_review_count": 0,
      "dismiss_stale_reviews_on_push": false,
      "required_reviewers": [],
      "require_code_owner_review": false,
      "dismissal_restriction": { "enabled": false, "allowed_actors": [] },
      "require_last_push_approval": false,
      "required_review_thread_resolution": false,
      "allowed_merge_methods": ["merge"]
    }
  },
  {
    "type": "required_status_checks",
    "parameters": {
      "strict_required_status_checks_policy": true,
      "do_not_enforce_on_create": false,
      "required_status_checks": [
        { "context": "Validate (no e2e)" },
        { "context": "e2e gate" }
      ]
    }
  }
]
```

Before doing anything, **re-capture the live baseline** so rollback restores the
exact current state (it may have drifted since this doc was written):

```bash
gh api /repos/jaunder-org/jaunder/rulesets/18086446 > /tmp/ruleset-baseline.json
```

## Enable the merge queue

Two changes to the `rules` array, applied together:

1. In the `required_status_checks` rule, set
   `strict_required_status_checks_policy` **`false`**. This is **mandatory**:
   GitHub does not allow the strict policy and a merge queue simultaneously —
   the queue supersedes up-to-date-before-merge.
2. **Add** a `merge_queue` rule.

### The `merge_queue` rule (parameters)

> **Confirm against GitHub's rulesets schema at apply time.** The baseline
> ruleset has no `merge_queue` rule, so these parameter names/enums come from
> GitHub's published "Update a repository ruleset" REST schema, **not** from our
> ruleset. Fetch that schema (or the merge-queue rule docs) when applying, and
> re-read the ruleset immediately after (see validation) to confirm the params
> were accepted as intended.

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

Rationale (per ADR / spec Decision 4):

- **`grouping_strategy: ALLGREEN`** — the required checks must pass for the
  queued PR **and every PR ahead of it** (GitHub stacks `main+A`, `main+A+B`, …
  and requires each green), so `main` stays green at every intermediate landing,
  not only at the combined tip. This is the stronger option and preserves the
  semantic-conflict guarantee.
- **`merge_method: MERGE`** — matches the ruleset's only allowed method
  (`allowed_merge_methods: ["merge"]`); no change to how commits land.
- **Small batch** (`max_entries_to_build: 5`) — the repo is effectively serial,
  so batching rarely engages; the win is removing manual re-sync, not
  throughput. Tune later without a code change.
- **`check_response_timeout_minutes: 60`** — comfortably above the ~13-min cold
  e2e.

### Apply

Write the full desired ruleset body (the update endpoint takes the whole object;
keep `name`, `target`, `enforcement`, `conditions`, `bypass_actors` as-is and
swap in the new `rules`). The enabled `rules` array is the baseline above with
the two changes:

```json
[
  { "type": "deletion" },
  { "type": "non_fast_forward" },
  {
    "type": "pull_request",
    "parameters": {
      "required_approving_review_count": 0,
      "dismiss_stale_reviews_on_push": false,
      "required_reviewers": [],
      "require_code_owner_review": false,
      "dismissal_restriction": { "enabled": false, "allowed_actors": [] },
      "require_last_push_approval": false,
      "required_review_thread_resolution": false,
      "allowed_merge_methods": ["merge"]
    }
  },
  {
    "type": "required_status_checks",
    "parameters": {
      "strict_required_status_checks_policy": false,
      "do_not_enforce_on_create": false,
      "required_status_checks": [
        { "context": "Validate (no e2e)" },
        { "context": "e2e gate" }
      ]
    }
  },
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
]
```

Build the request body from the live ruleset (preserves
`name`/`conditions`/etc.) and PUT it. For example, with the enabled `rules`
array saved to `/tmp/rules-enable.json`:

```bash
# Compose the full body from the freshly-captured baseline, swapping in the new rules:
jq --slurpfile rules /tmp/rules-enable.json \
  '{name, target, enforcement, conditions, bypass_actors, rules: $rules[0]}' \
  /tmp/ruleset-baseline.json > /tmp/ruleset-enable.json

gh api --method PUT /repos/jaunder-org/jaunder/rulesets/18086446 \
  --input /tmp/ruleset-enable.json
```

## Roll back

Restore the baseline: remove the `merge_queue` rule and set
`strict_required_status_checks_policy` back to `true`. If you captured
`/tmp/ruleset-baseline.json` before enabling, roll back to it directly:

```bash
gh api --method PUT /repos/jaunder-org/jaunder/rulesets/18086446 \
  --input /tmp/ruleset-baseline.json
```

If that capture is unavailable, reconstruct the body with the **Baseline `rules`
array** at the top of this doc (strict `true`, no `merge_queue`) and PUT it the
same way.

**Rollback trigger (#629).** `Validate (no e2e)` intermittently OOMs (~1-in-5 CI
failures, tracked in #629). **An ejected PR is _not_ requeued automatically** —
the live `merge_queue` rule has no requeue parameter, so a failed front-of-queue
`merge_group` drops the PR out of the queue and it stays `OPEN` until someone
re-enqueues it. So each OOM ejection costs a manual re-enqueue, and **if OOM
ejections thrash the queue** (PRs repeatedly ejected, batches failing to
converge), run the rollback above and revisit #629 before re-enabling.

**Observing queue state.** Ejection is silent — the queue entry vanishes while
the PR stays `OPEN`, which looks identical to "still queued". Don't eyeball it:
`cargo xtask pr watch <N>` reports `ejected` distinctly from `merged` and from
still-queued, and points at the `gh-readonly-queue/main/pr-<N>-…` run that
caused it. `cargo xtask pr watch <N> --once` answers the same question without
blocking.

## ADR promoter queue boundary

The tracked-draft lifecycle uses `.github/workflows/adr-promoter.yml` and
`cargo xtask adr promoter` after a feature merge; feature shipping never runs
the local promotion mutation. Pushes to `main` and manual dispatches share a
generation concurrency group that does not cancel an active run. Dequeue
recovery uses a separate per-PR operation group, so a generation event cannot
replace the event authorized to re-arm that head. The single job still derives
from fresh `main` and owns the stable `automation/adr-promoter` branch and at
most one open promoter PR. If that PR already exists or is queued, its head SHA
and generated diff remain immutable; drafts merged later wait for the next pass
after it lands.

The workflow mints an installation token with
`actions/create-github-app-token@v3` from repository variable
`ADR_PROMOTER_CLIENT_ID` and secret `ADR_PROMOTER_APP_PRIVATE_KEY`. The
dedicated GitHub App is limited to Actions read, Contents read/write, pull
requests read/write, checks read, commit statuses read, and GitHub's mandatory
Metadata read. Actions read is used only for historical `merge_group`
workflow-run metadata needed by dequeue correlation. It has no Administration,
Actions-write, ruleset or branch-protection bypass, or authority to write `main`
directly. The built-in `GITHUB_TOKEN` is not used for promoter GitHub
operations. Before committing, the controller runs the pinned Prettier over
every staged Markdown addition or modification and re-stages the result.
Promotion commits use the deterministic `jaunder-adr-promoter[bot]` author and
committer. Their `git commit --no-verify` is deliberate: the pre-commit hook's
auto-staging reconciliation rejects the generated tracked rename as unsafe,
while the promoter PR's required checks validate that exact formatted commit
before the queue can write it to `main`. The App arms auto-merge with the merge
method; the normal pull-request and `merge_group` checks run, and the queue
remains the only writer to `main`.

The workflow also receives `pull_request: dequeued` events. Recovery is limited
to the exact promoter head/base identity: the controller correlates the removed
entry with its ephemeral merge-group SHA and re-arms auto-merge only when the
required contexts on both that SHA and the unchanged PR head exist, are
complete, and are green. After arming, either an auto-merge request or live
queue membership on that exact head verifies success; GitHub commonly enqueues a
green PR immediately. Failed, missing, incomplete, ambiguous, or mismatched
evidence stops retry and leaves the immutable PR visible for diagnosis; it
cannot loop a deterministic failure through the queue.

A tracked draft remaining `proposed` for the promoter's ordinary PR checks,
merge-group checks, and queue interval is healthy acceptance lag. If no promoter
PR appears, its checks fail, dequeue recovery refuses to re-arm, or it otherwise
stops progressing, the longer proposed interval is failed-promoter lag. Diagnose
the visible workflow or PR; do not promote or renumber from the feature branch.

## Post-flip validation checklist (spec Acceptance #4)

After enabling, on the next real PR:

- [ ] Enqueue the PR (or let `gh pr merge --auto` enqueue it on green/approval —
      confirm auto-merge **enqueues** rather than direct-merging).
- [ ] In the Actions tab, confirm the required checks (`Validate (no e2e)`,
      `e2e gate`) **run on a `gh-readonly-queue/…` ref** — this is the
      mechanical proof that the combined-with-`main` state is re-tested, not the
      stale PR head.
- [ ] Confirm the PR reaches `main` through the queue **without any manual
      re-sync**.
- [ ] `gh api /repos/jaunder-org/jaunder/rulesets/18086446` shows
      `strict_required_status_checks_policy: false` and a `merge_queue` rule
      with the intended parameters (treadmill removed; params accepted as sent).

The two-PR semantic-conflict catch is **not** manufactured here (a serial
single-dev repo does not naturally produce two conflicting queued PRs); it rests
on `ALLGREEN` grouping + GitHub's documented stacked-build / bisect-on-failure
(ADR rationale).
