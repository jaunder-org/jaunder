# PublishUpdate Storage Contract Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for delegated work.
> This outline exists because the change crosses both storage dialect binding
> boundaries and must preserve publication-state semantics and backend parity.

## Scope

In:

- Move `PublishUpdate` into the posts storage contract and export it from the
  storage crate surface.
- Carry the sum through `PostUpdate` and `UpdatePostInput` to both dialects.
- Migrate every production caller, test-support builder, test, and current
  architecture reference in one clean cutover.

Out:

- Post creation inputs and rendering.
- Wire formats, publication policy, SQL behavior, schema, and migrations.
- DTO renames or unrelated Post consolidation.

## Task outline

- [x] Task 1: Make invalid publication updates unrepresentable through storage
  - Contract: `storage::PublishUpdate::{Unpublish, Publish { at }}` is the sole
    publication-update input; `UpdatePostInput` owns `publish: PublishUpdate`;
    only SQLite and PostgreSQL binding preparation may derive scalar bind
    values.
  - Verification: dual-backend `update_publish_timestamp_semantics` covers
    explicit scheduling/backdating, retention, publish-now stamping, and
    unpublishing;
    `devtool run -- cargo xtask test-local -- -p jaunder update_publish_timestamp_semantics`;
    then `devtool run -- cargo xtask precommit` before the task commit.

## Risk checks

- SQLite and PostgreSQL retain byte-for-byte-equivalent publication CASE
  semantics and bind ordering.
- Slug-freeze behavior still receives the pre-update publication state.
- Web, AtomPub, post-service tests, and `UpdateRawPost` migrate without an
  alias, flattened compatibility constructor, or precedence comment.
- `docs/ARCHITECTURE.md` and public Rust documentation name the new owning
  module and preserve ADR-0027's three-state policy.
- W1 creation symbols and ADR-0090's `RenderOutput` boundary remain untouched.
