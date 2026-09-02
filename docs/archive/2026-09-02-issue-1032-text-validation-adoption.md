# Issue #1032: Complete text validation helper adoption

## Outcome

Five string newtypes share one trim/non-empty policy, and the three bounded
newtypes share one post-trim Unicode-scalar limit seam. Their public behavior,
errors, ownership, and construction interfaces remain unchanged.

## Load-bearing decisions

- `common::text::non_empty` remains the owner of trim-then-empty validation.
- Add `text::bounded_non_empty(&str, usize) -> Option<&str>` as a crate-private
  helper. It delegates trimming and emptiness to `non_empty`, then accepts a
  value when its Unicode-scalar count is less than or equal to the supplied
  maximum.
- `AudienceName` and `DestinationPath` use owner-qualified `text::non_empty`.
- `Bio`, `DisplayName`, and `PostSummary` use owner-qualified
  `text::bounded_non_empty` with their existing local maximum constants.
- Each `FromStr` maps `None` to its existing error type and allocates its owned
  `String` only after all validation succeeds.
- Existing error display text, trim order, inclusive limits, serde/sqlx paths,
  and public APIs remain unchanged.

## Acceptance

- All five audited `FromStr` implementations use the selected shared seam.
- No audited implementation retains a local trim/empty/count condition.
- Focused tests prove whitespace rejection, trimmed success, inclusive
  Unicode-scalar limits with multibyte input, and the unchanged public error
  messages of all five newtypes.
- `cargo xtask test-local -- -p common` and `cargo xtask check` pass.

## Boundaries

- No validation-policy or error-message changes from #564 or #837.
- No new public helper or generalized validation abstraction.
- No unrelated text-normalization callsites are migrated.
