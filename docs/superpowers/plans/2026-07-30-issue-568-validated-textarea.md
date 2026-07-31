# `ValidatedTextarea<T>` Implementation Plan (#568)

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-07-30-issue-568-validated-textarea.md` —
read it for the _what/why_; this plan is the _how_ and does not restate it.

**Goal:** Collapse the four hand-wired `<textarea>` direct-bind sites onto one
shared `ValidatedTextarea<T>` widget, extracting the chrome `ValidatedInput`
already owns so both widgets share it.

**Architecture:** A private **non-generic** `Labelled` owns the wrapper
`<label>`, label span, help span, and touched-gated error node, taking two
erased signals (`error`, `touched`) plus `children`. `ValidatedInput<T>` and a
new `ValidatedTextarea<T>` become thin shells that supply only their own control
element. All adopting sites converge on the `j-form-*` chrome, which removes the
emitted `id`/`for=` pairs entirely.

**Tech Stack:** Rust, Leptos 0.8.2 (`#[component]`, `view!`), Playwright (e2e),
`cargo xtask` (gate).

## Global Constraints

- **These components never host-compile.** `web/src/forms/component.rs` is
  behind `#[cfg(target_arch = "wasm32")]` (`forms/mod.rs:9-10`). There is **no
  host unit test** for any of them, so this plan has **no red-green cycle** for
  the component code — it is a refactor whose contract is the **existing** e2e
  suite, which must keep passing. Do not invent host tests for `#[component]`
  code; do not add `#[cfg(test)]` blocks to `component.rs`.
- **ADR-0065 single validation source:** validity flows through
  `Field::error_for` / `field_error::<T>`. Never re-implement a newtype's rule
  client-side.
- **No `Co-Authored-By` trailer** on any commit.
- **Per-commit gate:** the pre-commit hook runs the full `cargo xtask check`.
  Run `cargo xtask check` yourself first so the commit passes clean
  (**`jaunder-commit`**).
- **Wasm clippy is the only compiler check that sees this code.** It runs inside
  `cargo xtask check` (`xtask/src/steps/static_checks.rs:228-247`). A plain
  `cargo check` proves nothing here.
- **`Labelled` must stay non-generic.** See Key risks — this is a correctness
  constraint, not a style preference.
- **leptosfmt relocates comments inside `view!`** — put explanatory comments
  _above_ the `view!` macro, not inside it.
- Chrome class strings are exact: `j-form-field`, `j-form-label`, `j-form-help`,
  `j-form-body`, `j-form-actions`, `j-card`, `j-card-head`, `j-sub`, `error`.

---

## Review header

**Scope (in):** `web/src/forms/{component.rs,mod.rs}`; the three posts summary
sites and the profile page; the e2e selectors that key on the ids being removed;
ADR-0065's two stale bullets.

**Scope (out):** the six remaining hand-wired `<input>` sites (#450); adding
`placeholder` to `ValidatedInput` (#450); `ComposerFields` / the post body
textarea; moving the error node out of the `<label>` (deferred — Task 1).

**Tasks:**

1. File two follow-up issues; commit the planning docs.
2. Extract `Labelled`; refactor `ValidatedInput` onto it. No behaviour change.
3. Add `ValidatedTextarea<T>`, export it, update the module doc.
4. Adopt at the three posts summary sites; swap the e2e selectors; add
   `SEL.postSummary`.
5. Profile: card treatment, `display_name` → `ValidatedInput`, bio →
   `ValidatedTextarea`.
6. `DefaultPostFormatControl` → its own card. _(Droppable — see Key risks.)_
7. Amend ADR-0065's coverage-boundary and direct-bind bullets.
8. Full `cargo xtask validate`.

**Key risks / decisions:**

- **`Labelled` is non-generic on purpose, and must stay that way.** Taking
  `Field<T>` directly would be a tidier signature, but it would make `Labelled`
  the repo's first _generic component with children_. All 13 existing
  generic-component call sites are self-closing (`auth/component.rs:47`, `:56`;
  `site:85`, `:94`; `backup:118`, `:128`; `invites:44`, `:53`;
  `registration:91`, `:100`; `email:41`; `password_reset:27`, `:87`). A generic
  tag with a close tag must match its opening generics **token-for-token**
  (`rstml-0.12.1/src/node/parse.rs:241-253`; `syn`'s `Punctuated` equality makes
  `<T,>` ≠ `<T>`, `syn-2.0.119/src/punctuated.rs:417-426`) — and every generic
  tag in this repo is leptosfmt-formatted _with_ a trailing comma, while
  `cargo xtask check` runs the formatter in fix mode. A formatter pass could
  therefore unbalance a hand-matched pair after the code compiled. Erasing to
  two signals costs one `Signal::derive` line per shell and stays on constructs
  the repo already uses everywhere.
- **`help` must be `#[prop(optional_no_strip)]` on `Labelled`.** A plain
  `#[prop(optional)]` on an `Option<_>` becomes typed-builder `strip_option`
  (`leptos_macro-0.8.17/src/component.rs:1033`), so the setter takes the _inner_
  type and no `Option`-accepting setter is generated. Both shells hold `help` as
  `Option<&'static str>` and forward it, so the stripped setter is a type error.
  `optional_no_strip` is at `component.rs:1003-1006`.
- **Task 6 is the flagged scope addition,** deliberately its own commit so it
  can be dropped with `git rebase --onto`. If vetoed, skip it — but AC8's final
  clause ("no `j-field-*` remains in profile") is satisfied _only_ by this task,
  since `DefaultPostFormatControl` is that page's last holder of those classes.
  Dropping Task 6 means amending AC8, not silently failing it.
- **Task 4 must be atomic.** Removing the ids breaks seven live selectors, so
  the markup change and the selector swap land in the same commit or the suite
  is red between commits. No other task has this coupling.
- **The `on:input` handler is deliberately NOT extracted.** `ValidatedInput`
  additionally applies `transform`, and a shared helper would have to name the
  concrete `web_sys` event type for a three-line saving. AC2 requires the _gate_
  and the chrome markup be centralised, which `Labelled` does; the two
  `on:input` closures staying inline is intended.

---

### Task 1: File the deferred follow-ups; commit the planning docs

Two things surfaced during design that are **not** this issue, plus the planning
docs need to be in the tree before Task 8 can run (`cargo xtask validate`
refuses a dirty working tree — `xtask/src/lib.rs:364-372`).

**Files:**

- Commit: `docs/superpowers/specs/2026-07-30-issue-568-validated-textarea.md`,
  `docs/superpowers/plans/2026-07-30-issue-568-validated-textarea.md`

**Interfaces:** none.

- [x] **Step 1: File the e2e-coverage gap** via **`jaunder-issues`** — filed as
      **#735**

Title:
`test(e2e): cover the compact composer's summary field (/app inline composer)`

Body, in essence: the `/app` inline composer (`web/src/posts/component.rs`,
`InlineComposer` at `:765`) renders a summary `<textarea>` bound to
`Field::<PostSummary>::optional()` whose validity gates both the "Save draft"
and "Publish" buttons, but no e2e exercises it — every existing
`#compose-summary` test drives `/posts/new`, the non-compact branch
(`:612-695`). Add coverage mirroring `posts.spec.ts:63` ("over-long post summary
shows an inline error and gates submit") against `/app`. Discovered while doing
#568; not a regression. Label `web`; milestone unset for triage.

- [x] **Step 2: File the invalid-HTML gap** via **`jaunder-issues`** — filed as
      **#736**

Title:
`fix(web/forms): the validated-field error node is invalid HTML inside <label>`

Body, in essence: `ValidatedInput` renders its `<p class="error">` **inside**
the wrapping `<label>` (`web/src/forms/component.rs:78`), but `<label>` accepts
only phrasing content, so a `<p>` there is invalid. Pre-existing across all 13
`ValidatedInput` sites; #568 moves the shared chrome into `Labelled` without
changing it, and adopts it at five more fields. Fixing it means emitting the
error as a **sibling** of the `<label>` (which is what
`web/src/profile/component.rs:69-75` does today) — safe for e2e, since
`SEL.error` is the position-independent `.error`, but it shifts every field's
error spacing from `.j-form-field`'s `gap:7px` to the parent container's gap, so
it is a visible change wanting its own review. Label `web`; milestone unset for
triage.

- [x] **Step 3: Commit the planning docs** — `1d48bc63`

```bash
git add docs/superpowers/specs/2026-07-30-issue-568-validated-textarea.md docs/superpowers/plans/2026-07-30-issue-568-validated-textarea.md
git commit -m "docs(plan): spec + plan for ValidatedTextarea<T> (#568)"
```

Run
`devtool run -- prettier -w docs/superpowers/specs/2026-07-30-issue-568-validated-textarea.md docs/superpowers/plans/2026-07-30-issue-568-validated-textarea.md`
first — the pre-commit hook runs prettier and would otherwise restage
mid-commit.

- [x] **Step 4: Confirm the tree is clean**

Run: `git status --porcelain` Expected: **empty**. Task 8's `validate` cannot
run otherwise.

---

### Task 2: Extract `Labelled` and refactor `ValidatedInput` onto it

**Files:**

- Modify: `web/src/forms/component.rs:1-82` (whole file)

**Interfaces:**

- Consumes: `Field<T>` from `web/src/forms/field.rs` —
  `field.value: RwSignal<String>`, `field.error: RwSignal<Option<String>>`,
  `field.error_for(&str) -> Option<String>`, `field.is_touched() -> bool`,
  `field.touch()`. `Field<T>` is unconditionally `Copy` (`field.rs:38`).
- Produces: `Labelled` — **private** to `web/src/forms/component.rs`, consumed
  by Task 3.

```rust
#[component]
fn Labelled(
    label: &'static str,
    name: &'static str,
    field_class: &'static str,
    #[prop(into)] error: Signal<Option<String>>,
    touched: Signal<bool>,
    #[prop(optional_no_strip)] help: Option<&'static str>,
    children: Children,
) -> impl IntoView;
```

`ValidatedInput<T>`'s public prop surface is **unchanged** — `label`, `name`,
`field`, `input_type`, `autocomplete`, `field_class`, `class`, `help`,
`transform`, same defaults. Its **13 existing call sites across 7 files** must
compile untouched: `auth/component.rs:47`, `:56`; `site:85`, `:94`;
`backup:118`, `:128`; `invites:44`, `:53`; `registration:91`, `:100`;
`email:41`; `password_reset:27`, `:87`.

- [x] **Step 1: There is no failing test to write — record why**

This task changes no behaviour and the code cannot be host-tested (see Global
Constraints). Its contract is that the **existing** e2e for the current
`ValidatedInput` consumers keeps passing, which Step 4 checks. Skip straight to
the implementation.

- [x] **Step 2: Write `Labelled`**

Add to `web/src/forms/component.rs`, above `ValidatedInput`. `Children`,
`Signal`, and `Signal::derive` all come from `leptos::prelude::*`, already
imported.

```rust
/// The chrome shared by every ADR-0065 validated field: the wrapping `<label>` (which
/// gives the control *implicit* label association, so no `for=`/`id=` pair is emitted
/// and none can drift), the label text, an optional help line, and the touched-gated
/// inline message. The control itself is supplied by the caller as `children`.
///
/// Deliberately NOT generic over the field's `T`: a generic component with children would
/// need its close tag's generics to match the open tag token-for-token, and leptosfmt
/// (which `cargo xtask check` runs in fix mode) formats generic tags with a trailing comma
/// — so a formatter pass could unbalance the pair. Taking the validity as two erased
/// signals keeps the gate in one place without that hazard.
#[component]
fn Labelled(
    label: &'static str,
    name: &'static str,
    /// The wrapping `<label>`'s class — always supplied by the shell, which owns the
    /// `j-form-field` default, so a caller can slot the field into a bespoke layout.
    field_class: &'static str,
    /// The field's true validity (`None` = valid), independent of whether it is shown.
    #[prop(into)]
    error: Signal<Option<String>>,
    /// Whether the field has been blurred — gates only whether `error` is *displayed*.
    touched: Signal<bool>,
    /// Optional hint line rendered under the control and wired to it via
    /// `aria-describedby` (id `{name}-help`), for a field whose format needs explaining.
    #[prop(optional_no_strip)]
    help: Option<&'static str>,
    children: Children,
) -> impl IntoView {
    view! {
        <label class=field_class>
            <span class="j-form-label">{label}</span>
            {children()}
            {help
                .map(|text| {
                    view! {
                        <span id=format!("{name}-help") class="j-form-help">
                            {text}
                        </span>
                    }
                })}
            {move || {
                touched
                    .get()
                    .then(|| error.get())
                    .flatten()
                    .map(|msg| view! { <p class="error">{msg}</p> })
            }}
        </label>
    }
}
```

`error=field.error` converts via `From<RwSignal<T>> for Signal<T>`
(`reactive_graph-0.2.14/src/wrappers.rs:853`), which is why `error` carries
`#[prop(into)]` and `touched` does not.

- [x] **Step 3: Rewrite `ValidatedInput<T>` to delegate**

Replace the body of `ValidatedInput` (currently `component.rs:52-81`) so it
emits **only** the `<input>` and hands the chrome to `Labelled`. Keep the doc
comment, the prop list, the `on_input` closure, and the `describedby` derivation
exactly as they are.

```rust
    view! {
        <Labelled
            label=label
            name=name
            field_class=field_class
            error=field.error
            touched=Signal::derive(move || field.is_touched())
            help=help
        >
            <input
                class=class
                type=input_type
                name=name
                autocomplete=autocomplete
                aria-describedby=describedby
                prop:value=field.value
                on:input=on_input
                on:blur=move |_| field.touch()
            />
        </Labelled>
    }
```

The `<input>`'s attribute set is unchanged from `component.rs:55-64` — `class`,
`type`, `name`, `autocomplete`, `aria-describedby`, `prop:value`, `on:input`,
`on:blur`. Only its surrounding chrome moved.

The `describedby` binding (`help.map(|_| format!("{name}-help"))`) stays in
`ValidatedInput`: the attribute belongs on the control, while the help span's id
is derived in `Labelled`. Both derive it from `name` independently — leave a
comment above the `view!` saying so.

- [x] **Step 4: Verify it compiles for wasm and behaves identically** —
      `auth.spec.ts` 12 passed, `backup.spec.ts` 4 passed

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS (this includes
the wasm-target clippy step).

Run: `devtool run -- cargo xtask e2e-local auth.spec.ts` Expected: PASS — login
uses two `ValidatedInput`s, proving the extraction preserved the chrome and the
touched-gate.

Run: `devtool run -- cargo xtask e2e-local backup.spec.ts` Expected: PASS —
backup is the **only** consumer that passes `help=` (`backup/component.rs:126`),
the exact prop the `optional_no_strip` decision governs. Skipping this leaves
that path unexercised.

- [x] **Step 5: Commit** — `ab0b20df`

```bash
git add web/src/forms/component.rs
git commit -m "refactor(web/forms): extract Labelled chrome from ValidatedInput (#568)"
```

Run `devtool run -- cargo xtask check` first so the hook passes clean
(**`jaunder-commit`**).

---

### Task 3: Add `ValidatedTextarea<T>`

**Files:**

- Modify: `web/src/forms/component.rs` (append after `ValidatedInput`)
- Modify: `web/src/forms/mod.rs:6-7` (module doc), `:13-14` (re-export)

**Interfaces:**

- Consumes: `Labelled` from Task 2.
- Produces — the widget every later task calls:

```rust
#[component]
pub fn ValidatedTextarea<T>(
    label: &'static str,
    name: &'static str,
    field: Field<T>,
    #[prop(default = 3)] rows: u32,
    #[prop(optional)] placeholder: Option<&'static str>,
    #[prop(default = "j-form-field")] field_class: &'static str,
    #[prop(default = "j-form-input")] class: &'static str,
    #[prop(optional)] help: Option<&'static str>,
) -> impl IntoView
where
    T: FromStr + 'static,
    T::Err: Display;
```

- [x] **Step 1: Write `ValidatedTextarea<T>`**

```rust
/// The multi-line sibling of [`ValidatedInput`]: a labelled `<textarea>` bound to a
/// [`Field<T>`], validating on input and showing the newtype's own message inline once
/// the field is touched (blur). `name` MUST match the `#[server]` struct field and the
/// e2e selector.
///
/// No `id` prop: the wrapping `<label>` from [`Labelled`] associates the control
/// implicitly. No `transform` prop: nothing multi-line needs live input massaging.
#[component]
pub fn ValidatedTextarea<T>(
    label: &'static str,
    name: &'static str,
    field: Field<T>,
    /// Visible rows. Defaults to 3 — the browser default of 2 is too short for the
    /// summary/bio fields this serves.
    #[prop(default = 3)]
    rows: u32,
    #[prop(optional)] placeholder: Option<&'static str>,
    #[prop(default = "j-form-field")] field_class: &'static str,
    #[prop(default = "j-form-input")] class: &'static str,
    #[prop(optional)] help: Option<&'static str>,
) -> impl IntoView
where
    T: FromStr + 'static,
    T::Err: Display,
{
    let on_input = move |ev| {
        let v = event_target_value(&ev);
        field.value.set(v.clone());
        field.error.set(field.error_for(&v));
    };
    // Only wire `aria-describedby` when a help line is actually rendered (its id must
    // resolve). Derived from `name` here and again in `Labelled` — the attribute belongs
    // on the control, the span lives in the chrome, and Leptos `children` is opaque so
    // the id cannot be handed down without a render-prop shape.
    let describedby = help.map(|_| format!("{name}-help"));
    view! {
        <Labelled
            label=label
            name=name
            field_class=field_class
            error=field.error
            touched=Signal::derive(move || field.is_touched())
            help=help
        >
            <textarea
                class=class
                name=name
                rows=rows
                placeholder=placeholder
                aria-describedby=describedby
                prop:value=field.value
                on:input=on_input
                on:blur=move |_| field.touch()
            ></textarea>
        </Labelled>
    }
}
```

`event_target_value` is generic over `T: JsCast`
(`leptos_dom-0.8.8/src/helpers.rs:97-107`) and already works on the existing
`<textarea>` sites, so no cast change is needed.

- [x] **Step 2: Export it and correct the module doc**

In `web/src/forms/mod.rs`, change line 7 to name both widgets and extend the
re-export:

```rust
//! Client-side form primitives: `Field<T>` state, its validator, and the
//! `ValidatedInput` / `ValidatedTextarea` widgets.
```

```rust
#[cfg(target_arch = "wasm32")]
pub use component::{ValidatedInput, ValidatedTextarea};
```

- [x] **Step 3: Verify it compiles for wasm**

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS. The widget is
`pub` and re-exported, so it is not dead code despite having no call site yet.

- [ ] **Step 4: Commit**

```bash
git add web/src/forms/component.rs web/src/forms/mod.rs
git commit -m "feat(web/forms): add ValidatedTextarea<T> (#568)"
```

---

### Task 4: Adopt at the three posts summary sites

**Files:**

- Modify: `web/src/posts/component.rs:550-570` (compact composer), `:671-694`
  (full composer), `:1326-1349` (editor)
- Modify: `end2end/tests/selectors.ts` (add `postSummary`)
- Modify: `end2end/tests/posts.spec.ts:43,69,87,107,110,116`,
  `end2end/tests/posts.ts:62`

**Interfaces:**

- Consumes: `ValidatedTextarea<T>` (Task 3),
  `common::post_summary::PostSummary`.
- Produces: `SEL.postSummary = 'textarea[name="summary"]'` for the e2e suite.

**This task is atomic** — the markup change removes the ids the current
selectors key on, so both halves land in one commit.

- [x] **Step 1: Replace all three site bodies**

Each site currently spans a `<label class="j-field-label">`, a `<textarea>`, and
a touched-gated error block. Replace all three of those with a single call,
keeping the enclosing `<div style="margin-top:10px">` wrapper (it supplies the
spacing; `.j-form-field` has no margin):

```rust
<ValidatedTextarea<
    PostSummary,
>
    label="Summary"
    name="summary"
    field=summary_field
    placeholder="Optional summary or excerpt"
/>
```

Self-closing, matching every other generic-component call site in the repo.
`rows` is omitted — the default of 3 matches all three sites' current `rows=3`.
The compact composer gains `name="summary"`, which it lacks today; this is inert
for submission (all composers dispatch typed args, `posts/component.rs:508`,
`:522`).

Import `ValidatedTextarea` alongside the existing `crate::forms::Field` import
at the top of the file.

- [x] **Step 2: Add the shared selector**

In `end2end/tests/selectors.ts`, inside the `SEL` object:

```ts
  /** Post summary textarea (compose + edit). */
  postSummary: 'textarea[name="summary"]',
```

- [x] **Step 3: Swap the selectors**

Replace every `"#compose-summary"` and `"#edit-summary"` literal with
`SEL.postSummary`:

- `end2end/tests/posts.spec.ts:43` —
  `await page.fill(SEL.postSummary, "This is a summary");`
- `:69` — `const summaryInput = page.locator(SEL.postSummary);`
- `:87` — `await page.fill(SEL.postSummary, "A summary to remove");`
- `:107` —
  `await expect(page.locator(SEL.postSummary)).toHaveValue("A summary to remove");`
- `:110` — `await page.fill(SEL.postSummary, "");`
- `:116` — `await expect(page.locator(SEL.postSummary)).toHaveValue("");`
- `end2end/tests/posts.ts:62` —
  `await page.fill(SEL.postSummary, opts.summary);` (`SEL` is already imported
  at `posts.ts:13`)

- [x] **Step 4: Verify no id survives, and the name landed** — no id hits; three
      `name="summary"`

Run: `rg -n 'compose-summary|edit-summary' web/src end2end` Expected: **no
output**.

Run: `rg -n 'name="summary"' web/src/posts/component.rs` Expected: **three
hits** — one per site. This is the only check on AC6; no e2e drives the compact
composer's summary (that is Task 1's first follow-up).

- [x] **Step 5: Run the posts e2e** — 34 passed

Run: `devtool run -- cargo xtask e2e-local posts.spec.ts` Expected: PASS —
specifically `posts.spec.ts:36` (create with a summary), `:63` (over-long
summary shows inline error and gates submit), `:81` (clearing a summary on edit
persists as empty).

- [x] **Step 6: Commit** — `bfc75508`

```bash
git add web/src/posts/component.rs end2end/tests/selectors.ts end2end/tests/posts.spec.ts end2end/tests/posts.ts
git commit -m "refactor(web/posts): adopt ValidatedTextarea for the three summary fields (#568)"
```

---

### Task 5: Profile — card treatment, both fields converged

**Files:**

- Modify: `web/src/profile/component.rs:24-124` (`ProfilePage`)

**Interfaces:**

- Consumes: `ValidatedTextarea<T>` (Task 3), `ValidatedInput<T>` (existing),
  `common::bio::Bio`, `common::display_name::DisplayName`.
- Produces: nothing consumed downstream.

- [x] **Step 1: Replace the form markup**

Inside the `Ok(data)` arm, the returned `view!` currently renders a `<p>`
username line, a hand-wired `display_name` `<label>`+`<input>`+error, a
hand-wired `bio` `<label>`+`<textarea>`+error, the Update button, and
`<DefaultPostFormatControl/>`. Replace everything from the `<p>` through the
Update button with:

```rust
<div class="j-card">
    <div class="j-card-head">
        <div>
            <h2>"Profile"</h2>
            <div class="j-sub">"Your display name and bio."</div>
        </div>
    </div>
    <div class="j-form-body">
        <p>"Username: " {data.username.to_string()}</p>
        <ValidatedInput<
            DisplayName,
        > label="Display Name" name="display_name" field=dn_field />
        <ValidatedTextarea<
            Bio,
        > label="Bio" name="bio" field=bio_field />
    </div>
    <div class="j-form-actions">
        <button
            type="button"
            class="j-btn is-primary"
            prop:disabled=move || { !dn_field.is_valid() || !bio_field.is_valid() }
            on:click=submit
        >
            "Update Profile"
        </button>
    </div>
</div>
<DefaultPostFormatControl />
```

`<DefaultPostFormatControl/>` stays **outside** the card and unchanged — Task 6
handles it. The `submit` closure, the two `Field` declarations (`:21-22`), and
the resource-seeding `.set(...)` calls (`:34-41`) are all unchanged. Import
`ValidatedInput` and `ValidatedTextarea` from `crate::forms` alongside the
existing `Field` import (`:2`).

- [x] **Step 2: Verify the old chrome is gone** — two hits, both in
      `DefaultPostFormatControl`; no `error_for`

Run: `rg -n 'j-field-label|j-field-val|error_for' web/src/profile/component.rs`
Expected: **two hits, both inside `DefaultPostFormatControl`** (its label and
its `<select>`), and **no** `error_for` — both fields now route through the
widgets. (Line numbers shift when Step 1 rewrites ~90 lines above them; check
the count and location, not the numbers.)

- [x] **Step 3: Run the profile e2e** — 7 passed

Run: `devtool run -- cargo xtask e2e-local profile.spec.ts` Expected: PASS — all
six field tests: `:13`, `:34`, `:53` (display name) and `:82`, `:104`, `:151`
(bio). The three bio tests re-prove `ValidatedTextarea`'s wiring; `:104` and
`:34` specifically prove the touched-gated `.error` and the disable-until-valid
gate still work through `Labelled`.

- [x] **Step 4: Commit** — `f651a09a`

```bash
git add web/src/profile/component.rs
git commit -m "refactor(web/profile): adopt the validated widgets and the standard card (#568)"
```

---

### Task 6: `DefaultPostFormatControl` → its own card

**Droppable.** This is the scope addition flagged at spec approval. If vetoed,
skip this task entirely; no other task depends on it. See Key risks for the AC8
consequence.

**Files:**

- Modify: `web/src/profile/component.rs` — the `view!` inside
  `DefaultPostFormatControl` (at `:148-192` before Task 5; renumbered after)

**Interfaces:** none produced or consumed.

- [x] **Step 1: Wrap the control in a card**

Replace the inner `view!` (the `j-field-label` + `<select>` + Save button) with:

```rust
view! {
    <div class="j-card">
        <div class="j-card-head">
            <div>
                <h2>"Default Post Format"</h2>
                <div class="j-sub">"The editor format new posts start in."</div>
            </div>
        </div>
        <div class="j-form-body">
            <label class="j-form-field">
                <span class="j-form-label">"Default post format"</span>
                <select
                    id="default-post-format"
                    class="j-form-input"
                    on:change=move |ev| {
                        if let Ok(f) = event_target_value(&ev).parse::<PostFormat>() {
                            format.set(f);
                        }
                    }
                >
                    {PostFormat::VARIANTS
                        .iter()
                        .copied()
                        .filter_map(|f| f.get_message().map(|label| (f, label)))
                        .map(|(f, label)| {
                            view! {
                                <option value=f.to_string() selected=move || format.get() == f>
                                    {label}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
            </label>
        </div>
        <div class="j-form-actions">
            <button
                type="button"
                class="j-btn"
                on:click=move |_| {
                    action.dispatch(SetDefaultPostFormat { format: format.get() });
                }
            >
                "Save"
            </button>
        </div>
    </div>
}
```

`id="default-post-format"` is **retained** — `profile.spec.ts:124` keys on
`select#default-post-format`. The `for=` is dropped because the wrapping
`<label>` now associates implicitly. The option list, the `on:change` parse, and
the dispatch are unchanged. The `<select>` moving from `j-field-val` to
`j-form-input` is a visible change, recorded in the spec's Known visible
changes.

- [x] **Step 2: Run the profile e2e** — 7 passed; no `j-field-*` remains in
      `profile/component.rs` (AC8)

Run: `devtool run -- cargo xtask e2e-local profile.spec.ts` Expected: PASS — in
particular `:127` "default post format round-trips through the typed dispatch".

- [ ] **Step 3: Commit**

```bash
git add web/src/profile/component.rs
git commit -m "style(web/profile): give the default-format control the standard card (#568)"
```

---

### Task 7: Amend ADR-0065

**Files:**

- Modify: `docs/adr/0065-client-side-domain-validation.md:57-64` and `:72-80`

**Interfaces:** none.

- [ ] **Step 1: Correct the coverage-boundary bullet (`:72-80`)**

The bullet is headed "(ADR-0056, superseding 0055 — no `target_arch` gating)"
and claims `<ValidatedInput<T>>` "host-compil[es] as dead-but-exempt".
**ADR-0056 is superseded by ADR-0070** (`docs/adr/0056-…:3-4`), and the code
follows ADR-0070: `forms/mod.rs:9-10` gates `mod component` on
`target_arch = "wasm32"`, so the components never host-compile. The ADR already
contradicts itself — `:41` says "wasm-only `<ValidatedInput<T>>`".

Re-head the bullet "(ADR-0070 — web verticals split host/wasm at the file
level)" and replace the `<ValidatedInput<T>>` sentence with: the widgets
(`ValidatedInput`, `ValidatedTextarea`, and the shared `Labelled` chrome) live
in a `target_arch = "wasm32"`-gated `component.rs` and **never host-compile**,
so they carry no coverage obligation and need no exemption marker; their
rendering and interaction are exercised via e2e. Leave the `field_error<T>` and
`Field<T>` sentences untouched — both are still host-compiled and host-tested.

- [ ] **Step 2: Refresh the direct-bind example (`:57-64`)**

The bullet cites "the post compose/edit forms" as the canonical direct-bind site
— precisely what this issue converts. Replace that parenthetical with a site
that is still direct-bind after this work: the backup destination field
(`web/src/backup/component.rs:50`), whose comment already explains why
(`:47-49`). Keep the rest of the bullet — direct bind remains valid for the
sites #450 covers. Add `<ValidatedTextarea<T>>` alongside `<ValidatedInput<T>>`
as a default renderer in the first sentence.

- [ ] **Step 3: Prettier the prose before staging**

Run: `devtool run -- prettier -w docs/adr/0065-client-side-domain-validation.md`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add docs/adr/0065-client-side-domain-validation.md
git commit -m "docs(adr): correct ADR-0065's coverage boundary and direct-bind example (#568)"
```

---

### Task 8: Full gate

**Files:** none.

**Interfaces:** none.

- [ ] **Step 1: Confirm every acceptance criterion's grep**

Run:
`rg -n 'error_for\(&v\)' web/src/posts/component.rs web/src/profile/component.rs`
Expected: **exactly two hits, both `slug_field`** (AC3).

Run: `rg -n '<textarea' web/src` Expected: **two hits** — `ComposerFields`
(`posts/component.rs:107`, the post _body_ textarea, out of scope) and
`ValidatedTextarea` itself in `forms/component.rs`. No other `<textarea>`
element remains (AC4).

Run: `rg -n 'compose-summary|edit-summary' web/src end2end` Expected: **no
output** (AC5, AC7).

Run: `rg -n 'class="error"' web/src/forms/component.rs` Expected: **exactly one
hit**, inside `Labelled` — the error node is rendered in one place, not once per
shell (AC2).

Run: `rg -n 'j-form-label' web/src/forms/component.rs` Expected: **exactly one
hit**, inside `Labelled` — neither shell renders its own label span (AC2).

Run: `rg -n 'field_error|error_for' web/src/forms/component.rs` Expected: only
`field.error_for(&v)` in the two shells' `on:input` closures — no re-implemented
validation rule (AC10).

- [ ] **Step 2: Run the full gate**

Run: `devtool run -- cargo xtask validate` — **foreground**, `timeout: 600000`.
Background mode gets killed. Expected: PASS across all four
`{sqlite,postgres}×{chromium,firefox}` combos.

- [ ] **Step 3: Confirm the tree is clean**

Run: `git status --porcelain` Expected: **empty**. `cargo xtask check`
auto-fixes formatting without committing, so a non-empty result means a
formatting fixup needs its own commit.

_(No commit — verification only. Proceed to `jaunder-ship`.)_
