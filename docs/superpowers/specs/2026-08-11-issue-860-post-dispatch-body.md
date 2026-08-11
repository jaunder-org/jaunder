# Spec — #860: a post dispatch must not silently no-op on a rejected body

Issue: jaunder-org/jaunder#860 · Milestone 2 (Observability & diagnostics) ·
Branch `worktree-issue-860-post-dispatch-body`

## Problem

The three post forms in `web/src/posts/component.rs` build their dispatch
payload through `ComposeState::inputs`, which returns `Option<PostInputs>` and
yields `None` when the body does not parse as a `PostBody`. Every call site
drops that `None` on the floor:

```rust
if let Some(post) = state.inputs(publish, slug) { action.dispatch(...); }
```

The issue asserts this arm is unreachable because all submit controls are gated
on the same predicate. **That is no longer true.** Measured on the fork point
(`wt-base-issue-860`):

| Form                       | body gate on its submit buttons                     |
| -------------------------- | --------------------------------------------------- |
| `CompactComposer` (`:551`) | `state.body.get().trim().is_empty()` — hand-spelled |
| `FullComposer` (`:624`)    | **none** — slug and summary only                    |
| `EditPostForm` (`:1112`)   | **none** — slug and summary only                    |

So on the full compose page an empty body leaves "Save draft" and "Publish"
enabled; clicking either does nothing at all — no error, no message, no state
change (the `None` is dropped at `component.rs:546`, `:620`, `:1097`). The same
holds in the editor once the author clears the textarea. This is a live
dead-button defect, not redundant defence.

`CompactComposer`'s gate re-spells `PostBody`'s rule (`trim().is_empty()`)
instead of using the newtype door, which is the breach ADR-0065 and #416 forbid
and the one #845 fixed here once already.

## Decisions

Reached in the design interview; each is load-bearing on the acceptance criteria
below.

1. **`ComposeState.body` becomes `Field<PostBody>`** (from `RwSignal<String>`),
   joining `summary_field` and `slug_field`. The composer's body stops being the
   one unvalidated text field in the bundle.
2. **`ComposeState::inputs` becomes infallible**: it takes an already-parsed
   `PostBody` and returns `PostInputs`, not `Option<PostInputs>`. There is no
   error arm left in it to swallow.
3. **A `submit_gate` helper owns the gate and the parse together** — a plain
   function, not a component, so the existing button markup and
   `EditSaveActions` are untouched (no button-markup or button-selector churn,
   and no component setup-complexity pressure).
4. **`submit_gate` lives in `web/src/posts/compose_state.rs`**, re-exported from
   `posts/mod.rs` beside `ComposeState`. It **cannot** live in `component.rs`:
   `posts/mod.rs:17` declares that module `#[cfg(target_arch = "wasm32")]`, so
   anything in it is neither host-testable nor coverage-measured
   (`CONTRIBUTING.md:555` names this exact exclusion). `compose_state.rs` exists
   for precisely this reason and its tests already drive leptos signals
   host-side under a `with_owner` harness.
5. **`submit_gate` derives both outputs from `Field::parsed()` and never calls
   `is_valid()`.** `is_valid()` reads the cached `error` signal while `parsed()`
   re-reads `value`; a programmatic write sets `value` alone, so the two can
   disagree. Deriving from one call makes "the button is disabled" and "there is
   no payload" the same condition by construction.
6. **A rejected body is visible**, on touch, like summary and slug — the issue's
   own stated principle that every other rejection in these forms is visible to
   the user.
7. **`ComposerFields` is rebuilt on `ValidatedTextarea<PostBody>`** rather than
   growing a second copy of the touch/error/aria plumbing. `ValidatedTextarea`
   gains an optional `on_input` passthrough prop; `ComposerFields` already has
   an `on_input` prop (`component.rs:126`) and **forwards** it rather than
   gaining one.
8. **The body label is visible.** `ValidatedTextarea` renders a label through
   `<Labelled>`; the body textarea gains a visible "Body" label at all three
   sites. This is an accepted, intentional UI change — and it brings a
   `<label class=field_class>` wrapper with it, which is a **layout** change,
   not only a textual one (see decision 9).
9. **The new wrapper is styled deliberately, not incidentally.** Today the body
   `<textarea>` is a direct flex child of `.j-composer-body` and
   `.j-edit-form-body` (`server/assets/jaunder.css:369-381`, `:1060-1066`).
   `<Labelled>` interposes a `<label>`, so each site passes an explicit
   `field_class` and the stylesheet gains rules for it. This is scoped work in
   this cycle, not a side effect to be discovered.
10. **`Field` gains `set_value(&self, value: &str)`**, writing `value` and
    `error` together, and every programmatic writer in
    `compose_state.rs::seed_from` uses it — the body **and** the summary, whose
    `value.set` at `compose_state.rs:105` has the same stale-error defect today.
11. **The decision is recorded as a new ADR draft**, numberless in
    `docs/adr/drafts/`, promoted at ship. It states the rule ADR-0065 does not:
    a dispatch closure never parses; the control's `disabled` and the dispatch
    payload come from one call.
12. **Three stale ADR citations are corrected.** `compose_state.rs:71`,
    `compose_state.rs:170` and `component.rs:542` cite **ADR-0102** for the
    blank-body invariant. ADR-0102 is the config-key closed registry; the
    correct record is **ADR-0105** (`0105-post-body-non-blank-invariant.md`).

## Known non-conformance, accepted

The ADR's rule is stated absolutely, but this cycle brings only the **body**
into conformance. The `also_blocked` predicate this cycle keeps passing to
`submit_gate` is still
`!slug_field.is_valid() || !state.summary_field.is_valid()` (`component.rs:624`,
`:1112-1114`), while those two fields' payload values come from `parsed()`
(`component.rs:620`, `compose_state.rs:87`). That is the two-source shape the
ADR prohibits, surviving for slug and summary on the day the ADR lands.

This is deliberate and must be stated in both the spec and the ADR rather than
discovered later: converting slug and summary means reaching every form in
`web/src`, which is the follow-up issue below. **The ADR's clause carries an
explicit "known-nonconforming call sites are tracked by \<issue\>" note.**
Shipping a rule the codebase silently violates is the failure mode this section
exists to prevent.

## Out of scope — filed as follow-up issues

The plan's **first task** files these, so they can be picked up concurrently
rather than blocked behind this cycle:

1. **Make `Field::error` derived from `value`** (a `Memo`) so `is_valid()` and
   `parsed()` cannot drift for _any_ field, and bring slug and summary into ADR
   conformance. Note `Field::value` and `Field::error` are `pub`
   (`web/src/forms/field.rs:23-24`), so `set_value` is a convention, not an
   enforcement — this follow-up is what would make the desync inexpressible.
2. **Unify the two composers' inline button markup with `EditSaveActions`.**

Not filed, simply excluded: any change to `PostBody`'s validation rule itself.

## Acceptance criteria

Each names the evidence that settles it. Where a criterion covers wasm-only code
(which no host test can observe and the coverage gate excludes), the evidence is
an e2e assertion or a stated inspection — never "review by eyeball".

### Behavior — settled by e2e

- **AC1** On the full compose page (`FullComposer`) with an empty body, "Save
  draft" and "Publish" are both **disabled**. _Evidence: new e2e assertion._
- **AC2** Same for a whitespace-only body (`"   \n\t "`), which
  `PostBody::from_str` rejects. _Evidence: the same new e2e test._
- **AC3** In the editor (`EditPostForm`), clearing the body textarea disables
  the save controls; restoring non-blank text re-enables them. _Evidence: new
  e2e assertion._
- **AC4** In `CompactComposer` the buttons disable on an unparseable body
  exactly as today. _Evidence: the existing composer e2e specs stay green,
  unmodified._
- **AC5** Once the body field has been blurred and does not parse, the message
  `post body must contain at least one non-blank line` is rendered beneath the
  body textarea. A pristine, never-blurred empty composer shows no message.
  _Evidence: new e2e assertion on the full compose page, against a body-scoped
  selector (see AC17)._

### Behavior — settled by host test

- **AC6** A successful create empties the composer (`reset`) and returns the
  body field to pristine: empty value, not touched, error re-seeded so the
  buttons are disabled.
- **AC7** `seed_from` leaves the body field valid with **no** error, and leaves
  the summary field's error consistent with its seeded value (decision 10).
- **AC8** `ComposeState::inputs` carries body, format, publish flag, publish-at,
  tags, summary and audience through unchanged. _Evidence: the existing
  `compose_state.rs` host tests, updated to the new signature._

### Behavior — settled by inspection of the diff

- **AC9** The per-form wiring in `component.rs` is unchanged apart from the
  gate: each form still dispatches its own action (`Create` / `Update`) with the
  same `slug_override` (`None` for the compact shape, `slug_field.parsed()` for
  the other two) and the same `publish` flag. _Evidence:
  `git diff wt-base-issue-860..HEAD` over `component.rs` shows no change to
  those arguments; the existing post create/edit e2e specs stay green._

### Structure — settled by reading the code

- **AC10** `ComposeState.body` has type `Field<PostBody>`.
- **AC11** `ComposeState::inputs` has signature
  `fn inputs(&self, body: PostBody, publish: bool, slug_override: Option<Slug>) -> PostInputs`
  — no `Option` return, no `parse` inside it.
- **AC12** `submit_gate` is defined in `web/src/posts/compose_state.rs` (**not**
  `component.rs`), re-exported from `posts/mod.rs`, returns
  `(Signal<bool>, Callback<bool>)`, and its body contains no call to
  `Field::is_valid`.
- **AC13** No dispatch closure in `web/src/posts/component.rs` parses a body,
  and none contains a silent `return` / `.ok()?` / `let … else` on the dispatch
  path.
- **AC14** No submit-gating predicate in `web/src/posts/component.rs` re-spells
  `PostBody`'s rule (no `trim().is_empty()` on the body).
- **AC15** `ComposerFields`' `body` prop has type `Field<PostBody>` (was
  `RwSignal<String>`, `component.rs:118`). `ComposerFields` is `pub` and
  re-exported (`posts/mod.rs:81`), so this is an intended public API change.
- **AC16** `ComposerFields` renders `ValidatedTextarea<PostBody>` and declares
  no `<textarea>` of its own; its existing `on_input` prop is forwarded to it.
- **AC17** The body textarea keeps `name="body"`. Every e2e body selector
  depends on it (`end2end/tests/selectors.ts:14`, used throughout
  `posts.spec.ts`).
- **AC18** `ValidatedTextarea` has an optional `on_input: Option<Callback<()>>`
  prop, invoked after the value and error are written. Omitting it leaves
  existing call sites (profile, summary) compiling and behaving unchanged.
- **AC19** `Field::set_value(&self, value: &str)` exists, writes both `value`
  and `error`, and `compose_state.rs::seed_from` uses it for **both** the body
  and the summary — no bare `field.value.set(…)` remains in that function.

### Layout

- **AC20** Each of the three `ComposerFields` call sites passes an explicit
  `field_class` and `class` to `ValidatedTextarea`, and
  `server/assets/jaunder.css` carries rules for each new `field_class` so the
  interposed `<label>` reproduces the flex behavior the bare `<textarea>` had in
  `.j-composer-body` (`:369-381`) and `.j-edit-form-body` (`:1060-1066`). No
  site relies on the default `j-form-field` (`:878-883`).
- **AC21** The existing e2e post specs pass unmodified except where AC22
  requires a selector change — i.e. the layout change breaks no existing flow.

### E2E hygiene

- **AC22** Adding a touched-gated `<p class="error">` under the body creates a
  second match for `SEL.error`, which `posts.spec.ts:78` asserts under
  Playwright strict mode. Every existing `SEL.error` assertion in `end2end/` is
  reviewed, and any that could now match two nodes is scoped (to the flash, or
  to the body field) rather than left to fail intermittently.

### Documentation

- **AC23** No **blank-body** citation names `ADR-0102`; the surviving ones name
  `ADR-0105`. (`compose_state.rs:170`'s citation sits on
  `a_blank_body_yields_no_payload`, which AC11 makes impossible and the test
  rewrite removes, so two of the original three survive to be corrected.)
  ADR-0102 remains a live record and may be cited legitimately elsewhere.
- **AC24** A numberless ADR draft exists in `docs/adr/drafts/`, states the
  gate-owns-its-parse rule, and carries the known-non-conformance note this spec
  requires.

### Gate

- **AC25** `cargo xtask validate` is green — including the `thin-components`
  step (no component surface over the setup-complexity budget of 2,
  `xtask/src/steps/thin_components.rs:44`) and the coverage policy.

## Notes for the implementer

- `Field::<PostBody>::new()` is the required (non-optional) constructor; it
  seeds `error` from the empty string so a pristine composer is already invalid
  — which is what AC1 and AC6 need.
- `submit_gate` returning leptos types is fine host-side: `compose_state.rs`'s
  tests already run signals under `Owner::new()`.
- Watch `PostCreateForm`'s setup-complexity budget (the issue flags it);
  `submit_gate` being a function rather than inline `if let` chains is what
  keeps the count down.
