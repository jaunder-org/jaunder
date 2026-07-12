# Spec — issue #414: client-side domain-value validation pattern

- Issue: [#414](https://github.com/jaunder-org/jaunder/issues/414)
- Milestone: #13 Domain-value type safety (newtypes)
- ADRs: draft (this issue) records the convention; extends
  [ADR-0063](../../adr/0063-domain-value-newtype-convention.md) §4 (boundary rule);
  constrained by [ADR-0055](../../adr/0055-web-host-wasm-boundary-module-level.md) /
  [ADR-0056](../../adr/0056-web-canonical-colocated-leptos.md) (web host/wasm boundary),
  [ADR-0040](../../adr/0040-web-rendering-leptos-csr.md) (leptos-CSR).
- Date: 2026-07-12

## Goal

The foundation that lets the #404 verticals **type the `#[server]` wire boundary**
with domain newtypes without the degraded error UX #350/#14 warned about. Provide a
reusable Leptos component that validates a form field by parsing input into a domain
newtype — the **same `FromStr` machinery** the server's typed-arg `Deserialize` uses —
and shows an **inline, client-local** error before submit, so a typed `#[server]` arg
is only ever sent a value that already parses.

## The problem it solves (confirmed in code)

`WebError: FromServerFnError` (`web/src/error.rs:68-74`) maps any framework/decode
error — including a typed `#[server]`-arg `Deserialize` failure — to the generic
`WebError::ServerFunction` variant, never `WebError::Validation`. So typing a wire arg
naively degrades the user-facing error on invalid input. Client-side pre-validation
with the newtype's own `FromStr` makes the typed-arg decode succeed for every
legitimate client, demoting the generic-error path to defense-in-depth.

## Design decisions (resolved in the interview)

1. **One source of truth — reuse `common`'s `FromStr` in wasm.** Call
   `input.parse::<T>()` client-side; do **not** re-implement the rule. `common` is
   already an all-target dep of `web` (`web/Cargo.toml:19`), and the newtypes' `FromStr`
   are small dependency-light checks. Re-implementing is a second source of truth that
   drifts (the concrete instance filed as **#416**). A wasm-bundle-size delta is
   recorded as an **informational** measurement, not a gate.
2. **A full `<ValidatedInput<T>>` component** over a pure `field_error<T>` core — build
   the abstraction now; verticals adopt it incrementally.
3. **`Field<T>` state handle**, parent-owned, passed into the component; exposes
   `is_valid()` (gating) and `parsed()` (the #417 forward-compat seam).
4. **Timing**: validity is computed on every input (always); the **visible** error is
   gated on a `touched` flag (set on blur) so a pristine field shows no error.
5. **Gating**: **disable-until-valid inside `ActionForm`** — `prop:disabled` on the
   submit button, driven by the fields' `is_valid()`. Lowest adoption friction; robust;
   no dependence on `ActionForm` submit-intercept internals.
6. **Error message = the newtype's `FromStr::Err` `Display`** (the same machinery), no
   per-field override in #414 (add later if a friendlier copy is wanted).
7. **Coverage boundary** (ADR-0056 accepted, superseding 0055): nothing is
   `target_arch`-gated. The pure `field_error<T>` **host-compiles and is
   coverage-measured** (host-tested). `<ValidatedInput<T>>` is a `#[component]`, so it
   host-compiles as **dead-but-exempt** code (ADR-0050 syntactic `#[component]`
   exemption — load-bearing on the host per ADR-0056). `Field<T>` is a plain struct
   (not a `#[component]`), so its reactive methods carry **`#[client_only]`**
   (ADR-0062/#370 — the sanctioned exemption for non-component client-only reactive
   helpers, already used on `Invalidator` in `web/src/reactive.rs`).

## The pieces

All three live in one new **ungated** cross-cutting module `web/src/forms.rs`
(`pub mod forms;` in `lib.rs`) — no `target_arch` gate, per ADR-0056. Coverage falls
out per-fn: `field_error` measured, `ValidatedInput` `#[component]`-exempt, `Field`'s
methods `#[client_only]`-exempt. **Not** in `pages/` (ADR-0056 is deleting it).

### `field_error<T>` — pure core (host-compiled, coverage-measured, host-tested)

```rust
/// `None` when `input` parses into the domain newtype `T`; otherwise the newtype's
/// own validation message (its `FromStr::Err` `Display`). The single client/server
/// validation chokepoint — the same `FromStr` the wire `Deserialize` routes through.
pub fn field_error<T>(input: &str) -> Option<String>
where
    T: ::std::str::FromStr,
    T::Err: ::std::fmt::Display,
{
    input.parse::<T>().err().map(|e| e.to_string())
}
```

### `Field<T>` — form-field state handle (reactive; `#[client_only]` methods)

Bundles the two signals so a form declares one handle per field.

**`Copy`/`Clone` are hand-written, not derived** — `#[derive(Copy, Clone)]` would inject
a `T: Copy`/`T: Clone` bound, but the newtypes (`Username`, `Password`, …) are not
`Copy`, so `Field<Username>` would lose `Copy`. The fields (`RwSignal<_>`,
`PhantomData<T>`) are `Copy` for **all** `T`, so the impls are unconditional:

```rust
pub struct Field<T: 'static> {
    pub value: RwSignal<String>,
    pub error: RwSignal<Option<String>>, // always the true validity; None = valid
    touched: RwSignal<bool>,             // display-gating only
    _ty: PhantomData<T>,
}
impl<T> Copy for Field<T> {}
impl<T> Clone for Field<T> { fn clone(&self) -> Self { *self } }

impl<T> Field<T> where T: FromStr + 'static, T::Err: Display {
    // Constructors SEED `error` from the initial value so a pristine empty field is
    // already invalid (else disable-until-valid would enable submit on an empty form):
    pub fn new() -> Self;              // error = field_error::<T>("")
    pub fn prefilled(s: &str) -> Self; // error = field_error::<T>(s)
    #[client_only] pub fn is_valid(&self) -> bool;    // self.error.get().is_none()
    #[client_only] pub fn parsed(&self) -> Option<T>; // self.value.get().parse().ok() — #417 seam
    #[client_only] pub fn touch(&self);               // self.touched.set(true)
    #[client_only] pub fn reset(&self);
}
```

### `<ValidatedInput<T>>` — the component (`#[component]`, ungated, exempt)

In `web/src/forms.rs`. Generic over `T` (the leptos generic is **turbofished at the
tag**, `<ValidatedInput<Username> …/>`; `T` then infers into the `field` prop — so `T`
is still written once per field).

```rust
#[component]
pub fn ValidatedInput<T>(
    label: &'static str,
    name: &'static str,              // MUST match the #[server] struct field AND the e2e
                                     // selector (name="username"/"password"), or the
                                     // auth/register/password_reset specs break.
    field: Field<T>,
    #[prop(default = "text")] input_type: &'static str,
    #[prop(optional)] autocomplete: Option<&'static str>,
    /// Live input massaging before validation/display, e.g. `str::to_lowercase` for a
    /// username, preserving the current login on:input behavior. Applied in on:input.
    #[prop(optional)] transform: Option<fn(&str) -> String>,
) -> impl IntoView
where T: FromStr + 'static, T::Err: Display
```

Renders the canonical `j-form-field` label+input; binds `prop:value=field.value` +
`on:input` (applies `transform`, updates `value`, recomputes `field.error` via
`field_error::<T>`); `on:blur` sets `touched`; renders the inline `<p class="error">`
only when `touched && error.is_some()` — the existing per-page error idiom, encapsulated.

### Worked example — the login form

- `web/src/auth/mod.rs`: change `login(username: String, …)` → `login(username: Username, …)`.
  This is a valid typed wire arg **today** — `Username` already carries the
  `#[serde(try_from = "String", into = "String")]` bridge (`common/src/username.rs:13-14`),
  no #407 needed. **No threading ripple:** the server-side `username.parse::<Username>()`
  line is *deleted* (the arg is already a `Username`), and the existing auth call receives
  it exactly as it did the post-parse value. `password` stays `String` on the wire
  (secret — `common/src/password.rs` has no serde), parsed to `Password` on entry.
- `web/src/pages/auth.rs` `LoginPage`: replace the hand-rolled username/password inputs
  with `<ValidatedInput<Username> name="username" transform=str::to_lowercase … />` (the
  `transform` preserves today's live-lowercase on:input) and
  `<ValidatedInput<Password> name="password" input_type="password" … />`; add
  `prop:disabled=move || !(user.is_valid() && pw.is_valid())` to the submit button.
  **Keep the `name="username"`/`name="password"` attributes and the submit selector** —
  the `end2end/tests/auth.spec.ts` suite drives them.
- Scope note: this edits `LoginPage` in place; it does **not** migrate the `auth` vertical
  to the ADR-0056 co-located layout (that's milestone #11). A wasm-only page importing the
  ungated `forms::ValidatedInput` is fine.
- Demonstrates in one form: typed-wire-arg validation, secret-`String`-wire validation,
  disable-until-valid gating, and touched-gated inline errors.

## ADR (draft in this cycle, promoted at ship)

`docs/adr/drafts/` — the convention: **typed `#[server]` wire args + mandatory
client-side pre-validation via the shared newtype `FromStr`; the error surfaces
client-local/inline from the newtype's `Display`; gating is disable-until-valid.**
Records: one validation source (no re-implementation — see #416); the secret exception
(no serde bridge → `String` wire arg + client validation); the pure/reactive coverage
split; and the `Field::parsed()` seam toward request aggregates (#417). Extends
ADR-0063 §4.

## Testing

- **Host unit tests** for `field_error<T>` across the real newtypes: `Username`
  (valid → `None`; `"a b"`/`""` → `Some(<its Display>)`), `Password` (short → `Some`),
  `Slug`, `Tag` — pinning that the message is the newtype's own `Display`.
- **Component/`Field` behavior** is verified via the login worked example and e2e
  (submit disabled until both fields valid; inline error appears after blur on invalid
  input; a valid login still succeeds). No wasm-bindgen-test infra exists yet, so the
  component is exercised through e2e, not a unit test (consistent with `TagInput`).

## Acceptance criteria (observable)

1. `field_error<T>` exists in a both-target module, host-tested; returns the newtype's
   `FromStr::Err` `Display` on failure and `None` on success, for each of
   Username/Slug/Tag/Password.
2. `Field<T>` provides `value`/`error`/`is_valid()`/`parsed()`/`touch()`/`reset()`.
3. `<ValidatedInput<T>>` renders a labelled input + touched-gated inline error, driving
   a parent-owned `Field<T>`.
4. The login form uses `<ValidatedInput<Username>>` + `<ValidatedInput<Password>>`, its
   submit button is `prop:disabled` until both parse (**including the pristine empty
   form** — constructors seed `error`), and `login`'s `username` arg is typed `Username`.
5. On invalid input, after blur, the user sees the newtype's message inline
   (client-local, no server round-trip); and the submit button **stays disabled while
   the username is invalid** (the observable proxy for "the typed-arg decode-error path
   is unreachable via the UI"). The existing login e2e (`end2end/tests/auth.spec.ts`)
   still passes.
6. `cargo xtask validate` clean (incl. e2e).
7. The ADR is drafted; the wasm-bundle delta from reusing `common`'s `FromStr` is
   recorded (informational).

## Separable concerns (already filed)

- **#416** — the Tag input re-implements `Tag::from_str` (drift bug); collapse onto the
  shared pattern (handled with the #409 Tag vertical). Blocked by #414.
- **#417** — explore shipping request-aggregate domain types across the boundary
  (larger, own ADR); `Field::parsed()` is its seam. Blocked by #414.
- A future **boilerplate-reducing macro** (`form_fields! { username: Username, … }`) —
  noted as a deliberate seam; file when wanted.
