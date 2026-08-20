# Add an xtask triaged issue creation command

- Issue: [#1091](https://github.com/jaunder-org/jaunder/issues/1091)
- Sibling: [#1090](https://github.com/jaunder-org/jaunder/issues/1090) owns
  read-only milestone candidate discovery.

## Problem

Creating a Jaunder issue has a fixed metadata discipline, but the workflow is
spread across skills, GitHub MCP calls, `gh project` commands, and readback
checks. The decisions are human/agent judgement; the application of those
decisions is mechanical.

A correct issue creation flow currently requires all of this:

- Conventional Commit issue title;
- GitHub issue type, never typeless;
- topic labels, not priority/layer labels;
- exactly one milestone;
- Jaunder Backlog project item;
- Priority project field;
- dependency links when explicit or clearly required;
- body readback because GitHub silently strips angle-bracket markup, including
  inside fenced code blocks.

Each step is documented, but documentation does not prevent drift. A command can
fail closed when required decisions are absent and can apply the boilerplate
reliably once supplied.

## Decision

Add a mutating command in the same `issue` xtask family as candidate discovery:

```bash
cargo xtask issue create \
  --title TITLE \
  --type TYPE \
  --milestone NAME_OR_NUMBER \
  --priority P2 \
  --label tooling \
  --label dx \
  --body-file PATH \
  --json
```

The command validates and applies explicit metadata. It must not infer type,
labels, milestone, or priority. Missing decisions are errors.

The command creates the issue, adds it to the Jaunder Backlog project, sets the
Priority field, reads the issue back, and reports whether the body changed in a
way that likely needs manual repair.

## Relationship to candidate discovery

#1090 and #1091 share the `cargo xtask issue` namespace but have opposite safety
profiles:

- `issue candidates` is read-only and gathers inputs for choosing work.
- `issue create` mutates GitHub and project state using explicit caller choices.

Do not merge these into a single auto-triage command. The shared namespace gives
humans and agents a predictable place to look; the subcommand names carry the
boundary.

## JSON contract

The exact Rust types may differ, but successful JSON output must expose the
created issue and all applied metadata:

```json
{
  "issue": {
    "number": 1091,
    "url": "https://github.com/jaunder-org/jaunder/issues/1091",
    "title": "feat(xtask): add triaged issue creation command",
    "type": "Task",
    "labels": ["tooling", "dx"],
    "milestone": "Developer tooling & DX"
  },
  "project": {
    "number": 1,
    "item_id": "PROJECT_ITEM_ID",
    "priority": "P2"
  },
  "body": {
    "readback_matches": true,
    "warning": null
  }
}
```

On validation failure, the command should fail before mutation when possible and
name every missing or invalid field.

## Acceptance criteria

- `cargo xtask issue create` creates a GitHub issue only when title, type,
  milestone, priority, and body file are supplied.
- Title validation enforces the repository's Conventional Commit issue-title
  convention.
- Type is validated against the repository's valid issue types.
- Labels are validated before issue creation; unknown labels fail before
  mutation when possible.
- Milestone names and numbers resolve to exactly one open milestone; ambiguity
  or absence fails before issue creation when possible.
- Priority accepts only the Jaunder Backlog priority options and applies the
  corresponding project field after the issue is added to the project.
- The issue is added to the Jaunder Backlog project exactly once.
- The command reads the issue body back after creation and detects likely GitHub
  body mangling, including stripped angle-bracket placeholders; it reports the
  mismatch and does not pretend the filed body is intact.
- The command emits stable JSON with issue number, URL, applied metadata,
  project item id, and any post-create body warning.
- `jaunder-issues` documents the command as the preferred path for new issues
  while preserving the human/agent decision boundary for type, labels,
  milestone, priority, and dependencies.
- Tests cover validation failures without network access, successful request
  construction, project Priority update, body readback mismatch detection, and
  JSON output stability.
- `cargo xtask check` passes.
