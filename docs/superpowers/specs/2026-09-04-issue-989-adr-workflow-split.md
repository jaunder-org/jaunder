# ADR Promotion Module Split

Issue: #989

## Outcome

Split the ADR promotion implementation into modules with one named
responsibility while preserving every command, callable path, mutation,
diagnostic, and test contract. The result is a navigable module facade over
deterministic content rewriting and the stateful promotion workflow; it does not
change ADR lifecycle behavior.

## Current workflow

ADR-0152 and completed issue #742 superseded the collision-era workflow
described by the original issue: feature pull requests commit tracked,
numberless drafts, and a serialized post-merge promoter assigns numbers.
`adr renumber` has been removed. This split follows the current workflow rather
than recreating a retired renumber module.

The promotion workflow remains one deep operation. It discovers and sorts
tracked drafts, assigns numbers from the current numbered ADR set, validates
every draft heading and confirms every draft source is tracked before moving any
draft, then moves and stages files, rewrites draft content and path citations,
synchronizes the README projection, and reports unresolved-link warnings. The
promoter controller in `xtask/src/pr/promoter.rs` continues to own GitHub and
queue policy and calls the unchanged local mutation seam.

## Module ownership

Replace `xtask/src/adr.rs` with this directory module:

- `xtask/src/adr/mod.rs` is the assembly-only facade. It contains module
  documentation, declarations, and explicit re-exports only, satisfying
  ADR-0128. The existing callable facade paths and visibility remain available
  for `adr::promote` and `adr::run_promote`; implementation helpers move behind
  the private `rewrite` owner rather than becoming unused facade surface.
- `xtask/src/adr/rewrite.rs` owns deterministic content transformations and
  their validation: number padding, stem replacement, one-level relative-link
  rewriting through the shared `doc_links` parser, draft-status acceptance, and
  draft-heading promotion.
- `xtask/src/adr/promote.rs` owns stateful promotion orchestration: ADR and
  draft population, deterministic assignment, pre-move heading and
  tracked-source validation, tracked Git moves and staging, repository-wide
  citation rewrites, README projection synchronization, warning collection, and
  summary construction.

The ordered promotion passes remain together in `promote.rs`; extracting
population or individual passes would expose tightly coupled implementation
details without creating a deeper interface. `doc_links`, `adr_readme`, `git`,
`ids`, and `StepResult` retain their current ownership.

## Tests

Tests move with the contract they prove:

- `rewrite.rs` contains the focused tests for padding, stem replacement,
  link-target edge cases, heading validation, and status rewriting.
- `promote.rs` contains repository/Git integration tests for input
  prevalidation, numbering, tracked renames, staging, cross-draft and repository
  citation rewrites, status transitions, README projection, warnings,
  no-op/rerun behavior, population filtering, and failure context.
- Existing CLI rejection and parsing tests remain in `xtask/src/lib.rs`;
  promoter controller ordering and failure-isolation tests remain in
  `xtask/src/pr/promoter.rs`.

No test is weakened or replaced with a source-shape assertion. Existing
observable assertions remain unchanged except for imports required by the move.

## Constraints

- Preserve ADR-0036's active collision-detection and generated-projection policy
  and ADR-0152's tracked-draft, post-merge promotion lifecycle.
- Preserve promotion as the acceptance event, including `proposed` to `accepted`
  rewriting and deliberate non-`proposed` statuses.
- Preserve all current success summaries, warnings, error context, operation
  ordering—including the pre-move heading and tracked-source checks and the
  existing post-mutation failure points—Git index effects, and no-op behavior.
- Preserve the `adr::run_promote(&Path) -> Result<String>` seam used by the
  promoter controller and the `adr::promote() -> StepResult` CLI wrapper.
- Keep `doc_links::links_in` as the shared Markdown-link parser; do not
  introduce a second parser.
- Do not change CLI grammar, workflow configuration, ADR authoring rules,
  promoter recovery policy, or documentation projections.

## Acceptance

- `xtask/src/adr/mod.rs` is an ADR-0128-compliant assembly-only facade with
  explicit re-exports.
- `rewrite.rs` and `promote.rs` each have one named responsibility and no
  retired renumbering concept is reintroduced.
- Existing crate call sites compile without path or behavior changes.
- Existing tests are preserved and pass from their concern-owned locations.
- The repository gate passes.
