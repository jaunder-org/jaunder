# ADR-0113: A submit gate owns its parse

- Status: accepted
- Date: 2026-08-11
- Issue: [#860](https://github.com/jaunder-org/jaunder/issues/860)
- Note: amended 2026-09-03 (#907) — `Field<T>` now privately owns its value and
  derives validity from it, so `is_valid()` and `parsed()` are two views of one
  authoritative current input rather than independently writable sources

## Context

[ADR-0065](0065-client-side-domain-validation.md) settled _which_ validator a
client-side form uses: the shared newtype's `FromStr`, never a re-spelling of
its rule. It did not settle _where the parse lives_ relative to the control it
gates.

The prevailing shape in `web/src` was two separate expressions — a `disabled`
predicate on the button, and an independent parse inside the click handler that
dropped its failure on the floor:

```rust
let dispatch = move |publish| {
    if let Some(post) = state.inputs(publish, slug) { action.dispatch(...); }
};
let submit_disabled = move || /* a separately-written predicate */;
```

Those two are kept in agreement by _placement_, not by the type system. #860 is
what happens when they drift: two of the three post forms had lost the body
clause from their `disabled` predicate entirely, so the button was enabled, the
parse failed, and the click did nothing — no error, no message, no state change.
A dead button is the hardest class of UI defect to report or reproduce, because
the user has nothing to describe.

The drift is not hypothetical bad luck. It is the predictable outcome of a shape
where adding a submit path, or editing one predicate and not the other, is
silently survivable.

Before #907, `forms::Field<T>` made it worse in one specific way: `is_valid()`
read a separately written `error` signal while `parsed()` re-read `value`. A
programmatic write to `value` updated one and not the other, so even "gate on
`is_valid`, dispatch on `parsed`" was two sources wearing one type's clothes.

## Decision

**A form control's `disabled` state and the payload it dispatches must come from
one authoritative parse source.**

Concretely, for any control that submits a validated value:

1. The dispatch closure **receives an already-validated value**. It does not
   parse, and it therefore has no error arm to swallow.
2. The gate and payload derive from one current input. A separately stored
   validity flag, cheap proxy, or independently authored validation expression
   alongside payload parsing is prohibited.

   `Field<T>` enforces this boundary after #907: its value signal is private and
   its read-only error memo, `is_valid()`, and `parsed()` all derive from that
   same current value. A gate may therefore use `is_valid()` while payload
   assembly uses `parsed()` without creating a second validation source.
   Optional blank input deliberately has `is_valid() == true` and
   `parsed() == None`: absence is a valid payload state, not a swallowed parse
   failure. This fulfills the former `slug_override` and `summary` debt named by
   this decision.

3. A silent `return`, `.ok()?`, or `let … else { return }` **in a form's
   dispatch closure is a defect, not a defence.** If such an arm is reachable it
   is a dead control.

   The rule is about _where_ the arm may live, not a claim that it can be
   eliminated: a reactive click handler is a `Fn`, so it must be total, and some
   arm has to cover "there is no value". This clause confines that arm to **one
   place — the gate helper itself** — where it is co-conditioned with the
   `disabled` signal by construction and is directly tested. `submit_gate`'s

   ```rust
   if let Some(body) = body.parsed() { on_submit.run((body, publish)); }
   ```

   is that one arm. It is not an exception to the rule; it is the reason form
   authors never have to write one. A second such arm appearing in a form is the
   defect this clause names.

4. A rejected value is **shown** to the user, gated on touch — consistent with
   every other rejection the forms already surface.

The first realization is `submit_gate`, which takes the field, any additional
blocking predicate, and a `Callback<(T, bool)>`, and returns the `disabled`
signal and click callback the button markup consumes. It is a plain function,
not a component, so it imposes no markup.

**It must live in a host-compiled module** — for the posts vertical,
`web/src/posts/compose_state.rs`. A `#[component]` module such as
`web/src/posts/component.rs` is declared `#[cfg(target_arch = "wasm32")]` under
[ADR-0070](0070-web-vertical-wasm-only-component-files.md), so a gate placed
there would be neither host-testable nor coverage-measured — which would defeat
the point of extracting it. The decision fold goes where the state bundle
already lives; only the markup stays wasm-only.

## Consequences

- A state bundle's payload constructor becomes **infallible**: it takes the
  parsed newtype rather than re-deriving it. `ComposeState::inputs` returns
  `PostInputs`, not `Option<PostInputs>`.
- Every text input backing a dispatch becomes a `Field<T>` rather than a bare
  `RwSignal<String>`, so the newtype door is the only door. The composer body
  was the last holdout in the posts forms.
- "The button is disabled" and "there is no required payload" become the same
  condition within a gate that follows this rule, so the class of defect #860
  reports cannot recur by editing one site. The gate helper enforces this for
  its submitted value; `Field<T>` enforces the shared current-input source for
  every additional field.
- This rules out gating on a cheap proxy, separately written validity flag, or
  second validation expression while dispatching from the real parse. A
  `Field<T>` validity read is not such a proxy: it is derived from the same
  private input as `parsed()`.
- It does **not** claim a disabled button is a security boundary. The server's
  typed `#[server]` args remain the guarantee; this ADR is about the client
  never presenting a control that cannot work.

Relates to [ADR-0065](0065-client-side-domain-validation.md) (which validator)
and [ADR-0105](0105-post-body-non-blank-invariant.md) (a blank body is
unrepresentable).
