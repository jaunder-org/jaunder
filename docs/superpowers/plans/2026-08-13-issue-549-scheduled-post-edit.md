# Issue #549 Scheduled Post Edit Controls Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating an individual task to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the existing Post editor distinguish draft, scheduled, and live
Posts so an author can preserve, reschedule, or clear a schedule without losing
the loaded UTC instant.

**Architecture:** Extend the author-only preview response with the server's time
snapshot, then classify the loaded editor once against that snapshot. Keep the
schedule edit state and its mapping to the existing `PostInputs` wire fields in
a host-compiled `posts` leaf; the wasm component only renders that state and
dispatches its validated intent. The existing `update` endpoint and storage
`PublishUpdate` paths remain the mutation seam.

**Tech Stack:** Rust 2024, Leptos signals/components and server functions,
`chrono` through `common::time`, rstest dual-backend HTTP tests, Playwright
TypeScript e2e.

**Spec:**
[`docs/superpowers/specs/2026-08-13-issue-549-scheduled-post-edit.md`](../specs/2026-08-13-issue-549-scheduled-post-edit.md).
**ADRs:** ADR-0027 (amended by this issue), ADR-0055, ADR-0065, ADR-0070,
ADR-0072, ADR-0083, ADR-0105, ADR-0113, and the approved draft
[`docs/adr/drafts/current-publication-state-slug-freeze.md`](../../adr/drafts/current-publication-state-slug-freeze.md).

## Review header

**Scope.**

- _In:_ a server-captured edit-preview timestamp; immutable draft/scheduled/live
  classification; scheduled schedule prefill; exact untouched preservation;
  reschedule/backdate, clear-and-save, invalid-input blocking; current-state
  slug behavior; dual-backend and browser-matrix regression coverage.
- _Out:_ issue #15's scheduled-Post listing/management surface, a new server or
  storage mutation, stale-edit rejection, a durable publication-history flag,
  AtomPub changes, and create-form behavior changes.

**Tasks.**

1. Add a timestamped author-edit preview DTO and dual-backend endpoint contract.
2. Add host-tested publication intent, local display formatting, and scheduled
   edit state.
3. Wire the three loaded states into `EditPostPage` and drive the full scheduled
   lifecycle end-to-end.
4. Run the full repository validation gate.

**Key risks / decisions.**

- `fetched_at` belongs only on the edit-preview DTO, not `AuthoredPost`:
  timeline and permalink payloads do not need it (ADR-0097's content-weight
  axis).
- Classification uses the response's `fetched_at`, never `Utc::now()` in wasm.
  The enum is created once from the fetched response, so a due-while-open Post
  does not switch branches under the author's hands.
- `PublicationIntent::{Draft, PublishNow, PublishAt}` makes the legal
  `publish`/`publish_at` wire combinations explicit. Live Save maps to
  `PublishNow`, preserving the stored timestamp through current storage
  semantics; scheduled Save maps only to `Draft` or `PublishAt`.
- Scheduled state keeps the original `UtcInstant` apart from its
  minute-precision local display string. Untouched Save returns the original
  value byte-for-value; parsing occurs only after an input/clear event.
- Keep create-form scheduling behavior unchanged: its existing invalid/empty
  local conversion still means no explicit timestamp. The stricter invalid
  non-empty rule is scoped to a loaded scheduled editor.
- The storage layer already freezes a slug based on the pre-update non-null
  `published_at`; no dialect or storage API change is planned. The e2e test
  proves pullback preserves that slug and a reopened draft may subsequently
  change it.

## Global constraints

- Follow `CONTRIBUTING.md`: backend parity, ADR-0016 dependency injection,
  host-tested pure logic, wasm/server boundary rules, and the verify ladder.
- No new dependency. Use the existing wasm-capable `chrono` dependency in
  `common`; no `chrono` type may cross a `#[server]` boundary—only `UtcInstant`.
- `web/src/posts/mod.rs` remains wiring/re-exports only. `component.rs` remains
  wasm-only and carries no internal `cfg` gates; pure branches live in an
  ungated host-compiled leaf.
- Do not change `PostInputs`, `update`, `PublishUpdate`, storage dialect files,
  AtomPub, or the create form's scheduling contract.
- Do not add `#[allow]`, `#[expect]`, fake host stubs, sleep/poll waits,
  full-page browser reloads, or `Co-Authored-By` trailers.
- For e2e, follow `jaunder-e2e`: shared helpers/selectors, element/URL settle
  signals, no `networkidle`, and no hand-written timeout.
- Before every kept commit, run `devtool run -- cargo xtask check`, stage the
  complete checked tree, then commit through `jaunder-commit`.

---

## Task 1: Timestamp the author-edit preview

**Files:**

- Modify: `web/src/posts/api.rs` — define the edit-only response and return it
  from `get_preview`.
- Modify: `web/src/posts/mod.rs` — re-export the response at
  `web::posts::EditPostPreview`.
- Modify: `web/src/posts/component.rs` — mechanically unwrap
  `EditPostPreview::post` at the existing editor call sites so this task remains
  buildable; do not classify scheduled/live yet.
- Modify/Test: `server/tests/web/web_posts.rs` — decode and assert the new
  dual-backend HTTP contract.

**Interfaces:**

- Produces:

  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
  pub struct EditPostPreview {
      pub post: AuthoredPost,
      pub fetched_at: UtcInstant,
  }

  #[macros::server]
  pub async fn get_preview(post_id: PostId) -> WebResult<EditPostPreview>;
  ```

- `fetched_at` is captured with `Utc::now()` after the owned, non-deleted Post
  is resolved and immediately before constructing the response. Authorization
  and indistinguishable-not-found behavior remain unchanged.
- Consumed immediately by `EditPostPage`: this task repoints its existing
  `seed_from`, slug, post-id, and `is_published` reads through the nested
  `AuthoredPost`; Task 3 adds `fetched_at` classification. No other endpoint
  gains the timestamp.

- [x] **Step 1: Change the dual-backend preview test to require the timestamped
      response**

  Import `EditPostPreview` and replace the author success assertion in
  `get_post_preview_shows_draft_to_author_only` with a typed contract (retain
  the stranger and anonymous 404 assertions):

  ```rust
  let before = chrono::Utc::now();
  let (status, body) =
      get_post_preview_form(&state, created.post_id, Some(&author_cookie)).await;
  let after = chrono::Utc::now();
  assert_eq!(status, StatusCode::OK, "author preview failed: {body}");

  let preview: EditPostPreview = serde_json::from_str(&body).unwrap();
  assert_eq!(preview.post.post.post_id, created.post_id);
  assert_eq!(preview.post.body.as_ref(), "# Preview Draft\n\ndraft\n");
  assert!(preview.post.post.published_at.is_none());
  assert!(preview.fetched_at.value() >= before);
  assert!(preview.fetched_at.value() <= after);
  ```

- [x] **Step 2: Run the focused test and verify RED**

  Run:
  `devtool run -- devtool pg run -- cargo nextest run -p jaunder web::web_posts::get_post_preview_shows_draft_to_author_only`

  Expected: FAIL to compile because `EditPostPreview` does not exist and
  `get_preview` still serializes a bare `AuthoredPost`.

- [x] **Step 3: Implement and re-export `EditPostPreview`**

  Keep `AuthoredPost` unchanged. Change only the `get_preview` success type and
  final construction; retain every existing auth/ownership/deletion branch.
  Update the endpoint doc comment to name its author-editor purpose and snapshot
  invariant. In `EditPostPage`, preserve current behavior while changing
  `fetched` reads to `fetched.post` and `fetched.post.post`; leave `fetched_at`
  deliberately unused until Task 3.

- [x] **Step 4: Run the focused test and verify GREEN**

  Run the Step 2 command. Expected: PASS for SQLite and PostgreSQL.

- [x] **Step 5: Run the per-commit gate and commit**

  Run: `devtool run -- cargo xtask check`. Expected: PASS.

  Stage `web/src/posts/api.rs`, `web/src/posts/mod.rs`,
  `web/src/posts/component.rs`, and `server/tests/web/web_posts.rs`; commit:

  ```text
  feat(web): timestamp post edit previews (#549)
  ```

---

## Task 2: Model scheduled edit intent outside wasm UI

**Files:**

- Modify/Test: `common/src/time.rs` — format a UTC instant as a browser-local
  minute-precision `datetime-local` value, with fixed-offset tests.
- Create/Test: `web/src/posts/edit_state.rs` — immutable loaded-state
  classification and reactive scheduled-field intent.
- Modify/Test: `web/src/posts/compose_state.rs` — consume a typed publication
  intent when constructing `PostInputs`.
- Modify: `web/src/posts/mod.rs` — wire and re-export the host-compiled leaf and
  its wasm-only consumers.
- Modify: `web/src/posts/component.rs` — mechanically migrate existing composer
  and editor call sites to `PublicationIntent` without changing rendered
  behavior yet.

**Interfaces:**

- Produces in `common::time`:

  ```rust
  #[must_use]
  pub fn local_datetime_from_utc(instant: UtcInstant) -> String;

  #[must_use]
  pub fn strict_utc_instant_from_local(local: &str) -> Option<UtcInstant>;
  ```

  Its private timezone-parametric core formats `%Y-%m-%dT%H:%M`; this is display
  text only, not the source for an untouched scheduled save.

- Produces in `posts::compose_state`:

  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum PublicationIntent {
      Draft,
      PublishNow,
      PublishAt(UtcInstant),
  }

  pub fn inputs(
      &self,
      body: PostBody,
      publication: PublicationIntent,
      slug_override: Option<Slug>,
  ) -> PostInputs;
  ```

  Mapping is exact: `Draft -> (false, None)`, `PublishNow -> (true, None)`, and
  `PublishAt(at) -> (true, Some(at))`.

- Produces in `posts::edit_state`:

  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum LoadedPublication {
      Draft,
      Scheduled(UtcInstant),
      Live,
  }

  #[must_use]
  pub fn loaded_publication(
      published_at: Option<UtcInstant>,
      fetched_at: UtcInstant,
  ) -> LoadedPublication;

  #[derive(Clone, Copy)]
  pub struct ScheduledEditState {
      pub value: RwSignal<String>,
      // original instant and edited signal remain private
  }

  #[derive(Clone, Copy)]
  pub enum EditPublicationState {
      Draft(RwSignal<String>),
      Scheduled(ScheduledEditState),
      Live,
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
  #[error("Enter a valid local date and time")]
  pub struct InvalidSchedule;

  impl EditPublicationState {
      pub fn from_loaded(
          loaded: LoadedPublication,
          draft_publish_at: RwSignal<String>,
      ) -> Self;
      pub fn loaded(self) -> LoadedPublication;
      pub fn scheduled(self) -> Option<ScheduledEditState>;
  }

  #[must_use]
  pub fn edit_submit_gate(
      body: Field<PostBody>,
      also_blocked: Signal<bool>,
      publication: EditPublicationState,
      on_submit: Callback<(PostBody, PublicationIntent)>,
  ) -> (Signal<bool>, Signal<Option<InvalidSchedule>>, Callback<bool>);
  ```

  `ScheduledEditState::publication()` returns `PublishAt(original)` while
  untouched, `PublicationIntent::Draft` after an edited empty value,
  `PublishAt(strict_utc_instant_from_local(value))` after a valid non-empty
  edit, and `InvalidSchedule` after any invalid non-empty edit. No `PublishNow`
  branch exists for a scheduled editor.

  `EditPublicationState` makes every loaded branch and its branch-specific
  signal structurally complete. `edit_submit_gate` is the ADR-0113 seam for all
  three branches: Draft preserves the create-form converter, Scheduled memoizes
  the strict exact-preserving result, and Live maps to `PublishNow`. Every
  branch combines `body.parsed()`, the caller's predicate, and the valid
  publication before returning one uniform disabled/error/click contract.
  `component.rs` dispatches that payload and never chooses or reconstructs a
  `PublicationIntent`.

- [x] **Step 1: Write failing time-format tests**

  Add fixed-offset tests beside the existing local-to-UTC tests:

  ```rust
  #[test]
  fn utc_instant_formats_as_local_datetime_control_value() {
      let instant = "2026-07-01T08:30:45Z".parse().unwrap();
      let tz = chrono::FixedOffset::east_opt(5 * 3600).unwrap();
      assert_eq!(local_datetime_from_utc_in(instant, &tz), "2026-07-01T13:30");
  }

  #[test]
  fn local_datetime_format_crosses_the_local_date_boundary() {
      let instant = "2026-07-02T04:00:00Z".parse().unwrap();
      let tz = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
      assert_eq!(local_datetime_from_utc_in(instant, &tz), "2026-07-01T23:00");
  }
  ```

- [x] **Step 2: Run the time tests and verify RED**

  Run:
  `devtool run -- cargo nextest run -p common time::tests::utc_instant_formats_as_local_datetime_control_value time::tests::local_datetime_format_crosses_the_local_date_boundary`

  Expected: FAIL to compile because the formatter functions do not exist.

- [x] **Step 3: Implement the formatter and verify GREEN**

  Convert `instant.value()` through the supplied `TimeZone`, format exactly to
  minutes, and let the public wrapper pass `chrono::Local`. Run the Step 2
  command. Expected: PASS.

  The coverage gate also requires the public wrapper's output to parse as the
  same minute-precision shape without assuming the host timezone.

- [x] **Step 4: Write failing publication-state tests**

  Create `edit_state.rs` tests under a fresh Leptos `Owner`. Cover every branch
  with these concrete contracts:

  ```rust
  #[test]
  fn loaded_publication_uses_the_server_snapshot() {
      let fetched_at = instant("2026-08-13T12:00:00Z");
      assert_eq!(loaded_publication(None, fetched_at), LoadedPublication::Draft);
      assert_eq!(
          loaded_publication(Some(instant("2026-08-13T12:00:01Z")), fetched_at),
          LoadedPublication::Scheduled(instant("2026-08-13T12:00:01Z")),
      );
      assert_eq!(
          loaded_publication(Some(fetched_at), fetched_at),
          LoadedPublication::Live,
      );
      assert_eq!(
          loaded_publication(Some(instant("2026-08-13T11:59:59Z")), fetched_at),
          LoadedPublication::Live,
      );
  }

  #[test]
  fn untouched_schedule_preserves_the_exact_original_instant() {
      with_owner(|| {
          let original = instant("2026-11-01T05:30:00.123456789Z");
          let state = ScheduledEditState::new(original, "2026-11-01T01:30".into());
          assert_eq!(state.value.get(), "2026-11-01T01:30");
          assert_eq!(state.publication(), Ok(PublicationIntent::PublishAt(original)));
      });
  }

  #[test]
  fn clear_is_local_and_maps_only_to_draft() {
      with_owner(|| {
          let state = ScheduledEditState::new(
              instant("2999-01-01T09:00:00Z"),
              "2999-01-01T09:00".into(),
          );
          state.clear();
          assert_eq!(state.value.get(), "");
          assert_eq!(state.publication(), Ok(PublicationIntent::Draft));
      });
  }

  #[test]
  fn edited_schedule_uses_the_parser_and_rejects_invalid_nonempty_input() {
      with_owner(|| {
          let state = ScheduledEditState::new(
              instant("2999-01-01T09:00:00Z"),
              "2999-01-01T09:00".into(),
          );
          state.set_input("not-a-date".into());
          assert_eq!(state.publication(), Err(InvalidSchedule));
          state.set_input("2999-02-03T10:15".into());
          assert!(matches!(state.publication(), Ok(PublicationIntent::PublishAt(_))));
          state.set_input("2020-03-05T12:00".into());
          assert!(
              matches!(state.publication(), Ok(PublicationIntent::PublishAt(_))),
              "a valid past value is a backdate, not an editor validation error",
          );
      });
  }

  #[test]
  fn scheduled_gate_dispatches_the_untouched_original_without_reparsing() {
      with_owner(|| {
          let original = instant("2026-11-01T05:30:00.123456789Z");
          let schedule =
              ScheduledEditState::new(original, "2026-11-01T01:30".into());
          let body = Field::<PostBody>::new();
          body.set_input("body");
          let seen = RwSignal::new(None);
          let (disabled, schedule_error, click) = scheduled_submit_gate(
              body,
              Signal::derive(|| false),
              schedule,
              Callback::new(move |(_, intent)| seen.set(Some(intent))),
          );

          assert!(!disabled.get());
          assert_eq!(schedule_error.get(), None);
          click.run(());
          assert_eq!(seen.get(), Some(PublicationIntent::PublishAt(original)));
      });
  }

  #[test]
  fn scheduled_gate_dispatches_draft_after_clear() {
      with_owner(|| {
          let schedule = ScheduledEditState::new(
              instant("2999-01-01T09:00:00Z"),
              "2999-01-01T09:00".into(),
          );
          schedule.clear();
          let body = Field::<PostBody>::new();
          body.set_input("body");
          let seen = RwSignal::new(None);
          let (disabled, schedule_error, click) = scheduled_submit_gate(
              body,
              Signal::derive(|| false),
              schedule,
              Callback::new(move |(_, intent)| seen.set(Some(intent))),
          );

          assert!(!disabled.get());
          assert_eq!(schedule_error.get(), None);
          click.run(());
          assert_eq!(seen.get(), Some(PublicationIntent::Draft));
      });
  }

  #[test]
  fn scheduled_gate_blocks_invalid_input_and_dispatches_nothing() {
      with_owner(|| {
          let schedule = ScheduledEditState::new(
              instant("2999-01-01T09:00:00Z"),
              "2999-01-01T09:00".into(),
          );
          schedule.set_input("not-a-date".into());
          let body = Field::<PostBody>::new();
          body.set_input("body");
          let ran = RwSignal::new(false);
          let (disabled, schedule_error, click) = scheduled_submit_gate(
              body,
              Signal::derive(|| false),
              schedule,
              Callback::new(move |_| ran.set(true)),
          );

          assert!(disabled.get());
          assert_eq!(schedule_error.get(), Some(InvalidSchedule));
          click.run(());
          assert!(!ran.get());
      });
  }

  #[test]
  fn scheduled_gate_blocks_body_and_caller_predicate() {
      with_owner(|| {
          let schedule = ScheduledEditState::new(
              instant("2999-01-01T09:00:00Z"),
              "2999-01-01T09:00".into(),
          );
          let body = Field::<PostBody>::new();
          let blocked = RwSignal::new(false);
          let ran = RwSignal::new(0_u32);
          let (disabled, schedule_error, click) = scheduled_submit_gate(
              body,
              Signal::derive(move || blocked.get()),
              schedule,
              Callback::new(move |_| ran.update(|count| *count += 1)),
          );

          assert!(disabled.get(), "blank body blocks");
          click.run(());
          assert_eq!(ran.get(), 0);

          body.set_input("body");
          blocked.set(true);
          assert!(disabled.get(), "the caller predicate blocks");
          click.run(());
          assert_eq!(ran.get(), 0);

          blocked.set(false);
          assert!(!disabled.get());
          assert_eq!(schedule_error.get(), None);
          click.run(());
          assert_eq!(ran.get(), 1);
      });
  }
  ```

  Also change `ComposeState::inputs` tests to assert all three wire mappings and
  retain the create-form local conversion test through a small
  `publication_from_local(publish: bool, value: &str) -> PublicationIntent`
  helper: draft wins when `publish == false`; a parsed value is `PublishAt`; an
  empty or invalid value remains `PublishNow`, preserving current create
  behavior.

- [x] **Step 5: Run the web state tests and verify RED**

  Run:
  `devtool run -- cargo nextest run -p web posts::edit_state posts::compose_state`

  Expected: FAIL because the new module/types/signatures are absent.

- [x] **Step 6: Implement the state seam and migrate callers**

  Add `edit_state` as an ungated module beside `compose_state`, export only the
  names consumed by `component.rs`, and keep `mod.rs` item-free. Migrate
  `CreatePostPage`, `PostCreateForm`, inline composer, and the current editor to
  build `PublicationIntent`; rendered controls and endpoint payloads must remain
  unchanged in this task. Keep the original schedule instant private so callers
  cannot accidentally overwrite it when setting display text.

- [x] **Step 7: Run focused tests and the per-commit gate, then commit**

  Run the Step 2 and Step 5 commands. Expected: PASS. Then run
  `devtool run -- cargo xtask check`. Expected: PASS.

  Stage `common/src/time.rs`, `web/src/posts/edit_state.rs`,
  `web/src/posts/compose_state.rs`, `web/src/posts/mod.rs`, and
  `web/src/posts/component.rs`; commit:

  ```text
  feat(web): model scheduled edit intent (#549)
  ```

---

## Task 3: Render and exercise the scheduled editor lifecycle

**Files:**

- Modify: `web/src/posts/component.rs` — consume `EditPostPreview`, render the
  three immutable loaded-state branches, and dispatch scheduled intent.
- Modify: `common/src/time.rs` — expose a strict round-trip converter without
  changing the create-form converter's browser-normalization contract.
- Modify: `web/src/posts/edit_state.rs` — use strict conversion only after a
  scheduled editor changes its schedule value.
- Modify: `end2end/tests/posts.ts` — let `createPostViaApi` carry an exact
  RFC3339 `publish_at` for the untouched-dispatch regression.
- Modify: `end2end/tests/selectors.ts` — name the now-high-frequency slug,
  schedule, and clear-schedule selectors; continue using the existing
  `publishButton` selector for Save.
- Modify/Test: `end2end/tests/posts.spec.ts` — expand scheduled publishing into
  the complete edit lifecycle; retain the separate draft-to-scheduled test.

**Interfaces / component contract:**

- `EditPostPage` seeds `ComposeState` from `fetched.post`, classifies exactly
  once with
  `loaded_publication(fetched.post.post.published_at, fetched.fetched_at)`, and
  constructs one `EditPublicationState` from that immutable classification.
- Replace boolean `is_published` props with the explicit loaded state. Extract a
  focused `ScheduleControl` subcomponent if needed to keep `thin-components`
  below its setup/view complexity limits.
- Draft UI is unchanged: slug + optional schedule, `Save draft`, `Publish`.
- Scheduled UI: no slug; schedule + inline error + `Clear schedule`; one primary
  `Save` (`button[name="publish"][value="true"]`). Call `edit_submit_gate` once
  and bind its returned `disabled`, `schedule_error`, and click callback
  directly; render the inline message only from `schedule_error`. The component
  never parses the body or schedule and never selects or reconstructs a
  `PublicationIntent`.
- Live UI is unchanged: no slug/schedule; one primary `Save` mapped to
  `PublicationIntent::PublishNow`.
- `Clear schedule` is `type="button"` and calls only
  `ScheduledEditState::clear`; it neither dispatches `Update` nor navigates.
- The existing `publish_redirect` remains authoritative: scheduled/rescheduled
  success navigates; `Draft` success remains on `/edit` and paints
  `EditSaveOutcome`'s `Draft saved.` summary.

- [x] **Step 1: Write the failing scheduled-editor e2e flow**

  Add selectors:

  ```typescript
  postSlug: 'input[name="slug_override"]',
  publishAt: 'input[name="publish_at"]',
  clearSchedule: 'button:has-text("Clear schedule")',
  ```

  Extend the existing API helper without changing its default behavior:

  ```typescript
  opts: {
    body: string;
    tags?: string[];
    publish?: boolean;
    publishAt?: string;
    slug?: string | null;
  }

  // inside post data
  ...(opts.publishAt ? { publish_at: opts.publishAt } : {}),
  ```

  Inside `test.describe("scheduled editor local time", …)` with
  `test.use({ timezoneId: "America/New_York" })`, first add an untouched
  dispatch regression:

  ```typescript
  const EXACT_FOLD_INSTANT = "2999-11-03T05:30:00.123456789Z";
  const created = await createPostViaApi(page, {
    body: "# Exact Scheduled Post\n\nbody",
    publish: true,
    publishAt: EXACT_FOLD_INSTANT,
  });

  // Reach the scheduled Post through /drafts, follow its author-visible
  // permalink, and use openEditor(page).
  await expect(page.locator(SEL.publishAt)).toHaveValue("2999-11-03T01:30");
  const before = await page.request.post(`${BASE_URL}/api/posts/get_preview`, {
    form: { post_id: String(created.post_id) },
  });
  expect(before.ok()).toBeTruthy();
  const original = (await before.json()).post.post.published_at as string;
  // PostgreSQL normalizes to microseconds while SQLite retains nanoseconds.
  expect(original).toContain(".123456");

  await click(page, SEL.publishButton("true")); // do not touch publishAt
  await page.waitForURL((url) => !url.pathname.endsWith("/edit"));

  const after = await page.request.post(`${BASE_URL}/api/posts/get_preview`, {
    form: { post_id: String(created.post_id) },
  });
  expect(after.ok()).toBeTruthy();
  expect((await after.json()).post.post.published_at).toBe(original);
  ```

  This assertion crosses the real component dispatch, `Update` server function,
  and storage backend. PostgreSQL normalizes the seeded nanoseconds to
  microseconds while SQLite retains them, so the pre-save assertion requires
  sub-minute precision common to both. Comparing the fetched canonical string
  before and after detects minute reparsing or any further fractional-precision
  loss, while the explicit timezone makes `01:30` a repeated wall-clock value.

  In the same timezone-scoped describe, put the management flow in a separate
  test. Expand the composer-scheduled test (or add one adjacent test reusing its
  setup) with one far-future creation and the following assertions/actions in
  order:
  1. Create `# Scheduled Draft` at `2999-01-01T09:00` through `/posts/new` and
     reach its author-visible permalink through `/drafts`.
  2. Open `/posts/<id>/edit`; assert `publishAt` has the original local value,
     `clearSchedule` and lone `Save` are visible, and `postSlug` is absent.
  3. Fill `publishAt` with `2999-02-03T10:15`, click Save, wait for the
     permalink redirect, reopen through `/drafts`, and assert the replacement
     value is prefilled.
  4. Fill the syntactically valid but nonexistent DST-gap wall time
     `2027-03-14T02:30`; assert the inline `Enter a valid local date and time`
     error and disabled Save, and assert the page remains on `/edit`. The
     explicit browser timezone makes this deterministic across every
     backend/browser matrix job.
  5. Restore/reopen, click `Clear schedule`, assert the field empties and the
     URL stays `/edit`; navigate away and reopen once to prove Clear alone did
     not mutate the stored schedule.
  6. Clear again, click Save, wait for `.j-save-summary`, assert `Draft saved.`
     and that the URL still ends in `/edit`.
  7. Reopen the resulting draft, assert the slug field is visible, change it,
     Save draft, follow the newly returned permalink, and assert the new slug is
     in the URL.
  8. Reopen that draft, schedule it again, follow the redirect, reopen the
     scheduled editor, and assert the slug control is hidden again.

  Use `navigateInApp`, `openEditor`, `followPermalink`, `click`, and
  `waitForSelector`; do not use `page.reload()`, `networkidle`, or a sleep. Keep
  the existing draft-to-scheduled coverage because it pins the unchanged draft
  branch and scheduling from edit.

- [x] **Step 2: Run the focused e2e test and verify RED**

  Run: `devtool run -- cargo xtask e2e-local posts.spec.ts`

  Expected: FAIL at the first scheduled-editor assertion: current editor hides
  the schedule control and offers no Clear action.

- [x] **Step 3: Implement the three-state editor**

  Thread `EditPostPreview` and `LoadedPublication` through `EditPostPage`,
  `EditPostForm`, options/schedule rendering, and save actions. Reuse
  `ComposeState` for body/format/summary/tags/audience; do not seed its create
  schedule field from a scheduled Post. For the scheduled branch, source the
  input exclusively from `ScheduledEditState`, leaving its original instant
  intact until `set_input` or `clear` marks the field edited.

  Keep the loaded state immutable after a successful update. This is deliberate:
  the existing redirect handles non-null results, while clear-and-save remains
  on the page and must not reinterpret a due timestamp or expose the slug until
  the author reopens the resulting draft.

- [x] **Step 4: Run the focused e2e flow and verify GREEN**

  Run the Step 2 command. Expected: PASS on the local Chromium projects. The
  untouched fold instant remains byte-identical through editor dispatch, and
  schedule create → prefill → reschedule → reopen → clear → draft → slug-edit →
  reschedule are observed through rendered UI, URLs, and the authenticated
  preview.

- [x] **Step 5: Run the Rust state/endpoint regressions and per-commit gate**

  Run:
  - `devtool run -- cargo nextest run -p web posts::edit_state posts::compose_state`
  - `devtool run -- cargo nextest run -p common time::tests`
  - `devtool run -- devtool pg run -- cargo nextest run -p jaunder web::web_posts::get_post_preview_shows_draft_to_author_only`
  - `devtool run -- cargo xtask check`

  Expected: PASS. Stage `common/src/time.rs`, `web/src/posts/edit_state.rs`,
  `web/src/posts/component.rs`, `end2end/tests/posts.ts`,
  `end2end/tests/selectors.ts`, and `end2end/tests/posts.spec.ts`. Commit:

  ```text
  feat(web): manage scheduled posts in editor (#549)
  ```

---

## Task 4: Validate the complete branch

**Files:**

- No planned source changes. If the verify-only gate exposes a real defect,
  repair it in the owning task, rerun that task's focused checks, and make a
  gated fixup commit before repeating this task.

- [ ] **Step 1: Run the full local gate**

  Run: `devtool run -- cargo xtask validate`

  Expected: PASS for static checks, coverage, and all four
  `{sqlite,postgres} × {chromium,firefox}` e2e combinations. This is the proof
  for acceptance criteria 13–14 and backend/browser parity.

- [ ] **Step 2: Confirm plan completion**

  Verify every checkbox above is ticked, the branch is clean, and the final
  checked tree is the committed tree. Do not archive the spec/plan here;
  `jaunder-ship` owns archival after whole-branch review and immediately before
  PR preparation.
