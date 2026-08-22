# Add an xtask milestone candidate discovery command

- Issue: [#1090](https://github.com/jaunder-org/jaunder/issues/1090)
- Sibling: [#1091](https://github.com/jaunder-org/jaunder/issues/1091) owns the
  mutating issue creation command.

## Problem

`/skill:jaunder-develop milestone N` spends its first turns rediscovering a
mostly deterministic fact set before it can choose the next issue. The choice is
judgement work; the gathering is not. Today each agent re-runs GitHub/project
queries to find milestone issues, skip claimed issues, inspect blockers, and
notice local resume artifacts.

That discovery has sharp edges already documented in `jaunder-develop` and
`jaunder-issues/claim-status.md`:

- milestone filtering must happen server-side;
- wide Jaunder Backlog project scans are expensive and silently truncated;
- project item fields are not all present in `item-list` output;
- Status = `In Progress` is the claim signal;
- `--blocked-by` relationships must affect candidate eligibility;
- local branch/spec/plan/PR state is derived, never stored.

Leaving those as prose means every agent pays the same query cost and can repeat
the same failure modes. Encoding the discovery substrate in xtask keeps the
choice boundary intact while making the input reliable.

## Decision

Introduce an `issue` xtask command family with a read-only discovery subcommand:

```bash
cargo xtask issue candidates --milestone NAME_OR_NUMBER --json
```

The sibling mutating command is `cargo xtask issue create` (#1091). The spelling
matters: `candidates` gathers inputs for a later judgement; `create` files a new
issue with explicit metadata. Do not use `next`, because that implies the
command chooses the issue.

The command resolves the milestone, fetches open milestone issues with
server-side filtering, obtains Jaunder Backlog Status without scanning the whole
project, resolves open blocker relationships, and overlays local resume state
from the current checkout.

The output separates eligible candidates from skipped issues. Every skipped
issue carries an explicit reason such as `claimed`, `blocked`, `closed`,
`missing-project-status`, or `invalid-milestone`. Candidate ordering must be
stable and documented; it is an ordering for review, not a recommendation.

## JSON contract

The exact Rust types may differ, but the top-level shape must express these
concepts without requiring follow-up GitHub discovery queries:

```json
{
  "milestone": {
    "number": 3,
    "title": "Developer tooling & DX"
  },
  "candidates": [
    {
      "number": 1090,
      "title": "feat(xtask): add milestone candidate discovery command",
      "url": "https://github.com/jaunder-org/jaunder/issues/1090",
      "labels": ["tooling", "dx"],
      "status": "Todo",
      "blocked_by": [],
      "local": {
        "branch": null,
        "spec": "docs/superpowers/specs/2026-08-20-issue-1090-milestone-candidate-discovery.md",
        "plan": null,
        "plan_progress": null,
        "open_pr": null
      }
    }
  ],
  "skipped": [
    {
      "number": 1080,
      "title": "example",
      "reason": "claimed",
      "detail": "Jaunder Backlog Status is In Progress"
    }
  ]
}
```

## Boundaries

This command is not an issue creator and not a triage bot. It must not infer a
milestone, priority, labels, or the best next issue. #1091 may enforce explicit
metadata for new issues, but this command remains read-only.

No persistent registry. All state is derived from GitHub and the current
checkout at command runtime.

## Acceptance criteria

- `cargo xtask issue candidates --milestone NAME_OR_NUMBER --json` returns
  stable JSON for milestone candidate discovery and exits non-zero on discovery
  errors that would make the result misleading.
- The command resolves milestone names and numbers, and reports ambiguity or
  absence explicitly.
- Milestone issue listing is filtered server-side; implementation does not scan
  the entire Jaunder Backlog project to find candidate issues.
- Jaunder Backlog Status is obtained exactly enough to classify issues as
  eligible, claimed, or missing project state; Status = `In Progress` is skipped
  as `claimed`.
- Open `--blocked-by` relationships are reported. Issues blocked by open issues
  are skipped by default with the blocker numbers in the detail.
- Local resume state is derived live for each issue: matching `issue-N-slug`
  branch, spec path, plan path, plan checkbox counts when present, and open PR
  for the branch when present.
- Human-readable output summarizes candidate count and skipped reasons without
  hiding the JSON contract.
- `jaunder-develop` uses the command for milestone selection and states that the
  command gathers inputs only; the agent or human still chooses the next issue.
- Tests cover milestone resolution, project Status classification, blocker
  classification, local artifact discovery, output schema stability, and
  no-network command logic through fixtures or fakes.
- `cargo xtask check` passes.
