# Spec — #950: split storage integration tests by concern

**Status:** design resolved. **Implements:**
[ADR-0128](../../adr/0128-mod-rs-assembles-module-surface.md). **Preserves:**
[ADR-0053](../../adr/0053-storage-test-homing-and-dual-backend.md).

## Problem

`server/tests/storage/mod.rs` is an approximately 6,000-line integration-test
suite containing 156 test functions and their helpers. It has no assembly
surface at all, so readers must scan one monolith to find a storage contract and
ADR-0128's workspace-wide rule remains unsatisfied for the final offender
deferred from #942.

A mechanical move alone would leave duplicated smoke tests and helpers visible
only because unrelated concerns previously shared one lexical scope. Conversely,
cleaning while moving would make it impossible to distinguish relocated coverage
from deleted or rewritten coverage.

## Decisions

1. `server/tests/storage/mod.rs` becomes an assembly-only module containing
   private `mod` declarations. Tests are internal, so it adds no re-exports to
   preserve their former qualified paths.
2. The suite is split into these 19 concern modules:
   - `fixtures.rs`: helpers used by at least two concern modules;
   - `database.rs`: database opening, migration re-entry, and connection-option
     parsing;
   - `lookups.rs`: seeded lookup-table and enum correspondence;
   - `subscriptions.rs`, `audiences.rs`, `users_auth.rs`, `sessions.rs`,
     `invites.rs`, `email_verification.rs`, and `password_reset.rs`: the
     corresponding storage or atomic-operation contracts;
   - `site_config.rs`: SiteConfigStorage contracts and the configuration-driven
     mailer-construction test;
   - `posts.rs`: Post create/read/update/delete, revision, rendering, audience
     persistence, and batch-mutation contracts;
   - `listing.rs`: Post query surfaces, pagination, scheduling boundaries,
     hybrid windows, and feed-catchup selection, including tag-filtered Post
     queries;
   - `tags.rs`: tag persistence, normalization, reconciliation, inventory, and
     PostRecord tag payloads;
   - `feed_events.rs`, `media.rs`, `user_config.rs`, `fk_constraints.rs`, and
     `resolution.rs`: the corresponding queue, storage, schema-integrity, and
     viewer-resolution contracts.
3. A cross-store test lives with the operation whose atomic contract it asserts,
   not with every store it happens to call. The existing combined email
   verification/password-reset smoke test has no single owning operation and is
   split into two tests, preserving its assertions in the two corresponding
   modules.
4. Concern-local helpers remain private in their concern file. A helper moves to
   `fixtures.rs` only when at least two concern modules use it; consumers import
   shared helpers explicitly, and visibility widens only as far as compilation
   requires.
5. Each concern module owns only the `storage::{...}` imports it uses. Every
   module containing `#[apply(backends)]` imports `rstest::*`,
   `rstest_reuse::*`, and the bare `storage::test_support::backends` template in
   that module, preserving ADR-0124's cross-module macro resolution.
6. Relocation and cleanup remain reviewably separate. Preparation commits may
   change shared-helper visibility and other prerequisites. A relocation commit
   does not edit moved test/helper bodies except for formatter output; it may
   add the concern's `mod` declaration and the imports required to compile the
   moved bodies. Cleanup follows in separate commits.
7. Cleanup is limited to exact redundancy and locality:
   - a test may be removed only when every observable assertion it makes is
     already exercised by named retained tests on both backends;
   - genuinely duplicated helpers may be consolidated without changing test
     inputs or assertions;
   - a test may be renamed only when its old name misstates the retained
     contract;
   - no storage behavior, production API, test input domain, or assertion is
     added or weakened.
8. Qualified Rust test paths may change from `storage::<test>` to
   `storage::<concern>::<test>`. Leaf test names stay unchanged except for the
   narrowly authorized locality split or a demonstrably inaccurate name.
   `CONTRIBUTING.md` is updated to describe the new path shape and provide a
   valid concern-qualified filter example.
9. This work records no new architectural decision. It applies ADR-0128's
   existing assembly rule and ADR-0053's existing integration-test and
   dual-backend rules to this suite.

## Required cleanup dispositions

These removals are part of the deliverable, not optional follow-up. The named
retained tests account for the removed assertions:

| Removed test                                  | Retained replacement coverage                                                                                                                                                                                                                                                                                          |
| --------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `site_config_set_then_get_roundtrips`         | `site_config_round_trips_through_typed_keys` performs the same typed `SiteTitle` set/get round trip.                                                                                                                                                                                                                   |
| `get_missing_key_returns_none`                | `site_config_round_trips_through_typed_keys` and `site_config_operations` both assert that an unset registered key returns `None`.                                                                                                                                                                                     |
| `set_overwrites_existing_value`               | `site_config_operations` sets, overwrites, and reads `SiteTitle`.                                                                                                                                                                                                                                                      |
| `create_user_duplicate_and_authenticate_work` | `create_user_succeeds_and_get_by_username_returns_record`, `duplicate_username_returns_username_taken`, and `authenticate_correct_password_returns_record_and_sets_last_authenticated_at` cover its create/read, duplicate, authenticate, and timestamp assertions.                                                    |
| `session_lifecycle_works`                     | `create_session_then_authenticate_returns_correct_record`, `list_sessions_returns_only_sessions_for_given_user`, and `revoke_session_then_authenticate_returns_session_not_found` cover its authentication payload, listing, revocation, and post-revocation error assertions.                                         |
| `invite_and_atomic_registration_work`         | `create_user_with_invite_creates_user_and_marks_invite_used` and `create_user_with_invite_second_call_returns_already_used` cover its created-user and consumed-invite assertions.                                                                                                                                     |
| `email_verification_and_password_reset_work`  | `create_email_verification_and_use_returns_user_id_and_email` and the new password-reset success test together retain its verification result, reset claim, confirmed credential, and authentication assertions; existing `set_email_persists_and_get_user_reflects_it` retains its intervening email update contract. |

Any additional removal requires adding another row with the removed leaf,
retained replacements, and per-assertion accounting before implementation.

## Acceptance criteria

1. `server/tests/storage/mod.rs` contains only module documentation, attributes,
   and the 19 private `mod` declarations above; it contains no functions, types,
   implementations, constants, macros, inline modules, or re-exports.
2. Every retained storage integration test and helper lives in the concern file
   defined by the seams above. No catch-all sibling replaces the monolith, and
   no concern file is merely a differently named copy of the original suite.
3. Every pre-split test leaf name appears exactly once after the split except
   for the tests listed in Required cleanup dispositions. Those tests are
   absent, their named replacements are present, and the new password-reset
   success test plus the existing email-verification test together retain the
   assertions from `email_verification_and_password_reset_work`.
4. Every helper in `fixtures.rs` has consumers in at least two concern modules;
   every helper used by only one concern remains in that concern module. Shared
   helper imports are explicit rather than globbed.
5. Each concern module's `storage::{...}` import contains only items used by
   that module. Every module containing `#[apply(backends)]` locally imports
   `rstest::*`, `rstest_reuse::*`, and bare `storage::test_support::backends`.
6. All backend-parametrized tests still use the repository's shared `backends`
   template and execute for SQLite and PostgreSQL. Backend-specific setup,
   isolation, and PostgreSQL guard lifetimes remain unchanged.
7. `CONTRIBUTING.md` no longer describes storage tests as a single file; its
   test-filter guidance shows a working `storage::<concern>::<leaf-test-name>`
   example.
8. Production source and production behavior are unchanged. Test cleanup does
   not add, remove, or weaken an observable storage contract.
9. Every preparation, relocation, and cleanup commit is independently formatted,
   passes the targeted storage integration suite, and passes `cargo xtask check`
   before the next commit. The completed branch passes `cargo xtask validate`,
   including both storage backends and the full e2e matrix required by
   CONTRIBUTING.md.

## Explicitly out of scope

- Changing storage traits, dialect implementations, schema, migrations, or
  application behavior.
- Introducing a new test framework, backend fixture convention, or assembly
  enforcement gate.
- Preserving former fully qualified test filter paths through forwarding modules
  or re-exports.
- Broadly redesigning tests, adding coverage, or rewriting assertions while the
  suite is being relocated.
