# Web Post Integration-Test Modules — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the 2,800-line `web_posts.rs` test module with focused
`web::posts` endpoint-family modules while preserving all 64 test functions and
146 registered backend/case instances.

**Architecture:** Keep the existing single server integration-test binary and
replace one leaf module with a private directory module. An assembly-only
`posts/mod.rs` declares six test modules plus a narrowly visible `fixtures`
module; test bodies move mechanically by observable endpoint family. The same
commit updates live references to the clean `web::posts::<concern>::<name>`
path.

**Tech Stack:** Rust, `rstest`/`rstest_reuse`, Axum server-function integration
tests, cargo-nextest, GitHub Issues.

**Spec:**
[`2026-08-13-issue-19-web-post-test-modules.md`](../specs/2026-08-13-issue-19-web-post-test-modules.md)

## Review header

**Scope — in:** file 17 focused audit follow-ups; split
`server/tests/web/web_posts.rs` into `server/tests/web/posts/`; change the test
module identity to `web::posts`; update the live source/doc references; verify
all test functions and registered cases survive on SQLite and PostgreSQL.

**Scope — out:** production behavior, server-function or wire changes, test
renames/assertion rewrites, a compatibility module, a second integration-test
target, the 17 follow-up implementations, any size gate.

**Tasks:**

1. File and triage 17 focused cohesion-audit follow-up issues.
2. Atomically move the web-post tests into endpoint-family modules, update live
   documentation, verify the full population and both backends, then commit.

**Key risks/decisions:**

- This is one atomic code commit. A transitional `legacy.rs` would be throwaway
  scaffold, and a partially split `mod.rs` would violate ADR-0128; neither is
  introduced.
- `web::posts` intentionally replaces `web::web_posts`. There is no alias or
  compatibility module.
- Read ownership is assertion-based: representation/parsing/existence belongs to
  `read`; viewer-dependent results belong to `visibility`.
- Baseline measured before planning: 64 `#[tokio::test]` functions expand to
  exactly 146 registered nextest instances under `web::web_posts`.

## Global Constraints

- Preserve the single `jaunder::integration` test binary required by ADR-0067.
- `server/tests/web/posts/mod.rs` contains only module documentation,
  attributes, and `mod`/`use` items permitted by ADR-0128.
- Every concern file using `#[apply(backends)]` or `#[apply(backends_matrix)]`
  imports `rstest::*`, `rstest_reuse::*`, and the applicable bare template from
  `storage::test_support` (ADR-0124).
- Preserve all 64 function names, every `#[case]`, request input, and assertion;
  only module paths and the imports/visibility required by movement change.
- Shared helper visibility is at most `pub(super)`; concern-local helpers remain
  private.
- Run commands through `devtool run --`; no `npx`, package-manager wrappers, or
  `nix develop -c`.
- Before the code commit, invoke `jaunder-commit` and pass
  `devtool run -- cargo xtask check`.
- Conventional Commit, no `Co-Authored-By` trailer.

---

### Task 1: File the remaining cohesion-audit findings

**Files:** none — GitHub tracker changes only.

**Interfaces:**

- Consumes: the audited path/seam/constraint table below.
- Produces: 17 open `Task` issues in `jaunder-org/jaunder`, each fully triaged
  under the repository's mandatory issue workflow: milestone
  `Code quality ratchet` (#9), project `Jaunder Backlog` (#1), Status `Todo`,
  Priority `P3`, and topic label `dx`. These fields are tracker hygiene required
  by `jaunder-issues`, not implementation scope added to #19.

All issues use this body shape, filled from the table:

```markdown
`<path>` is <line-count> lines and contains independently nameable concerns:
<seams>. Size triggered the audit; the split is warranted by separate reasons to
change, not by a hard line cap.

## Scope

The future issue's design interview chooses exact filenames. This issue records
the independently nameable seams and the load-bearing constraints from the
audit: <seams and constraints from the table>.

## Acceptance

- Each resulting file has one named responsibility.
- Existing public/test interfaces and observable behavior are unchanged.
- Tests move with the implementation or contract they prove.
- Any new `mod.rs` satisfies ADR-0128.
- The repository gate passes.

## Coordination

<overlapping issues/ADRs from the table; no dependency unless explicitly stated>

_Split out of the repository-wide cohesion audit performed for #19._
```

| Title                                                        | Path / seams / coordination                                                                                                                                                                                                                                       |
| ------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `test(atompub): split post protocol tests by contract`       | `server/tests/atompub/atompub_posts.rs` (1,789): collection/member reads, Entry mutation/mapping, ETag preconditions, visibility, scheduling, idempotency, media persistence. Keep AtomPub distinct from syndication; preserve ADR-0015/0023/0024/0089 contracts. |
| `refactor(server): split command implementations by concern` | `server/src/commands.rs` (1,229): storage bootstrap, account/operator commands, backup/restore, server lifecycle/composition, site config. Keep dispatch as the interface and `prepare_server` as ADR-0016's composition root.                                    |
| `refactor(server): split observability internals by concern` | `server/src/observability.rs` (1,142): OTel lifecycle, diagnostics/panic capture, slow spans, HTTP middleware. Keep trace/meter lifecycle unified (ADR-0011) and diagnostics with its panic hook (ADR-0049).                                                      |
| `refactor(storage): split post storage by concern`           | `storage/src/posts.rs` (4,354): model/errors, tags, cursors, generic store, visibility, syndication window. Preserve ADR-0019's trait/generic-store seam and ADR-0053 test homing; note overlaps #747/#797/#716/#754/#619/#700/#937.                              |
| `refactor(common): split media types by concern`             | `common/src/media.rs` (2,155): hashes, filename intake, stored-media addressing/layout, MIME policy, byte limits/wire values. Preserve ADR-0080/0084 filename-layout coupling; note #782.                                                                         |
| `test(storage): split shared test support by concern`        | `storage/src/test_support.rs` (1,890): backend environment/fault injection, PostgreSQL lifecycle, user/post/media fixtures, service helpers. Preserve ADR-0033/0053 template interface; coordinate #841/#874.                                                     |
| `refactor(atompub): split entry extensions from rendering`   | `common/src/atompub/entry.rs` (1,262): foreign-marker namespace mutation, Collection rendering, media Member rendering. Preserve ADR-0023/0089; coordinate #813 and the relocation in #855.                                                                       |
| `refactor(storage): split backup mechanics by concern`       | `storage/src/backup.rs` (1,233): orchestration, manifest/table format, archive mechanics, media-tree mirroring. Preserve ADR-0019/0054/0064; coordinate #725.                                                                                                     |
| `refactor(xtask): split identifier gate internals`           | `xtask/src/steps/ident_gate.rs` (1,508): owner/type resolution, syntax traversal, marker policy, gate/report orchestration. Preserve ADR-0085/0110; #894 may later replace traversal only.                                                                        |
| `build(nix): split flake outputs by concern`                 | `flake.nix` (1,434): NixOS module, packages/source filtering, checks/e2e, dev shell, output assembly. Keep `flake.nix` as assembly; preserve ADR-0028/0034/0052/0118 and avoid behavior changes owned by #802/#828/#893/#276.                                     |
| `refactor(xtask): split trace analysis reports`              | `xtask/src/traces/analyze.rs` (1,328): report model, browser/navigation/resource sections, span-tree analysis, file/project orchestration. Preserve ADR-0011/0096/0100; coordinate #831.                                                                          |
| `refactor(xtask): split ADR README checks by concern`        | `xtask/src/adr_readme.rs` (1,206): ADR discovery/format, README-table sync/parity, architecture-view parity. Preserve ADR-0036/0127; coordinate #742.                                                                                                             |
| `refactor(xtask): split CLI grammar from dispatch`           | `xtask/src/lib.rs` (1,201): clap grammar/metadata, command dispatch, run lifecycle/preconditions. Keep a small crate facade; preserve ADR-0028/0029/0034; coordinate #824.                                                                                        |
| `refactor(xtask): split ADR rewrite workflows`               | `xtask/src/adr.rs` (1,113): rewrite primitives, renumber workflow, promotion workflow. Preserve ADR-0036; coordinate with #742 because it may retire renumbering.                                                                                                 |
| `refactor(web): split post UI by surface`                    | `web/src/posts/component.rs` (1,643): display/card, audience picker, composers, permalink/editor, drafts, tag pages. Preserve ADR-0070 file-level wasm gating and keep pure logic host-tested; note #907/#908/#899/#896/#799/#783.                                |
| `test(e2e): split fixture infrastructure by responsibility`  | `end2end/tests/fixtures.ts` (1,013): performance/OTel capture, timeout policy, identity/mail/page provisioning. Keep one composed `test` export and preserve fixture dependency order, ADR-0039/0098/0111; note #887/#828.                                        |
| `test(elisp): split suite by package module`                 | `elisp/test/jaunder-test.el` (1,228): transport/auth, Org/datetime, config, Atom, media, publish, warnings/discovery. Mirror production `jaunder-<module>.el` names; the existing `test/*-test.el` runner requires no change; preserve ADR-0031.                  |

- [x] **Step 1: Search before creating each issue**

Use GitHub issue search scoped to `repo:jaunder-org/jaunder is:issue` (all
states) with the exact path and the words `split` or the concern names.
Expected: no existing issue owns these exact 17 splits. If an exact open owner
appeared after the spec audit, reuse and normalize that issue's metadata instead
of filing a duplicate. If a closed issue already delivered the split, verify the
current tree before omitting the now-resolved path; if it closed without
delivery, reopen/update it rather than creating a duplicate.

The existing owners that must **not** be duplicated are #950
(`server/tests/storage/mod.rs`), #855 (`common/src/render.rs`), and #776
(`sqlx_newtype_decode_check.rs`).

- [x] **Step 2: Create and read back every issue**

For each table row, call GitHub `issue_write` with:

```text
owner=jaunder-org, repo=jaunder, method=create, type=Task,
labels=[dx], milestone=9, title=<exact table title>, body=<completed template>
```

Immediately call `issue_read(method=get)` and confirm the title, full path,
seams, coordination notes, type, label, and milestone survived GitHub's body
processing.

Expected: 17 distinct open issue numbers, one per audited path.

- [x] **Step 3: Add each issue to Jaunder Backlog and set P3**

Add each issue URL to project #1. Resolve its project item through the issue's
`projectItems` query, then set Priority using field
`PVTSSF_lADOECw7os4BblPPzhWUx50` and option `0bba09bc` (`P3`). Leave Status at
`Todo`; none has a proven open prerequisite, so add no speculative blocked-by
links.

Expected: every issue reads back as `Task`, `dx`, milestone
`Code quality ratchet`, project Status `Todo`, Priority `P3`.

- [x] **Step 4: Record the 17 issue numbers in this task**

Add a short completion note under Task 1 mapping each path to its issue number.
This is the durable evidence for spec AC9. Confirm all GitHub creation
timestamps precede the first code-move commit in Task 2.

This tracker-only task makes no repository commit.

Completed 2026-08-13. All 17 owner issues are open `Task` issues with label
`dx`, milestone `Code quality ratchet`, Backlog Status `Todo`, and Priority
`P3`. Five exact owners filed concurrently were reused; the duplicate issues
#981, #982, #983, #987, and #990 were closed as duplicates and removed from
the Backlog. Owner-issue creation timestamps are 2026-08-13
20:13:35–20:14:14 UTC, before the first code-move commit:

- `server/tests/atompub/atompub_posts.rs` → #976
- `server/src/commands.rs` → #977
- `server/src/observability.rs` → #978
- `storage/src/posts.rs` → #979
- `common/src/media.rs` → #980
- `storage/src/test_support.rs` → #963
- `common/src/atompub/entry.rs` → #967
- `storage/src/backup.rs` → #959
- `xtask/src/steps/ident_gate.rs` → #984
- `flake.nix` → #985
- `xtask/src/traces/analyze.rs` → #986
- `xtask/src/adr_readme.rs` → #973
- `xtask/src/lib.rs` → #988
- `xtask/src/adr.rs` → #989
- `web/src/posts/component.rs` → #974
- `end2end/tests/fixtures.ts` → #991
- `elisp/test/jaunder-test.el` → #992

---

### Task 2: Split `web_posts.rs` into `web::posts` and verify the cutover

**Files:**

- Delete: `server/tests/web/web_posts.rs`
- Create: `server/tests/web/posts/mod.rs`
- Create: `server/tests/web/posts/fixtures.rs`
- Create: `server/tests/web/posts/create.rs`
- Create: `server/tests/web/posts/read.rs`
- Create: `server/tests/web/posts/update.rs`
- Create: `server/tests/web/posts/listing.rs`
- Create: `server/tests/web/posts/visibility.rs`
- Create: `server/tests/web/posts/audiences.rs`
- Modify: `server/tests/web/mod.rs`
- Modify: `common/src/render.rs:22-24`
- Modify: `CONTRIBUTING.md:464-479`
- Include: `docs/superpowers/specs/2026-08-13-issue-19-web-post-test-modules.md`
- Include: `docs/superpowers/plans/2026-08-13-issue-19-web-post-test-modules.md`

**Interfaces:**

- Consumes: the existing
  `crate::helpers::{create_session_for, create_user_and_session, post_form, post_json}`
  and `storage::test_support::{backends, backends_matrix, Backend, ...}`.
- Produces: private module path `web::posts::<concern>::<test>` and these exact
  sibling helper signatures in `fixtures.rs`:

```rust
pub(super) async fn create_post_json(
    state: &Arc<storage::AppState>,
    body: &str,
    format: &str,
    slug_override: Option<&str>,
    publish: bool,
    cookie: Option<&str>,
) -> (StatusCode, String);

pub(super) async fn update_post_json(
    state: &Arc<storage::AppState>,
    post_id: PostId,
    body: &str,
    format: &str,
    slug_override: Option<&str>,
    publish: bool,
    cookie: Option<&str>,
) -> (StatusCode, String);

pub(super) async fn get_post_form(
    state: &Arc<storage::AppState>,
    username: &str,
    year: i32,
    month: u32,
    day: u32,
    slug: &str,
    cookie: Option<&str>,
) -> (StatusCode, String);

pub(super) async fn list_drafts(
    state: &Arc<storage::AppState>,
    cursor: Option<PageCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String);

pub(super) async fn publish_post_form(
    state: &Arc<storage::AppState>,
    post_id: PostId,
    cookie: Option<&str>,
) -> (StatusCode, String);

pub(super) async fn list_user_posts(
    state: &Arc<storage::AppState>,
    username: &str,
    cursor: Option<PageCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String);

pub(super) async fn list_local_timeline(
    state: &Arc<storage::AppState>,
    cursor: Option<PageCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String);

pub(super) async fn list_home_feed(
    state: &Arc<storage::AppState>,
    cursor: Option<PageCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String);

pub(super) async fn login_and_state(
    backend: Backend,
) -> (TestBase, Arc<storage::AppState>, String);
```

Concern-local helpers move with their sole consumer module:

- `visibility.rs`: `get_post_preview_form`, `UnauthEndpoint`,
  `unauthenticated_request`, `create_targeted_post`, `timeline_slugs`;
- `update.rs`: `unpublish_post_form`, `delete_post_form`;
- `listing.rs`: `list_posts_by_tag`, `list_user_posts_by_tag`;
- `audiences.rs`: `author_with_cookie`, `user_with_cookie`.

- [ ] **Step 1: Confirm the pre-move population matches the plan contract**

Run:

```text
devtool run -- cargo nextest list -p jaunder web::web_posts
```

Expected: PASS, exactly 146 registered test-instance lines. Confirm the source
contains exactly 64 `#[tokio::test]` functions and that every function appears
once in the ownership map below. A mismatch means `main` moved after planning;
stop and reconcile the plan before moving code.

- [x] **Step 2: Create the assembly and fixture modules**

`server/tests/web/posts/mod.rs` contains only:

```rust
//! Web post server-function integration tests, grouped by endpoint family.

mod audiences;
mod create;
mod fixtures;
mod listing;
mod read;
mod update;
mod visibility;
```

Change `server/tests/web/mod.rs` from `mod web_posts;` to `mod posts;`.

Move only the nine cross-concern helpers listed in **Interfaces** to
`fixtures.rs`, retaining their bodies and intent comments byte-for-byte except
for required imports and `pub(super)`. Do not move any helper into the global
`crate::helpers` module.

- [x] **Step 3: Move all 64 tests exactly once by this ownership map**

Each bullet is the complete function population for that file. Move attached doc
comments, `#[apply]`, named `#[case]` attributes, and the full function body
together.

**`create.rs` — 13 functions**

```text
create_post_persists_rendered_published_post
create_post_retries_slug_conflicts_for_same_user_and_date
create_post_accepts_slug_override_and_saves_draft
create_post_accepts_titleless_body
create_post_extracts_markdown_heading_title
create_post_rejects
create_post_with_future_publish_at_is_scheduled
create_post_publish_without_publish_at_is_live_now
create_post_applies_tags_from_param
create_post_rejects_invalid_tag_token
create_post_rejects_more_than_25_tags
get_default_post_format_returns_markdown_by_default
set_default_post_format_persists_and_retrieves_markdown
```

**`read.rs` — 5 functions**

```text
get_post_returns_published_post
get_post_rejects_invalid_username
get_post_rejects_invalid_slug
get_post_returns_not_found_for_missing_post
get_post_carries_tags
```

**`update.rs` — 20 functions**

```text
update_post_updates_draft_content_and_slug
update_post_freezes_slug_when_published
update_post_publishes_draft
update_post_rejects_non_author
update_post_rejects
update_post_returns_not_found_for_missing_post
update_post_returns_not_found_for_deleted_post
publish_post_publishes_draft_and_returns_permalink
publish_post_rejects_non_author
publish_post_returns_not_found_for_missing_or_deleted_posts
delete_post_soft_deletes_post
delete_post_rejects_non_author
delete_post_rejects_unauthenticated
delete_post_returns_not_found_for_already_deleted_post
deleted_post_excluded_from_timelines_and_returns_404_at_permalink
unpublish_post_reverts_published_post_to_draft
unpublish_post_returns_the_draft_permalink
unpublish_post_rejects_non_author
update_post_applies_tag_set_diff
update_post_with_tags_unset_leaves_existing_tags_alone
```

**`listing.rs` — 14 functions**

```text
list_drafts_returns_current_user_drafts_with_cursor_pagination
list_drafts_surfaces_scheduled_with_marker_excludes_live
list_rejects_invalid_cursor_inputs
list_user_posts_returns_published_posts_with_cursor_pagination
list_user_posts_rejects_invalid_username
list_by_user_takes_a_nested_json_cursor_and_no_longer_the_flat_pair
timeline_page_two_uses_the_cursor_the_first_page_returned
list_local_timeline_returns_published_posts_with_cursor_pagination
list_home_feed_returns_authenticated_users_published_posts_only
list_user_posts_carries_tags_per_post
list_posts_by_tag_returns_matching_posts_from_all_users
list_posts_by_tag_returns_empty_for_unknown_tag
list_user_posts_by_tag_scopes_to_user
list_user_posts_by_tag_unknown_user_returns_not_found
```

**`visibility.rs` — 7 functions**

```text
endpoint_rejects_unauthenticated
get_post_returns_draft_to_author_only
get_post_preview_shows_draft_to_author_only
get_post_hides_drafts_from_guests
get_post_returns_scheduled_post_at_canonical_permalink_to_author
local_timeline_enforces_visibility_for_viewer
single_post_permalink_hides_subscribers_post_from_anonymous
```

**`audiences.rs` — 5 functions**

```text
default_audience_selection_returns_public_by_default
default_audience_selection_rejects_unauthenticated
post_audience_selection_returns_public_for_new_post
post_audience_selection_rejects_missing_post
post_audience_selection_rejects_non_owner
```

Partition the old import preamble per file. Every test file imports its own
`rstest::*`, `rstest_reuse::*`, and bare `backends` or `backends_matrix`
template. Use `super::fixtures::{...}` only for helpers actually called by that
file; let rustc/clippy expose any accidental wholesale import copy.

- [x] **Step 4: Update the two live path references**

In `common/src/render.rs`, change the rustdoc reference from
`server/tests/web/web_posts.rs` to `server/tests/web/posts/create.rs`.

In `CONTRIBUTING.md` §Targeted Rust tests, generalize the path grammar from the
fixed `<subsystem>::<file>::<name>` shape to nested module paths. Retain
`web::web_auth` as a shallow example, add
`web::posts::create::create_post_persists_rendered_published_post` as the nested
example, explain that concern submodules add segments, and use
`cargo nextest run -p jaunder web::posts::create` as the focused filter example.
Do not edit archived planning documents.

- [x] **Step 5: Format and compile-check the split**

Run:

```text
devtool run -- cargo xtask check --no-test
```

Expected: PASS. It may format the moved Rust/Markdown files. Fix unused/missing
imports and visibility at their source; do not add lint suppressions.

- [x] **Step 6: Prove exact population preservation**

Run:

```text
devtool run -- cargo nextest list -p jaunder web::posts
```

Expected: PASS and exactly 146 registered test-instance lines. Normalize the
old/new listings by removing only the prefix
`web::web_posts::`/`web::posts::<concern>::`; the remaining function,
named-case, and backend suffix sets must be identical.

Independently inspect the new sources: exactly 64 `#[tokio::test]` functions,
with per-file counts `13 + 5 + 20 + 14 + 7 + 5 = 64`; no duplicate or missing
name relative to Step 1.

Search only the live reference surfaces—`server/tests`, `common/src`, and
`CONTRIBUTING.md`—for `server/tests/web/web_posts.rs` and `web::web_posts`.
Expected: no matches. The approved spec/plan intentionally describe the old
path, and `docs/archive/` is frozen; neither is part of this stale-reference
check.

- [x] **Step 7: Run the changed test contract on both backends**

Run:

```text
devtool run -- devtool pg run -- cargo nextest run -p jaunder web::posts
```

Expected: PASS for all 146 registered instances, including both `case_1_sqlite`
and `case_2_postgres` expansions and every `backends_matrix` row.

- [x] **Step 8: Run the per-commit gate**

Invoke `jaunder-commit`, then run:

```text
devtool run -- cargo xtask check
```

Expected: PASS — formatting, static checks, clippy, the instrumented SQLite and
PostgreSQL suites, and coverage all green. If fix mode changes files, stage the
formatted result and rerun until the checked tree is exactly the tree to commit.

- [x] **Step 9: Commit the atomic cutover**

Stage the complete checked change, including the approved spec and this plan:

```text
git add CONTRIBUTING.md common/src/render.rs server/tests/web docs/superpowers/specs/2026-08-13-issue-19-web-post-test-modules.md docs/superpowers/plans/2026-08-13-issue-19-web-post-test-modules.md
git commit -m "refactor(server tests): split web post tests by endpoint (#19)"
```

Expected: one commit containing no production behavior change, no compatibility
module, and no unrelated audit implementation.

---

## Final verification before ship

`jaunder-ship` performs the whole-branch review and final machine gate. Because
this branch changes tests and documentation but no UI/runtime behavior, no new
e2e scenario is required. Before push, run
`devtool run -- cargo xtask validate --no-e2e`; the targeted dual-backend run
and full `cargo xtask check` above are the behavioral proof for the changed
contract.
