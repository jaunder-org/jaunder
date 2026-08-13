# Shared Full-Compose Layout Classes — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the full Post composer and Post editor consume one grid class and
one aside class without changing layout or behavior.

**Architecture:** Keep both Leptos renderers structurally distinct and perform a
clean CSS-token cutover at their shared layout seam. Reuse the existing
`j-compose-*` rules, delete only their property-identical `j-edit-form-*`
duplicates, and verify the unchanged surfaces through source contracts and the
existing Post browser suite.

**Tech Stack:** Rust, Leptos `view!`, CSS, Playwright, `cargo xtask`.

**Spec:**
[`2026-08-13-issue-872-shared-compose-layout.md`](../specs/2026-08-13-issue-872-shared-compose-layout.md)

## Global Constraints

- Canonical shared classes are exactly `j-compose-grid` and `j-compose-aside`.
- Preserve every canonical grid/aside declaration and body-before-aside child
  order.
- Preserve every other class, rule, element, child, and handler in both
  renderers.
- Do not add e2e assertions coupled to CSS class names.
- Do not edit historical documents under `docs/archive/`.
- Do not add a glossary entry, ADR, dependency, compatibility alias, or retired
  selector group.

## Review

**Scope — in:** the two wrapper tokens in `EditPostForm`; the two duplicate CSS
rules; the compose-section comment; source-contract searches; existing Post e2e
coverage; required gate and commit.

**Scope — out:** body/field/textarea/action class consolidation; component
extraction; markup restructuring; layout values; responsive behavior; archived
docs; new class-token tests.

**Tasks:**

1. Cut both surfaces over to the canonical wrapper classes, delete the duplicate
   CSS rules, verify the pure rename, and commit the checked tree.

**Key risks/decisions:**

- `j-compose-*` is intentionally shared vocabulary, not create-only vocabulary;
  it already matches `ComposeState`, `ComposeOptions`, and `ComposerFields`.
- The grid child order is load-bearing because `1fr 320px` assigns body first
  and aside second; edit only class attributes and CSS definitions.
- No red test is appropriate: class tokens are implementation details and the
  specification explicitly rejects adding selector-coupled assertions. The
  existing browser suite protects behavior; exact source searches protect the
  clean cutover.

## File structure

- `web/src/posts/component.rs` — renders both full-page Post forms; change only
  `EditPostForm`'s outer grid and aside class values.
- `server/assets/jaunder.css` — owns all bespoke web classes; retain the
  canonical compose rules, update their section comment, and remove only the
  duplicate edit-form grid/aside rules.
- `docs/superpowers/specs/2026-08-13-issue-872-shared-compose-layout.md` —
  approved behavioral contract; no implementation edits expected.
- `docs/superpowers/plans/2026-08-13-issue-872-shared-compose-layout.md` — this
  execution checklist; tick steps as they complete.

---

### Task 1: Consolidate the full-compose layout classes

**Files:**

- Modify: `web/src/posts/component.rs:1122-1145`
- Modify: `server/assets/jaunder.css:725-753,1062-1086`
- Verify: `end2end/tests/posts.spec.ts`

**Interfaces:**

- Consumes: the existing `FullComposer` and `EditPostForm` DOM structure and
  canonical `.j-compose-grid` / `.j-compose-aside` declarations.
- Produces: both renderers use the canonical class pair; the live stylesheet
  defines that pair once and contains neither retired edit-form wrapper name. No
  Rust signature, state, handler, or test interface changes.

- [x] **Step 1: Record the pre-change source contract**

Run the Grep tool with:

- pattern: `j-(compose|edit-form)-(grid|aside)`
- path: `web/src;server/assets;end2end`

Expected: exactly eight matches across `web/src/posts/component.rs` and
`server/assets/jaunder.css`: one call site and one stylesheet rule for each of
the four names, with no e2e consumer.

This is the baseline rather than a failing test: the spec deliberately keeps CSS
class tokens out of Playwright contracts.

- [x] **Step 2: Change only the edit renderer's wrapper tokens**

In `EditPostForm`, make these exact substitutions:

```rust
<div class="j-compose-grid">
    <div class="j-edit-form-body">
        // existing ComposerFields unchanged
    </div>
    <aside class="j-compose-aside">
        // existing ComposeOptions, MediaSection, and actions unchanged
    </aside>
</div>
```

Do not reorder or retype any child. Do not rename `j-edit-form-body`,
`j-edit-form-field`, `j-edit-form-textarea`, or `j-edit-form-actions`.

- [x] **Step 3: Remove only the duplicate CSS rules**

Keep the `.j-compose-grid` and `.j-compose-aside` rule bodies byte-for-byte
unchanged. Update the `/* compose page */` section comment to state that its
grid and aside are shared by the Post editor. Delete the complete
`.j-edit-form-grid { ... }` and `.j-edit-form-aside { ... }` blocks; leave every
other edit-section rule in its current order with unchanged declarations.

- [x] **Step 4: Verify the source contract after cutover**

Run the Grep tool with:

- pattern: `j-(compose|edit-form)-(grid|aside)`
- path: `web/src;server/assets;end2end`

Expected: exactly six matches, all canonical — each `j-compose-*` name appears
at both renderer call sites and once in the stylesheet; neither retired
`j-edit-form-*` name appears.

Run a second Grep with:

- pattern: `^\\.j-compose-(grid|aside) \\{$`
- path: `server/assets/jaunder.css`

Expected: exactly two matches, proving one live rule for each canonical class.
Historical matches under `docs/archive/` are expected and must not be edited.

Capture the focused diff:

```bash
devtool run -- git diff -- web/src/posts/component.rs server/assets/jaunder.css
```

Read the parked stdout and confirm the diff changes only the two class values,
the compose-section comment, and deletion of the two duplicate rule blocks.

- [x] **Step 5: Exercise both surfaces in a browser**

Run:

```bash
devtool run -- cargo xtask e2e-local posts.spec.ts
```

Expected: PASS. The existing suite exercises the full composer and Post editor
without any selector migration or new CSS-token assertion.

- [x] **Step 6: Run the per-commit gate**

Follow `jaunder-commit` exactly. Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS with `xtask-done: command=check ok=true exit=0`. If fix mode
changes files, inspect and stage those changes, then rerun until the tree is
clean and the checked tree is the tree to commit.

- [x] **Step 7: Commit the checked tree**

Mark every completed step in this plan, then stage exactly:

```bash
git add docs/superpowers/specs/2026-08-13-issue-872-shared-compose-layout.md
git add docs/superpowers/plans/2026-08-13-issue-872-shared-compose-layout.md
git add web/src/posts/component.rs
git add server/assets/jaunder.css
git commit -m "refactor(web): share full-compose layout classes"
```

Do not use a commit pathspec. Verify the commit contains exactly the staged tree
that passed the gate; do not add a `Co-Authored-By` trailer.
