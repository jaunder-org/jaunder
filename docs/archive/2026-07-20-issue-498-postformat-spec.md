# Spec — #498: thread `PostFormat` end-to-end (editor signal + server-fn wire args)

**Issue:** jaunder-org/jaunder#498 · **Milestone:** #13 Domain-value type safety
(newtypes) · **Blocker:** #562 (StrEnum derive) — landed.

## Goal

Kill the `PostFormat` → `String` → `PostFormat` double conversion in the post
editor and the three server-fn boundaries. `PostFormat` already rides the
`StrEnum` trailer with a `serde` string bridge (`common/src/render.rs:19-24`),
so it drops directly into `#[server]` wire args/responses (ADR-0065 typed wire
arg; no proffered twin needed). The editor's format state becomes a `Copy`
`PostFormat` signal, and the server bodies stop re-parsing a string they were
handed.

## Context / as-built (post #562, #567)

The issue's line numbers predate the `StrEnum` migration (#562) and the ADR-0070
posts-vertical relayout (#567). The real surfaces:

- **Posts — typed JSON `Action`.** `create_post`/`update_post` use
  `#[server(input = Json)]` with `CreatePostArgs`/`UpdatePostArgs`; the client
  builds the struct in JS and dispatches a typed `Action` (buttons are
  `type="button"`, `component.rs:441-468`). There is **no form POST** for
  compose — the hidden `<input name="format">` in `ComposerFields`
  (`component.rs:104`) is dead markup left from the former ActionForm (cf. the
  `pages/profile.rs:17` comment).
- **Profile default-format — `ActionForm`.** `set_default_post_format` is
  submitted by a real `<ActionForm>` + `<select name="format">`
  (`pages/profile.rs:135-147`), so its typed arg must decode from **serde_qs
  form-encoded** data, not JSON. This path has precedent:
  `SubscribeTo`/`UnsubscribeFrom` already thread a typed `Username` newtype
  through an `<ActionForm>` + hidden input over the same default serde_qs codec
  (`component.rs:1284-1307`), and `PostFormat`'s `StrEnum` string bridge
  deserializes identically — so the typed-arg-over-ActionForm move is
  mechanically the same as an already-shipping case (still exercised by AC6, not
  assumed).
- `TimelinePostSummary` carries no `format` field — the timeline is out of the
  blast radius.

## Decisions

- **D1 — outbound response fields are typed too.** `PostResponse.format` and
  `get_default_post_format()`'s return become `PostFormat`, not just the inbound
  args. Required to make the editor seed and the profile `<option selected>`
  compares typed, and mandated by #470 (newtypes are used on serialization
  surfaces; no flattening without express permission).
- **D2 — remove the vestigial hidden input.** The forced-to-change
  `<input name="format">` in `ComposerFields` is deleted, not adapted with
  `.as_str()` — nothing POSTs the compose form, so it is pure dead markup.
- **D3 — no client-side format validator.** The format control is a closed
  toggle/select that can only emit valid variant tokens, so ADR-0065's "client
  pre-validation" is trivially satisfied — there is no free-text to validate. A
  bad `format` value can only arrive from a crafted request, and the server
  rejects it at arg-decode (non-OK), asserted by test, not by message.
- **D4 — wire tokens unchanged.** `StrEnum` serializes the lowercased variant
  name (`"markdown"`/`"org"`/`"html"`), identical to today's strings, so the
  on-the-wire representation and all existing e2e behavior are unchanged.

No new ADR: this is an application of the existing ADR-0063/ADR-0065
conventions, not a new decision.

## Scope — sites to change (worktree paths)

**Inbound wire args → `PostFormat`** (and drop the in-body parse):

- `CreatePostArgs.format: String → PostFormat` (`web/src/posts/api.rs:142`);
  drop `format.parse::<PostFormat>()?` (`api.rs:194`).
- `UpdatePostArgs.format: String → PostFormat` (`api.rs:157`); drop the parse
  (`api.rs:357`).
- `set_default_post_format(format: String → PostFormat)`
  (`web/src/profile/mod.rs:90`); drop the in-body parse (`mod.rs:94`).

**Outbound responses → `PostFormat`:**

- `PostResponse.format: String → PostFormat` (`api.rs:121`).
- `post_response()` — the sole `PostResponse.format` producer — builds
  `format: format.to_string()` from a `PostRecord.format`
  (`web/src/posts/server.rs:75`); drop the `.to_string()` (assign the
  `PostFormat` directly).
- `get_default_post_format() -> WebResult<PostFormat>` (`mod.rs:79`).
- Test fixture `test_fixtures::sample_post()` builds `format: "markdown".into()`
  (`web/src/posts/render.rs:274`); becomes `format: PostFormat::Markdown`
  (`StrEnum` gives `TryFrom<&str>`/`FromStr`, not an infallible `From<&str>`, so
  `"markdown".into()` stops compiling).

**Editor (`web/src/posts/component.rs`) → `RwSignal<PostFormat>`:**

- `ComposerFields` `format` prop: `RwSignal<String> → RwSignal<PostFormat>`
  (`:50`).
- Two signal inits
  `RwSignal::new("markdown".to_string()) → RwSignal::new(PostFormat::Markdown)`
  (`:403`, `:1621`).
- All `format.get() == "markdown"` / `== "org"` compares →
  `== PostFormat::Markdown` / `== PostFormat::Org` (the `ComposerFields` toggle,
  the inline compact-composer toggle, and the edit-form toggle).
- All `format.set("markdown".to_string())` / `"org"` →
  `format.set(PostFormat::Markdown)` / `PostFormat::Org`.
- `format.set(fetched.format.clone()) → format.set(fetched.format)` (`:1692`;
  Copy, no clone).
- `format: format.get()` in the arg-struct builders stays as-is (now yields a
  `PostFormat` directly).
- Delete the vestigial hidden `<input name="format">` (`:104`) (D2), and update
  the `ComposerFields` doc comment (`:43-46`) that still describes it.

**Profile page (`web/src/pages/profile.rs`):**

- The `<option selected=current == "markdown">` compares become
  `current == PostFormat::Markdown` (etc.); `current` is now `PostFormat`.
- The error fallback `unwrap_or_else(|_| "html".to_string())` becomes
  `unwrap_or(PostFormat::Html)`.
- The `<select name="format">` `<option value="markdown">` literals stay string
  literals (they are the HTML wire tokens the ActionForm submits, matching the
  `StrEnum` tokens).

**Imports:** `PostFormat` must be imported **ungated** wherever it now appears
in an un-cfg'd wire struct or the wasm client stub (the `#[server]` client half
compiles on wasm). Note `api.rs` currently imports `storage::PostFormat`
**gated** under `#[cfg(feature = "server")]`; the retyped wire fields need
`common::render::PostFormat` imported **ungated** (same underlying type,
re-exported) — a distinct import from the existing gated one. Verify with
`cargo clippy -p web --target wasm32`, not just a default `cargo check` (per
project note on typed-wire-arg wasm scope).

## Acceptance criteria (observable)

1. `rg 'parse::<PostFormat>\(\)' web/src` returns **no** hits in server-fn
   bodies (`create_post`, `update_post`, `set_default_post_format`).
2. `rg '"markdown"|"org"' web/src/posts/component.rs` returns **no** hits (no
   string literals in the editor toggles/inits/compares).
3. `CreatePostArgs.format`, `UpdatePostArgs.format`, `PostResponse.format`,
   `get_default_post_format` return, and `set_default_post_format` arg are all
   `PostFormat` in the source.
4. The hidden `<input name="format">` no longer exists in `ComposerFields`.
5. **Server rejects a bad wire value at decode.** Because the arg is now a typed
   `PostFormat`, a bad token cannot be constructed in Rust — rejection happens
   at the wire-decode layer, and the two transports use different codecs, so the
   test is a **decode** test per codec (asserts the _outcome_ — `Err` — not the
   message):
   - **serde_json** (the `input = Json` posts path): deserializing
     `CreatePostArgs`/`UpdatePostArgs` from JSON with `"format":"bogus"` returns
     `Err`.
   - **serde_qs** (the profile ActionForm path): form-decoding `format=bogus`
     into the `set_default_post_format` arg returns `Err`.
6. **Behavior unchanged, verified live**: an e2e drives compose (Markdown &
   Org), edit (seeded format round-trips), and the profile default-format
   `<select>` save/reload, all green — proving the serde_qs form-encoded decode
   of `PostFormat` on the ActionForm path works end-to-end.
7. `cargo xtask validate --no-e2e` is green (host gate: static + clippy incl.
   wasm target + coverage); e2e matrix green in CI.

## Out of scope

- The `PostFormat::Html` variant remains authored only via the profile default
  and pre-rendered ingest paths; the compose toggle continues to offer only
  Markdown/Org (unchanged).
- The `name="body"` textarea attribute in `ComposerFields` (also vestigial but a
  `String` signal, unaffected by this change) is left as-is.
- Any broader #91 boundary audit beyond `PostFormat`.
