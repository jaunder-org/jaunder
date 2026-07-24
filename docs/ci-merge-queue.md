# CI: GitHub merge queue — enable / rollback runbook

This is the operational runbook for the merge queue adopted in
`docs/adr/drafts/adopt-github-merge-queue.md` (ADR, numbered at ship) for issue
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
failures, tracked in #629). Under the queue an OOM-ejected PR is auto-requeued,
so occasional ejection is tolerable — but **if OOM ejections thrash the queue**
(PRs repeatedly ejected, batches failing to converge), run the rollback above
and revisit #629 before re-enabling.

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
