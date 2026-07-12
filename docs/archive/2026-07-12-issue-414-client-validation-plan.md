# Client-side domain-value validation — Implementation Plan

> **For agentic workers:** Execute task-by-task with **jaunder-iterate**
> (delegating a task to a subagent via **jaunder-dispatch** when useful). Steps
> use `- [ ]` checkboxes.

**Goal:** Build the reusable client-side validation pattern (`field_error<T>`,
`Field<T>`, `<ValidatedInput<T>>`) in `web/src/forms.rs` and adopt it on the
login form as the worked example — a typed `Username` wire arg +
client-validated `Password`, disable-until-valid.

**Architecture:** One new ungated module `web/src/forms.rs` (ADR-0056 — no
`target_arch`). Pure `field_error<T>` is host-tested/coverage-measured;
`Field<T>`'s reactive methods are `#[client_only]`-exempt; `<ValidatedInput<T>>`
is a `#[component]` (host-compiles dead-but- exempt). Component behavior is
verified via the existing login e2e.

**Tech Stack:** Rust, leptos 0.8 (CSR), `common` newtypes' `FromStr`
(wasm-callable), `macros::client_only`.

**Spec:**
[`docs/superpowers/specs/2026-07-12-issue-414-client-validation.md`](../specs/2026-07-12-issue-414-client-validation.md)
— read its "The pieces" + "Design decisions". This plan is "how".

## Global Constraints

- **ADR-0056:** new code is **ungated** in `web/src/forms.rs`; **no
  `#[cfg(target_arch)]`**, **not** in `pages/`.
- **Coverage:** `field_error<T>` is measured → **must be host-tested**. Every
  `Field<T>` method (constructors included — they create `RwSignal`s) carries
  **`#[client_only]`** (`use macros::client_only;`, as `web/src/reactive.rs`
  does). `<ValidatedInput>` is a `#[component]` (exempt). Net: the gate stays
  green with no new measured-but-untested lines.
- **`Field<T>` `Copy`/`Clone` are hand-written**, never `#[derive]`d (derive
  would inject `T: Copy`/`T: Clone`, which the `String`-backed newtypes don't
  satisfy).
- **Constructors seed `error`** via `field_error::<T>(initial)` so a pristine
  field is invalid (disable-until-valid must gate the empty form).
- **Preserve e2e selectors:** the `<input>`s keep `name="username"` /
  `name="password"` and the submit button stays a `type="submit"` —
  `end2end/tests/auth.spec.ts` drives them.
- **Username is a valid typed wire arg today** (its `#[serde(try_from/into)]`
  bridge, `common/src/username.rs:13-14`); **Password stays `String`** (secret,
  no serde).
- **Gate:** `cargo xtask check` clean before each commit (**jaunder-commit**);
  **no `Co-Authored-By`**.

## Task list (review layer)

1. **`field_error<T>` + host tests** — the measured pure core; register
   `pub mod forms`.
2. **`Field<T>`** — `Copy` handle, hand `Copy`/`Clone`, seeded ctors,
   `#[client_only]` methods.
3. **`<ValidatedInput<T>>`** — generic `#[component]`, `transform` prop,
   touched-gated error.
4. **Worked example** — type `login`'s `username` arg; swap `LoginPage` to
   `ValidatedInput`s
   - disable-until-valid; login e2e green.
5. **Bundle-size delta** — informational measurement, recorded.

**Key risks/decisions:** (a) coverage rests on `#[client_only]` (Field) +
`#[component]` (ValidatedInput) exemptions — verified present in `web`
(reactive.rs); (b) leptos generic component `T` is **turbofished at the tag**,
inferred into the `field` prop; (c) disable- until-valid needs the **seeded**
initial `error` or the empty form submits.

**Separable concerns:** already filed — **#416** (Tag re-impl drift), **#417**
(request- aggregate types); the boilerplate macro is noted in the ADR. No new
filing.

**ADR:** already authored (design phase) at
[`docs/adr/0065-client-side-domain-validation.md`](../../adr/0065-client-side-domain-validation.md)
— numberless; `cargo xtask adr promote` numbers it at ship. **No writing task**
(AC7's "ADR drafted" is already satisfied); keep it current if a task surfaces a
new decision.

---

### Task 1: `field_error<T>` + host tests + module registration

**Files:**

- Create: `web/src/forms.rs`
- Modify: `web/src/lib.rs` (add `pub mod forms;` — ungated, alphabetical among
  the cross-cutting modules like `error`, `reactive`)
- Test: in-file `#[cfg(test)] mod tests` in `web/src/forms.rs`

**Interfaces:**

- Produces:
  `pub fn field_error<T>(input: &str) -> Option<String> where T: FromStr, T::Err: Display`

- [ ] **Step 1: Write the failing host tests** (`web/src/forms.rs`):

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      // `common` has no top-level re-exports — qualify by module.
      use common::password::Password;
      use common::slug::Slug;
      use common::tag::Tag;
      use common::username::Username;

      #[test]
      fn valid_input_is_none() {
          assert_eq!(field_error::<Username>("alice"), None);
          assert_eq!(field_error::<Tag>("rust"), None);
          assert_eq!(field_error::<Slug>("hello"), None);
          assert_eq!(field_error::<Password>("hunter2!"), None); // >= 8 chars
      }

      #[test]
      fn invalid_input_is_the_newtypes_own_message() {
          // Message is exactly the newtype's FromStr::Err Display (one source of truth).
          assert_eq!(
              field_error::<Username>("a b").as_deref(),
              Some("username must be non-empty and match [a-z0-9_-]+"),
          );
          assert_eq!(field_error::<Username>(""), Some("username must be non-empty and match [a-z0-9_-]+".to_owned()));
          assert!(field_error::<Password>("short").is_some()); // < 8 chars
          assert!(field_error::<Tag>("Bad Tag").is_some());
      }
  }
  ```

  (Confirm the exact `Username`/`Password` messages against their
  `#[error(...)]` strings and adjust the literals if they differ — the test pins
  that the message _is_ the newtype's.)

- [ ] **Step 2: Run, verify FAIL.** `cargo nextest run -p web field_error` (or
      `forms::tests`). Expected: FAIL — `field_error` / `mod forms` not defined.

- [ ] **Step 3: Implement.** In `web/src/forms.rs`:

  ```rust
  use std::fmt::Display;
  use std::str::FromStr;

  /// `None` when `input` parses into the domain newtype `T`; otherwise the newtype's own
  /// validation message (its `FromStr::Err` `Display`). The single client/server validation
  /// source — the same `FromStr` the typed `#[server]`-arg `Deserialize` routes through.
  #[must_use]
  pub fn field_error<T>(input: &str) -> Option<String>
  where
      T: FromStr,
      T::Err: Display,
  {
      input.parse::<T>().err().map(|e| e.to_string())
  }
  ```

  Add `pub mod forms;` to `web/src/lib.rs`.

- [ ] **Step 4: Run, verify PASS.** `cargo nextest run -p web forms`. Expected:
      PASS.

- [ ] **Step 5: Commit** (`cargo xtask check` clean first).
  ```bash
  git add web/src/forms.rs web/src/lib.rs
  git commit -m "feat(web): field_error<T> — the shared client/server validation core (#414)"
  ```

---

### Task 2: `Field<T>` — the form-field state handle

**Files:** Modify `web/src/forms.rs`.

**Interfaces:**

- Consumes: `field_error` (Task 1); `macros::client_only`.
- Produces: `pub struct Field<T>` (`Copy`) with
  `new`/`prefilled`/`is_valid`/`parsed`/ `touch`/`reset` and public fields
  `value: RwSignal<String>`, `error: RwSignal<Option<String>>`.

_No host unit test: `Field` is reactive (`RwSignal`), `#[client_only]`-exempt,
and needs a leptos runtime — its behavior is covered by the Task-4 worked
example + e2e. Verification is `cargo xtask check` (compiles, clippy clean,
coverage gate green with the exemptions)._

- [ ] **Step 1: Implement** in `web/src/forms.rs`:

  ```rust
  use std::marker::PhantomData;
  use leptos::prelude::*;
  use macros::client_only;

  /// A validated form field: its live input value + current validation error, bundled so a
  /// form declares one `Copy` handle per field.
  pub struct Field<T: 'static> {
      pub value: RwSignal<String>,
      pub error: RwSignal<Option<String>>, // always the true validity; None = valid
      touched: RwSignal<bool>,             // gates only message *visibility*
      _ty: PhantomData<T>,
  }

  // Hand-written, unconditional: `Field` holds no `T` by value (only `PhantomData<T>`), so it
  // is `Copy` for every `T` — a `#[derive]` would wrongly demand `T: Copy`/`T: Clone`.
  impl<T> Copy for Field<T> {}
  impl<T> Clone for Field<T> {
      fn clone(&self) -> Self { *self }
  }

  impl<T> Field<T>
  where
      T: FromStr + 'static,
      T::Err: Display,
  {
      #[client_only]
      pub fn new() -> Self { Self::prefilled("") }

      /// Seed `error` from `initial` so a pristine field is already invalid (disable-until-
      /// valid must gate the empty form).
      #[client_only]
      pub fn prefilled(initial: &str) -> Self {
          Self {
              value: RwSignal::new(initial.to_owned()),
              error: RwSignal::new(field_error::<T>(initial)),
              touched: RwSignal::new(false),
              _ty: PhantomData,
          }
      }

      #[client_only]
      pub fn is_valid(&self) -> bool { self.error.get().is_none() }

      /// The already-parsed value (`None` if invalid) — the seam for request-aggregate DTOs (#417).
      #[client_only]
      pub fn parsed(&self) -> Option<T> { self.value.get().parse::<T>().ok() }

      #[client_only]
      pub fn touch(&self) { self.touched.set(true); }

      #[client_only]
      pub fn is_touched(&self) -> bool { self.touched.get() }

      #[client_only]
      pub fn reset(&self) {
          self.value.set(String::new());
          self.error.set(field_error::<T>(""));
          self.touched.set(false);
      }
  }
  ```

  (`Default` is intentionally omitted — clippy's `new_without_default` is
  satisfied because `new` carries the `T: FromStr` bound; add `#[allow]` only if
  the lint fires, with note.)

- [ ] **Step 2: Verify.** `cargo xtask check` — compiles, clippy clean, coverage
      gate green (every `Field` method `#[client_only]`-exempt; no new measured
      lines). Sanity-check `Copy`: a throwaway
      `let a = Field::<Username>::new(); let _b = a; let _c = a;` compiles (do
      **not** commit it — it needs a reactive owner; just confirm the trait
      bound resolves).

- [ ] **Step 3: Commit** (`cargo xtask check` clean).
  ```bash
  git add web/src/forms.rs
  git commit -m "feat(web): Field<T> — Copy handle bundling a form field's value + validity (#414)"
  ```

---

### Task 3: `<ValidatedInput<T>>` — the component

**Files:** Modify `web/src/forms.rs`.

**Interfaces:**

- Consumes: `field_error`, `Field<T>`.
- Produces: `#[component] pub fn ValidatedInput<T>(...) -> impl IntoView` per
  the spec signature (label, name, `field: Field<T>`, `input_type` default
  `"text"`, optional `autocomplete`, optional
  `transform: fn(String) -> String`).

_Reactive `#[component]` (coverage-exempt); behavior verified via the Task-4
e2e._

- [ ] **Step 1: Implement** in `web/src/forms.rs`:

  ```rust
  /// A labelled input bound to a `Field<T>`: validates on input via `field_error::<T>`, shows
  /// the newtype's message inline once the field is touched (blur). `name` MUST match the
  /// `#[server]` struct field and the e2e selector.
  #[component]
  pub fn ValidatedInput<T>(
      label: &'static str,
      name: &'static str,
      field: Field<T>,
      #[prop(default = "text")] input_type: &'static str,
      #[prop(optional)] autocomplete: Option<&'static str>,
      // `fn(&str) -> String` so `transform=str::to_lowercase` binds directly (a
      // `fn(String)->String` prop would reject `str::to_lowercase`, whose sig is `&str`).
      #[prop(optional)] transform: Option<fn(&str) -> String>,
  ) -> impl IntoView
  where
      T: FromStr + 'static,
      T::Err: Display,
  {
      let on_input = move |ev| {
          let raw = event_target_value(&ev);
          let v = match transform { Some(f) => f(&raw), None => raw };
          field.value.set(v.clone());
          field.error.set(field_error::<T>(&v));
      };
      view! {
          <label class="j-form-field">
              <span class="j-form-label">{label}</span>
              <input
                  class="j-form-input"
                  type=input_type
                  name=name
                  autocomplete=autocomplete
                  prop:value=field.value
                  on:input=on_input
                  on:blur=move |_| field.touch()
              />
              {move || {
                  (field.is_touched())
                      .then(|| field.error.get())
                      .flatten()
                      .map(|msg| view! { <p class="error">{msg}</p> })
              }}
          </label>
      }
  }
  ```

  (Verify the leptos 0.8 attr syntax for an `Option` `autocomplete` — if
  `attr:autocomplete` or a conditional is needed, adjust; the input's
  class/markup mirrors the current `pages/auth.rs` hand-rolled field.)

- [ ] **Step 2: Verify.** `cargo xtask check` — compiles, clippy clean, coverage
      green.

- [ ] **Step 3: Commit** (`cargo xtask check` clean).
  ```bash
  git add web/src/forms.rs
  git commit -m "feat(web): <ValidatedInput<T>> component — inline, touched-gated field validation (#414)"
  ```

---

### Task 4: Worked example — the login form

**Files:**

- Modify: `web/src/auth/mod.rs` (`login`, ~147)
- Modify: `web/src/pages/auth.rs` (`LoginPage`, ~98)

**Interfaces:**

- Consumes: `crate::forms::{Field, ValidatedInput}`.
- `login` signature becomes
  `login(username: Username, password: String, label: Option<String>)`.

- [ ] **Step 1: Type the `login` arg** (`web/src/auth/mod.rs`). Change
      `username: String` → `username: Username` in the signature; **delete** the
      `parse_username` block (lines ~151-154) — the arg is already a validated,
      lowercased `Username` (its serde bridge parses via `FromStr`, which
      lowercases). Keep `password.parse::<Password>()?`. The
      `authenticate(&username, &password)` call is unchanged (already
      `&Username`). Remove the now- dead `web.auth.login.parse_username` span.
      **Relocate the `Username` import:** it currently sits inside the
      `#[cfg(feature = "server")]` block (~lines 18-28), but the `#[server]`
      macro emits the `Login { username: Username, … }` struct
      **unconditionally** (client + server), so the wasm build needs `Username`
      in scope. Add a top-level `use common::username::Username;` and **remove**
      it from the server-only block (leaving it in both is an E0252
      duplicate-import on the server build). `Password` stays server-gated —
      it's only used in the body, not the signature.

- [ ] **Step 2: Swap `LoginPage` inputs** (`web/src/pages/auth.rs`). Replace the
      two hand-rolled `<label class="j-form-field">…</label>` inputs (lines
      ~119-140) with:

  ```rust
  use crate::forms::{Field, ValidatedInput};
  use common::password::Password;
  use common::username::Username;
  // …
  let username = Field::<Username>::new();
  let password = Field::<Password>::new();
  // (replace the old `let username = RwSignal::new(...)`; update the marker Effect to
  //  `crate::auth::marker::set(&username.value.get_untracked())`.)

  <ValidatedInput<Username>
      label="Username" name="username" autocomplete="username"
      field=username transform=str::to_lowercase
  />
  <ValidatedInput<Password>
      label="Password" name="password" input_type="password"
      autocomplete="current-password" field=password
  />
  ```

  Add `prop:disabled=move || !(username.is_valid() && password.is_valid())` to
  the submit `<button>` (line ~143). Leave the `ActionForm`, the result-render
  block, and the button's `type="submit"` unchanged.

- [ ] **Step 3: Verify — compile + coverage.** `cargo xtask check`. Expected:
      green (the `login` change is net-deletion; `LoginPage` is a
      `#[component]`, exempt).

- [ ] **Step 4: Verify — behavior (login e2e).** Run the auth e2e:
      `cargo xtask e2e sqlite chromium` (or rely on the ship-time `validate`).
      Expected: PASS — `end2end/tests/auth.spec.ts` ("login page shows form",
      "login with valid credentials succeeds", "login with wrong password shows
      error") stays green; the submit button is disabled until both fields
      parse. If a spec asserts an always-enabled button or submits an empty
      form, update it to the disable-until-valid behavior (note the change).

- [ ] **Step 5: Commit** (`cargo xtask check` clean).
  ```bash
  git add web/src/auth/mod.rs web/src/pages/auth.rs
  git commit -m "feat(web): login uses ValidatedInput + typed Username wire arg (#414)"
  ```

---

### Task 5: Bundle-size delta (informational)

**Files:** none (measurement recorded in the PR description at ship).

- [ ] **Step 1: Measure.** Build the CSR wasm bundle for `web` (the project's
      csr build, e.g. via the `csr` crate / `cargo leptos build`) at
      `wt-base-issue-414` and at this branch's HEAD; record the `.wasm` byte
      delta from reusing `common`'s `FromStr` in the client. Purely
      **informational** (not a gate) — decision 1 stands regardless. If the
      delta is surprisingly large (> ~50 KB), note it for discussion rather than
      acting.
- [ ] **Step 2:** No commit — record the number for the ship PR body.

---

## Definition of done

- `cargo xtask validate` clean (incl. the auth e2e).
- `field_error<T>` host-tested; `Field<T>`/`<ValidatedInput>` compile,
  clippy-clean, coverage- exempt; login form disables submit until valid, shows
  touched-gated inline errors, and sends a typed `Username`.
- ADR draft in place (promoted at ship); bundle delta recorded.
