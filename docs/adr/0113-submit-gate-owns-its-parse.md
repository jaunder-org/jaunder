# ADR-0113: A submit gate owns its parse

- Status: accepted
- Date: 2026-08-11
- Issue: [#860](https://github.com/jaunder-org/jaunder/issues/860)

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

`forms::Field<T>` makes it worse in one specific way: `is_valid()` reads a
cached `error` signal while `parsed()` re-reads `value`. A programmatic write to
`value` updates one and not the other, so even "gate on `is_valid`, dispatch on
`parsed`" is two sources wearing one type's clothes.

## Decision

**A form control's `disabled` state and the payload it dispatches must be
produced by the same call.**

Concretely, for any control that submits a validated value:

1. The dispatch closure **receives an already-validated value**. It does not
   parse, and it therefore has no error arm to swallow.
2. The gate and the payload derive from **one expression over the field's
   current input** — for a `Field<T>`, `parsed()`. A gate written against
   `is_valid()` while the payload comes from `parsed()` is **two** sources and
   is prohibited.

   **Known non-conforming call sites at the time of writing:** the
   `slug_override` and `summary` fields of the post composer and editor still
   gate on `is_valid()` while taking their payload from `parsed()`. Bringing
   them into conformance means deriving `Field::error` from `Field::value` for
   every form in `web/src`; that is tracked by
   [#907](https://github.com/jaunder-org/jaunder/issues/907). This clause is the
   target state, and those sites are the outstanding debt against it — not a
   silent exemption.

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
- "The button is disabled" and "there is no payload" become the same condition
  **within a gate that follows this rule**, so the class of defect #860 reports
  cannot recur by editing one site. Note this is enforced by review and by the
  gate helper's shape, not by the type system: `Field::value` and `Field::error`
  are public fields, so a caller can still write one without the other. Making
  the desync inexpressible is the follow-up named above.
- This rules out the convenience of gating a control on a cheap proxy (a length
  check, a cached validity flag) while dispatching from the real parse. The
  proxy must go.
- It does **not** claim a disabled button is a security boundary. The server's
  typed `#[server]` args remain the guarantee; this ADR is about the client
  never presenting a control that cannot work.
- Follow-up it creates: `Field::error` is a separately-written `RwSignal`, which
  is why clause 2 has to name `is_valid()` as prohibited. Deriving `error` from
  `value` would make the prohibition unnecessary; that is
  [#907](https://github.com/jaunder-org/jaunder/issues/907), and it touches
  every form in `web/src`.

Relates to [ADR-0065](0065-client-side-domain-validation.md) (which validator)
and [ADR-0105](0105-post-body-non-blank-invariant.md) (a blank body is
unrepresentable).
