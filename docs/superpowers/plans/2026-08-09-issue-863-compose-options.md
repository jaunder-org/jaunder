# ComposeOptions extraction — implementation plan (#863)

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-08-09-issue-863-compose-options.md` —
referenced by decision (D1–D6) and acceptance criterion (AC1–AC14). Not restated
here.

**Goal:** Collapse the duplicated options aside in `FullComposer` and
`EditPostForm` into one `ComposeOptions` component plus a `MediaSection`.

**Architecture:** Pure Leptos CSR component extraction inside
`web/src/posts/component.rs`. The composer's `publish-at` field moves up beside
its slug so both shapes share one field order (D1); the two shapes'
`id`/`placeholder` differences are unified rather than parameterized (D3); a
single `is_published: bool` gates the slug + schedule pair (D4).

**Tech Stack:** Rust, Leptos 0.8 CSR (wasm-only module), Playwright e2e,
`cargo xtask`.

## Review header

**Scope — in:** `web/src/posts/component.rs`, `web/src/posts/compose_state.rs`
(doc comments only), `end2end/tests/posts.spec.ts`, the body of issue #863.

**Scope — out:** moving `slug_field` into `ComposeState`; the forms-crate
implicit `<label>` convention (**#871**); unifying the two property-identical
aside classes (**#872**); a `/posts/new` format-toggle e2e test (**#873**).

**Tasks:**

1. File the three separable concerns as issues; commit the spec and this plan.
2. Add the missing e2e test for the **editor's** schedule field, against today's
   `#edit-publish-at` — a characterization test that must pass before anything
   moves.
3. Extract `MediaSection` — a pure move, no DOM change.
4. Extract `ComposeOptions`: reorder, unify ids/placeholder, rewire both
   parents, update the two e2e id references in the same commit.
5. Correct the five stale doc comments and edit #863's body.

**Key risks / decisions:**

- **The new components must each emit exactly one root `<div>`** (D2). Both
  asides are flex columns with `gap:18px`; a bare fragment would put 18px
  between every field. This is the single most likely way to get a silently
  wrong result — `cargo xtask check` cannot catch it, and neither can any
  existing e2e test.
- **A scheduled publish does not render `.j-save-summary`.** `EditSaveOutcome`
  (`component.rs:1329-1347`) takes its `Ok(_)` "Redirecting…" arm whenever
  `published_at.is_some()`, which a future schedule makes true. Copying the
  composer schedule test's `waitForSelector(SEL.saveSummary)` settle step would
  hang. Task 2 runs the test against unchanged code precisely so this gets
  settled before the refactor.
- **Task ordering keeps the e2e suite green at every commit.** The id rename
  (Task 4) and its two e2e references land together; Task 2's guard exists
  before the rename it guards.
- e2e is the only observable check on rendered output —
  `web/src/posts/component.rs` is wasm-only (`web/src/posts/mod.rs:17`) and the
  repo has no render-to-string harness (CONTRIBUTING.md:715-717). No host tests
  are addable for AC6–AC10.

## Global Constraints

- `thin-components` (ADR-0086): max **2** control-flow units per surface (setup
  / view). A unit is `if`, `match`, `for`, `while`, `?`, `let … else`, or a
  guarded match arm. `<Show>`, `<For>`, child components, and `.then()` cost
  **nothing**.
- A plain `fn -> impl IntoView` does **not** count as decomposition (ADR-0086
  §4) — every extraction must be a `#[component]`.
- No `Co-Authored-By` trailer on any commit.
- The pre-commit hook runs the full `cargo xtask check`; run it first so it
  passes clean (`jaunder-commit`).
- Stage, then commit. Never `git commit -- <paths>`.
- Do **not** rewrite `edit-slug` in `xtask/src/steps/thin_components.rs:455` or
  in `docs/archive/` — fixture and historical text (AC9).

---

### Task 1: File the separable concerns, and commit the planning docs

**Files:**

- Modify: `docs/superpowers/plans/2026-08-09-issue-863-compose-options.md`
  (Step 2)
- Commit: that plan plus
  `docs/superpowers/specs/2026-08-09-issue-863-compose-options.md`

**Interfaces:** Produces three issue numbers; referenced nowhere else in this
plan.

- [x] **Step 1: File three issues** via `jaunder-issues`, all labelled `web`,
      milestone "Web: canonical Leptos CSR convergence", each linking back to
      #863:
  1. **"web/posts: move the compose slug and schedule fields to the forms
     crate's implicit-`<label>` convention"** —
     `web/src/forms/component.rs:8-11` documents that validated fields wrap
     their control in a `<label>` so no `for=`/`id=` pair can drift. The two
     hand-rolled fields in `ComposeOptions` still carry an explicit pair. #863
     unified their ids but deliberately did not restructure the labels.
  2. **"web: `j-compose-aside` and `j-edit-form-aside` are property-identical;
     collapse to one class"** — `server/assets/jaunder.css:735-743` and
     `:1068-1076` have identical rule bodies. Same for `j-compose-grid` (`:723`)
     and `j-edit-form-grid` (`:1053`).
  3. **"e2e: no `/posts/new` coverage for the format toggle"** —
     `posts.spec.ts:715` tests the toggle only on the compact `.j-composer`. The
     full composer's toggle is untested. Pre-existing gap, unrelated to #863.

- [x] **Step 2: Record the numbers** in this plan's Review header "Scope — out"
      line. Filed as #871 (forms-crate labels), #872 (duplicate aside/grid
      classes), #873 (format-toggle e2e gap).

- [x] **Step 3: Commit the spec and the plan**

Both are tracked-but-uncommitted, and Step 2 edits the plan again. They must
land now: `cargo xtask validate` refuses a dirty worktree
(`xtask/src/lib.rs:490-494`), so leaving them uncommitted would block the pre-PR
gate, and until then every later task's pre-commit run carries them as unrelated
dirt.

```bash
git add docs/superpowers/specs/2026-08-09-issue-863-compose-options.md docs/superpowers/plans/2026-08-09-issue-863-compose-options.md
git commit -m "docs(plan): spec and plan for the ComposeOptions extraction (#863)"
```

---

### Task 2: Characterization test for the editor's schedule field

The editor's `publish-at` control has **no e2e coverage today**. Task 4 renames
its id; without this test that rename ships unverified (AC14). Written against
the **current** `#edit-publish-at` so it must pass before any code moves — which
is also how the settle-step uncertainty gets resolved empirically rather than
guessed.

**Files:**

- Modify: `end2end/tests/posts.spec.ts` (append after the composer schedule
  test, which ends at `:1015`)

**Interfaces:**

- Consumes: `goto`, `click`, `waitForSelector` from `./helpers`; `SEL` from
  `./selectors` (`SEL.postBody` = `textarea[name="body"]`, `SEL.saveSummary` =
  `.j-save-summary`, `SEL.publishButton(v)` =
  `button[name="publish"][value="${v}"]`, `SEL.topbarHeading`) — all already
  imported by this file.
- Produces: a test named
  `"scheduling from the edit page shows a Scheduled-for badge on the drafts page"`.
  Task 4 changes exactly one line of it.

- [x] **Step 1: Write the test**

```ts
test("scheduling from the edit page shows a Scheduled-for badge on the drafts page", async ({
  registeredPage: page,
}) => {
  test.slow();
  // The editor's schedule control had no coverage before #863, which is what made the
  // `edit-publish-at` -> `options-publish-at` rename unverifiable. Mirrors the
  // composer-side test above, with one deliberate difference in the settle step: a
  // *scheduled* publish sets `published_at` to a future instant, so `EditSaveOutcome`
  // takes its `Ok(_)` "Redirecting…" arm rather than rendering `.j-save-summary`.
  const FUTURE_DATETIME_LOCAL = "2999-01-01T09:00";

  // Create a draft and reach its edit page, the same route posts.spec.ts:167 uses.
  await goto(page, "/posts/new");
  await page.fill(SEL.postBody, "# Scheduled From Editor\n\nbody");
  await click(page, SEL.publishButton("false"));
  await waitForSelector(page, SEL.saveSummary);

  const permalinkHref = (await page
    .locator(SEL.saveSummary)
    .locator('[data-test="permalink-link"]')
    .getAttribute("href"))!;
  await goto(page, permalinkHref);
  const editLink = page.locator('.j-post-acts a:has-text("Edit")');
  await editLink.waitFor();
  const postId = (await editLink.getAttribute("href"))!.match(
    /\/posts\/(\d+)\/edit/,
  )![1];

  await goto(page, `/posts/${postId}/edit`);
  await expect(page.locator(SEL.topbarHeading)).toHaveText("Edit Post");

  // The draft is unpublished, so the slug and schedule controls are rendered.
  await page.fill("#edit-publish-at", FUTURE_DATETIME_LOCAL);
  await click(page, SEL.publishButton("true"));

  await goto(page, "/drafts");
  const scheduledRow = page.locator("li", { hasText: "Scheduled From Editor" });
  await expect(scheduledRow).toBeVisible();
  const badge = scheduledRow.locator(".j-badge-scheduled");
  await expect(badge).toBeVisible();
  await expect(badge).toContainText("Scheduled for");
});
```

- [x] **Step 2: Run it against unchanged code** — **36 passed** (was 35), 275s.

Run:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-863-compose-options -- cargo xtask e2e-local posts.spec.ts
```

Expected: **PASS.** This is a characterization test — the behaviour already
works; the test is new.

- [x] **Step 3: If it fails, fix the test, not the app** — not needed; the test
      passed as written, so neither settle-step fallback below was used.

The one step this plan cannot fully determine statically is how the click on
Publish settles, because a scheduled publish leaves the editor via the
"Redirecting…" arm rather than rendering `.j-save-summary`
(`component.rs:1347`). If `/drafts` is read before the update lands, insert a
settle between the click and the `goto`:

```ts
await expect(page.getByText("Redirecting")).toBeVisible();
```

and if the redirect races that assertion, use the navigation itself:

```ts
await page.waitForURL((url) => !url.pathname.endsWith("/edit"));
```

Re-run Step 2 until it passes. Do not change `web/src` in this task.

- [x] **Step 4: Commit**

```bash
git add end2end/tests/posts.spec.ts
git commit -m "test(e2e): cover the editor's schedule control before extracting it (#863)"
```

---

### Task 3: Extract `MediaSection`

The smallest, lowest-risk extraction: byte-identical in both parents, no DOM
change.

**Files:**

- Modify: `web/src/posts/component.rs` — add the component; replace the two
  media blocks, **located by content, not line number**: each is a
  `<div style="margin-top:16px">` holding a `j-sb-head` reading `"Media"` and a
  `<MediaUpload show_result=true />`. At HEAD they are `:698-703` (composer) and
  `:1238-1243` (editor), but replacing the first shifts the second by −5.

**Interfaces:**

- Produces: `#[component] fn MediaSection() -> impl IntoView` — no props. Task
  4's rewired parents render it.

- [x] **Step 1: Add the component**

Place it immediately after the `EditSaveActions` component (ends `:1315` at
HEAD). The wrapper `<div>` is part of the component (D2, AC2) — both asides are
flex columns with `gap:18px`, so emitting a fragment would insert an 18px gap
between the heading and the upload control.

```rust
/// The media column shared by the two full-page compose shapes.
///
/// Extracted from `FullComposer` and `EditPostForm` (#863), which held byte-identical
/// copies. Emits a single wrapping `<div>` on purpose: both asides are flex columns
/// with `gap:18px`, so a bare fragment would space the heading off the control.
#[component]
fn MediaSection() -> impl IntoView {
    view! {
        <div style="margin-top:16px">
            <div class="j-sb-head" style="padding:0 0 10px">
                "Media"
            </div>
            <MediaUpload show_result=true />
        </div>
    }
}
```

- [x] **Step 2: Replace both call sites**

Replace the media block in `FullComposer`, then the one in `EditPostForm` —
found by content as described in **Files** above. Both become exactly:

```rust
<MediaSection />
```

- [x] **Step 3: Run the gate**

Run:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-863-compose-options -- cargo xtask check
```

Expected: PASS, including `thin-components` (`MediaSection` has 0 control-flow
units on both surfaces). — **PASS** (`check --no-test`; the full `check` runs in
the pre-commit hook at Step 5). Diff reviewed: the two blocks collapsed to
`<MediaSection />` and nothing else moved.

- [x] **Step 4: Verify no DOM change**

Run:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-863-compose-options -- cargo xtask e2e-local media.spec.ts
```

Expected: PASS — `media.spec.ts:163` drives the full composer's upload widget. —
**11 passed**, 149s.

- [x] **Step 5: Commit**

```bash
git add web/src/posts/component.rs
git commit -m "refactor(web/posts): extract MediaSection from both compose shapes (#863)"
```

---

### Task 4: Extract `ComposeOptions`

The substantive task: the extraction plus the three approved DOM changes (D1
reorder, D3 ids, D3 placeholder) and the two e2e id references, in one commit so
the suite is green at every commit.

**Files:**

- Modify: `web/src/posts/component.rs` — add the component; replace the two
  Options blocks, **located by content, not line number** (Task 3's two edits
  shift everything below them by −5 and −10 respectively). Each is the first
  inner `<div>` of an `<aside>`, opening with a `j-sb-head` reading `"Options"`
  and closing after the `<FormatToggle …/>`. At HEAD they are `:636-697`
  (composer) and `:1170-1237` (editor); after Task 3 the editor's is
  `:1165-1232`.
- Modify: `end2end/tests/posts.spec.ts:992` (comment), `:1003` (fill), and the
  `#edit-publish-at` fill added in Task 2

**Interfaces:**

- Consumes: `MediaSection` (Task 3); `ComposeState` (`compose_state.rs:30`),
  `Field<Slug>`, `PostSummary`, `ValidatedTextarea`, `TagInput`,
  `AudiencePicker`, `FormatToggle` — all already in scope in this file.
- Produces:
  `#[component] fn ComposeOptions(state: ComposeState, slug_field: Field<Slug>, is_published: bool) -> impl IntoView`
  (AC1).

- [ ] **Step 1: Add the component**

Place it immediately before `MediaSection`. This body is written out in full
because no test can pin markup — `component.rs` is wasm-only and the repo has no
render-to-string harness. Every line below is moved from an existing copy except
the three approved changes, marked inline.

```rust
/// The options aside shared by the two full-page compose shapes: slug and schedule
/// while the post is still a draft, then summary, tags, audience and format.
///
/// Extracted from `FullComposer` and `EditPostForm` (#863), which rendered
/// near-identical copies that had to be edited in lockstep. The composer's schedule
/// control moved up beside its slug as part of that collapse, so both shapes now share
/// one field order; the two shapes' `compose-`/`edit-` id prefixes were unified, since
/// they never render on the same page.
///
/// Emits a single wrapping `<div>` on purpose: both asides are flex columns with
/// `gap:18px`, so a bare fragment would put 18px between every field.
#[component]
fn ComposeOptions(
    state: ComposeState,
    /// Page-level rather than held by [`ComposeState`], because the compact shape uses
    /// that bundle too and has no slug field — see that type's `seed_from`.
    slug_field: Field<Slug>,
    /// A published post shows neither the slug nor the schedule control: its URL is
    /// already public, and it has no publish time left to choose. The composer passes
    /// `false` — a post being composed is not yet published.
    is_published: bool,
) -> impl IntoView {
    view! {
        <div>
            <div class="j-sb-head" style="padding:0 0 10px">
                "Options"
            </div>
            {(!is_published)
                .then(|| {
                    view! {
                        <div class="j-field-row" style="grid-template-columns:auto 1fr">
                            <label class="j-field-label" for="options-slug">
                                "Slug"
                            </label>
                            <input
                                id="options-slug"
                                type="text"
                                name="slug_override"
                                placeholder="auto"
                                class="j-field-val"
                                prop:value=slug_field.value
                                on:input=move |ev| {
                                    let v = event_target_value(&ev);
                                    slug_field.value.set(v.clone());
                                    slug_field.error.set(slug_field.error_for(&v));
                                }
                                on:blur=move |_| slug_field.touch()
                            />
                            {move || {
                                slug_field
                                    .is_touched()
                                    .then(|| slug_field.error.get())
                                    .flatten()
                                    .map(|msg| view! { <p class="error">{msg}</p> })
                            }}
                        </div>
                        // Optional schedule: a future time schedules the post;
                        // a past time backdates it; empty publishes immediately.
                        <div style="margin-top:10px">
                            <label class="j-field-label" for="options-publish-at">
                                "Publish at (optional)"
                            </label>
                            <input
                                id="options-publish-at"
                                type="datetime-local"
                                class="j-field-val"
                                prop:value=state.publish_at
                                on:input=move |ev| state.publish_at.set(event_target_value(&ev))
                            />
                        </div>
                    }
                })}
            <div style="margin-top:10px">
                <ValidatedTextarea<
                PostSummary,
            >
                    label="Summary"
                    name="summary"
                    field=state.summary_field
                    placeholder="Optional summary or excerpt"
                />
            </div>
            <div style="margin-top:10px">
                <TagInput tags=state.tags />
            </div>
            <div style="margin-top:10px">
                <AudiencePicker selection=state.audience />
            </div>
            <FormatToggle format=state.format style="margin-top:10px" />
        </div>
    }
}
```

The changes relative to the code being replaced — four to the composer's markup,
two to the editor's:

| change                                                                  | was                                                       | spec |
| ----------------------------------------------------------------------- | --------------------------------------------------------- | ---- |
| the publish-at `<div>` now follows the slug row                         | composer had it after `AudiencePicker` (`:684-695`)       | D1   |
| `id="options-slug"` / `id="options-publish-at"` and the matching `for=` | `compose-slug`/`edit-slug`, `compose-publish-at`/`edit-…` | D3   |
| the slug input carries `placeholder="auto"` on both pages               | composer only (`:648`); the editor's had none             | D3   |
| the composer's publish-at is now inside the `(!is_published)` guard     | composer rendered it unconditionally (`:684-695`)         | D4   |

That fourth row is behaviourally inert — the composer passes
`is_published=false`, so the guard is always taken — but it is a real change to
the composer's markup structure and is listed so a reviewer is not surprised by
it.

Two lines have **mixed provenance**, deliberately: the `.then()` wrapper comes
from the editor, but the publish-at comment and the `on:input` handler inside it
are the composer's wording and expression form (`:682-683`, `:693`) rather than
the editor's (`:1202-1203`, `:1213-1215`). The composer's comment is the fuller
of the two — it covers backdating, which the editor's omits — and the handlers
are semantically identical. Everything else is byte-identical to one of the two
originals. `leptosfmt` produced the odd `ValidatedTextarea<\nPostSummary,\n>`
wrapping in the original; keep it and let the gate reformat.

- [ ] **Step 2: Rewire `FullComposer`**

Replace the composer's whole Options `<div>` (located by content per **Files**)
with:

```rust
<ComposeOptions state=state slug_field=slug_field is_published=false />
```

The `<aside class="j-compose-aside">` and the trailing
`<div style="margin-top:auto;…">` button pair stay exactly as they are — the
button div must remain the last flex child for its `margin-top:auto` bottom-pin
(AC4). The `slug_field` local at `:616` and the `dispatch` / `submit_disabled`
closures at `:618-623` are unchanged: they still own the field, they just no
longer render it.

- [ ] **Step 3: Rewire `EditPostForm`**

Replace the editor's whole Options `<div>` (located by content per **Files**)
with:

```rust
<ComposeOptions state=state slug_field=slug_field is_published=is_published />
```

`<aside class="j-edit-form-aside">` and the `<div class="j-edit-form-actions">`
wrapper around `EditSaveActions` stay as they are (AC4).

- [ ] **Step 4: Update the two e2e id references**

In `end2end/tests/posts.spec.ts`:

- `:992` — the comment now reads
  `// The non-compact composer's optional schedule control is `#options-publish-at``
- `:1003` — `page.fill("#compose-publish-at", …)` becomes
  `page.fill("#options-publish-at", …)`
- Task 2's test — `page.fill("#edit-publish-at", …)` becomes
  `page.fill("#options-publish-at", …)`, and its comment about the rename
  updates to past tense.

- [ ] **Step 5: Verify no old id survives**

Run:

```bash
rg -n 'compose-slug|edit-slug|compose-publish-at|edit-publish-at' web/src end2end
```

Expected: **no matches.** (`xtask/src/steps/thin_components.rs:455` and
`docs/archive/` are deliberately out of this sweep — see Global Constraints.)

- [ ] **Step 6: Run the gate**

Run:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-863-compose-options -- cargo xtask check
```

Expected: PASS. `thin-components` budgets after the split (AC5):
`ComposeOptions` 0/0 (`.then()` and `.map()` are free), `MediaSection` 0/0,
`FullComposer` 1 setup (the `if let Some(post)` in `dispatch`) / 0 view,
`EditPostForm` 1 setup / 0 view — all within the budget of 2.

- [ ] **Step 7: Run the e2e specs that touch the aside**

Run:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-863-compose-options -- cargo xtask e2e-local posts.spec.ts
```

Expected: PASS — including `:41`/`:68`/`:86` (summary), `:124`/`:280` (slug),
`:207` (audience), `:733`/`:786`/`:870`–`:948` (tags), `:986` (composer
schedule), and Task 2's editor-schedule test. Note `:313`, inside the "editing a
published post freezes the slug" test at `:280`, asserts
`input[name="slug_override"]` is **not** visible on a published post's edit page
— that is the existing coverage of AC8's slug half.

Then:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-863-compose-options -- cargo xtask e2e-local visibility.spec.ts
```

Expected: PASS — `:39`/`:240` drive the full composer's audience picker.

Then re-run the media spec: Task 3 verified it before `ComposeOptions` existed,
and Task 4 restructures the aside around it.

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-863-compose-options -- cargo xtask e2e-local media.spec.ts
```

Expected: PASS — `:163` drives the full composer's upload widget.

- [ ] **Step 8: Eyeball all three shapes**

No test checks that the aside still _looks_ right, and the wrapper-`<div>` risk
(see Key risks) is invisible to both `check` and e2e. Visit all three states:

1. **`/posts/new`** (AC6) — the Options column reads: heading, Slug, "Publish at
   (optional)", Summary, Tags, Audience, format toggle. Buttons still pinned to
   the bottom of the column; no extra gap between fields (that gap is the
   fragment-instead-of-`<div>` failure mode).
2. **A draft's `/posts/:id/edit`** (AC7) — the same seven elements in the same
   order. Values differ (the editor seeds them); only the sequence is asserted.
3. **A published post's `/posts/:id/edit`** (AC8) — Slug and "Publish at" both
   absent, the remaining five unchanged, `EditSaveActions` showing the lone
   "Save". The slug half is covered by `posts.spec.ts:313`; the publish-at half
   has no test, so this eyeball is its only check.

- [ ] **Step 9: Commit**

```bash
git add web/src/posts/component.rs end2end/tests/posts.spec.ts
git commit -m "refactor(web/posts): extract ComposeOptions from both compose shapes (#863)"
```

---

### Task 5: Correct the stale docs and the issue body

D5's premise falsifies five comments; the issue's Constraints section
contradicts what shipped.

**Files:**

- Modify: `web/src/posts/compose_state.rs:96-99`
- Modify: `web/src/posts/component.rs` — the `FullComposer` doc, the
  `// The slug is not part of the bundle` comment, the `EditPostForm` doc, and
  the `slug_field` prop doc (line numbers shift after Task 4; find them by text)

**Interfaces:** none — documentation only.

- [ ] **Step 1: `compose_state.rs` — `seed_from`'s doc**

Replace the sentence beginning "The slug is deliberately **not** seeded here"
with:

```rust
    /// The slug is deliberately **not** seeded here: this type does not hold that
    /// field, because the compact composer uses this same bundle and renders no slug
    /// control (`inputs` takes `slug_override` as a parameter for exactly that reason).
    /// The two full shapes own the field at page level and hand it to `ComposeOptions`,
    /// so the editor sets it at the call site rather than handing the field in here.
```

- [ ] **Step 2: `component.rs` — `FullComposer`'s doc**

The current text names the old field order and calls the slug local. Replace
with:

```rust
/// The full compose page: body column plus the options aside ([`ComposeOptions`]), the
/// media column ([`MediaSection`]) and the dispatch buttons. Split out of
/// [`PostCreateForm`] (#301); the slug field is owned here and passed down, because the
/// compact shape shares [`ComposeState`] and has no slug — see that type's `seed_from`.
```

- [ ] **Step 3: `component.rs` — the `EditPostPage` inline comment**

Replace `// The slug is not part of the bundle — see seed_from.` with:

```rust
                        // The slug is not part of the bundle (the compact shape has
                        // none) — see `seed_from`.
```

- [ ] **Step 4: `component.rs` — `EditPostForm`'s doc and its `slug_field` prop
      doc**

The doc still enumerates markup the component no longer holds. Replace with:

```rust
/// The editor's form: body column plus the options aside ([`ComposeOptions`], which
/// hides the slug and schedule once the post is published), the media column
/// ([`MediaSection`]) and the save controls.
/// Split out of [`EditPostPage`] (#301), which keeps only the fetch and its branch.
```

and the `slug_field` prop doc:

```rust
    /// Page-level rather than held by [`ComposeState`], because the compact shape
    /// shares that bundle and has no slug field — see that type's `seed_from`.
```

- [ ] **Step 5: Verify no "local to that shape" justification survives**

Run:

```bash
rg -n 'local to (that|this) shape' web/src
```

Expected: **no matches** (AC11).

- [ ] **Step 6: Edit issue #863's body** (AC12)

Replace the third bullet of the **Constraints** section — "No markup change: the
rendered output of both shapes must be identical, which the e2e suite is the
real check on" — with:

```markdown
- Three DOM changes were approved during the #863 design interview, superseding
  this issue's original "no markup change" constraint:
  - the composer's `publish-at` control moves up beside its slug, so both shapes
    share one field order — this is what made a single component possible at
    all;
  - `compose-slug`/`edit-slug` and `compose-publish-at`/`edit-publish-at` unify
    to `options-slug`/`options-publish-at`; the two shapes never render on the
    same page, so the prefixes distinguished nothing;
  - the editor's slug input gains `placeholder="auto"`, which the composer
    already had and the editor was missing by omission.

    Everything else must render identically, which the e2e suite is the real
    check on. See
    `docs/superpowers/specs/2026-08-09-issue-863-compose-options.md` D1 and D3.
```

Leave the rest of the body — Problem, Why it is filed rather than fixed, What to
do, Notes — unchanged.

- [ ] **Step 7: Run the gate**

Run:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-863-compose-options -- cargo xtask check
```

Expected: PASS.

**The gate does not check the new intra-doc links.** `cargo xtask check` runs no
rustdoc step, so `rustdoc::broken_intra_doc_links` never fires; the `doc-links`
step is unrelated — it resolves relative Markdown links on disk
(`xtask/src/steps/doc_links.rs:1-2`). So verify the four new
`[`ComposeOptions`]` / `[`MediaSection`]` references by hand:

```bash
rg -n 'fn (ComposeOptions|MediaSection)' web/src/posts/component.rs
```

Expected: both found, and both are items in the same module as every doc comment
that links them — which is what makes the shorthand links resolve.

- [ ] **Step 8: Commit**

```bash
git add web/src/posts/component.rs web/src/posts/compose_state.rs
git commit -m "docs(web/posts): correct the slug-placement rationale after the extraction (#863)"
```

---

## Before the PR

Run the full local gate — all four `{sqlite,postgres}×{chromium,firefox}`
combos:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-863-compose-options -- cargo xtask validate
```

Expected: PASS. Use Bash background mode; this is the long, cold run.

Then hand off to `jaunder-ship`.
