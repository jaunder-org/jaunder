# Issue #1169 implementation outline

Spec:
[`docs/archive/2026-09-01-issue-1169-remove-adr-renumber-spec.md`](2026-09-01-issue-1169-remove-adr-renumber-spec.md)

## Risk trigger

A plan is required because this removes a public CLI surface and coordinates
authoritative workflow guidance in a separate repository.

## Contracts

- `cargo xtask adr renumber` becomes an unknown subcommand; help omits it.
- Numbered-ADR collision detection remains, but its live recovery text directs
  feature work back to tracked drafts and serialized promotion.
- Promotion, index synchronization, migration checks, and historical ADR/archive
  records are unchanged.
- Authoritative skill sources are committed on the current agent-configuration
  branch and regenerated into all Jaunder worktrees; they are not part of the
  Jaunder PR.

## Implementation slices

### 1. Remove the command and legacy implementation

Own `xtask/src/lib.rs`, `xtask/src/adr.rs`, `xtask/src/git.rs`, and
`xtask/src/steps/sequence_check.rs`.

- Use LSP references before removing exported `adr::renumber` and the CLI
  variant.
- Remove the parser variant, command-name mapping, dispatch, public entry point,
  deprecation constant, renumber orchestration, renumber-only helpers, and
  command-specific tests.
- Remove renumber-only Git helpers and their combined test after confirming
  references; retain Git utilities, ADR helpers, and tests used by promotion and
  index synchronization.
- Replace the positive parser test with unknown-subcommand rejection.
- Replace the collision gate's deprecated hint with tracked-draft/promoter
  recovery and update its focused test. Keep migration collision behavior.

### 2. Update maintained repository guidance

Own `CONTRIBUTING.md`, `docs/ARCHITECTURE.md`, and `docs/adr/drafts/README.md`.

- Remove compatibility-period wording now made obsolete by #1169.
- Reuse existing promoter failure language: leave the tracked draft on `main`,
  diagnose or rerun visible automation, and never number/promote from the
  feature checkout.
- Do not edit numbered ADRs, archived plans/specifications, `CONTEXT.md`, or
  promoter behavior.

### 3. Update authoritative skill sources

Own
`~/src/agent-configuration/projects/jaunder/.rulesync/skills/jaunder-adr/SKILL.md`
and `jaunder-ship/SKILL.md`.

- Remove named use of the retired command while preserving tracked-draft and
  promoter diagnosis instructions.
- Format, review, and commit the authoritative changes together with the prior
  post-land cleanup guidance already present on
  `issue-1119-focused-e2e-local-skills`; do not push.
- Run `refresh-agent-config jaunder` and its `--check` mode. Generated ignored
  mirrors are verification outputs, not Jaunder PR files.

## Integration order

1. Capture symbol references, then implement the three slices concurrently.
2. Integrate shared wording around tracked drafts and serialized promoter
   recovery; avoid introducing a second recovery convention.
3. Format Rust and Markdown once.

## Verification

1. Focused xtask tests for parser rejection, collision recovery text, and
   retained ADR promotion behavior.
2. `cargo xtask adr --help`: `renumber` absent.
3. `cargo xtask adr renumber`: exit 2 unknown-subcommand rejection and no
   mutation.
4. Active-source search over xtask, contributor/architecture/draft guidance, and
   authoritative skills: no live `adr renumber`; historical ADR/archive
   references remain.
5. `cargo xtask check --no-test`, dual standards/spec review, commit gate,
   pre-push gate, PR CI, explicit merge approval, and ordinary post-land
   cleanup.
6. Agent-configuration commit exists on the selected local branch and
   `refresh-agent-config --check jaunder` reports every worktree current.
