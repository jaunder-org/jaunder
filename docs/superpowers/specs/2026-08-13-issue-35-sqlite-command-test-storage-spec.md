# Issue #35: Centralize SQLite command-test storage setup

- Status: Draft
- Issue: [#35](https://github.com/jaunder-org/jaunder/issues/35)
- Date: 2026-08-13

## Context

Five unit tests in `server/src/commands.rs` independently create a temporary
storage root, choose its `jaunder.db` path, format and parse the SQLite URL, and
assemble `StorageArgs`. Three tests then open and migrate the database before
calling a command. Two deliberately leave it absent because automatic
initialization or refusal-before-open is the behavior under test.

`server/src/test_support.rs` is the established in-crate unit-test seam for
SQLite construction. Its `migrated_sqlite_db` already repeats the same
`jaunder.db` path and `DbConnectOptions` construction. ADR-0016 requires tests
to construct only the storage handles their subject needs; the
backend-parametric integration harness in `storage::test_support` is a different
seam established by ADR-0033.

A repository-wide audit reviewed hand-maintained production code, tests,
tooling, Nix, CI, and Elisp for repeated knowledge rather than repeated syntax.
A cluster qualified with at least three sites, or with two sites when it encoded
load-bearing policy or lifecycle. Shallow wrappers and intentional backend or
protocol differences were rejected. The audit found 29 unowned qualifying
clusters. This branch implements only the SQLite command-test cluster; the other
28 become separately triaged issues before code implementation begins.

## Decisions

### D1. Share SQLite options construction at the in-crate test seam

Add this narrow helper to `server/src/test_support.rs`:

```rust
pub(crate) fn sqlite_db_options(dir: &Path) -> DbConnectOptions
```

It joins `jaunder.db` to `dir`, formats the `sqlite:` URL, parses it as
`DbConnectOptions`, and fails with the existing test-only expectation on an
invalid URL. `migrated_sqlite_db` delegates its options construction to this
helper and continues to open and migrate the pool itself.

The helper does not create the directory, open the database, run migrations, or
return `AppState`. Those lifecycle decisions remain visible to each test.

### D2. Share `StorageArgs` assembly only inside command tests

Add a private helper inside `server/src/commands.rs`'s test module:

```rust
fn sqlite_storage_args(temp: &TempDir) -> StorageArgs
```

It sets `storage_path` to the temporary directory and obtains `db` from
`crate::test_support::sqlite_db_options(temp.path())`. The five duplicated test
setups use it.

The initialized tests remain explicit:

- `cmd_site_config_set_upserts_and_get_and_list_read_back` opens the database
  before invoking site-config commands;
- `cmd_user_invite_creates_invite_expiring_in_the_future` opens the database and
  retains its state to inspect the resulting invite;
- `cmd_user_invite_with_base_url_configured_prints_link` opens the database,
  seeds the site base URL, and retains its state for inspection.

The uninitialized tests remain explicit:

- `prepare_server_auto_initializes_in_dev_mode` computes the expected
  `jaunder.db` path and proves it is absent before `prepare_server` and present
  afterward;
- `prepare_server_refuses_on_live_holder_before_db_open` computes that path and
  proves refusal occurs before database creation.

The `TempDir` bindings remain in the tests so the storage root lifetime is
obvious. No initialized fixture object or backend-neutral builder is introduced.

### D3. Track every other qualifying audit cluster separately

The plan's first implementation task searches open and closed issues, then
ensures exactly one focused issue owns each of these 28 clusters. It reuses or
updates an exact owner discovered during that search and creates only missing
issues. Every resulting issue is a `Task` with label `dx`, milestone
`Code quality ratchet` (#9), Jaunder Backlog status `Todo`, and priority P3
unless its actual dependency or urgency evidence requires a different value.
Real prerequisites and file-movement coordination are linked explicitly; mere
file overlap is not recorded as a blocker.

#### Server and storage

1. Replace twelve manual Axum response-body UTF-8 translations in server
   integration tests with `helpers::body_string`.
2. Centralize the non-verbose `Cli { command: Some(...) }` envelope and repeated
   initialization prerequisite in `server/src/main.rs` tests.
3. Package the initialized dual-backend command-test lifetime used by 19 sites
   in `server/tests/misc/commands.rs`; coordinate, but do not merge, with #977.
4. Centralize the local-user subscription fixture lifecycle used by server and
   storage tests; preserve #750's production subscriber-reference scope and the
   structural scopes of #950 and #963.
5. Use typed `web::posts::PostInputs` fixtures for 15 valid create/update
   request envelopes while keeping malformed-wire tests as raw JSON.
6. Extract the six typed SMTP site-config reads without moving their distinct
   required, optional, and default policies.
7. Centralize the two-copy invalid stored-URL purge policy in
   `storage/src/site_config.rs` without logging stored values.
8. Replace four hand-built media rows in `server/tests/web/web_media.rs` with
   the existing `storage::test_support::seed_media` seam.
9. Share an opinionated feed-cache fixture across the seven constructor sites
   without hiding storage-contract inputs.

#### Product crates

10. Replace raw page revalidation counters with the existing `Invalidator`
    lifecycle in home, cockpit, and media pages.
11. Make `authored_post` own the shared `PostRecord -> RenderedPost` translation
    while preserving `rendered_post`'s public-only guard.
12. Centralize the four `TagLabel -> TagSummary` conversions as one invariant.
13. Replace 21 manual Leptos owner setup/teardown sites with `Owner::with`.
14. Share feed-renderer input fixtures without sharing or weakening renderer
    assertions; coordinate the typed-fixture portion with #694.
15. Complete adoption of `common::text::non_empty` for submitted-value
    validation and add only the narrow bounded-non-empty seam the five sites
    need.
16. Centralize the session-cookie security attribute policy while leaving #677's
    response-plumbing migration intact.

#### Tooling crates

17. Share the Nix producer, consumer-gate, and sentinel-detail lifecycle between
    coverage and doctest steps without changing their distinct diagnostics.
18. Route the three ADR renumber scratch Git repositories through the existing
    `xtask::test_support` fixture seam; coordinate file movement with #989.
19. Share proc-macro public unit-error emission while preserving public
    signatures and exact compiler diagnostics.
20. Share fallible string serde bridge emission while preserving token shape and
    generated error behavior.

#### CI, Nix, end-to-end, and Elisp

21. Centralize the four-copy GitHub Actions Nix, Cachix, and xtask-cache
    bootstrap in a pinned repository-local composite action while keeping job
    policy in each workflow.
22. Collapse `mkE2eSqliteCheck` and `mkE2ePostgresCheck` behind one
    backend-aware Nix e2e builder without changing VM resources, service
    ordering, worker counts, or artifacts; coordinate with #985 and #828.
23. Promote the existing end-to-end raw media uploader from the delete-guard
    block to the whole media spec. Route the three inline successful uploads
    through it, retain its three existing successful callers, and keep the
    unauthenticated negative request raw.
24. Give media-library entry one wait-disciplined navigation helper without
    hiding upload or route assertions.
25. Centralize the mirrored admin settings re-entry policy while preserving the
    distinct browser contexts and routes.
26. Centralize feed alternate-link DOM translation in the end-to-end suite.
27. Centralize Elisp integration temporary-directory lifetime and recursive
    cleanup without absorbing the live-server lifetime.
28. Replace the two remaining inline end-to-end username generators with the
    existing `generateUsername` fixture constructor.

Exact-owner findings already tracked by issues such as #700, #841, #913, and
#914 remain with those issues. Structural split issues such as #950, #959, #963,
#976, #977, #978, #979, #985, and #992 remain separate. Partial overlap with an
existing issue is recorded as coordination, not treated as permission to close
or silently widen that issue.

### D4. Preserve observable behavior

Production code and command interfaces do not change. Test names, inputs,
assertions, database initialization order, and temporary-directory lifetime do
not change. The helper extraction does not add a new behavioral contract, so no
new test is required; the five existing tests are the regression proof.

## Non-goals

- Implementing any audit cluster other than the five SQLite command-test setups.
- Creating duplicate issues for exact-owner findings.
- Factoring syntax-only repetition whose interface would be as wide as its
  implementation.
- Changing production database opening, migrations, command behavior, or CLI
  output.
- Moving the helper into `storage::test_support` or changing the dual-backend
  integration harness.
- Hiding initialized versus uninitialized database state in a fixture object.

## Acceptance criteria

1. `server::test_support::sqlite_db_options` is the sole constructor for the
   `jaunder.db` `DbConnectOptions` used by `migrated_sqlite_db` and the five
   command-test setups.
2. The five tests construct `StorageArgs` through one private
   `sqlite_storage_args` helper; no compatibility helper or generic fixture is
   added.
3. The three initialized tests still open the database before invoking their
   subject, and the two uninitialized tests still prove absence before automatic
   initialization or refusal.
4. Existing test names, inputs, assertions, initialization order, and `TempDir`
   ownership remain unchanged apart from construction delegation.
5. Exactly 28 focused sibling issues own the clusters listed in D3. Each issue
   is searched before creation, fully triaged, and linked to genuine
   prerequisites or coordination owners without duplicating exact-owner issues.
6. No issue is created for rejected shallow repetition, intentional
   backend/protocol differences, or an already exact-owned cluster.
7. The five affected command tests pass on SQLite.
8. `cargo xtask check` passes on the completed change.
