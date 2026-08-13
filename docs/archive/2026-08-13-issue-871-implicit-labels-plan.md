# Compose Options Implicit Labels — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the compose-options labels structurally associate with their
controls, while preserving layout and using valid error markup inside every
implicit label.

**Architecture:** Keep `ComposeOptions` hand-built: move its existing layout
onto/nested beneath wrapping labels, retain signal wiring, and expose stable
control names. Correct the shared `Labelled` validation element at the same HTML
boundary. Playwright pins DOM association, presentation, selectors, and both
composer/editor consumers.

**Tech Stack:** Rust, Leptos `view!`, Playwright/TypeScript, CSSOM assertions.

**Spec:**
[`2026-08-12-issue-871-implicit-labels.md`](../specs/2026-08-12-issue-871-implicit-labels.md)

## Review header

**Scope — in:** the two `ComposeOptions` labels and schedule name; valid
label-contained validation markup in `ComposeOptions` and shared `Labelled`;
Playwright selectors/assertions on composer and draft editor; stale
`ComposeOptions` rustdoc.

**Scope — out:** forms-component migration for either option; standalone
action/page errors; other explicit label associations; styling, field order, or
Post behavior changes.

**Tasks:**

1. Pin the implicit-label DOM contract in Playwright, implement it in Leptos,
   validate, and commit the single cohesive change.

**Key risks/decisions:**

- The slug label itself becomes the grid container; its text span and input stay
  sibling grid items. The error becomes a third grid item.
- Label-contained errors use `<span class="error">`; the shared `.error` class
  supplies `display:block`, and standalone errors remain `<p>`.
- Assertions use semantic `name` selectors and CSSOM for inline layout values,
  avoiding brittle serialized-style matching.
- No new dependency, module, domain term, or ADR.

## Global Constraints

- Preserve all existing values, signal handlers, touched gating, conditional
  rendering, input classes/types, slug placeholder, visible copy, and field
  order.
- Preserve the slug's `.j-field-row` / `auto 1fr` grid and the schedule's
  `margin-top:10px` wrapper.
- Use exactly `name="slug_override"` and `name="publish_at"`.
- No `<p>` may be nested in an implicit `<label>`; do not change standalone
  action/page error paragraphs.
- Follow `jaunder-e2e`: one document boot, existing in-app navigation, semantic
  readiness waits, no timing waits or `networkidle`.
- No `Co-Authored-By` trailer.

---

### Task 1: Convert compose options and label-contained errors

**Files:**

- Modify: `end2end/tests/posts.spec.ts` — shared validation selector and
  composer/editor `ComposeOptions` DOM assertions.
- Modify: `web/src/posts/component.rs:1210-1275` — rustdoc, implicit labels,
  schedule name, and slug validation element.
- Modify: `web/src/forms/component.rs:20-64` — valid shared validation element.
- Modify: `server/assets/jaunder.css:1229-1242` — shared block presentation for
  `.error` spans.

**Interfaces:**

- Consumes: `Field<Slug>`, `ComposeState::publish_at`, existing
  `forms::Labelled(error, touched, children)` interface; no signature changes.
- Produces: `input[name="slug_override"]` and `input[name="publish_at"]`
  structurally nested in their visible labels; label-contained validation as
  block-displayed `span.error`.

- [x] **Step 1: Write the failing Playwright contract**

Change the existing body-validation locator to:

```ts
const bodyError = page.locator(".j-composer-field span.error");
```

Retain its zero-before-blur and exact-message assertions. This pins the shared
`forms::Labelled` element and behavior. After the message appears, assert
`await expect(bodyError).toHaveCSS("display", "block");`.

In the composer scheduling scenario, immediately after
`registeredPage("/posts/new")`, add:

```ts
const slug = page.locator('input[name="slug_override"]');
const slugLabel = page.locator(
  'label.j-field-row:has(input[name="slug_override"])',
);
await expect(slugLabel).toHaveCount(1);
await expect(slugLabel.locator(":scope > .j-field-label")).toHaveText("Slug");
await expect(slug).not.toHaveAttribute("id", /.+/);
await expect(slugLabel).not.toHaveAttribute("for", /.+/);
await expect(slugLabel).toHaveCSS("grid-template-columns", /.+/);
expect(
  await slugLabel.evaluate((label) => label.style.gridTemplateColumns),
).toBe("auto 1fr");

const schedule = page.locator('input[name="publish_at"]');
const scheduleLabel = page.locator(
  'label.j-field-label:has(input[name="publish_at"])',
);
await expect(scheduleLabel).toHaveCount(1);
await expect(scheduleLabel).toContainText("Publish at (optional)");
await expect(schedule).not.toHaveAttribute("id", /.+/);
await expect(scheduleLabel).not.toHaveAttribute("for", /.+/);
expect(
  await scheduleLabel.evaluate(
    (label) => (label.parentElement as HTMLElement).style.marginTop,
  ),
).toBe("10px");
```

Use `schedule` for the scenario's existing `fill` call. Remove the scenario's
existing `test.slow()`: the repository's ambient scaled budget applies. Do not
add a manual budget unless a measured run proves the default insufficient; then
use `setTestBudget(ms)` as the test's first line per `jaunder-e2e`.

In the editor scheduling scenario, after the `Edit Post` heading settles, add
the same semantic association/absence assertions (layout is already pinned on
the shared composer rendering):

```ts
const slug = page.locator('input[name="slug_override"]');
const slugLabel = page.locator(
  'label.j-field-row:has(input[name="slug_override"])',
);
await expect(slugLabel).toContainText("Slug");
await expect(slug).not.toHaveAttribute("id", /.+/);
await expect(slugLabel).not.toHaveAttribute("for", /.+/);

const schedule = page.locator('input[name="publish_at"]');
const scheduleLabel = page.locator(
  'label.j-field-label:has(input[name="publish_at"])',
);
await expect(scheduleLabel).toContainText("Publish at (optional)");
await expect(schedule).not.toHaveAttribute("id", /.+/);
await expect(scheduleLabel).not.toHaveAttribute("for", /.+/);
```

Use `schedule` for the editor scenario's existing `fill`. Rewrite both stale
comments to describe the `name="publish_at"` selector and implicit-label
contract, not an id. Remove this scenario's existing `test.slow()` under the
same ambient-budget rule.

In the existing published-editor assertion beside the absent slug control, pin
the shared conditional-rendering gate for both controls:

```ts
await expect(page.locator('input[name="slug_override"]')).not.toBeVisible();
await expect(page.locator('input[name="publish_at"]')).not.toBeVisible();
```

- [x] **Step 2: Run the focused suite and verify the contract fails**

Run: `devtool run -- cargo xtask e2e-local posts.spec.ts`

Expected: FAIL because the body error is still a `p`, the option labels are not
wrappers, and the schedule has no `name="publish_at"`.

- [x] **Step 3: Implement the Leptos markup against the tests**

In `forms::Labelled`, replace only the nested touched-gated error node:

```rust
.map(|msg| view! { <span class="error">{msg}</span> })
```

Add `display: block` to the existing `.error` CSS rule so both label-contained
spans keep the former paragraphs' presentation without duplicated inline style.

In `ComposeOptions`:

- replace the slug's outer `.j-field-row` `div` with a `label` carrying the same
  class and inline grid override;
- replace the old slug label with `<span class="j-field-label">"Slug"</span>`;
- remove slug `for`/`id`, retaining all input attributes and handlers;
- render its touched-gated error with the same block-displayed `span.error` as
  `Labelled`;
- keep the schedule's outer margin `div`, make `.j-field-label` wrap its visible
  text and input, remove `for`/`id`, and add `name="publish_at"`;
- update the component rustdoc from “one id prefix” to the now-true shared-shape
  description.

- [x] **Step 4: Run focused verification and source-contract searches**

Run: `devtool run -- cargo xtask e2e-local posts.spec.ts`

Expected: PASS.

Run:
`rg -n 'options-(slug|publish-at)|j-composer-field p\.error' web/src end2end/tests || true`

Expected: no matches. Historical archived docs are deliberately outside this
search.

Run:
`rg -n -U '<label[^>]*>[\s\S]{0,1200}<p class="error"' web/src/forms/component.rs web/src/posts/component.rs || true`

Expected: no matches.

- [x] **Step 5: Run the per-commit gate**

Follow `jaunder-commit` exactly.

Run: `devtool run -- cargo xtask check`

Expected: PASS with `xtask-done: command=check ok=true exit=0`; if fix mode
changes files, inspect and stage those changes, then rerun until clean.

- [x] **Step 6: Commit the checked tree**

```bash
git add docs/superpowers/specs/2026-08-12-issue-871-implicit-labels.md
git add docs/superpowers/plans/2026-08-13-issue-871-implicit-labels.md
git add end2end/tests/posts.spec.ts
git add server/assets/jaunder.css
git add web/src/forms/component.rs
git add web/src/posts/component.rs
git commit -m "fix(web): use implicit labels for compose options"
```

Verify the commit contains exactly the staged tree that passed the gate; never
use a commit pathspec.
