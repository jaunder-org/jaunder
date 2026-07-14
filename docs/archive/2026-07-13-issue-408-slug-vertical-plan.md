# `Slug` vertical (#408) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax; tick
> them in real time.

**Goal:** Adopt `#[derive(StrNewtype)]` on `Slug` and thread `Slug` everywhere a
resolved slug travels, so no slug value is a bare `String`/`&str` — per spec
[`2026-07-13-issue-408-slug-vertical.md`](../specs/2026-07-13-issue-408-slug-vertical.md).

**Architecture:** Six tasks ordered for per-commit compile-safety. Task 1 is the
only compiler-forced change (deleting the inherent `as_str()` breaks every
`.as_str()` call), so it adopts the derive **and** sweeps all obviated sites in
one commit (mirrors #407's `e05895c7`). Tasks 2–5 are type-threading that each
leave the tree green: read path → generation-zone internal typing → forms-helper
optional support → the coupled `slug_override` write-path vertical (wire +
storage

- forms, which must change together). Task 6 records the ADR amendment.

**Tech Stack:** Rust (workspace: `common`, `storage`, `server`, `web`,
`macros`), `cargo nextest`, Leptos (web, dual-target host+wasm), `cargo xtask`
gate.

## Global Constraints

_(verbatim from the spec; every task implicitly includes these)_

- **No wire/behavior change.** `Option<Slug>` must serialize identically to
  `Option<String>` (the `StrNewtype` serde bridge serializes as a plain string);
  the projector shell-fallback UX and atompub `j:slug` output are unchanged.
- **`Slug` derive list:** `Clone, Debug, PartialEq, Eq, StrNewtype` — **no
  `Hash`** (ADR-0063; verified no `Slug`-keyed collections).
- **`Slug::from_str` stays the single construction chokepoint.** `slugify_title`
  and `candidate_slug` remain the only `-> String` slug-candidate producers,
  funneling through `from_str`; the projector inbound path segment is the only
  raw-`String` slug at an HTTP boundary — each carries a justifying comment.
- **Gate:** the pre-commit hook runs `cargo xtask check`; run it first so it
  passes clean (**jaunder-commit**). Storage tests follow the dual-backend
  template (`CONTRIBUTING.md` backend parity). **No `Co-Authored-By` trailer.**
- Governing: ADR-0063 (§4 boundary rule), ADR-0065 (typed wire args + client
  validation), ADR-0023 (`j:slug` emit-only).

---

## Review header (approve this layer)

**Scope — in:** `common/src/slug.rs` derive adoption; `Slug` through 6 web DTOs,
`get_post`, `create_post`/`update_post`, the storage generation path and
metadata structs; `ValidatedInput<Slug>` on the create/edit forms; forms-helper
optional support; the `.as_str()` sweep; ADR-0065 amendment; a projector
justification comment. **Scope — out:** the `Tag` newtype (`TagSummary.slug`,
`tag_slug`); typing the projector/feed `Path<String>` extractors (deliberately
kept — Decision 1); any wire-format change.

**Tasks:**

1. `common`: adopt `StrNewtype`, delete the hand-written trailer, sweep all
   compiler-forced `.as_str()` (common/storage/server). Green, no behavior
   change.
2. `web` read path: `get_post(slug: Slug)` + 6 DTO `slug` fields → `Slug` +
   `PostPage` route-parse; projector justification comment (Decision 1).
3. `storage` generation zone: `candidate_slug(base: &Slug)`, `slug_seed: Slug`,
   slugify output parsed at the call sites (Decision 5). Override param
   unchanged.
4. `web/src/forms.rs`: optional-field support (`Field::optional`,
   `optional_prefilled`, `error_for`) + tests (Decision 3).
5. `slug_override` write-path vertical: `create_post`/`update_post` →
   `Option<Slug>`, storage metadata → `Option<&Slug>`, drop both override
   re-parses, adopt `ValidatedInput<Slug>` on all three forms, relocate the
   invalid-override test to the boundary (Decision 2).
6. Amend **ADR-0065** with the optional-field variant paragraph.

**Key risks/decisions:** validation relocation in Task 5 (a storage test becomes
unconstructible — confirm boundary coverage before deleting); `Slug`'s generated
trailer attributes to `slug.rs` but is CRAP-safe (Risk 1, no new trailer tests);
leptosfmt mangles `<ValidatedInput<Slug>>` (#420 — mirror the `auth.rs`
workaround); optional-field `is_valid()` must leave submit **enabled** for an
empty slug.

---

## Task 1: `common` — adopt `StrNewtype`, delete trailer, sweep `.as_str()`

Spec §A, §I. The derive adoption deletes the inherent `as_str()`, making every
`.as_str()` on a `Slug` a compile error — so the sweep is compiler-forced and
must land in the same commit to keep the tree green. Pure refactor, zero
behavior change, no new tests (Risk 1: the generated trailer is tested in
`macros/tests/str_newtype.rs` and is CRAP-safe).

**Files:**

- Modify: `common/src/slug.rs:1-87` (derive + delete trailer + imports),
  `common/src/slug.rs:146-295` (in-file test `.as_str()` asserts)
- Modify: `storage/src/posts.rs:83, 981, 1869` (SQL binds / summary)
- Modify: `storage/src/post_service.rs` (10 test asserts:
  `record.slug.as_str()`)
- Modify: `server/src/atompub/mapping.rs:164` (`set_j_slug`)
- Modify: `server/tests/misc/backup_fixture.rs:196`,
  `server/tests/web/web_posts.rs:234, 1972`,
  `server/tests/atompub/atompub_posts.rs:1169, 1215` (integration-test
  `PostRecord.slug.as_str()` — `cargo xtask check` compiles these, so they must
  sweep in this commit too)

**Interfaces:**

- Produces: `Slug` with the full generated trailer — `Display`, `AsRef<str>`,
  `Borrow<str>`, `Deref<Target = str>`, `TryFrom<String>`,
  `From<Slug> for String`, `PartialEq<str>`/`<&str>`, and the validating serde
  bridge. **No** inherent `as_str()`. `FromStr`, `InvalidSlug`, `slugify_title`,
  `MAX_SLUG_CHARS`, `base_is_alphanumeric` unchanged.

- [ ] **Step 1: Adopt the derive in `common/src/slug.rs`.**
  - Replace the header (L23-25):
    ```rust
    use macros::StrNewtype;
    // ...
    #[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
    pub struct Slug(String);
    ```
    (Delete the `#[derive(... Serialize, Deserialize)]` and
    `#[serde(try_from = "String", into = "String")]` lines; **no `Hash`**.)
  - Delete `impl TryFrom<String> for Slug` (L62-68),
    `impl From<Slug> for String` (L70-74), the inherent `impl Slug { as_str }`
    (L76-81), and `impl fmt::Display for Slug` (L83-87).
  - Drop now-unused imports: `use serde::{Deserialize, Serialize};` (L3) and
    `use std::fmt` (keep `str::FromStr`). Keep `thiserror`, `unicode_*`.
  - Keep `FromStr` (L34-60), `InvalidSlug`, `base_is_alphanumeric`,
    `slugify_title`, `MAX_SLUG_CHARS` verbatim.

- [ ] **Step 2: Sweep the in-file test asserts** (`slug.rs:146-295`). The
      inherent `as_str()` is gone, so rewrite the assert idiom. Every
      `x.parse::<Slug>().unwrap().as_str()` → compare against the `&str` via
      generated `PartialEq<&str>`, e.g.:

  ```rust
  assert_eq!("hello-world".parse::<Slug>().unwrap(), "hello-world");
  ```

  For `slugify_title(word).parse::<Slug>().unwrap().as_str()` (idempotence,
  L178, L188) →
  `assert_eq!(slugify_title(word).parse::<Slug>().unwrap(), nfc.as_str());`. In
  `slug_display_returns_inner_string` (L242-246) drop the `.as_str()` assert;
  keep `s.to_string()` (Display). Leave `slugify_title` asserts (they compare
  `String == &str`, unaffected).

- [ ] **Step 3: Sweep the storage + server production sites.**
  - `storage/src/posts.rs:83` `self.slug.as_str()` → `self.slug.as_ref()` (or
    `&*self.slug`), `:981` `.bind(slug.as_str())` → `.bind(slug.as_ref())`,
    `:1869` `.bind(input.slug.as_str())` → `.bind(input.slug.as_ref())`.
  - `storage/src/post_service.rs` test asserts `record.slug.as_str()` (10 sites:
    L568, 601, 628, 747, 774, 860-862, 908-909) → compare via `PartialEq`
    (`assert_eq!(record.slug, "expected")`) or `.as_ref()` where a `&str` is
    needed.
  - `server/src/atompub/mapping.rs:164` `post.slug.as_str()` →
    `post.slug.as_ref()`. (`:141` `post.slug.to_string()` uses `Display` —
    leave.)
  - **Integration tests** (compiled by the gate): `backup_fixture.rs:196`,
    `web_posts.rs:234, 1972`, `atompub_posts.rs:1169, 1215` — each a
    `PostRecord.slug.as_str()` → `.as_ref()` or `PartialEq`. A quick
    `rg -n 'slug\.as_str\(\)' server/ storage/ common/` before committing
    catches any straggler (only `tag_slug` sites should remain — those are
    `Tag`).

- [ ] **Step 4: Run the gate.** Run: `devtool run -- cargo xtask check`
      Expected: PASS (fmt + clippy + Nix coverage/tests green). If `.as_str()`
      sites were missed, the build fails with `no method named as_str` — sweep
      those too.

- [ ] **Step 5: Commit.**
  ```bash
  git add common/src/slug.rs storage/src/posts.rs storage/src/post_service.rs server/src/atompub/mapping.rs server/tests/
  git commit -m "refactor(common): Slug adopts StrNewtype derive; sweep .as_str()"
  ```

---

## Task 2: `web` read path — `get_post(slug: Slug)` + DTOs + `PostPage` + projector comment

Spec §B, §C, Decision 1, §F (projector). Types the read/lookup path. The DTO
`slug` fields become `Slug` (serde bridge keeps the wire identical).
`get_post`'s in-body parse moves client-side into `PostPage`.
Behavior-preserving except a malformed slug in the SPA route now skips the fetch
(client 404) instead of a server `Validation` error — the projector
shell-fallback backstops bare URLs.

**Files:**

- Modify: `web/src/posts/mod.rs` (`get_post` L307-314 signature/body;
  `PostResponse` L181, `CreatePostResult` L49, `UpdatePostResult` L61,
  `DraftSummary` L154, `PublishPostResult` L170; construction sites L282, 473,
  553, 592, 617 and `web/src/posts/server.rs`)
- Modify: `web/src/posts/listing.rs:39` (`TimelinePostSummary.slug`)
- Modify: `web/src/pages/posts.rs` (`PostPage` route-slug parse ~L164-170;
  `EditPostPage` consumer at L677)
- Modify: `server/src/projector/mod.rs:136` (justification comment only)

**Interfaces:**

- Consumes: `Slug` trailer (Task 1); `storage` `get_post_by_permalink`/
  `fetch_post_record`/`find_draft_by_permalink_for_user` already take `&Slug`.
- Produces:
  - `pub async fn get_post(username: Username, year: i32, month: u32, day: u32, slug: Slug) -> WebResult<PostResponse>`
  - DTO fields `slug: Slug` on `PostResponse`, `CreatePostResult`,
    `UpdatePostResult`, `DraftSummary`, `PublishPostResult`,
    `TimelinePostSummary`.

- [ ] **Step 1: Type `get_post` and drop the in-body parse.**
      `web/src/posts/mod.rs:307-314` — change `slug: String` → `slug: Slug`;
      delete `let slug_parsed = slug.parse::<Slug>()?;` (L318) and replace the
      two `&slug_parsed` uses (L328, L356) with `&slug`.

- [ ] **Step 2: Type the 6 DTO `slug` fields → `Slug`.** Change each field decl
      to `pub slug: Slug`. Fix construction sites the compiler flags:
      `slug: record.slug.to_string()` (L282, 473),
      `slug: draft.slug.to_string()` (L553), `slug: updated.slug.to_string()`
      (L617), and `server.rs`'s two `slug: slug.to_string()` → drop
      `.to_string()` (move/`.clone()` the `Slug` directly).
      `L592 slug: existing.slug` is already a move — unchanged.

- [ ] **Step 3: Fix DTO consumers the compiler flags.**
  - `web/src/pages/posts.rs:677` `slug_override.set(fetched.slug.clone())` —
    `fetched.slug` is now `Slug` and `slug_override: RwSignal<String>`, so use
    `slug_override.set(fetched.slug.to_string())` for now (Task 5 replaces this
    signal with a `Field<Slug>`).
  - **Leptos `view!` nodes need `.to_string()` — a newtype does NOT implement
    `IntoRender`/`IntoAttributeValue`** (the `Username` precedent always
    `.to_string()`s before rendering, e.g. `ui.rs:327`). Retyping the DTO `slug`
    fields breaks these render sites in `pages/posts.rs`, each needing
    `.to_string()`: `:63` `data-slug=slug_for_attr` (attribute), `:65` and
    `:872` `{slug_value}` (from `created.slug`/`updated.slug`), `:986`
    `{draft.slug}` (text node). The gate will flag any missed one; enumerate as
    you go.
  - `Display` interpolation inside `format!`/`tracing` (not `view!`) is
    unchanged.

- [ ] **Step 4: Move `get_post`'s parse into `PostPage` (Decision 1 / Risk 3).**
      `web/src/pages/posts.rs` ~L164-170: where the route `slug` (`String` from
      `params.get("slug")`) is passed to `get_post`, parse it client-side:

  ```rust
  // Route slug is parsed here (client-side) so get_post takes a typed Slug;
  // an unparseable slug can't name a real post — skip the fetch (client 404).
  let slug = params.get("slug").and_then(|s| s.parse::<Slug>().ok())?;
  ```

  Match the existing `Username` route-parse pattern already in this component
  (#407). Ensure the resource guard skips the server call when the parse fails.

- [ ] **Step 5: Add the projector justification comment (no functional
      change).** `server/src/projector/mod.rs:136`, above the
      `Path<(String, i32, u32, u32, String)>` extractor:

  ```rust
  // Extractor stays `String` (not `Path<(Username, .., Slug)>`): a malformed
  // segment is parsed inside the handler so an unresolvable public URL serves
  // the SPA shell (client 404), not an axum 400 that a typed extractor would
  // raise before the handler runs. This is the projector-vs-atompub boundary
  // split (ADR-0063 §4): atompub handlers *are* typed; the projector is not.
  ```

- [ ] **Step 6: Run the gate.** Run: `devtool run -- cargo xtask check`
      Expected: PASS. (Wire unchanged: `Slug` serializes as a plain string.)

- [ ] **Step 7: Commit.**
  ```bash
  git add web/src/posts/mod.rs web/src/posts/server.rs web/src/posts/listing.rs web/src/pages/posts.rs server/src/projector/mod.rs
  git commit -m "refactor(web): thread Slug through get_post + post DTOs; type PostPage route slug"
  ```

---

## Task 3: `storage` — type the slug-generation zone (`candidate_slug`, seed)

Spec Decision 5, §E/§I (generation). Types the internal generation seed to
`Slug` and `candidate_slug`'s input to `&Slug`, keeping `from_str` the
chokepoint. **Only `perform_post_creation` is touched** — it is the sole caller
of `candidate_slug` (the dedup/collision path). `perform_post_update` (L312-319)
does **not** call `candidate_slug` and has no seed intermediate (an update keeps
its slug, no dedup); leave it for Task 5's re-parse drop. The `slug_override`
**param** stays `Option<&str>` here (Task 5 types it) — so this task is
storage-internal and independently green.

**Files:**

- Modify: `storage/src/post_service.rs` (`candidate_slug` L394-405; the
  `slug_seed` local in `perform_post_creation` L470-479; the direct
  `candidate_slug` unit test ~L778) — **not** `perform_post_update`.
- Modify: `web/src/posts/mod.rs:792, 797, 798` (the re-exported
  `use storage::candidate_slug;` unit-test callers —
  `candidate_slug("hello-world", N)` with `&str` literals break when the param
  becomes `&Slug`).

**Interfaces:**

- Consumes: `Slug` trailer (Task 1); `slugify_title(&str) -> String` unchanged.
- Produces: `fn candidate_slug(base: &Slug, attempt: usize) -> String` — returns
  the (unvalidated) collision candidate, parsed to `Slug` by the caller.

- [ ] **Step 1: Retype `candidate_slug`'s parameter and fix its body.**
      `storage/src/post_service.rs:394` —
      `fn candidate_slug(base: &Slug, attempt: usize) -> String`. The
      `base.chars()`/`format!("{base}-{n}")` reads compile via `Deref`, but the
      early-return `return slug_seed.to_owned();` (L396) would now resolve
      `<Slug as ToOwned>::to_owned` → `Slug`, mismatching the `String` return
      (E0308). Change it to `return base.as_ref().to_owned();` (or
      `base.to_string()`). Return type stays `String` (the candidate funnels
      through `from_str` at the call site).

- [ ] **Step 2: Type the seed to `Slug` in `perform_post_creation` only.** In
      `perform_post_creation` (L470-479), make the seed a `Slug`:

  ```rust
  let slug_seed: Slug = match slug_override.and_then(common::text::non_empty) {
      Some(raw) => raw.parse().map_err(PerformCreationError::InvalidSlug)?,
      // slugify_title never fails, but funnel it through from_str (the chokepoint)
      // rather than bypass-constructing a Slug.
      None => slugify_title(&metadata.slug_seed)
          .parse()
          .map_err(PerformCreationError::InvalidSlug)?,
  };
  // loop:
  let slug = candidate_slug(&slug_seed, attempt)
      .parse()
      .map_err(PerformCreationError::InvalidSlug)?;
  ```

  (The `slug_override` param is still `Option<&str>` here, so the `Some` branch
  still parses — Task 5 removes that parse once the param is `&Slug`.)

- [ ] **Step 3: Update the `candidate_slug` callers/tests.**
  - `candidate_slug_keeps_suffix_within_cap` (~L778) now passes a `&Slug` base:
    ```rust
    let base: Slug = "a".repeat(MAX_SLUG_CHARS).parse().unwrap();
    let out = candidate_slug(&base, 2);
    assert!(out.chars().count() <= MAX_SLUG_CHARS);
    assert!(out.parse::<Slug>().is_ok());
    ```
    Adjust the exact assertions to the test's existing intent (suffix within
    cap).
  - `web/src/posts/mod.rs:792, 797, 798` — bind a `Slug` and pass `&base`:
    `let base: Slug = "hello-world".parse().unwrap();` then
    `candidate_slug(&base, 0/1/2)`.

- [ ] **Step 4: Run the gate.** Run: `devtool run -- cargo xtask check`
      Expected: PASS.

- [ ] **Step 5: Commit.**
  ```bash
  git add storage/src/post_service.rs web/src/posts/mod.rs
  git commit -m "refactor(storage): type slug-generation seed and candidate_slug as Slug"
  ```

---

## Task 4: `web/src/forms.rs` — optional-field validation support

Spec Decision 3, §H. `slug_override` is the first **optional** domain-value
field; the #414 helper models required fields only (empty ⇒ invalid). Add an
optional mode where empty ⇒ valid, so a pristine empty optional field leaves
submit enabled while a non-empty invalid entry still gates it. New behavior →
TDD.

**Files:**

- Modify: `web/src/forms.rs` (`Field<T>` struct + impls; `ValidatedInput`
  on-input)

**Interfaces:**

- Produces (on `impl<T> Field<T> where T: FromStr + 'static, T::Err: Display`):
  - `pub fn optional() -> Self` — optional, empty ⇒ `error = None` (valid).
  - `pub fn optional_prefilled(initial: &str) -> Self` — optional, seeded (empty
    ⇒ valid; non-empty ⇒ validated).
  - `pub fn error_for(&self, input: &str) -> Option<String>` — the
    optionality-aware validator: `None` if `optional && input.is_empty()`, else
    `field_error::<T>(input)`.
  - `Field<T>` gains a private `optional: bool` (a `Copy` `bool`; `Field` stays
    `Copy`). `new()`/`prefilled()` set `optional: false` (behavior unchanged).

- [ ] **Step 1: Write the failing tests** in `web/src/forms.rs` `mod tests`
      (host-tested under an `Owner`, like the existing `Field` tests):

  ```rust
  #[test]
  fn optional_empty_field_is_valid_and_submittable() {
      let owner = Owner::new(); owner.set();
      let f = Field::<Slug>::optional();
      assert!(f.is_valid());        // empty optional ⇒ valid ⇒ submit not gated
      assert!(!f.is_touched());
      assert_eq!(f.parsed(), None); // Option<Slug> None for empty
      drop(owner);
  }

  #[test]
  fn optional_nonempty_invalid_shows_the_newtypes_message() {
      let owner = Owner::new(); owner.set();
      let f = Field::<Slug>::optional();
      // Mimic on:input with a bad slug.
      f.value.set("Bad Slug!".to_owned());
      f.error.set(f.error_for("Bad Slug!"));
      assert!(!f.is_valid());
      assert!(f.error.get().is_some()); // exactly InvalidSlug's Display
      drop(owner);
  }

  #[test]
  fn optional_nonempty_valid_parses() {
      let owner = Owner::new(); owner.set();
      let f = Field::<Slug>::optional();
      f.value.set("hello".to_owned());
      f.error.set(f.error_for("hello"));
      assert!(f.is_valid());
      assert_eq!(f.parsed(), "hello".parse::<Slug>().ok());
      drop(owner);
  }

  #[test]
  fn optional_prefilled_seeds_valid_from_existing_slug() {
      let owner = Owner::new(); owner.set();
      let f = Field::<Slug>::optional_prefilled("my-post");
      assert!(f.is_valid());
      assert_eq!(f.value.get(), "my-post");
      drop(owner);
  }

  #[test]
  fn required_new_still_invalid_on_empty() { // regression: required unchanged
      let owner = Owner::new(); owner.set();
      assert!(!Field::<Slug>::new().is_valid());
      drop(owner);
  }
  ```

- [ ] **Step 2: Run the tests, verify they fail.** Run:
      `devtool run -- cargo nextest run -p web forms::` Expected: FAIL —
      `optional`/`optional_prefilled`/`error_for` not defined.

- [ ] **Step 3: Implement against the tests.** Add `optional: bool` to
      `Field<T>` (init `false` in `new`/`prefilled`). Add:

  ```rust
  #[must_use]
  pub fn optional() -> Self { Self::optional_prefilled("") }

  #[must_use]
  pub fn optional_prefilled(initial: &str) -> Self {
      let mut f = Self::prefilled(initial);
      f.optional = true;
      f.error.set(f.error_for(initial)); // re-seed: empty ⇒ None
      f
  }

  #[must_use]
  pub fn error_for(&self, input: &str) -> Option<String> {
      if self.optional && input.is_empty() { None } else { field_error::<T>(input) }
  }
  ```

  In `ValidatedInput`'s `on_input` (L133-134), replace
  `field.error.set(field_error::<T>(&v));` with
  `field.error.set(field.error_for(&v));` so the rendered validity honors
  optionality. (`optional` is a `bool` field, so `Field` remains `Copy`.)

- [ ] **Step 4: Run the tests, verify they pass.** Run:
      `devtool run -- cargo nextest run -p web forms::` Expected: PASS (all
      five, including the required-field regression).

- [ ] **Step 5: Commit.**
  ```bash
  git add web/src/forms.rs
  git commit -m "feat(web): optional-field support in the client-validation helper"
  ```

---

## Task 5: `slug_override` write-path — `Option<Slug>` wire + storage + forms

Spec Decision 2, §B/§D/§E. The whole override chain changes together (typing the
wire arg forces its callers, and the storage metadata, to change in lockstep),
so this is one cohesive task. Validation relocates to the boundary: an invalid
override is now caught client-side (`error_for`, Task 4) and by the serde bridge
— never at the storage layer, which receives a pre-validated `&Slug`.

**Files:**

- Modify: `web/src/posts/mod.rs` (`create_post` L226-236, `update_post` L392-402
  signatures + the `slug_override.as_deref()` → `.as_ref()` at L264, L438)
- Modify: `storage/src/post_service.rs` (`PostCreation.slug_override` L420,
  `PostUpdate.slug_override` L266 → `Option<&'a Slug>`; drop the `Some`-branch
  re-parse in `perform_post_creation` **and** `perform_post_update`;
  delete/adjust tests L606-650)
- Modify: `server/tests/storage/storage.rs:2508-2520` (the shared `update_input`
  helper `fn update_input(..., slug: &str, ...) -> PostUpdate` sets
  `slug_override: Some(slug)`; retype its `slug` param to `&Slug` — it breaks
  across many storage integration tests otherwise. The one production
  cross-crate caller, `server/src/atompub/posts.rs:384,507`, passes
  `slug_override: None` and stays compatible.)
- Modify: `web/src/pages/posts.rs` (`CreatePostPage` L28-…, `EditPostPage` slug
  input L621, L677, L683-691, L726-736), `web/src/pages/ui.rs` (composer slug
  input L723-765, dispatches L609/621/727)

**Interfaces:**

- Consumes: `Field::optional`/`optional_prefilled`/`parsed` (Task 4);
  `ValidatedInput<Slug>`; `Slug` trailer.
- Produces:
  - `create_post(..., slug_override: Option<Slug>, ...)`,
    `update_post(..., slug_override: Option<Slug>, ...)`
  - `PostCreation.slug_override: Option<&'a Slug>`,
    `PostUpdate.slug_override: Option<&'a Slug>`.

- [ ] **Step 1: Type the storage metadata + drop the override re-parse (both
      functions).** `post_service.rs`: `PostCreation.slug_override` and
      `PostUpdate.slug_override` → `Option<&'a Slug>`. The two functions have
      _different_ shapes — apply each:
  - `perform_post_creation` (seed feeds `candidate_slug`): the `Some` branch now
    holds an already-valid `&Slug`:
    ```rust
    let slug_seed: Slug = match slug_override {
        Some(slug) => slug.clone(),           // pre-validated at the boundary
        None => slugify_title(&metadata.slug_seed)
            .parse().map_err(PerformCreationError::InvalidSlug)?,
    };
    ```
  - `perform_post_update` (no dedup — yields the slug directly, L312-319):
    ```rust
    let slug = match slug_override {
        Some(slug) => slug.clone(),
        None => slugify_title(&metadata.slug_seed)
            .parse().map_err(|_| PerformUpdateError::InvalidSlug)?,
    };
    ```
    In both, `common::text::non_empty` is **no longer applied** to the override
    (it takes `&str` and wouldn't typecheck against `Option<&Slug>`; an empty
    override is `None` at the wire, not `Some("")`).

- [ ] **Step 2: Relocate the invalid-override test; fix the valid one.** Delete
      `test_perform_post_creation_invalid_slug_override` (L633-650) — an invalid
      `&Slug` is unconstructible. Change
      `test_perform_post_creation_slug_override` (L606-628) to bind a `Slug` and
      pass `Some(&slug)`:

  ```rust
  let slug: Slug = "my-custom-slug".parse().unwrap();
  // ... slug_override: Some(&slug), ...
  ```

  Confirm boundary coverage of the invalid case survives: the serde reject test
  (`slug.rs:249-258`) and Task 4's `optional_nonempty_invalid_*` test. Grep for
  any `perform_post_update` invalid-override test and relocate likewise
  (`rg -n 'invalid_slug|Invalid Slug' storage/`).

- [ ] **Step 3: Type the `#[server]` args + call sites.**
      `web/src/posts/mod.rs`: `create_post`/`update_post`
      `slug_override: Option<Slug>`. At the `PostCreation`/`PostUpdate`
      construction (L264, L438) pass `slug_override: slug_override.as_ref()`
      (`Option<&Slug>`). Delete the `.as_deref()`.

- [ ] **Step 4: Adopt `ValidatedInput<Slug>` on the three forms.** For
      `EditPostPage` (`pages/posts.rs`), `CreatePostPage` (`pages/posts.rs:28`),
      and the `ui.rs` composer — replace the raw
      `slug_override: RwSignal<String>` + hand-rolled
      `<input name="slug_override">` with:

  ```rust
  // create form: empty ⇒ auto-generate (valid). edit form: seed from the draft.
  let slug = Field::<Slug>::optional();                       // CreatePostPage
  let slug = Field::<Slug>::optional_prefilled(fetched.slug.as_ref()); // EditPostPage
  // in view (mind leptosfmt #420 — mirror auth.rs):
  <ValidatedInput<Slug> label="Slug" name="slug_override" field=slug />
  ```

  Dispatch reads the typed value: `slug_override: slug.parsed()`
  (`Option<Slug>`), replacing the
  `common::text::non_empty(&slug).map(str::to_owned)` juggling
  (posts.rs:683-691, ui.rs:725-730). Keep `name="slug_override"` (e2e selector).
  Submit gating: if a form gates on the slug field, gate on `slug.is_valid()` —
  empty optional is valid, so submit stays enabled (Risk: verify empty doesn't
  disable submit).

- [ ] **Step 5: Run the gate + affected e2e.** Run:
      `devtool run -- cargo xtask check` Expected: PASS. Then (forms + storage
      behavior touched): `devtool run -- cargo xtask validate` Expected: PASS —
      e2e green for create/edit-post (slug override) and permalink flows. (Watch
      the known csr-e2e local flake, `posts.spec.ts`; re-run once.)

- [ ] **Step 6: Commit.**
  ```bash
  git add web/src/posts/mod.rs storage/src/post_service.rs server/tests/storage/storage.rs web/src/pages/posts.rs web/src/pages/ui.rs
  git commit -m "feat(web): typed Option<Slug> slug_override with client pre-validation"
  ```

---

## Task 6: Amend ADR-0065 — optional-field variant

Spec Decision 3. Record the optional-field convention (empty ⇒ valid) added in
Task 4, so the ADR that governs client-side domain validation documents both
required and optional fields.

**Files:**

- Modify: `docs/adr/0065-client-side-domain-validation.md`

- [ ] **Step 1: Add an "Optional fields" paragraph.** After the
      required-field/disable-until-valid discussion, add:

  > **Optional fields.** A field whose empty state is _valid_ (e.g. an
  > auto-generated slug override) uses `Field::optional()` /
  > `optional_prefilled(initial)`: `error_for` treats empty input as valid
  > (`None`) and non-empty input through the newtype's `FromStr` as before. The
  > wire arg is `Option<T>`; the form reads `field.parsed() -> Option<T>`.
  > Because empty is valid, `is_valid()` leaves submit enabled for a blank
  > optional field while still gating a non-empty invalid entry. First adopter:
  > `slug_override` (#408).

- [ ] **Step 2: Prettier the edited Markdown before staging** (avoids the
      pre-commit fail-restage):
      `devtool run -- prettier -w docs/adr/0065-client-side-domain-validation.md`

- [ ] **Step 3: Run the gate.** Run: `devtool run -- cargo xtask check`
      Expected: PASS.

- [ ] **Step 4: Commit.**
  ```bash
  git add docs/adr/0065-client-side-domain-validation.md
  git commit -m "docs(adr): ADR-0065 covers optional client-validated fields (#408)"
  ```

---

## Self-review

- **Spec coverage:** §A→T1; §B (get_post) →T2, §B (create/update args) →T5; §C
  DTOs →T2; §D forms →T5; §E storage metadata/re-parse →T5, §E seed unification
  →T3; §F atompub →T1, §F projector comment →T2; §G host — no change (noted); §H
  forms helper →T4; §I sweep →T1, §I generation zone →T3, projector residual →T2
  comment; Decision 3 ADR →T6; Decision 4 (no Hash) →T1; Decision 5 →T3. All
  acceptance criteria map to a task.
- **No placeholders:** every step names exact files/lines, signatures, and test
  bodies. Task 4 (new behavior) has full test contracts; refactor tasks (1–3, 5)
  are compiler-forced with precise site lists.
- **Type consistency:** `candidate_slug(base: &Slug) -> String` (T3) consumed by
  T5's seed match; `Field::optional`/`optional_prefilled`/`error_for`/`parsed`
  (T4) consumed by T5's forms; `Option<Slug>` args (T5) fed by `field.parsed()`
  (T4). `get_post(slug: Slug)` (T2) fed by `PostPage`'s route parse (T2).
