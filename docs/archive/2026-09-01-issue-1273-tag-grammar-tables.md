# Tag Grammar Case Tables

Issue: #1273 Status: Approved

## Outcome

`Tag` grammar and lowercase canonicalization are covered by two compact, named
data-driven test tables. Grammar changes have one accepted/canonicalized table
and one rejected table to update, without changing production behavior.

## Load-bearing decisions

- Use local named `rstest` rows beside `Tag`; no shared template or new test
  abstraction.
- Every accepted row supplies an input and its exact canonical lowercase output.
  Parsing success alone is insufficient.
- Every rejected row supplies one invalid input and asserts `Tag` parsing fails.
- Consolidate semantically duplicate literals while retaining every distinct
  boundary represented by the 17 superseded tests.
- Distinct accepted coverage includes canonical ASCII, uppercase and mixed-case
  normalization, trailing/middle/consecutive hyphens, digit-only, leading-digit,
  and long canonical and mixed-case inputs.
- Distinct rejected coverage includes empty input, leading hyphen, spaces,
  underscore, and special ASCII and non-ASCII characters in both leading and
  non-leading positions.
- Case names describe the behavior or boundary, so each row is independently
  identifiable in test output.

## Acceptance

- The 17 grammar/canonicalization test functions named in #1273 are replaced by
  exactly two `rstest` functions: accepted-and-canonicalized and rejected.
- Accepted rows compare the parsed `Tag` with the exact expected canonical
  string.
- Rejected rows assert parsing returns an error.
- No distinct grammar or canonicalization boundary from the superseded tests is
  lost, and redundant literals do not survive merely because they appeared in
  multiple old tests.
- The seven distinct trait/error tests named in #1273 remain registered
  separately.
- Existing serde, `TagLabel`, and `parse_and_validate_tags` tests remain
  separate and unchanged.
- Focused `common` Tag tests pass with every case reported as a named row.

## Boundaries

- Do not modify `Tag`, `InvalidTag`, `TagLabel`, parsing grammar,
  canonicalization, serialization, or public APIs.
- Do not add cross-module fixtures, `rstest_reuse` templates, dependencies, or
  an ADR.
- Do not refactor tests outside the 17 registrations named in #1273.
