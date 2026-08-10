# Spec — `ComposeOptions`: collapse the duplicated options aside (#863)

Issue: [#863](https://github.com/jaunder-org/jaunder/issues/863) Milestone: Web:
canonical Leptos CSR convergence

## Problem

`FullComposer` (`web/src/posts/component.rs:609`) and `EditPostForm` (`:1146`)
each render an options aside. The two are near-identical and adjacent in one
file, so a change to the options column means editing both or silently diverging
them.

## What was measured

The issue estimates "~40 lines" of verbatim duplication. Measuring the two
asides found the duplication is larger than that but **not contiguous**, because
the two shapes order one field differently:

|                                  | composer (`FullComposer`)                                   | editor (`EditPostForm`)                                                    |
| -------------------------------- | ----------------------------------------------------------- | -------------------------------------------------------------------------- |
| order inside the Options `<div>` | head, slug, summary, tags, audience, **publish-at**, format | head, [drafts only: slug, **publish-at**], summary, tags, audience, format |
| slug input                       | `id="compose-slug"`, `placeholder="auto"`                   | `id="edit-slug"`, no placeholder                                           |
| publish-at input                 | `id="compose-publish-at"`                                   | `id="edit-publish-at"`                                                     |
| slug/schedule shown              | always                                                      | only when `!is_published`                                                  |
| trailing actions                 | inline `Save draft` / `Publish` pair                        | `EditSaveActions` in `.j-edit-form-actions`                                |
| `<aside>` class                  | `j-compose-aside`                                           | `j-edit-form-aside`                                                        |

Everything else — the "Options" heading, the summary `ValidatedTextarea`,
`TagInput`, `AudiencePicker`, `FormatToggle`, and the entire Media block — is
byte-identical.

The single largest duplicated block is the **slug input (~24 lines)**, which the
issue did not call out; it differs only by `id` and `placeholder`.

## Decisions

### D1 — Normalize the field order; one `ComposeOptions`

The composer's `publish-at` moves up to sit directly after its slug field,
matching the editor. This is a deliberate, user-visible DOM reorder, taken
because it is what turns three disjoint fragments into one component.

Once reordered, the composer's Options `<div>` is **spacing-identical** to the
editor's draft branch: every inline `margin-top:10px` lands on the same element,
the leading `j-sb-head` keeps `padding:0 0 10px`, and the slug `j-field-row`
remains a non-first child (so `.j-field-row:first-child { border-top: none }`,
`jaunder.css:856`, still does not apply to it). No CSS rule changes behaviour.

_Rejected:_ keeping both orders behind a `schedule_after_audience` flag — a
parameter that exists only to preserve an inconsistency nobody defends.

### D2 — Boundary: the options column only; Media is its own component

`ComposeOptions` spans the "Options" heading through the format toggle. The
Media block becomes a separate `MediaSection` component. Each parent keeps its
own `<aside>` element and its own trailing actions.

**Each new component owns its wrapping `<div>`, and emits exactly one flex
child.** Both asides are `display:flex; flex-direction:column; gap:18px`
(`jaunder.css:735-743` and `:1068-1076`) with exactly three children today: the
Options `<div>`, the Media `<div style="margin-top:16px">`, and the action row.
A component that emitted a bare fragment of siblings instead would give the
aside nine children and put an 18px gap between every field — so the wrappers
are load-bearing, not incidental.

_Rejected:_ one component swallowing the whole aside, including the trailing
actions. This is not prohibited — ADR-0083 §3 forbids a `ViewFn`/closure prop,
but §4 explicitly endorses caller chrome arriving as `children`, and
`web/src/forms/component.rs:11` does exactly that. The reason is cohesion: "the
options fields" and "the action row" have separate reasons to change, and the
editor's actions are already their own component (`EditSaveActions`).

Both parents keep their action row as the **last** flex child of the aside,
because `margin-top:auto` (composer, `component.rs:704`) and
`.j-edit-form-actions` (`jaunder.css:1125`) bottom-pin only from that position.

Both new components live in `web/src/posts/component.rs` alongside the 32
already there; no new module, so no new `#[cfg(target_arch = "wasm32")]` gate is
needed.

Note: `j-compose-aside` and `j-edit-form-aside` have property-identical CSS.
Collapsing them to one class is a separate cleanup, out of scope here.

### D3 — Unify the ids and the placeholder

One component cannot emit two id sets. Both pages emit `id="options-slug"` and
`id="options-publish-at"`, and both slug inputs get `placeholder="auto"`. The
two shapes never render on the same page, so the `compose-`/`edit-` prefixes
distinguished nothing.

The editor's slug input gaining `placeholder="auto"` is a behaviour improvement,
not just a side effect: the editor's slug is also auto-derived when left blank,
so the placeholder was missing there by omission.

_Rejected:_ an `id_prefix` prop — `web/src/forms/component.rs:8-11` documents
that this codebase deliberately avoids `for=`/`id=` pairs rather than
parameterizing them, so a prefix prop would entrench the pattern the forms crate
moved away from.

_Not adopted here:_ dropping ids entirely in favour of implicit `<label>`
wrapping (the forms-crate convention). It is the right long-term direction but
restructures label markup the issue did not ask about.

### D4 — Gate prop is `is_published: bool`

One condition governs both the slug and the schedule field, so one prop. It uses
domain vocabulary and is true of both callers: the composer passes `false` (a
post being composed is not yet published), the editor passes its own flag.
`EditSaveActions` (`component.rs:1266`) already takes `is_published`, so the
vocabulary is consistent.

The guard stays `(!is_published).then(|| …)` — the exact construct
`EditPostForm` uses today, moved rather than rewritten. `is_published` is a
plain `bool`, not a signal, so `<Show>` is not applicable.

### D5 — `slug_field` stays a prop; the stale doc comments are corrected

`ComposeOptions` takes `slug_field: Field<Slug>`. Slug does **not** move into
`ComposeState`, because a third consumer — `CompactComposer`, which calls
`state.inputs(publish, None)` at `component.rs:546` — has no slug field and
never renders one.

The doc comments that justify the split today say the slug is "local to that
shape". This change makes that false, since both full shapes now hand the field
to the same component. They are rewritten to cite the actual reason: the compact
shape has no slug. Four comments are affected — `compose_state.rs:96-99`,
`component.rs:1149-1150`, `component.rs:606-608` (`FullComposer`'s doc, which
also enumerates the old field order), and `component.rs:1123`
(`// The slug is not part of the bundle`). `component.rs:1143-1144`
(`EditPostForm`'s doc) also describes markup the component no longer contains
and is updated with them.

### D6 — No ADR

Nothing here establishes a new cross-cutting rule. The governing decisions
already exist (ADR-0086 thin components; ADR-0083 §3–§4 on how per-page
variation travels); this is their local application.

## Acceptance criteria

Numbered so the ship-time conformance review can check each one.

**Structure**

- **AC1** `ComposeOptions` exists as a `#[component]` in
  `web/src/posts/component.rs` (not a plain `fn -> impl IntoView`, per ADR-0086
  §4) taking exactly `state: ComposeState`, `slug_field: Field<Slug>`,
  `is_published: bool`. It emits a **single** root `<div>` — the Options wrapper
  — containing the heading, the gated slug + publish-at pair, summary, tags,
  audience, and format toggle.
- **AC2** `MediaSection` exists as a `#[component]` in the same file and emits a
  **single** root `<div style="margin-top:16px">` containing the "Media"
  `j-sb-head` and `<MediaUpload show_result=true />`.
- **AC3** Both `FullComposer` and `EditPostForm` render `<ComposeOptions …/>`
  followed by `<MediaSection/>`, and neither contains any inline slug, summary,
  tag, audience, publish-at, format-toggle, or media markup of its own —
  including the two wrapper `<div>`s, which move into the components.
- **AC4** Each `<aside>` has exactly three children in order:
  `<ComposeOptions/>`, `<MediaSection/>`, the action row — so the action row
  stays last and the aside's `gap:18px` still applies only between those three.
- **AC5** `cargo xtask check` passes, including `thin-components`:
  `ComposeOptions`, `MediaSection`, `FullComposer`, and `EditPostForm` are each
  within the budget of 2 control-flow units per surface.

**Rendered output**

- **AC6** On `/posts/new` the Options column renders, in order: "Options"
  heading, slug, publish-at, summary, tags, audience, format toggle.
- **AC7** On `/posts/:id/edit` for a **draft**, the Options column renders the
  same _element sequence_ as AC6 — same tags, ids and classes in the same order.
  Field _values_ differ (the editor seeds them from the post); only structure is
  asserted.
- **AC8** On `/posts/:id/edit` for a **published** post, the slug and publish-at
  inputs are absent and the rest is unchanged.
- **AC9** Both pages emit `id="options-slug"` (with `name="slug_override"` and
  `placeholder="auto"`) and `id="options-publish-at"`; no `compose-slug`,
  `edit-slug`, `compose-publish-at`, or `edit-publish-at` remains in `web/src`.
  Each `<label for=>` matches its input's new id. The sweep is scoped to
  `web/src` and `end2end` deliberately: `xtask/src/steps/thin_components.rs:455`
  and two `docs/archive/` files contain `edit-slug` as test-fixture and
  historical text and must **not** be changed.
- **AC10** Every other selector the e2e suite keys on inside the aside is
  unchanged: `#audience-base`, `name="slug_override"`,
  `textarea[name="summary"]`, the `TagInput` classes (`.j-tag-text`,
  `.j-tag-chip-label`, `.j-tag-chip-remove`, `.j-tag-suggest*`, `.j-tag-error`),
  and `.j-seg button` for the format toggle. These live inside child components
  and so survive the move by construction; AC10 is the check that nothing was
  rewritten in passing.

**Documentation**

- **AC11** None of the five doc comments listed in D5 justifies slug's placement
  by locality or describes the old field order; each cites the compact shape or
  the new structure as appropriate.
- **AC12** #863's **body** is edited to replace the "No markup change: the
  rendered output of both shapes must be identical" constraint with the three
  approved DOM changes (D1 reorder, D3 ids, D3 placeholder) and the reason each
  was accepted — the owner approved them during this cycle's design interview.
  Editing the body, not just appending a comment, is what stops a later reader
  hitting a constraint the merged code violates.

## Verification

`web/src/posts/component.rs` is wasm-only (`web/src/posts/mod.rs:17`), and the
repo has no render-to-string harness — CONTRIBUTING.md:715-717 states component
bodies are "validated by the e2e matrix — not host-compiled". So **e2e is the
only observable check** on AC6-AC10; there is no host test to add for them.

- Tight loop: `cargo xtask e2e-local posts.spec.ts` (chromium, temp SQLite).
- Must stay green, all touching the options aside: `posts.spec.ts` (summary
  :41/:68/:86, slug :124/:280, audience :207, tags :733/:786/:870-:948, schedule
  :986), `media.spec.ts:163`, `visibility.spec.ts:39/:240`,
  `unicode-slug.spec.ts:10/:36`.
- **AC13** `posts.spec.ts:1003`'s `page.fill("#compose-publish-at", …)` is
  updated to `#options-publish-at`, the stale comment at `posts.spec.ts:992`
  naming the old id is updated with it, and the test still passes.
- **AC14** A new test in `end2end/tests/posts.spec.ts` **interacts** with the
  editor's draft schedule control: open an existing draft's edit page,
  `page.fill` on `#options-publish-at` with a future time, save, and assert the
  post shows the Scheduled-for badge on the drafts page — mirroring the existing
  composer-side test at `:986`. A visibility-only assertion is insufficient:
  filling by id is what proves the `edit-publish-at` → `options-publish-at`
  rename actually landed on the editor, which no test covers today.
- Full gate before the PR: `cargo xtask validate`.

## Out of scope

- Moving `slug_field` into `ComposeState` (D5).
- Converting these fields to the forms crate's implicit-`<label>` convention
  (D3).
- Adding a `/posts/new` format-toggle e2e test — a pre-existing gap, unrelated
  to this change.
