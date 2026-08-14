# SQLite Command-Test Storage Setup — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace five hand-written SQLite command-test storage setups with one
narrow options constructor and one local `StorageArgs` constructor, without
changing database lifecycle or assertions.

**Architecture:** Keep SQLite URL/options knowledge in the existing
`server::test_support` in-crate unit-test seam. Keep command-specific
`StorageArgs` assembly private to `commands` tests, and leave opening,
migrations, path assertions, and `TempDir` ownership at each call site. Before
code moves, turn every other qualified repository-audit cluster into one
separately triaged issue.

**Tech Stack:** Rust, Tokio, SQLx SQLite, cargo-nextest, GitHub Issues and
Projects.

**Spec:**
[`2026-08-13-issue-35-sqlite-command-test-storage-spec.md`](2026-08-13-issue-35-sqlite-command-test-storage-spec.md)

## Review header

**Scope — in:** file and triage 28 focused boilerplate-audit follow-ups; add
`server::test_support::sqlite_db_options`; reuse it from `migrated_sqlite_db`;
add private `commands::tests::sqlite_storage_args`; migrate exactly five SQLite
unit-test setups; verify lifecycle preservation and the repository gate.

**Scope — out:** implementing any follow-up; production behavior; command or CLI
interfaces; backend-neutral fixtures; integration-harness changes; initialized
fixture objects; new behavioral tests; shallow or intentional repetition.

**Tasks:**

1. Search, file/reuse, read back, and fully triage 28 focused audit issues.
2. Add the two narrow SQLite constructors, migrate five tests, verify, and
   commit the complete issue-cycle change.

**Key risks/decisions:**

- Construction is shared; initialization is not. Three tests still open the
  database before their subject, while two still prove the file is absent.
- `server/src/test_support.rs` remains the SQLite-only in-crate seam. The
  dual-backend `storage::test_support` harness does not acquire server CLI
  types.
- No new test is justified: the observable contract is exact preservation of
  five existing tests. Run them before and after the refactor.
- Issue filing is part of spec acceptance, not permission to implement those
  issues on this branch. Exact owners are reused; partial overlaps remain
  separate and receive coordination notes only.

## Global Constraints

- Follow ADR-0016: test subjects receive only the narrow storage handles they
  need; do not return `AppState` from the new helper.
- Follow ADR-0033 and ADR-0067: do not move this SQLite-only in-crate helper
  into the backend-parametric integration harness.
- `sqlite_db_options(dir: &Path)` owns only `jaunder.db` path selection, SQLite
  URL formatting, and `DbConnectOptions` parsing.
- `sqlite_storage_args(temp: &TempDir)` owns only `StorageArgs` assembly.
- Preserve all five test names, inputs, assertions, initialization order, and
  explicit `TempDir` lifetime.
- The two `prepare_server` tests retain explicit `jaunder.db` path assertions;
  the helper must not create or open the database.
- Run commands through `devtool run --`; no `npx`, package-manager wrappers, or
  `nix develop -c`.
- Before the code commit, invoke `jaunder-commit` and pass
  `devtool run -- cargo xtask check`.
- Conventional Commit; no `Co-Authored-By` trailer.

---

### Task 1: File and triage the remaining boilerplate-audit findings

**Files:** none — GitHub tracker changes only. The completion mapping is
recorded in this plan and committed with Task 2.

**Interfaces:**

- Consumes: spec D3 and the audited issue contracts below.
- Produces: exactly 28 open owner issues in `jaunder-org/jaunder`, one per table
  row. Each issue is a `Task` with label `dx`, milestone `Code quality ratchet`
  (#9), project `Jaunder Backlog` (#1), Status `Todo`, and Priority P3 unless
  read-back evidence establishes a real reason to vary the metadata.
- Produces no repository code and implements none of the follow-ups.

Each owner issue body is deterministic:

1. `## Audit finding`, followed verbatim by that row's evidence/seam cell;
2. `## Acceptance`, followed by these four fixed requirements:
   - every audited caller named in the finding uses the selected seam, except
     the exclusions named in the finding;
   - existing observable behavior and public/test interfaces are unchanged;
   - the affected focused tests pass;
   - `cargo xtask check` passes;
3. `## Coordination`, stating `No prerequisite` when the row names no existing
   issue, or listing the row's issue/ADR references and explicitly
   distinguishing coordination from blockage; and
4. the footer
   `_Tracked from the repository-wide boilerplate audit performed for #35._`

The evidence/seam cell is the complete issue contract: exact current paths or
symbols, repetition count, minimal seam, preserved behavior, exclusions, and
known overlaps. Do not broaden it during issue creation. Where a row names an
existing issue only for coordination, the created owner issue remains separate.

| #   | Exact title                                                     | Evidence, seam, exclusions, coordination                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| --- | --------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `test(server): reuse response body decoding helper`             | Twelve manual text translations in `server/tests/projector/{listing,permalink,tags}.rs` and `server/tests/web/router.rs` repeat Axum body collection and UTF-8 decoding. Replace them with existing `server/tests/helpers/http.rs::body_string`; preserve response/status assertions, `permalink.rs`'s separate byte-identity comparison, and all other raw-byte assertions. No new helper.                                                                                                                                                                                             |
| 2   | `test(server): centralize CLI command envelopes`                | Twenty-nine non-verbose `Cli { command: Some(...) }` wrappers and repeated initialization prerequisite in `server/src/main.rs` tests. Add only local constructors that preserve each command, expected exit, and initialized/uninitialized setup. Do not change clap or production dispatch.                                                                                                                                                                                                                                                                                            |
| 3   | `test(server): package initialized command environments`        | Nineteen non-init tests in `server/tests/misc/commands.rs` call `storage_args`, immediately run `cmd_init`, and separately retain `TempDir` plus an optional `PostgresDbGuard`. Use private `InitializedCommandEnv { args, base, _postgres }`; keep the four `cmd_init` contract tests at the start of that module on raw `storage_args`. Coordinate with #977 only if production command files move.                                                                                                                                                                                   |
| 4   | `test(storage): centralize local subscription fixtures`         | Fourteen fixture-only subscriptions: `server/tests/misc/backup_fixture.rs::populate_backup_fixture` (1), five audience tests in `server/tests/web/audiences.rs`, two visibility tests in `server/tests/web/posts/visibility.rs`, and six setups in `server/tests/storage/mod.rs` (including two in `resolution_matrix`). Add `storage::test_support::seed_local_subscription(&AppState, author, subscriber) -> SubscriptionId`; subscription contract tests stay explicit. #750 owns the production reference type; #950/#963 are structural coordination only.                         |
| 5   | `test(server): use typed post request fixtures`                 | Fifteen valid manual create/update JSON envelopes: six in `server/tests/feed/feed_events_hook.rs`, two in `server/tests/web/posts/create.rs`, two in `server/tests/web/posts/listing.rs`, and five in `server/tests/web/posts/update.rs`. Add a defaulted `PostInputs` constructor and make request wrappers accept typed inputs. Malformed tag, count, format, body, and cursor payloads remain raw JSON. #976 owns AtomPub file splitting, not this server-function seam.                                                                                                             |
| 6   | `refactor(storage): centralize typed SMTP config reads`         | Six `SiteConfigStore::get_smtp_config` typed SELECT/bind/fetch/key-labelled decode copies in `storage/src/site_config.rs`. Add a private generic `read_typed` method with the required SQLx row bound; required/optional/default policy stays at each caller. Preserve ADR-0071 decode-through-type behavior.                                                                                                                                                                                                                                                                           |
| 7   | `refactor(storage): centralize corrupt URL purge policy`        | Two state-mutating recovery paths in `get_feeds_websub_hub_url` and `get_identity`: empty means unset; parse valid tagged URL; warn, delete corrupt config, and return `None`. Add a private typed reader/purger keyed by `SiteConfigKey`; never log the stored value.                                                                                                                                                                                                                                                                                                                  |
| 8   | `test(server): reuse the shared media fixture`                  | Four eight-field `MediaRecord` fixtures in `server/tests/web/web_media.rs` repeat upload defaults and `create_media` setup. Reuse `storage::test_support::seed_media` and its returned `MediaRef`; do not add a general record builder unless an observed field requires it. #963 owns test-support splitting only.                                                                                                                                                                                                                                                                     |
| 9   | `test(storage): centralize feed cache fixtures`                 | Seven constructors: three in `server/tests/feed/feed_handlers.rs`, one in `server/tests/feed/feed_worker.rs`, `server/tests/storage/mod.rs::feed_urls_needing_catchup_returns_stale_feeds`, `storage/src/posts.rs::feed_urls_needing_catchup_skips_a_row_whose_feed_url_no_longer_parses`, and `storage/src/feed_cache.rs::tests::sample`. Add an opinionated `SeedFeedCache` with valid typed defaults and overrides only for currently varied fields. Storage-contract inputs stay explicit. Coordinate with #950/#963; neither owns this fixture policy.                             |
| 10  | `refactor(web): replace revalidation counters with Invalidator` | Raw counters in `HomePage`, `CockpitPage`, and `MediaPage` drive four resources. Adopt existing `Invalidator::{new,notify,track}` and `client::reactive::{resource,action}`; Media keeps one shared invalidator and success-only delete invalidation per ADR-0060. Exclude posts work owned by #974; #363 is structural coordination only.                                                                                                                                                                                                                                              |
| 11  | `refactor(common): centralize rendered post translation`        | `web/src/posts/server.rs::{rendered_post,authored_post}` duplicate `PostRecord -> RenderedPost`. Keep `rendered_post`'s published-only guard, derive `is_author`, and delegate to `authored_post(...).post`. Preserve permalink and timestamp mappings. #804 may absorb the change but remains open for its other wire/page decisions; coordinate with #748.                                                                                                                                                                                                                            |
| 12  | `refactor(web): centralize tag summary conversion`              | Four production/test `TagLabel -> TagSummary` constructions in tags API and input logic/state repeat `slug = display.slug()` while preserving display case. Add `impl From<TagLabel> for TagSummary`; preserve the current list endpoint display. #694/#697 are not exact owners.                                                                                                                                                                                                                                                                                                       |
| 13  | `test(web): use Leptos owner closure lifecycle`                 | Twenty-one synchronous sites: six local wrappers in `web/src/{cockpit/state,media/upload_state,posts/compose_state,posts/page_state,tags/input_state,timeline/state}.rs`, thirteen prologues in `web/src/forms/field.rs`, and two in `web/src/reactive/invalidator.rs`. Replace them with existing `Owner::with`; exclude async tests and helpers returning an owner, which must outlive future polling. No repository helper.                                                                                                                                                          |
| 14  | `test(common): centralize feed renderer fixtures`               | Seven constructors: `common/src/feed/atom.rs`, `rss.rs`, and `json.rs` each define one `FeedMetadata` and one `FeedItem` fixture; `common/src/feed/metadata.rs` defines the seventh `FeedItem`. Add typed constructors in `common/src/feed/test_support.rs`; keep format-specific assertions and per-format URL/title/summary/tag variations local. Coordinate typed JSON fixture work with #694; #689/#832 retain substantive behavior scopes.                                                                                                                                         |
| 15  | `refactor(common): complete text validation helper adoption`    | Five `FromStr` implementations repeat trim/non-empty in `common/src/{audience,backup,bio,display_name,post_summary}.rs`; the final three also repeat post-trim Unicode-scalar limits. Adopt existing `common::text::non_empty` and add only a private `bounded_non_empty` returning an optional borrowed string slice if needed. Preserve each newtype's error type/message, trim order, inclusive maximum, and allocation ownership. #564/#837 remain separate behavior work.                                                                                                          |
| 16  | `refactor(host): centralize session cookie attributes`          | `host/src/auth.rs::{session_cookie_header,clear_session_cookie_header}` duplicate cookie name and `HttpOnly; SameSite=Lax; Path=/; [Secure]`. Use one private typed set/clear formatter; preserve exact ordering, `Max-Age=0` only on clear, and `RawToken` injection. #677 owns response plumbing and cannot be closed by this work.                                                                                                                                                                                                                                                   |
| 17  | `refactor(xtask): centralize Nix producer gate lifecycle`       | `xtask/src/steps/nix.rs::{coverage,doctests}` duplicate producer → consumer gate → sentinel detail handling. Add one private lifecycle helper while preserving step names/order, omission of opaque failed gate steps, fallback detail, and coverage-only success post-processing. High risk; ADR-0028. #806 is coordination only.                                                                                                                                                                                                                                                      |
| 18  | `test(xtask): reuse scratch Git repository fixtures`            | `xtask/src/adr.rs` tests `renumber_bumps_newcomer_and_rewrites_refs`, `renumber_syncs_the_readme_table`, and `renumber_assigns_distinct_numbers_to_multiple_newcomers` manually create, initialize, and configure scratch repos despite `xtask::test_support::temp_repo`. Reuse that seam with process-unique tags; preserve `main`, fixed identity, GIT-variable scrubbing, and domain-specific commits. Do not migrate non-Git `adr_readme` scratch trees. Coordinate file movement with #989.                                                                                        |
| 19  | `refactor(macros): centralize public unit error emission`       | `num_newtype::error_type` and `text_enum::error_type` emit the same public unit struct/derive/Display/Error policy. Add a shared `macros/src/lib.rs` codegen helper while preserving caller docs/messages, public token shape, paths, derives, and ADR-0091's constructible unit expression. #913/#709 are not exact owners.                                                                                                                                                                                                                                                            |
| 20  | `refactor(macros): centralize fallible string serde emission`   | `str_newtype::serde_impls` and `text_enum::serde_impls` repeat borrowed string serialization plus owned-`String` → `FromStr` decoding. Share impl emission while preserving generic lifetime ordering, zero-copy serialization, owned form decode, validation chokepoint, and error mapping. Exclude numeric/infallible paths. #857/#913/#709 remain separate.                                                                                                                                                                                                                          |
| 21  | `ci: centralize GitHub Actions bootstrap`                       | Four copies across `.github/workflows/{ci,mutants}.yml`. Add `.github/actions/setup-ci/action.yml`, invoked only after checkout, with explicit GitHub/Cachix token inputs. Preserve exact action pins, cache key/restore prefix, push filter, and job-specific runner/policy. #629 is unrelated OOM work.                                                                                                                                                                                                                                                                               |
| 22  | `refactor(nix): centralize backend e2e check builder`           | `flake.nix::{mkE2eSqliteCheck,mkE2ePostgresCheck}` duplicate VM, OTel, service, boot, copy, seed, and run lifecycle. Replace them with one backend-aware builder while preserving ADR-0034 matrix identity, resources, ordering, diagnostics, seed DB, workers, and derivation inputs. Coordinate with #985/#828/#802; do not change their behavior scopes.                                                                                                                                                                                                                             |
| 23  | `test(e2e): reuse the media upload request helper`              | `end2end/tests/media.spec.ts` has four success-shaped request implementations: three inline in `authenticated user can upload and access media`, `a filename needing percent-encoding uploads and serves`, and `the media row decodes its label but not its delete key`, plus `uploadMedia` inside the delete-guard block. Move `uploadMedia` to spec scope and route those three inline tests plus its three existing delete-guard callers through it. Keep `unauthenticated upload is rejected` raw; keep served-file, canonical-name, cache, delete, and reference assertions local. |
| 24  | `test(e2e): centralize media library navigation`                | Three setup sites in `end2end/tests/media.spec.ts` repeat click/URL/readiness sequencing: `the media row decodes its label but not its delete key`, `media manage page is reachable via nav link`, and `attemptDelete` used by the three delete-guard tests. Add spec-local `openMediaLibrary(page)` using `navigateInApp` with `/media` and the established content barrier; callers retain initial `goto`. Preserve ADR-0111 already-satisfied barrier discipline and use only from current non-media states.                                                                         |
| 25  | `test(e2e): centralize admin settings re-entry`                 | `admin-site.spec.ts::reenterSiteSettings` and `backup.spec.ts::reenterBackupSettings` duplicate the leave/remount/return lifecycle and mirrored route/link/readiness map. Add a target-typed helper; preserve distinct browser contexts, routes, and semantically required intermediate navigation.                                                                                                                                                                                                                                                                                     |
| 26  | `test(e2e): centralize feed alternate-link translation`         | Four materializations plus one live poll in `feeds.spec.ts` repeat alternate-link DOM extraction. Add typed `readAlternateLinks(page)` in existing `end2end/tests/feeds.ts`; preserve browser-resolved absolute URLs, keep count/content assertions local, and leave the live predicate live.                                                                                                                                                                                                                                                                                           |
| 27  | `test(elisp): centralize integration temp directory lifecycle`  | Four tests in `elisp/test/jaunder-media-integration.el` duplicate temp-directory creation, `unwind-protect`, and recursive cleanup. Add a macro in `jaunder-integration-helper.el` owning only directory lifetime; fixture files/assertions and live-server lifetime remain local. Preserve cleanup on signal and current cleanup error behavior. Coordinate only with structural #992.                                                                                                                                                                                                 |
| 28  | `test(e2e): reuse the username fixture constructor`             | Inline uniqueness formulas in `auth.spec.ts` and `invite.spec.ts` duplicate existing `helpers.ts::generateUsername`. Import and call it with `newuser`/`invitee`; preserve the distinct UI registration flows. No new helper.                                                                                                                                                                                                                                                                                                                                                           |

- [x] **Step 1: Search all issue states before creating anything**

Use GitHub issue search scoped to `repo:jaunder-org/jaunder is:issue` for each
exact title's core phrase plus its primary path or symbol. Search open and
closed states. Batch independent searches, but map every table row explicitly.

Expected: a 28-row owner map. If an exact open owner appeared after the audit,
reuse it and normalize its scope/metadata. If an exact owner is closed while the
audited cluster still exists in this branch, reopen and update it rather than
creating a duplicate. A closed issue claiming delivery while the cluster is
still present is stale tracker state, not permission to omit the row. If the
cluster disappeared because the branch was updated, stop and reconcile the
approved spec before continuing. Partial overlaps named in the table are not
exact owners.

Do not duplicate exact-owner findings already tracked by #700, #841, #913, or
#914, or structural splits #950, #959, #963, #976, #977, #978, #979, #985, and
#992. Those are outside the 28 table rows except where a row explicitly records
coordination.

- [x] **Step 2: Create or normalize and read back every owner issue**

For each missing row, call GitHub `issue_write` once with owner `jaunder-org`,
repository `jaunder`, method `create`, type `Task`, label `dx`, milestone `9`,
the row's exact title, and the deterministic body assembled from that row as
specified above.

For a reused exact owner, update only the missing scope/acceptance/metadata; do
not overwrite delivered history or silently add unrelated work. Reopen it when
the current cluster remains unresolved. Immediately call
`issue_read(method=get)` for every owner and verify title, evidence/count,
minimal seam, exclusions, coordination, type, label, milestone, and open state.

Expected: exactly 28 distinct open issue numbers mapped one-to-one to the table.

- [x] **Step 3: Add every owner to Jaunder Backlog and finish triage**

Use the concrete issue URLs and numbers returned by Step 2. First resolve the
project and current field/option IDs:

```bash
devtool run -- gh project view 1 --owner jaunder-org --format json
devtool run -- gh project field-list 1 --owner jaunder-org --format json
```

For each issue, read its current project item using the concrete issue number:

```bash
devtool run -- gh api graphql -F number=NUMBER -f 'query=query($number:Int!){repository(owner:"jaunder-org",name:"jaunder"){issue(number:$number){projectItems(first:10){nodes{id project{number} status:fieldValueByName(name:"Status"){... on ProjectV2ItemFieldSingleSelectValue{name}} priority:fieldValueByName(name:"Priority"){... on ProjectV2ItemFieldSingleSelectValue{name}}}}}}}'
```

Select the node whose project number is `1`. If none exists, add the issue and
retain the returned item ID:

```bash
devtool run -- gh project item-add 1 --owner jaunder-org --url https://github.com/jaunder-org/jaunder/issues/NUMBER --format json
```

Set both fields on that concrete item:

```bash
devtool run -- gh project item-edit --project-id PROJECT_ID --id ITEM_ID --field-id STATUS_FIELD_ID --single-select-option-id TODO_OPTION_ID
devtool run -- gh project item-edit --project-id PROJECT_ID --id ITEM_ID --field-id PRIORITY_FIELD_ID --single-select-option-id P3_OPTION_ID
```

Substitute the concrete IDs returned by the immediately preceding project
operations. Do not copy IDs from an archived plan. Run the GraphQL read again
after both mutations and verify the project #1 node's Status is `Todo` and
Priority is `P3`.

Add `blocked-by` only for a proven functional prerequisite. Treat file movement
and overlapping future edits as coordination, not blockage.

Expected: every row reads back as open `Task`, label `dx`, milestone #9, Backlog
Status `Todo`, and Priority `P3`, except any evidence-backed deviation recorded
in the completion note.

- [x] **Step 4: Record the issue map in this task**

Append a dated completion note here mapping all 28 table numbers/titles to issue
numbers. Record reused owners and any metadata deviation with its evidence.
Confirm all owner issues exist before Task 2's first code edit.

Completed 2026-08-14. Exact owner map:

| Audit row | Owner                                                                   |
| --------- | ----------------------------------------------------------------------- |
| 1         | #1010 — `test(server): reuse response body decoding helper`             |
| 2         | #1005 — `test(server): centralize CLI command envelopes`                |
| 3         | #1025 — `test(server): package initialized command environments`        |
| 4         | #1026 — `test(storage): centralize local subscription fixtures`         |
| 5         | #1000 — `test(server): use typed post request fixtures`                 |
| 6         | #999 — `refactor(storage): centralize typed SMTP config reads`          |
| 7         | #1027 — `refactor(storage): centralize corrupt URL purge policy`        |
| 8         | #1028 — `test(server): reuse the shared media fixture`                  |
| 9         | #1029 — `test(storage): centralize feed cache fixtures`                 |
| 10        | #1006 — `refactor(web): replace revalidation counters with Invalidator` |
| 11        | #1030 — `refactor(common): centralize rendered post translation`        |
| 12        | #1003 — `refactor(web): centralize tag summary conversion`              |
| 13        | #1031 — `test(web): use Leptos owner closure lifecycle`                 |
| 14        | #1002 — `test(common): centralize feed renderer fixtures`               |
| 15        | #1032 — `refactor(common): complete text validation helper adoption`    |
| 16        | #997 — `refactor(host): centralize session cookie attributes`           |
| 17        | #1014 — `refactor(xtask): centralize Nix producer gate lifecycle`       |
| 18        | #1033 — `test(xtask): reuse scratch Git repository fixtures`            |
| 19        | #1034 — `refactor(macros): centralize public unit error emission`       |
| 20        | #1013 — `refactor(macros): centralize fallible string serde emission`   |
| 21        | #1035 — `ci: centralize GitHub Actions bootstrap`                       |
| 22        | #1017 — `refactor(nix): centralize backend e2e check builder`           |
| 23        | #1036 — `test(e2e): reuse the media upload request helper`              |
| 24        | #1020 — `test(e2e): centralize media library navigation`                |
| 25        | #1023 — `test(e2e): centralize admin settings re-entry`                 |
| 26        | #1037 — `test(e2e): centralize feed alternate-link translation`         |
| 27        | #1024 — `test(elisp): centralize integration temp directory lifecycle`  |
| 28        | #1022 — `test(e2e): reuse the username fixture constructor`             |

All 28 were newly created. Read-back verified every issue is open, type `Task`,
label `dx`, milestone #9, Backlog Status `Todo`, and Priority `P3`. No metadata
deviations or functional prerequisites were found; no `blocked-by` links were
added.

This tracker-only task makes no repository commit.

---

### Task 2: Centralize SQLite command-test storage construction

**Files:**

- Modify: `server/src/test_support.rs:12-43`
- Modify: `server/src/commands.rs:772-805,997-1177`
- Modify:
  `docs/superpowers/plans/2026-08-13-issue-35-sqlite-command-test-storage.md`
- Include unchanged approved spec in commit:
  `docs/superpowers/specs/2026-08-13-issue-35-sqlite-command-test-storage-spec.md`

**Interfaces:**

- Consumes: existing `storage::DbConnectOptions`, `StorageArgs`,
  `tempfile::TempDir`, and `migrated_sqlite_pool(&Path)`.
- Produces:
  `pub(crate) fn server::test_support::sqlite_db_options(dir: &Path) -> DbConnectOptions`.
- Produces private test helper:
  `fn commands::tests::sqlite_storage_args(temp: &TempDir) -> StorageArgs`.
- Changes no production/public interface.

- [x] **Step 1: Establish the behavior-preservation baseline**

Run:

```bash
devtool run -- cargo nextest run -p jaunder commands::tests
```

Expected: PASS, including these five tests:

```text
commands::tests::cmd_site_config_set_upserts_and_get_and_list_read_back
commands::tests::cmd_user_invite_creates_invite_expiring_in_the_future
commands::tests::cmd_user_invite_with_base_url_configured_prints_link
commands::tests::prepare_server_auto_initializes_in_dev_mode
commands::tests::prepare_server_refuses_on_live_holder_before_db_open
```

This is a behavior-neutral refactor with complete existing coverage. Do not add
a test that merely duplicates these assertions or inspects source text.

- [x] **Step 2: Add the shared SQLite options constructor**

In `server/src/test_support.rs`, add this exact interface beside
`migrated_sqlite_pool`:

```rust
/// Connect options for `jaunder.db` inside `dir`.
pub(crate) fn sqlite_db_options(dir: &Path) -> DbConnectOptions {
    format!("sqlite:{}", dir.join("jaunder.db").display())
        .parse()
        .expect("db options")
}
```

Change `migrated_sqlite_db` to call `sqlite_db_options(dir)` rather than format
and parse its own URL. It still computes the `jaunder.db` path needed by
`migrated_sqlite_pool`, awaits migrations, and returns the same
`(DbConnectOptions, SqlitePool)` tuple. Do not make the new helper async or open
the database.

- [x] **Step 3: Add the command-test constructor and migrate exactly five
      sites**

Remove the now-unused `use storage::DbConnectOptions;` import. In the
`commands.rs` test module, after `site_config_args`, add:

```rust
fn sqlite_storage_args(temp: &TempDir) -> StorageArgs {
    StorageArgs {
        storage_path: temp.path().to_path_buf(),
        db: crate::test_support::sqlite_db_options(temp.path()),
    }
}
```

For each of the five named tests, retain `let temp = TempDir::new()...` and
replace only the repeated URL/options/`StorageArgs` construction with
`sqlite_storage_args(&temp)`.

For the three initialized tests, pass `&storage_args.db` to
`storage::open_database` before the command call and retain the same `AppState`
where assertions use it. For the two `prepare_server` tests, retain
`let db_path = temp.path().join("jaunder.db")`, the before/after nonexistence
assertions, and the planted runtime file. The helper call must occur without
opening or creating the database.

Expected source invariant: no manual `DbConnectOptions` parse remains in these
five tests; the three open calls and two explicit `db_path` assertions remain.
Do not migrate `site_config_args`, whose dual-backend initialized lifecycle is a
different contract.

- [x] **Step 4: Run the affected command tests**

Run:

```bash
devtool run -- cargo nextest run -p jaunder commands::tests
```

Expected: PASS with the same five test names and observable assertions as the
baseline. A failure in either `prepare_server` test means the helper
accidentally changed initialization order; fix the source rather than changing
the test.

- [x] **Step 5: Run the per-commit gate and inspect the complete diff**

Invoke `jaunder-commit`, then run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS — formatting, static checks, clippy, instrumented SQLite and
PostgreSQL coverage suites, and coverage gate all green. If the gate
auto-formats and restages files, rerun it until the checked tree is exactly the
tree to commit.

Inspect the complete branch diff against
`issue-35-factor-boilerplate-helpers-base`. Confirm it contains only:

- the approved spec and this plan, including Task 1's 28-issue completion map;
- `sqlite_db_options` plus `migrated_sqlite_db` delegation;
- `sqlite_storage_args` plus the five mechanical migrations; and
- no production behavior, test assertion, initialization-order, or unrelated
  audit-cluster change.

- [x] **Step 6: Commit the checked issue-cycle change**

Stage the complete checked change, including the approved spec and plan. Verify
the staged diff, then commit without a pathspec:

```bash
git commit -m "refactor(server): centralize SQLite command test setup (#35)"
```

Expected: one clean commit containing the tracker evidence and behavior-neutral
helper extraction, with no `Co-Authored-By` trailer.
