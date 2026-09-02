# Issue #1169: Remove deprecated ADR renumber command

## Outcome

Remove `cargo xtask adr renumber` now that the serialized post-merge promoter
has replaced collision-era branch renumbering in production. Keep detection of
invalid numbered ADR collisions, but direct recovery to the tracked-draft and
promoter flow.

## Removal gate

The compatibility gate is satisfied:

- #742 landed through PR #1175 on 2026-08-25.
- The promoter has completed repeated production cycles since then, including
  merged promoter PRs #1181 through #1326.

Issue #1169 now defines that exercised production period as its compatibility
release because this repository does not publish GitHub Release objects.

## Command cutover

- Remove `AdrCommand::Renumber`, its command-name and dispatch arms, and the
  positive parser test.
- Add a parser contract proving `cargo xtask adr renumber` is rejected as an
  unknown subcommand.
- Remove the public `adr::renumber` entry point, deprecation constant, renumber
  orchestration, renumber-only rewrite helpers, and command-specific tests.
- Retain helpers and tests shared with `adr promote`, including padding, stem
  rewriting, promotion, index synchronization, and Git utilities used elsewhere.
- `cargo xtask adr --help` must not list `renumber`.

## Recovery guidance

- Keep `identifier-collisions` detection. Replace its deprecated-command hint
  with the existing repository rule: feature ADRs are tracked, numberless drafts
  under `docs/adr/drafts/`; serialized promotion allocates numbers after merge.
  A promoter failure is diagnosed or rerun rather than repaired by local
  numbering.
- Remove live compatibility wording from `CONTRIBUTING.md`,
  `docs/ARCHITECTURE.md`, and `docs/adr/drafts/README.md`. Preserve current
  promoter failure semantics and migration-number guidance.
- Do not rewrite historical records: numbered ADRs, archived
  plans/specifications, issue history, and other prose describing the former
  command remain intact.

## Maintained skills

Update the authoritative sources in
`~/src/agent-configuration/projects/jaunder/.rulesync/skills/`:

- `jaunder-adr`: remove the deprecated-command reference; retain the instruction
  to leave the tracked draft on `main` and diagnose or rerun promoter
  automation.
- `jaunder-ship`: replace the branch-renumber warning with the tracked-draft
  flow and promoter recovery.

Commit the authoritative changes on the agent-configuration repository's current
`issue-1119-focused-e2e-local-skills` branch, as explicitly chosen for this
issue; keep that commit separate from the Jaunder feature PR. Then run
`refresh-agent-config jaunder` and `refresh-agent-config --check jaunder` so
every Jaunder worktree receives the generated `.agents` and `.claude`
projections. Do not push the agent-configuration branch without a separate
instruction.

## Verification

- Focused xtask tests cover unknown-subcommand rejection, collision recovery
  wording, and retained promotion behavior.
- Real CLI smoke tests prove help omission and invocation rejection.
- Search active production, contributor, architecture, draft-authoring, and
  authoritative skill sources for live `adr renumber` instructions; only
  historical records may match.
- Confirm the authoritative skill-source change is committed on the selected
  agent-configuration branch and every generated worktree projection is current.
- Run `cargo xtask check --no-test`, the commit gate, and the pre-push gate
  according to the repository verify ladder.

## Non-goals

- Changing serialized promoter behavior or permissions.
- Removing collision detection or migration sequence checks.
- Rewriting historical ADR decisions or archived planning records.
- Adding a replacement local numbering command.
