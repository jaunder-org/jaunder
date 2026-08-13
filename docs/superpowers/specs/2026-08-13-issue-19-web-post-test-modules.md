# Issue #19: Split web post integration tests by endpoint family

- Status: Draft
- Issue: [#19](https://github.com/jaunder-org/jaunder/issues/19)
- Date: 2026-08-13

## Context

`server/tests/web/web_posts.rs` is a 2,800-line integration-test module
containing 64 test functions. It mixes post creation, reads, updates and
publication transitions, cursor-based listings, access control, and
audience-selection contracts with the request/setup helpers those tests share.

The module belongs to the single server integration-test binary established by
ADR-0067. The split must improve locality without creating another test target,
changing production behavior, or changing the external web-post interface.
ADR-0128 also requires every resulting `mod.rs` to contain assembly only.

A repository-wide audit reviewed every hand-maintained code or test file at
least 1,000 lines. Size was an audit trigger, not an automatic split rule:
cohesive files remain whole, separately tracked splits stay with their existing
issues, and independently actionable concerns receive focused sibling issues
rather than widening #19.

## Decisions

### D1. Establish the `web::posts` module

Replace `server/tests/web/web_posts.rs` with a directory module at
`server/tests/web/posts/`. `server/tests/web/mod.rs` changes its declaration to
`mod posts;`, so the test module name no longer repeats its `web` parent and
matches the `web/src/posts/` vertical.

`server/tests/web/posts/mod.rs` is assembly-only. It declares the concern
modules and contains no functions, types, constants, statics, macros, or inline
test module.

### D2. Split by endpoint family

The directory contains these modules:

- `fixtures.rs`: request builders and setup helpers used by more than one
  concern module;
- `create.rs`: post creation, creation-time scheduling, creation-time tag
  handling, and default-post-format contracts used by the composer;
- `read.rs`: single-post and preview reads whose contract does not vary by
  viewer identity, including input rejection, not-found behavior, and tag
  hydration on returned posts;
- `update.rs`: update, publish, delete, unpublish, and update-time tag mutation
  contracts;
- `listing.rs`: draft, user, local, home, and tag listings, including cursor
  wire and pagination contracts;
- `visibility.rs`: cross-endpoint unauthenticated rejection and every read whose
  core assertion compares or restricts results by viewer identity. This includes
  the draft/preview author-only cases, guest draft hiding, the scheduled-post
  author read, local-timeline audience resolution, and subscriber-post hiding;
- `audiences.rs`: default-audience and per-post audience-selection endpoint
  contracts.

Each existing test function moves once to the module whose endpoint family owns
its observable contract. Parameterized cases stay attached to the same test
function.

The ownership rule is decisive: a read that asserts only representation,
parsing, or existence belongs to `read.rs`; a read that asserts who may observe
the post belongs to `visibility.rs`.

### D3. Keep helper visibility narrow

A helper used by multiple concern modules moves to `fixtures.rs` and receives
only the `pub(super)` visibility needed by sibling consumers. A helper used by
one concern remains private in that concern module.

The split does not introduce a new helper abstraction, change request
construction, combine test cases, or rewrite assertions. Movement and the
minimum visibility/import adjustments required by Rust module boundaries are the
only code changes.

Every concern file that uses `#[apply(backends)]` or `#[apply(backends_matrix)]`
imports the applicable `rstest`, `rstest_reuse`, and bare template names
required by ADR-0124.

### D4. Keep the change behavior-neutral

Production implementation, server-function signatures, endpoint paths, wire
codecs, storage behavior, and test assertions do not change. Existing test
function names remain unchanged. Their fully qualified paths gain the concern
module segment, such as
`web::posts::create::create_post_persists_rendered_published_post`.

Two live documentation references change with the clean module cutover:

- the `common/src/render.rs` rustdoc points to
  `server/tests/web/posts/create.rs` instead of the removed source file; and
- `CONTRIBUTING.md` describes integration-test paths as arbitrary nested module
  paths and uses `web::posts::<concern>::<name>` plus a concrete
  `web::posts::create` filter example.

No new behavioral tests are required: the observable contract is preservation of
the existing integration-test population and behavior.

### D5. Track the remaining audit findings separately

The plan's first implementation task files focused Task issues for these 17
independently actionable splits and keeps them in the Code quality ratchet
milestone. Every issue exists before the first commit that removes or moves an
item from `web_posts.rs`:

- `server/tests/atompub/atompub_posts.rs`;
- `server/src/commands.rs`;
- `server/src/observability.rs`;
- `storage/src/posts.rs`;
- `common/src/media.rs`;
- `storage/src/test_support.rs`;
- `common/src/atompub/entry.rs`;
- `storage/src/backup.rs`;
- `xtask/src/steps/ident_gate.rs`;
- `flake.nix`;
- `xtask/src/traces/analyze.rs`;
- `xtask/src/adr_readme.rs`;
- `xtask/src/lib.rs`;
- `xtask/src/adr.rs`;
- `web/src/posts/component.rs`;
- `end2end/tests/fixtures.ts`;
- `elisp/test/jaunder-test.el`.

Do not create duplicates for the three splits already owned by existing issues:

- `server/tests/storage/mod.rs`: #950;
- `common/src/render.rs`: #855;
- `xtask/src/steps/sqlx_newtype_decode_check.rs`: #776.

The audit found no warranted split for these cohesive or explicitly constrained
files:

- `server/src/cli.rs`;
- `macros/src/lib.rs`;
- `storage/src/post_service.rs`;
- `storage/src/site_config.rs`;
- `end2end/tests/posts.spec.ts`;
- `web/src/posts/api.rs`.

In particular, ADR-0070 and ADR-0082 require the post vertical's server
functions to remain directly in `web/src/posts/api.rs`; its size is an accepted
consequence of compile-time endpoint identity.

## Non-goals

- Splitting any file other than `server/tests/web/web_posts.rs` on this branch.
- Changing production behavior or web-post wire contracts.
- Renaming, combining, deleting, or adding behavioral test cases.
- Creating another server integration-test binary.
- Introducing a file-size gate or a maximum source-file length.
- Refactoring shared server test infrastructure outside the minimum moves
  required for `web::posts`.

## Acceptance criteria

1. `server/tests/web/web_posts.rs` no longer exists, and
   `server/tests/web/posts/` contains assembly-only `mod.rs` plus `fixtures.rs`,
   `create.rs`, `read.rs`, `update.rs`, `listing.rs`, `visibility.rs`, and
   `audiences.rs`.
2. `server/tests/web/mod.rs` declares `mod posts;`, with no `web_posts`
   compatibility module, second test target, or other top-level replacement.
3. All 64 existing `#[tokio::test]` functions remain present exactly once with
   unchanged function names, parameterized cases, request inputs, and
   assertions.
4. Shared helpers live in `fixtures.rs` only when at least two concern modules
   use them; all moved helper visibility is no wider than `pub(super)`.
5. Every new Rust file satisfies the repository's backend-template import
   requirements, and `posts/mod.rs` satisfies ADR-0128's assembly-only rule.
6. Live documentation contains no reference to the removed
   `server/tests/web/web_posts.rs` path: `common/src/render.rs` points to
   `server/tests/web/posts/create.rs`, and `CONTRIBUTING.md` documents and
   exemplifies the nested `web::posts::<concern>::<name>` test path.
7. The backend-parametric `web::posts` integration tests pass for SQLite and
   PostgreSQL.
8. `cargo xtask check` passes on the completed change.
9. Seventeen focused sibling Task issues exist for the actionable audit findings
   listed in D5, with no duplicate issues for #950, #855, or #776. Their GitHub
   creation timestamps precede the first branch commit that removes or moves an
   item from `web_posts.rs`.
