# Thread `PostFormat` end-to-end (#498) — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Replace the `PostFormat → String → PostFormat` double conversion in
the post editor and the three server-fn boundaries with a `Copy` `PostFormat`
threaded directly over the wire.

**Architecture:** `PostFormat` already rides the `StrEnum` trailer with a
`serde` string bridge (`common/src/render.rs:19-24`), so it drops into
`#[server]` wire args/responses per ADR-0065 (typed wire arg; no proffered
twin). Two independent compile units — the profile default-format ActionForm
(serde_qs codec) and the posts compose/edit typed-`Action` (serde_json codec) —
are done as two tasks.

**Tech Stack:** Rust, Leptos server fns (`#[server]`), `serde`/`serde_json`,
Playwright e2e.

**Spec:** `docs/superpowers/specs/2026-07-20-issue-498-postformat.md` — this
plan is the "how"; see the spec for "what/why", the site inventory, and the
acceptance criteria (referenced by number below).

## Global Constraints

_Every task's requirements implicitly include this section._

- **Wire tokens unchanged.** `StrEnum` serializes the lowercased variant name
  (`"markdown"`/`"org"`/`"html"`) — byte-identical to today's strings. No
  `<option value>` / e2e token changes.
- **Import `PostFormat` ungated.** Where `PostFormat` now appears in an un-cfg'd
  wire struct or a `#[server]` client stub (compiles on wasm), import
  `common::render::PostFormat` **ungated** — distinct from any existing
  `#[cfg(feature = "server")] use storage::… PostFormat` (same type,
  re-exported; remove the name from the gated list to avoid a duplicate import).
  Verify with `cargo clippy -p web --target wasm32-unknown-unknown`, not just
  default `cargo check`.
- **Prefer already-vendored dep versions.** Adding a dep is allowed; just
  version-match the existing vendor to avoid a cold shared-vendor rebuild.
  `serde_json`, `serde_qs`, and `serde_urlencoded` are all already in
  `Cargo.lock` (transitive via leptos), so the decode tests add a dev-dep at
  zero rebuild cost.
- **No client-side format validator** (ADR-0065): the format control is a closed
  toggle/select that can only emit valid tokens; rejection of a crafted token is
  the type's `Deserialize` at the wire boundary.
- **No new ADR** — this applies existing ADR-0063/ADR-0065 conventions.
- Commit via **jaunder-commit** (pre-commit hook runs full `cargo xtask check`).
  **No `Co-Authored-By` trailer.**

---

## Review header

**Scope (in):** the three server-fn boundaries (`create_post`, `update_post`,
`set_default_post_format`) and their outbound twins (`PostResponse.format`,
`get_default_post_format`), the editor `format` signal + `ComposerFields` prop,
the three inline format toggles, the profile `<select>` compares, and the two
producers/fixtures the retype forces (`post_response`, `sample_post`).

**Scope (out):** the `name="body"` vestigial textarea attribute (a `String`
signal, unaffected); the `PostFormat::Html` compose toggle (still not offered);
any broader #91 boundary audit.

**Tasks:**

1. **Profile default-format (serde_qs / ActionForm path)** — type
   `set_/get_default_post_format`
   - fix the `<select>` compares/fallback; pin the `PostFormat` serde-reject;
     add a profile default-format save/reload e2e.
2. **Posts editor + wire (serde_json / typed-`Action` path)** — type
   `CreatePostArgs`/`UpdatePostArgs`/`PostResponse.format`, drop the two in-body
   parses, retype the editor signal + `ComposerFields` prop + the three toggles,
   fix the seed + `post_response` producer + `sample_post` fixture, delete the
   vestigial hidden input; pin `CreatePostArgs`/`UpdatePostArgs` decode-reject.

**Key risks / decisions:**

- The profile path is the one **serde_qs form-encoded** surface. De-risked by
  precedent (`SubscribeTo`/`UnsubscribeFrom` already thread a typed `Username`
  newtype through an `<ActionForm>` over the same codec,
  `component.rs:1284-1307`); exercised live by the Task 1 e2e rather than
  assumed.
- Task 2 is **atomic** at the compile level: retyping `CreatePostArgs.format`
  forces the editor signal, the arg builders, the seed, and the server bodies to
  change together — it cannot be partially landed without reintroducing a parse
  (which the spec forbids).

---

## Task 1: Profile default-format (serde_qs / ActionForm path)

**Files:**

- Modify: `web/Cargo.toml` (`[dev-dependencies]`: add `serde_qs`)
- Modify: `web/src/profile/mod.rs:1-21,78-98` (imports +
  `get_/set_default_post_format` + new `#[cfg(test)] mod tests`)
- Modify: `web/src/pages/profile.rs:126-154` (`DefaultPostFormatControl`)
- Test (e2e): `end2end/tests/profile.spec.ts`

**Interfaces:**

- Consumes: `common::render::PostFormat` (`Copy`, `StrEnum`:
  `as_str`/`Display`/`FromStr`/ `serde`, variants `Markdown`/`Org`/`Html`,
  `Default = Markdown`); `storage::{get,set}_default_post_format` (already
  take/return `storage::PostFormat`).
- Produces: `get_default_post_format() -> WebResult<PostFormat>` and
  `set_default_post_format(format: PostFormat) -> WebResult<()>` (consumed by
  `pages/profile.rs`; the wire token is unchanged). The `#[server]` macro's
  generated wire struct `SetDefaultPostFormat { pub format: PostFormat }` is
  what the ActionForm submits.

- [x] **Step 1: Add the serde_qs dev-dep and write the failing wire-decode
      reject test** (AC5, serde_qs prong — `set_default_post_format` uses
      server_fn's default `Url` codec = serde_qs).

  In `web/Cargo.toml` `[dev-dependencies]`, add `serde_qs = "0.15"` (matches the
  leptos-transitive version already in `Cargo.lock`, so no cold vendor rebuild).

  In `web/src/profile/mod.rs`, add a test module exercising the **generated wire
  struct** through the real form codec:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::SetDefaultPostFormat;
      use common::render::PostFormat;

      #[test]
      fn set_default_post_format_wire_rejects_unknown_token() {
          // The profile control submits `format=<token>` via an <ActionForm>, decoded
          // through server_fn's default Url codec (serde_qs). A valid token decodes; a
          // bogus one is rejected at the wire boundary once the arg is a typed PostFormat.
          let ok: SetDefaultPostFormat = serde_qs::from_str("format=markdown").unwrap();
          assert_eq!(ok.format, PostFormat::Markdown);
          assert!(serde_qs::from_str::<SetDefaultPostFormat>("format=bogus").is_err());
      }
  }
  ```

- [x] **Step 2: Run it, verify it FAILS.**

  Run:
  `cargo nextest run -p web --lib set_default_post_format_wire_rejects_unknown_token`
  Expected: FAIL — with `format: String` the arg still decodes `format=bogus` as
  a plain string, so the `is_err()` assertion fails (this is the red that drives
  the retype).

- [x] **Step 3: Type the two profile server fns and drop the in-body parse.**

  In `web/src/profile/mod.rs`: **remove** `PostFormat` from the gated
  `use storage::{…}` block (line 18 — after the retype it is otherwise an unused
  import) and add `use common::render::PostFormat;` to the ungated
  shared-imports block (after line 6) — the `#[server]` client stubs reference
  it on wasm. `storage::PostFormat` and `common::render::PostFormat` are the
  same re-exported type, so the `storage_*` calls below still type-check.

  Retype to these exact signatures/bodies (the `?`-parse is deleted; `storage`'s
  fns already speak `PostFormat`):

  ```rust
  /// Retrieves the authenticated user's default post format preference.
  #[server(endpoint = "/get_default_post_format")]
  pub async fn get_default_post_format() -> WebResult<PostFormat> {
      boundary!("get_default_post_format", {
          let auth = require_auth().await?;
          let config = expect_context::<Arc<dyn UserConfigStorage>>();
          let format = storage_get_default_post_format(config.as_ref(), auth.user_id).await?;
          Ok(format)
      })
  }

  /// Sets the authenticated user's default post format preference.
  #[server(endpoint = "/set_default_post_format")]
  pub async fn set_default_post_format(format: PostFormat) -> WebResult<()> {
      boundary!("set_default_post_format", {
          let auth = require_auth().await?;
          let config = expect_context::<Arc<dyn UserConfigStorage>>();
          storage_set_default_post_format(config.as_ref(), auth.user_id, format).await?;
          Ok(())
      })
  }
  ```

  (Cf. `update_profile` at :59-74 — the existing ADR-0065 typed-wire-arg
  precedent in this same file.)

- [x] **Step 4: Run the wire-decode test, verify it PASSES.**

  Run:
  `cargo nextest run -p web --lib set_default_post_format_wire_rejects_unknown_token`
  Expected: PASS — the typed `PostFormat` now rejects `format=bogus` at serde_qs
  decode.

- [x] **Step 5: Retype the profile control's compares + fallback.**

  In `web/src/pages/profile.rs` `DefaultPostFormatControl` (:126-154): `initial`
  now resolves to `PostFormat`. Change the fallback and the `<option selected>`
  compares:

  ```rust
  let current = initial.await.unwrap_or(PostFormat::Html);
  // …
  <option value="markdown" selected=current == PostFormat::Markdown>"Markdown"</option>
  <option value="org" selected=current == PostFormat::Org>"Org"</option>
  <option value="html" selected=current == PostFormat::Html>"HTML"</option>
  ```

  The `<option value="…">` literals stay (HTML wire tokens = `StrEnum` tokens).
  Add `use common::render::PostFormat;` to `pages/profile.rs` imports. The
  `<select name="format">` + `ServerAction::<SetDefaultPostFormat>` are
  unchanged.

- [x] **Step 6: Verify host build + wasm client stub compile.**

  Run: `cargo clippy -p web --all-targets` then
  `cargo clippy -p web --target wasm32-unknown-unknown` Expected: PASS (proves
  the ungated `PostFormat` import satisfies the wasm client stub).

- [x] **Step 7: Add the profile default-format save/reload e2e** (AC6, serde_qs
      happy path).

  `end2end/tests/profile.spec.ts` has no default-format coverage. Add a test
  that, as a logged-in user, selects a non-default format in the "Default post
  format" `<select>`, submits "Save", reloads the profile page, and asserts the
  `<select>` reflects the saved value — proving the typed `PostFormat`
  round-trips through the ActionForm's serde_qs encode/decode. Mirror the
  existing `profile.spec.ts` login/fixture setup and its `<select>`/submit
  selector idioms; use the `.j-field-val` select + the "Save" button.

- [x] **Step 8: Run the host gate** (locally the e2e VM is reaped — run the host
      gate and let CI's matrix gate e2e; if running a single spec locally,
      expect it may be killed, not failed).

  Run: `cargo xtask validate --no-e2e` Expected: PASS.

- [x] **Step 9: Commit.**

  ```bash
  git add web/Cargo.toml web/src/profile/mod.rs web/src/pages/profile.rs end2end/tests/profile.spec.ts
  git commit -m "refactor(web): type default-post-format server fns as PostFormat (#498)"
  ```

  Run `cargo xtask check` first so the pre-commit gate passes clean
  (**jaunder-commit**).

---

## Task 2: Posts editor + wire (serde_json / typed-`Action` path)

**Files:**

- Modify: `web/src/posts/api.rs:121,142,157,194,357`
  (`PostResponse`/`CreatePostArgs`/ `UpdatePostArgs` fields + the two in-body
  parses) + tests module (append decode tests)
- Modify: `web/src/posts/server.rs:75` (`post_response` producer)
- Modify: `web/src/posts/render.rs:274` (`sample_post` fixture)
- Modify:
  `web/src/posts/component.rs:43-46,50,82-104,403,513-532,682-701,1621,1692,1818-1837`
  (doc comment, `ComposerFields` prop, three toggles, two inits, seed, hidden
  input)

**Interfaces:**

- Consumes: `common::render::PostFormat` (as Task 1); `storage::PostFormat` for
  the server-gated producer paths (same type).
- Produces: `CreatePostArgs.format: PostFormat`,
  `UpdatePostArgs.format: PostFormat`, `PostResponse.format: PostFormat` (the
  compose/edit components and any DTO consumer now see a typed field; the wire
  token is unchanged).

- [x] **Step 1: Write the failing decode-reject tests** (AC5, serde_json path).

  Append to the `web/src/posts/api.rs` tests module. Build a valid value,
  serialize, then corrupt only the format token — so the test never hardcodes
  the full wire shape and can't drift:

  ```rust
  #[test]
  fn create_post_args_rejects_unknown_format_token() {
      use super::CreatePostArgs;
      use common::render::PostFormat;
      let args = CreatePostArgs {
          body: "hi".into(),
          format: PostFormat::Markdown,
          slug_override: None,
          publish: false,
          publish_at: None,
          tags: None,
          summary: None,
          audience: None,
      };
      let json = serde_json::to_string(&args).unwrap();
      assert!(serde_json::from_str::<CreatePostArgs>(&json).is_ok());
      let bad = json.replace("\"markdown\"", "\"bogus\"");
      assert!(serde_json::from_str::<CreatePostArgs>(&bad).is_err());
  }

  #[test]
  fn update_post_args_rejects_unknown_format_token() {
      use super::UpdatePostArgs;
      use common::ids::PostId;
      use common::render::PostFormat;
      let args = UpdatePostArgs {
          post_id: PostId::from(1),
          body: "hi".into(),
          format: PostFormat::Markdown,
          slug_override: None,
          publish: false,
          publish_at: None,
          tags: None,
          summary: None,
          audience: None,
      };
      let json = serde_json::to_string(&args).unwrap();
      assert!(serde_json::from_str::<UpdatePostArgs>(&json).is_ok());
      let bad = json.replace("\"markdown\"", "\"bogus\"");
      assert!(serde_json::from_str::<UpdatePostArgs>(&bad).is_err());
  }
  ```

  (`body: "hi".into()` matches the existing `PostBody` `.into()` idiom in this
  file, e.g. `api.rs:692`, and needs no import — `.into()` infers `PostBody`
  from the field type. At the outer `mod tests` level
  `CreatePostArgs`/`UpdatePostArgs`/`PostId` are **not** in scope — they're
  imported per nested fn — so each new test brings its own via `use super::…` /
  `use common::ids::PostId;` as shown.)

- [x] **Step 2: Run them, verify they FAIL to compile.**

  Run:
  `cargo nextest run -p web --lib posts::api create_post_args_rejects update_post_args_rejects`
  Expected: FAIL — `CreatePostArgs.format`/`UpdatePostArgs.format` are still
  `String`, so `format: PostFormat::Markdown` doesn't type-check.

- [x] **Step 3: Retype the wire structs + drop the in-body parses.**

  In `web/src/posts/api.rs`: add `use common::render::PostFormat;` **ungated**
  (it now appears in un-cfg'd wire structs + client stubs). **Remove**
  `PostFormat` from the gated `use storage::{…}` list at :48 — its only current
  references are the two `parse::<PostFormat>()` calls being deleted in this
  step, so after deletion it is an **unused import** (clippy-deny → gate fail).
  No alias is needed: `storage::PostFormat` and `common::render::PostFormat` are
  the same re-exported type (no `E0252` clash), and the server-gated code that
  constructs `PostCreation`/`PostUpdate { … format … }` resolves the type via
  the new ungated import. The nested-test-submodule imports at :680/:720/:788
  are separate and stay. Then:
  - `PostResponse.format: String → PostFormat` (:121)
  - `CreatePostArgs.format: String → PostFormat` (:142)
  - `UpdatePostArgs.format: String → PostFormat` (:157)
  - Delete `let format = format.parse::<PostFormat>()?;` (:194) — `format` is
    already a `PostFormat`; the destructured `format` flows straight into
    `PostCreation`.
  - Delete the same parse in `update_post` (:357).

- [x] **Step 4: Fix the `PostResponse.format` producer + fixture.**
  - `web/src/posts/server.rs:75` — `post_response` builds
    `format: format.to_string()` from a `PostRecord.format` (a `PostFormat`);
    change to `format: format` (drop `.to_string()`).
  - `web/src/posts/render.rs:274` — `sample_post` builds
    `format: "markdown".into()`; change to `format: PostFormat::Markdown`
    (StrEnum has no infallible `From<&str>`, so `.into()` no longer compiles).
    Add `use common::render::PostFormat;` if not in scope.

- [x] **Step 5: Retype the editor signal, the `ComposerFields` prop, and the
      three toggles.**

  In `web/src/posts/component.rs` (add `use common::render::PostFormat;` if not
  already imported). Apply this exact mechanical mapping at every site — the
  contract is "no string literal or `.to_string()`/`.clone()` remains on the
  `format` signal", pinned by AC2
  (`rg '"markdown"|"org"' web/src/posts/component.rs` → empty):
  - `ComposerFields` `format` prop (:50):
    `RwSignal<String> → RwSignal<PostFormat>`.
  - Signal inits (:403, :1621):
    `RwSignal::new("markdown".to_string()) → RwSignal::new(PostFormat::Markdown)`.
  - Every compare
    `format.get() == "markdown" → format.get() == PostFormat::Markdown` and
    `== "org" → == PostFormat::Org`, at all three toggles: the `ComposerFields`
    toggle (:82,:95), the inline compact-composer toggle (:513,:526), and the
    edit-form toggle (:682,:695 and :1818,:1831).
  - Every set
    `format.set("markdown".to_string()) → format.set(PostFormat::Markdown)` and
    `"org" → PostFormat::Org`, at the same three toggles (:88,:97 / :519,:532 /
    :688,:701 / :1824,:1837).
  - Seed (:1692):
    `format.set(fetched.format.clone()) → format.set(fetched.format)`
    (`PostFormat` is `Copy`; `fetched.format` is now `PostFormat`).
  - The arg builders (`format: format.get()`, e.g. :446,:460,:580,:1706) need
    **no** edit — `format.get()` now yields a `PostFormat` that fits the retyped
    field directly.

- [x] **Step 6: Delete the vestigial hidden input + update the doc comment**
      (D2).
  - Delete
    `<input type="hidden" name="format" prop:value=move || format.get() />`
    (:104) — nothing POSTs the compose form (all editors dispatch a typed
    `Action`), so it is dead markup and, with a `PostFormat` signal,
    `prop:value` would no longer accept it.
  - Update the `ComposerFields` doc comment (:43-46) to drop the "and a
    `name="format"` hidden input" clause.

- [x] **Step 7: Run the decode tests, verify they PASS.**

  Run:
  `cargo nextest run -p web --lib posts::api create_post_args_rejects update_post_args_rejects`
  Expected: PASS.

- [x] **Step 8: Verify host + wasm client builds and the full host gate.**

  Run: `cargo clippy -p web --target wasm32-unknown-unknown` (proves the ungated
  import satisfies the client stub), then `cargo xtask validate --no-e2e`.
  Expected: PASS. Existing posts e2e (`posts.spec.ts:118` Org compose, `:601`
  toggle selection) exercises the compose path unchanged and gates in CI's e2e
  matrix.

- [x] **Step 9: Confirm the acceptance greps.**

  Run: `rg 'parse::<PostFormat>\(\)' web/src` → expect **no** hits in
  `create_post`/ `update_post`/`set_default_post_format` bodies (AC1). Run:
  `rg '"markdown"|"org"' web/src/posts/component.rs` → expect **no** hits (AC2).

- [x] **Step 10: Commit.**

  ```bash
  git add web/src/posts/api.rs web/src/posts/server.rs web/src/posts/render.rs web/src/posts/component.rs
  git commit -m "refactor(web): thread PostFormat through posts editor and wire args (#498)"
  ```

  Run `cargo xtask check` first so the pre-commit gate passes clean
  (**jaunder-commit**).

---

## Self-review notes

- **Spec coverage:** AC1 → T2 S3/S9; AC2 → T2 S5/S9; AC3 → T1 S3 + T2 S3; AC4 →
  T2 S6; AC5 → **serde_qs prong** T1 S1–S4 (decode `SetDefaultPostFormat` via
  the real Url codec)
  - **serde_json prong** T2 S1 (`CreatePostArgs`/`UpdatePostArgs`); AC6 → T1 S7
    (profile save/reload) + existing `posts.spec.ts:118,601` (compose); AC7 → T1
    S8 + T2 S8. Decisions D1–D4 all realized.
- **Type consistency:** every site uses `common::render::PostFormat` with
  variants `Markdown`/`Org`/`Html`; imported ungated at each wire/wasm surface
  per Global Constraints.
- **No placeholders:** every implementation step carries the exact
  signature/body or an explicit before→after mapping plus a grep-gate; the two
  mechanical sweeps (T2 S5) are fully enumerated by line + rule, not "handle the
  rest".
