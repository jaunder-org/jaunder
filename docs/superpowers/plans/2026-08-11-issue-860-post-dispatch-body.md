# #860 — A submit gate owns its parse: implementation plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-08-11-issue-860-post-dispatch-body.md` —
read it for the what and the why. This plan is the how; it does not restate the
problem analysis.

**Goal:** Make a post dispatch structurally incapable of silently no-opping on a
rejected body, and fix the live dead-button defect that shape already produced
on the full compose page and in the editor.

**Architecture:** The composer's body joins `summary` and `slug` as a
`Field<PostBody>`. `ComposeState::inputs` stops parsing and becomes infallible.
A new host-compiled `submit_gate` derives both the button's `disabled` signal
and the dispatch payload from one `Field::parsed()` call, so they cannot drift.
`ComposerFields` is rebuilt on the existing `ValidatedTextarea<T>` so a rejected
body is visible.

**Tech Stack:** Rust, Leptos (CSR), `cargo nextest`, Playwright (`end2end/`),
`cargo xtask` gate.

## Review header

**Scope — in:** `web/src/forms/field.rs`, `web/src/forms/component.rs`,
`web/src/posts/compose_state.rs`, `web/src/posts/component.rs`,
`web/src/posts/mod.rs`, `server/assets/jaunder.css`,
`end2end/tests/posts.spec.ts`, the ADR draft.

**Scope — out:** deriving `Field::error` from `value` repo-wide; unifying the
composers' button markup with `EditSaveActions`; any change to `PostBody`'s
rule. Task 1 files the first two as issues.

| #   | Task                                                                   |
| --- | ---------------------------------------------------------------------- |
| 1   | File the two follow-up issues; commit spec + plan                      |
| 2   | `Field::set_input` — write value and error together                    |
| 3   | `ValidatedTextarea` gains an `on_input` passthrough                    |
| 4   | **The atomic change:** `Field<PostBody>` + `submit_gate` + all 3 forms |
| 5   | CSS for the interposed `<label>` wrapper                               |
| 6   | e2e regressions + `SEL.error` strict-mode hygiene                      |
| 7   | Full gate and branch review                                            |

**Key risks / decisions:**

- **Task 4 is deliberately one large commit, and cannot be split.**
  `cargo xtask check` runs a `wasm-clippy` step that compiles
  `web/src/posts/component.rs` for `wasm32-unknown-unknown`
  (`xtask/src/steps/static_checks.rs:58-95`). Changing `ComposeState`'s shape
  breaks that build until every call site in `component.rs` is rewritten, so
  there is **no clean gate between the two halves**. Splitting them would mean
  committing behind a skipped gate, which violates "what lands is what was
  checked". TDD still applies _within_ the task: `cargo nextest run -p web` is a
  host build and stays runnable throughout.
- **`submit_gate` must NOT go in `component.rs`** — `posts/mod.rs:17` declares
  that module `#[cfg(target_arch = "wasm32")]`, so code there is neither
  host-testable nor coverage-measured.
- **The ADR draft is gitignored** (`.gitignore:48`) and must **never** be
  `git add`ed. It stays out of git until `cargo xtask adr promote` numbers it at
  ship (`jaunder-ship`). Its bare-form sibling links (`0065-….md`) are correct
  as written — see `docs/adr/drafts/README.md:29-34`.
- **The `<Labelled>` wrapper is a layout change,** not just a text one. Task 5
  is mandatory; skipping it breaks composer and editor layout.
- **A second `.error` node appears on the page.** `SEL.error` is `.error` and
  several specs assert it under Playwright strict mode. Task 6 audits them.
- Task 4 touches a `thin-components` budget of 2. `submit_gate` being a function
  is what keeps each component's setup complexity under it.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- Every commit is preceded by a clean
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-860-post-dispatch-body -- cargo xtask check`
  (**`jaunder-commit`**). Stage, then commit — never `git commit -- <paths>`.
- `cargo xtask check` compiles **both** the host and the wasm target for `web`.
  A change to a host-compiled type used by `component.rs` is not committable
  until `component.rs` is updated in the same commit.
- `web/src/posts/component.rs` and `web/src/forms/component.rs` are
  **wasm-only** and coverage-exempt. Any logic that can be host-tested must not
  be placed there.
- Client-side validation goes through the shared newtype's `FromStr` — never a
  re-spelling of its rule (ADR-0065, #416).
- The body textarea keeps `name="body"`; every e2e body selector depends on it
  (`end2end/tests/selectors.ts:14`).
- `thin-components` setup-complexity budget is **2**
  (`xtask/src/steps/thin_components.rs:44`).

---

### Task 1: File the follow-up issues; commit the spec and plan

**Files:**

- Commit: `docs/superpowers/specs/2026-08-11-issue-860-post-dispatch-body.md`,
  `docs/superpowers/plans/2026-08-11-issue-860-post-dispatch-body.md`
- Edit but **do not commit**: `docs/adr/drafts/submit-gate-owns-its-parse.md`

**Interfaces:**

- Produces: one issue number, referenced from the ADR draft's clause-2 note and
  its Consequences section (both currently say "tracked separately"). The second
  issue (button markup) has no ADR referent and needs none.

- [x] **Step 1: File the `Field::error` follow-up** → **#907**. Milestone "Code
      quality ratchet". Title:
      `forms: Field::error is written, not derived — is_valid() and parsed() can disagree`.
      Body must state: `Field::value` and `Field::error` are separate
      `RwSignal`s (`web/src/forms/field.rs:23-24`); `is_valid()` reads the
      cached error while `parsed()` re-parses the value; any programmatic
      `value.set` desyncs them. Deriving `error` as a `Memo` over `value` would
      make the desync inexpressible and bring the slug and summary gates into
      conformance with the #860 ADR, whose clause 2 they currently violate
      (`web/src/posts/component.rs:624`, `:1112-1114`).

- [x] **Step 2: File the button-markup follow-up** → **#908**. Milestone "Code
      quality ratchet". Title:
      `web/posts: the two composers' submit buttons duplicate EditSaveActions`.
      Body: `CompactComposer` and `FullComposer` each hand-roll the "Save
      draft" + "Publish" pair that `EditSaveActions` already renders
      (`web/src/posts/component.rs:579-598`, `:640-659`, `:1131-1177`).

- [x] **Step 3: Reference Step 1's issue number in the ADR draft.** Both sites
      now cite **#907**. The draft is **not** `git add`ed — it is gitignored by
      design.

- [x] **Step 4: Commit** (spec and plan only) — `bef823ff`

```bash
git add docs/superpowers/specs/2026-08-11-issue-860-post-dispatch-body.md docs/superpowers/plans/2026-08-11-issue-860-post-dispatch-body.md
git commit -m "docs(posts): spec and plan for the submit-gate rule (#860)"
```

---

### Task 2: `Field::set_input` — write value and error together

**Files:**

- Modify: `web/src/forms/field.rs` (impl block at `:55-142`; tests at `:144`)

**Interfaces:**

- Produces: `pub fn set_input(&self, input: &str)` on `Field<T>` where
  `T: FromStr + 'static, T::Err: Display`. Consumed by Task 4's `seed_from`.

This task is additive — no existing caller changes — so it compiles and gates
clean on its own.

**Renamed during implementation:** originally specified as `set_value`, which
collides with leptos's `SetValue` trait method (in scope via the prelude). An
inherent method shadows it, but relying on that is fragile; `set_input` also
matches `error_for(input)`'s vocabulary. Spec AC19 updated to match.

- [x] **Step 1: Write the failing tests** — append to the `mod tests` block in
      `web/src/forms/field.rs`. It already imports `Slug` (`:155`) and `Owner`
      (`:159`), so no new imports are needed. These pin every branch: required,
      optional, invalid, and the untouched postcondition.

```rust
    /// `set_input` is the programmatic writer that cannot desync `value` from `error`.
    /// A bare `field.value.set(..)` leaves `error` stale — the defect #860's ADR names.
    #[test]
    fn set_input_writes_value_and_error_together() {
        let owner = Owner::new();
        owner.set();

        let field = Field::<Slug>::new();
        field.set_input("hello");
        assert_eq!(field.value.get(), "hello");
        assert!(field.is_valid(), "a valid slug leaves no error");

        field.set_input("not a slug");
        assert_eq!(field.value.get(), "not a slug");
        assert!(!field.is_valid(), "an invalid slug sets the error");
        assert_eq!(
            field.is_valid(),
            field.parsed().is_some(),
            "is_valid and parsed must agree after a programmatic write"
        );

        drop(owner);
    }

    /// Optionality is honored: an empty *optional* field is valid, an empty *required*
    /// one is not — `set_input` must route through `error_for`, not `field_error`.
    #[test]
    fn set_input_honors_optionality() {
        let owner = Owner::new();
        owner.set();

        let optional = Field::<Slug>::optional();
        optional.set_input("");
        assert!(optional.is_valid(), "an empty optional field is valid");

        let required = Field::<Slug>::new();
        required.set_input("");
        assert!(!required.is_valid(), "an empty required field is not valid");

        drop(owner);
    }

    /// `set_input` seeds content, it does not simulate user interaction, so it must not
    /// mark the field touched — otherwise the editor would flash an error on load.
    #[test]
    fn set_input_does_not_touch_the_field() {
        let owner = Owner::new();
        owner.set();

        let field = Field::<Slug>::new();
        field.set_input("not a slug");
        assert!(!field.is_touched(), "seeding is not interaction");

        drop(owner);
    }
```

- [x] **Step 2: Run the tests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-860-post-dispatch-body -- cargo nextest run -p web set_input`
Expected: FAIL — no method named `set_input` found for struct `Field` Actual
(under the original `set_value` name): FAIL, E0599 — `set_value` resolved to
leptos's `SetValue` trait method and reported an unsatisfied `WriteValue` bound.
That collision is what prompted the rename.

- [x] **Step 3: Implement against the tests**

Add to the `impl<T> Field<T>` block in `web/src/forms/field.rs`, to signature
`pub fn set_input(&self, input: &str)`. Every branch is pinned by Step 1's tests
— required vs optional (via `error_for`, not `field_error`), valid vs invalid,
and the untouched postcondition — so write the body those tests determine.
Document it as the programmatic counterpart to the components' on-input handler,
and note that `value` and `error` are `pub`, so this is a convention rather than
an enforcement (Task 1 Step 1's follow-up is what would make the desync
inexpressible).

- [x] **Step 4: Run the tests, verify they pass**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-860-post-dispatch-body -- cargo nextest run -p web set_input`
Expected: PASS — 3 tests. Actual: 3 passed, 205 skipped.

- [ ] **Step 5: Commit**

```bash
git add web/src/forms/field.rs
git commit -m "feat(forms): Field::set_input writes value and error together (#860)"
```

---

### Task 3: `ValidatedTextarea` gains an `on_input` passthrough

**Files:**

- Modify: `web/src/forms/component.rs:148-203`

**Interfaces:**

- Produces: `ValidatedTextarea`'s new optional prop
  `#[prop(optional)] on_input: Option<Callback<()>>`, fired **after**
  `field.value` and `field.error` are written. Task 4 forwards `ComposerFields`'
  existing `on_input` into it.

Additive and `#[prop(optional)]`, so existing call sites are untouched and this
gates clean on its own — which is why it precedes Task 4 rather than being
folded into it.

> `web/src/forms/component.rs` is wasm-only (`web/src/forms/mod.rs:9`), so this
> task has no host test. Its behavior is settled by Task 6's e2e.

- [x] **Step 1: Add the prop and fire it**

Done, with one improvement over the plan: the handler's two lines
(`field.value.set` + `field.error.set`) were exactly what Task 2's
`Field::set_input` does, so the handler now calls it. That removes the last
hand-spelled value+error pair in the forms layer.

Add to the prop list, after `help`:

```rust
    /// Optional callback fired on every input event, after the value and error are
    /// written. `ComposerFields` forwards the composer's flash-clearing callback through
    /// it (#860); every other call site omits it.
    #[prop(optional)]
    on_input: Option<Callback<()>>,
```

The local closure at `:174` is already named `on_input` and would shadow the
prop — rename it to `handle_input` and update its use in the `view!` block at
`:198`. Extend it to run the callback **last**, after `field.value.set` and
`field.error.set`, so a consumer reading validity from the callback sees the new
state.

- [x] **Step 2: Verify existing call sites still compile — both targets**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-860-post-dispatch-body -- cargo xtask check --no-test`
Expected: PASS, including the `wasm-clippy` step — `#[prop(optional)]` means the
profile and summary call sites need no change.

- [ ] **Step 3: Commit**

```bash
git add web/src/forms/component.rs
git commit -m "feat(forms): ValidatedTextarea forwards an optional on_input callback (#860)"
```

---

### Task 4: The atomic change — `Field<PostBody>`, `submit_gate`, and all three forms

**Files:**

- Modify: `web/src/posts/compose_state.rs` — struct `:30-42`, `new` `:51-63`,
  `inputs` `:78-90`, `seed_from` `:102-114`, `reset` `:121-126`, tests
  `:135-246`
- Modify: `web/src/posts/mod.rs:55` (extend the `compose_state` re-export)
- Modify: `web/src/posts/component.rs` — `ComposerFields` `:117-146`,
  `CompactComposer` `:530-604`, `FullComposer` `:610-665`, `EditPostForm`
  `:1084-1121`

**Interfaces:**

- Consumes: `Field::set_input` (Task 2), `ValidatedTextarea`'s `on_input` (Task
  3).
- Produces:
  - `pub body: Field<PostBody>` on `ComposeState` (was `RwSignal<String>`).
  - `pub fn inputs(&self, body: PostBody, publish: bool, slug_override: Option<Slug>) -> PostInputs`
    — no `Option`, no parse inside.
  - ```rust
    pub fn submit_gate(
        body: Field<PostBody>,
        also_blocked: Signal<bool>,
        on_submit: Callback<(PostBody, bool)>,
    ) -> (Signal<bool>, Callback<bool>)
    ```
    re-exported as `crate::posts::submit_gate`.
  - `ComposerFields`' `body` prop type changes from `RwSignal<String>` to
    `Field<PostBody>`, and it gains a `field_class` prop. `ComposerFields` is
    `pub` and re-exported (`posts/mod.rs:81`), so this is an intended public API
    change.

> **Why this is one commit:** `cargo xtask check` runs `wasm-clippy` over
> `component.rs` (`xtask/src/steps/static_checks.rs:58-95`). Changing
> `ComposeState` breaks that build at `component.rs:546`, `:620`, `:1097`
> (arity) and `:556`, `:629`, `:1104` (`ComposerFields`' prop type) until Steps
> 6–9 land. There is no clean gate in between. Steps 1–5 are still test-driven —
> `cargo nextest run -p web` is host-only and stays runnable — but nothing is
> committed until Step 11.

- [x] **Step 1: Extend the test module's imports**

`compose_state.rs`'s `mod tests` imports only `use super::ComposeState;`
(`:137`). Add what the new tests need:
`use super::{ComposeState, submit_gate};`, `use crate::forms::Field;`,
`use common::post_body::PostBody;`.

- [x] **Step 2: Rewrite the four existing tests to the new shapes**

All four use `state.body` as an `RwSignal<String>` and/or the old `inputs`
arity. Rewrite each — none is deleted except where noted:

- `inputs_carry_the_edited_body_and_the_publish_flag` (`:152`) → new `inputs`
  signature.
- `an_empty_publish_at_schedules_nothing` (`:187`) →
  `state.body.set_input("body")` and
  `state.inputs(body, true, None).publish_at.is_none()` with no `.expect(…)`.
- `seed_from_loads_an_existing_post_into_the_editor_fields` (`:205`) →
  `state.body.value.get()` in place of `state.body.get()` (a `Field` has no
  `get()`).
- `reset_clears_the_post_body_but_keeps_format_and_audience` (`:224`) →
  `state.body.set_input(…)` and `state.body.value.get()`.

`a_blank_body_yields_no_payload` (`:174`) is **deleted**: the decision it pinned
moves to `submit_gate`, where Step 4's tests cover it.

```rust
    #[test]
    fn inputs_carry_the_edited_body_and_the_publish_flag() {
        with_owner(|| {
            let state = ComposeState::new();
            let body: PostBody = "hello".parse().expect("a non-blank body parses");

            let draft = state.inputs(body.clone(), false, None);
            assert_eq!(draft.body.as_ref(), "hello");
            assert!(!draft.publish);
            assert_eq!(draft.format, PostFormat::Markdown);
            assert!(draft.slug_override.is_none());

            assert!(
                state.inputs(body, true, None).publish,
                "publish flag passes through"
            );
        });
    }
```

- [x] **Step 3: Write the failing seeding/reset tests**

These pin spec AC6 and AC7 — including the summary half of AC7, which is the
reason `seed_from` gains a second `set_input`. Reuse the existing
`crate::posts::render::test_fixtures::sample_post()` fixture (already used at
`:208`, body `"raw"`, no summary); do **not** hand-build an `AuthoredPost` — it
holds a full `RenderedPost` (`common/src/seed.rs:108-112`) and has no
`summary`/`tags` of its own.

```rust
    /// The body is now a validated field, so seeding must leave it consistent: a real
    /// post's body is valid, and the seeded field must say so rather than carrying the
    /// stale "blank" error `Field::new` seeded at construction. The summary is seeded
    /// through the same door, for the same reason.
    #[test]
    fn seed_from_leaves_the_seeded_fields_consistent() {
        with_owner(|| {
            let state = ComposeState::new();
            assert!(!state.body.is_valid(), "a pristine composer is invalid");

            state.seed_from(&crate::posts::render::test_fixtures::sample_post());

            assert_eq!(state.body.value.get(), "raw");
            assert!(state.body.is_valid(), "a seeded body is valid");
            assert!(!state.body.is_touched(), "seeding is not interaction");
            assert!(
                state.summary_field.is_valid(),
                "an absent summary seeds an empty, valid optional field"
            );
        });
    }

    /// After a successful create the composer returns to pristine: empty, untouched, and
    /// invalid again — which is what re-disables the submit buttons (#860 AC6).
    #[test]
    fn reset_returns_the_body_field_to_pristine() {
        with_owner(|| {
            let state = ComposeState::new();
            state.body.set_input("some text");
            state.body.touch();

            state.reset();

            assert_eq!(state.body.value.get(), "");
            assert!(!state.body.is_touched());
            assert!(!state.body.is_valid(), "an empty body is not a PostBody");
        });
    }
```

- [x] **Step 4: Write the failing `submit_gate` tests**

One per branch: blocked-by-body, blocked-by-caller, dispatch-with-payload, and
the invariant tying disabled to absent-payload.

```rust
    /// The gate blocks when the body does not parse — whatever the other predicate says.
    #[test]
    fn the_gate_blocks_an_unparseable_body() {
        with_owner(|| {
            let body = Field::<PostBody>::new();
            let (disabled, _) =
                submit_gate(body, Signal::derive(|| false), Callback::new(|_| {}));

            assert!(disabled.get(), "an empty body blocks");

            body.set_input("   \n\t ");
            assert!(disabled.get(), "a whitespace-only body blocks");

            body.set_input("real text");
            assert!(!disabled.get(), "a parsing body with nothing else blocking");
        });
    }

    /// The gate also blocks on the caller's predicate (an invalid slug or summary),
    /// independently of the body.
    #[test]
    fn the_gate_blocks_on_the_callers_predicate() {
        with_owner(|| {
            let body = Field::<PostBody>::new();
            body.set_input("real text");
            let blocked = RwSignal::new(true);
            let (disabled, _) = submit_gate(
                body,
                Signal::derive(move || blocked.get()),
                Callback::new(|_| {}),
            );

            assert!(disabled.get(), "blocked by the caller despite a valid body");
            blocked.set(false);
            assert!(!disabled.get(), "unblocked once the caller's predicate clears");
        });
    }

    /// The click hands through the *parsed* body — the dispatch closure never parses.
    #[test]
    fn the_click_hands_through_a_parsed_body() {
        with_owner(|| {
            let body = Field::<PostBody>::new();
            body.set_input("real text");
            let seen: RwSignal<Vec<(String, bool)>> = RwSignal::new(Vec::new());
            let (_, on_click) = submit_gate(
                body,
                Signal::derive(|| false),
                Callback::new(move |(b, publish): (PostBody, bool)| {
                    seen.update(|v| v.push((b.as_ref().to_owned(), publish)));
                }),
            );

            on_click.run(false);
            on_click.run(true);

            assert_eq!(
                seen.get(),
                vec![
                    ("real text".to_owned(), false),
                    ("real text".to_owned(), true),
                ],
                "each click runs on_submit once with the parsed body and its flag"
            );
        });
    }

    /// A click that should be impossible runs nothing — and, crucially, the two
    /// conditions are the same one: disabled iff there is no payload.
    #[test]
    fn a_blocked_gate_dispatches_nothing() {
        with_owner(|| {
            let body = Field::<PostBody>::new();
            let ran = RwSignal::new(0_u32);
            let (disabled, on_click) = submit_gate(
                body,
                Signal::derive(|| false),
                Callback::new(move |_: (PostBody, bool)| ran.update(|n| *n += 1)),
            );

            on_click.run(true);
            assert_eq!(ran.get(), 0, "an unparseable body dispatches nothing");
            assert!(disabled.get(), "and the control reporting that is disabled");

            body.set_input("real text");
            on_click.run(true);
            assert_eq!(ran.get(), 1);
            assert!(!disabled.get());
        });
    }
```

- [x] **Step 5: Run the host tests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-860-post-dispatch-body -- cargo nextest run -p web compose_state`
Expected: FAIL — `submit_gate` not found; `inputs` arity; no `set_input` on
`RwSignal<String>`.

(Filter on `compose_state`, not `submit_gate`: nextest matches the
fully-qualified name `posts::compose_state::tests::<name>`, and none of the four
test names contains `submit_gate`.)

- [x] **Step 6: Reshape `ComposeState`**

Four edits in `compose_state.rs`, each pinned by Steps 2–3:

1. `pub body: Field<PostBody>` in the struct, `Field::<PostBody>::new()` in
   `new()`.
2. `inputs` takes `body: PostBody`, returns `PostInputs`. Delete the parse and
   the `Option`. Rewrite its doc comment (`:65-77`): the payload is infallible
   because the caller already holds a parsed body. Its current `#811, ADR-0102`
   citation (`:71`) is wrong — ADR-0102 is the config-key closed registry; use
   **ADR-0105** and reference the draft by path,
   `docs/adr/drafts/submit-gate-owns-its-parse.md`. (The struct field itself
   carries no doc comment today; there is nothing to update there.)
3. `seed_from` uses `self.body.set_input(…)` and — per spec decision 10 —
   `self.summary_field.set_input(…)` in place of the bare `value.set` at `:105`.
4. `reset` uses `self.body.reset()` in place of `self.body.set(String::new())`.

- [x] **Step 7: Write `submit_gate`**

In `compose_state.rs`, to the signature in **Interfaces**. Step 4's tests pin
every branch, and the invariant they enforce determines the body: **both outputs
come from `body.parsed()`, and `Field::is_valid` is not called.** Reading the
cached `error` for the gate while parsing `value` for the payload is the
two-source shape this cycle exists to remove.

Document: what it guarantees (a disabled control and an absent payload are one
condition), why it lives here rather than in `component.rs` (that module is
`#[cfg(target_arch = "wasm32")]`, so neither host-testable nor
coverage-measured), and the draft path.

Then extend `web/src/posts/mod.rs:55`:

```rust
pub use compose_state::{ComposeState, submit_gate};
```

- [x] **Step 8: Run the host tests, verify they pass**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-860-post-dispatch-body -- cargo nextest run -p web compose_state`
Expected: PASS. `component.rs` is still broken for wasm at this point — that is
expected and is fixed next.

- [x] **Step 9: Rebuild `ComposerFields`**

Change the `body` prop to `Field<PostBody>`, add a `field_class: &'static str`
prop (no default — spec AC20 requires every site to pass one explicitly), and
replace the hand-rolled `<textarea>` with `<ValidatedTextarea<PostBody> …/>`,
keeping the `{show_seg.then(…)}` `FormatToggle` line. Required attributes:

- `label="Body"` — visible, per spec decision 8.
- `name="body"` — **load-bearing**; every e2e body selector depends on it.
- `rows=rows`, `placeholder=placeholder`, `on_input=on_input`,
  `class=textarea_class`, `field_class=field_class`.

Note a cosmetic delta to accept: `Labelled` hardcodes the label span's class as
`j-form-label` (`web/src/forms/component.rs:47`), so the editor's body label
will not match its siblings' uppercase `.j-edit-form-label` styling. That is
acceptable; do not add a prop to fix it in this cycle.

- [x] **Step 10: Wire all three forms to `submit_gate`**

Each replaces its `dispatch` closure and its `submit_disabled`/`disabled`
predicate with one `submit_gate` call, binds the returned pair to the buttons'
`disabled` and `on:click`, and passes both `field_class` and `class` to
`ComposerFields` explicitly (spec AC20 — no site relies on a default).

- **`CompactComposer`** (`:545-551`):
  `also_blocked = Signal::derive(move || !state.summary_field.is_valid())` (no
  slug in this shape); `on_submit` dispatches
  `Create { post: state.inputs(body, publish, None) }`.
  `field_class="j-composer-field"`, `class=textarea_class`. The hand-spelled
  `trim().is_empty()` must be **gone** — it is the ADR-0065 breach.
- **`FullComposer`** (`:619-624`):
  `also_blocked = Signal::derive(move || !slug_field.is_valid() || !state.summary_field.is_valid())`;
  `state.inputs(body, publish, slug_field.parsed())`.
  `field_class="j-composer-field"`, and pass `class` explicitly rather than
  taking the `j-edit-form-textarea` default. This is the site with the live
  dead-button defect.
- **`EditPostForm`** (`:1096-1114`): same; `on_submit` dispatches
  `super::Update { post_id, post: state.inputs(body, publish, slug_field.parsed()) }`.
  Pass the gate's `disabled` to `EditSaveActions`' existing `disabled` prop and
  its callback to `on_save` — `EditSaveActions` itself is **not** modified.
  `field_class="j-edit-form-field j-edit-form-field--body"` (both classes
  already exist, `server/assets/jaunder.css:1077-1084`, and are currently
  unused), `class="j-edit-form-textarea"`.

Also fix the stale citation at `:542`: rewrite the comment to describe the new
shape (the gate and the payload are one call, so there is no dropped dispatch
left to explain) and cite **ADR-0105** plus the draft path.

- [x] **Step 11: Verify both targets, and that the swallow is gone**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-860-post-dispatch-body -- cargo xtask check`
Expected: PASS, including `wasm-clippy` and `thin-components`. If a component
reports "setup complexity N exceeds budget 2", the remedy is moving the
offending branch into a host-tested function in `compose_state.rs` — not
inlining it differently.

Then run:
`rg -n 'trim\(\)\.is_empty\(\)|let Ok\(body\)|\.ok\(\)\?|if let Some\(post\)' web/src/posts/component.rs`
Expected: no matches. (These four patterns currently match exactly `:546`,
`:551`, `:620` and `:1097` — the lines being removed.)

Two deviations found while implementing, both folded in:

- `ValidatedTextarea`'s new `on_input` needed `#[prop(optional_no_strip)]`, not
  `#[prop(optional)]` — on an `Option<_>` the latter generates a `strip_option`
  setter taking the inner type, which will not accept `ComposerFields`'
  forwarded `Option`. This is the same trap `Labelled`'s `help` prop documents.
- `submit_gate` needed `#[must_use]` (clippy, `-D warnings`).

- [ ] **Step 12: Commit**

```bash
git add web/src/posts/compose_state.rs web/src/posts/mod.rs web/src/posts/component.rs
git commit -m "fix(posts): a submit gate owns its parse; no form can present a dead button (#860)"
```

---

### Task 5: CSS for the interposed `<label>` wrapper

**Files:**

- Modify: `server/assets/jaunder.css` (near `.j-composer-body` `:369-381`)

**Interfaces:**

- Consumes: the `field_class` values Task 4 Step 10 passes — `j-composer-field`
  (both composer shapes) and `j-edit-form-field j-edit-form-field--body` (the
  editor).

> Why this is needed: `.j-composer-body` is
> `display:flex; flex-direction:column; gap:10px` (`:374-377`). `<Labelled>`
> interposes a `<label>`, so the **`<label>`** becomes the flex item and the
> textarea no longer participates in that column layout.
> (`.j-composer-body textarea` at `:379-381` is a plain _descendant_ selector,
> so `width:100%` still applies — the breakage is flex participation, not
> width.)

- [x] **Step 1: Add the composer field rule**

```css
/* The body control's wrapper. `<Labelled>` interposes a `<label>` between the composer
   column and the textarea (#860), so the `<label>` — not the textarea — is now the flex
   item of `.j-composer-body`. Restate the column layout one level down. */
.j-composer-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}
```

- [x] **Step 2: Confirm the full compose page needs nothing further**

`.j-compose-body` (`:730-734`) is a plain block with no `display:flex`, and no
`.j-compose-body > textarea` rule exists, so the wrapper is an ordinary block
child. Verify with `rg -n 'j-compose-body' server/assets/jaunder.css` before
concluding.

- [x] **Step 3: Confirm the editor needs no new class**

`.j-edit-form-field` (`:1077-1081`,
`display:flex; flex-direction:column; gap:6px`) and `.j-edit-form-field--body`
(`:1082-1084`, `flex:1`) already exist and are currently unused anywhere in
`web/src`. Task 4 passes both, so the editor's body control finally consumes the
styling that was written for it. No new rule.

- [x] **Step 4: Verify in a browser**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-860-post-dispatch-body -- cargo xtask e2e sqlite chromium`
Expected: PASS — the composer and editor specs exercise all three surfaces.

- [x] **Step 5: Commit**

```bash
git add server/assets/jaunder.css
git commit -m "style(web): size the body field's new label wrapper (#860)"
```

---

### Task 6: e2e regressions and `SEL.error` strict-mode hygiene

**Files:**

- Modify: `end2end/tests/posts.spec.ts`
- Modify (only if Step 1 requires it): `end2end/tests/selectors.ts`

**Interfaces:**

- Consumes: `SEL.postBody` (`selectors.ts:14`), `SEL.publishButton`, `SEL.error`
  (`selectors.ts:21`, `".error"`).

- [ ] **Step 1: Audit every `SEL.error` assertion on a composer/editor surface**

The body field now renders its own touched-gated `<p class="error">`
(`web/src/forms/component.rs:63`), so `.error` can match two nodes and
Playwright's strict mode fails a multi-match `expect`. The at-risk sites are
exactly `posts.spec.ts:78`, `:272`, `:277`, `:567` and `helpers.ts:267`, `:271`,
`:359` — every other `SEL.error` site renders no composer. For each, decide
whether its flow can leave the body **both** unparseable **and** blurred.

`posts.spec.ts:78` is safe — but not for the obvious reason: filling the summary
field at `:75-76` _does_ blur the body. It is safe because the body was filled
with valid text at `:73`, so no body error exists. Confirm each site on that
basis, and scope any genuinely at-risk assertion to the flash or to the body
field rather than weakening it to `.first()`.

- [ ] **Step 2: Write the failing regression tests**

Append to `end2end/tests/posts.spec.ts`. These are the e2e half of spec AC1,
AC2, AC3 and AC5 — the surfaces no host test can reach. `/posts/new` renders
`FullComposer` (`component.rs:509-524`), so `SEL.publishButton("true")` is
unique on the page.

```ts
// #860: the full compose page's submit controls had NO body clause in their disabled
// predicate, so an empty body left "Publish" enabled and clicking it did nothing at
// all — no error, no message, no state change. The gate and the parse are now one call
// (docs/adr/drafts/submit-gate-owns-its-parse.md), so a control that cannot dispatch
// cannot be pressed.
test("an empty body disables the compose page's submit controls", async ({
  registeredPage: page,
}) => {
  await goto(page, "/posts/new");

  await expect(page.locator(SEL.publishButton("true"))).toBeDisabled();
  await expect(page.locator(SEL.publishButton("false"))).toBeDisabled();

  // Whitespace-only is rejected by PostBody::from_str just as empty is.
  await page.fill(SEL.postBody, "   \n\t ");
  await expect(page.locator(SEL.publishButton("true"))).toBeDisabled();

  await page.fill(SEL.postBody, "real body text");
  await expect(page.locator(SEL.publishButton("true"))).toBeEnabled();
});

// #860: a rejected body is now visible, like every other rejection in these forms.
// Gated on touch, so a pristine composer stays quiet.
test("a blurred blank body shows the newtype's own message", async ({
  registeredPage: page,
}) => {
  await goto(page, "/posts/new");

  const bodyError = page.locator("p.error", {
    hasText: "post body must contain at least one non-blank line",
  });
  await expect(bodyError).toHaveCount(0);

  await page.locator(SEL.postBody).click();
  await page.locator(SEL.postBody).blur();

  await expect(bodyError).toBeVisible();
});

// #860: the editor had the same missing body clause — clearing the textarea left Save
// enabled and silently inert.
test("clearing the body in the editor disables save", async ({
  registeredPage: page,
}) => {
  test.slow();
  // Use this file's existing create-then-edit helpers; see the edit tests from :86.
  await expect(page.locator(SEL.publishButton("true"))).toBeEnabled();
  await page.fill(SEL.postBody, "");
  await expect(page.locator(SEL.publishButton("true"))).toBeDisabled();
  await page.fill(SEL.postBody, "restored body");
  await expect(page.locator(SEL.publishButton("true"))).toBeEnabled();
});
```

The third test's setup is left to the file's own conventions deliberately — read
the existing edit tests (from `posts.spec.ts:86`) and reuse whatever they call
to create a post and open its editor. Do not invent helper names.

- [ ] **Step 3: Run the e2e suite**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-860-post-dispatch-body -- cargo xtask e2e sqlite chromium`
Expected: PASS — the three new tests plus every existing post spec.

- [ ] **Step 4: Commit**

```bash
git add end2end/tests/posts.spec.ts end2end/tests/selectors.ts
git commit -m "test(e2e): pin the composer and editor body gates (#860)"
```

---

### Task 7: Full gate and branch review

- [ ] **Step 1: Run the complete local gate**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-860-post-dispatch-body -- cargo xtask validate`
Expected: PASS — static, clippy, coverage, and all four
`{sqlite,postgres}×{chromium,firefox}` e2e combos. Use the Bash tool's
background mode; this is a long, cold run.

- [ ] **Step 2: Confirm no stale ADR-0102 citation survives**

Run: `rg -n 'ADR-0102' web/ common/` Expected: no matches. (ADR-0102 remains a
live record — the config-key closed registry — and may be cited legitimately
elsewhere in the repo; this check is scoped to the two crates whose citations
were wrong.)

- [ ] **Step 3: Review the whole branch against the spec**

Run `git diff wt-base-issue-860..HEAD` and walk spec AC1–AC25. Confirm the ADR
draft is still **uncommitted** (`git status` should show it as ignored, not
staged) — `jaunder-ship` promotes and commits it. Then hand off to
`jaunder-ship`.
