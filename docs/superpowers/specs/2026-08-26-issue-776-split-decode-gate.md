# Split the SQLx newtype decode gate

## Outcome

`sqlx-newtype-decode` retains its existing interface, verdicts, and diagnostics
while its implementation is divided into focused private modules that are easier
to navigate and change independently.

## Load-bearing decisions

- Keep `steps::sqlx_newtype_decode_check::run` as the sole external interface.
- Replace the monolithic source file with a `sqlx_newtype_decode_check/` module
  directory.
- Keep the ADR-0085 conformance argument in `mod.rs`; under the repository's
  assembly-only rule, that file contains only module documentation, private
  module declarations, and the `run` re-export.
- Divide implementation ownership by existing concern: macro-model self-audit,
  approved-type construction and composite proof, decode scanning, allowlist
  policy, and verdict/report orchestration.
- Keep all child modules private and expose only the minimum sibling-visible
  types and functions needed by the existing dependency direction.
- Move each unit test beside the implementation it exercises. Full-diagnostic
  and end-to-end gate tests remain with verdict/report orchestration.
- Preserve the allowlist data, matching keys, category ordering, multiplicity
  rules, failure ordering, and every diagnostic byte. This is relocation, not a
  gate redesign.

## Acceptance

- Each implementation file has one named reason to change, and no replacement
  file becomes a second catch-all.
- The public step registration still calls
  `steps::sqlx_newtype_decode_check::run` without caller changes.
- The module-level ADR-0085 argument remains complete and directly discoverable
  from the registered step.
- Capture the gate's complete `StepResult` (`name`, `ok`, and full `detail`,
  excluding outer timing metadata) before relocation and compare it byte for
  byte after relocation on the clean tree and on each of #728's four one-line
  revert proofs:
  - decode the `FeedPath` catch-up row as `String`;
  - retype `FeedEventRecord.status` to `String`;
  - retype the `TargetKind` audience rows to
    `Vec<(String, Option<AudienceId>)>`;
  - remove one `ColumnInfo` row-get turbofish.
- All existing focused `sqlx_newtype_decode_check` tests pass after relocation
  without weakening assertions.
- The applicable xtask verification ladder is green.

## Boundaries

- No change to which source roots, declarations, macros, decode targets,
  wrappers, composites, or foreign types are recognized.
- No allowlist addition, removal, reclassification, or rewritten justification.
- No new abstraction layer, generic framework, or reusable gate infrastructure.
- No production crate, domain vocabulary, or architectural decision changes.
