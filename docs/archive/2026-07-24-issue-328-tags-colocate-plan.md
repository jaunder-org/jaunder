# Plan — #328 web(tags): converge the tags vertical onto the co-located Leptos layout

Spec:
[`2026-07-24-issue-328-tags-colocate.md`](../specs/2026-07-24-issue-328-tags-colocate.md).
Governing ADR:
[ADR-0070](../../adr/0070-web-vertical-wasm-only-component-files.md).

## Review header

**Goal.** Give the tags vertical its own co-located UI home by moving the
`TagInput` autocomplete widget out of `posts/component.rs` into a new wasm-only
`tags/component.rs`, and apply the #505 `Signal::derive` tidy to the two tag
pages in place. `tags/api.rs` + `mod.rs` are already ADR-0070-canonical
(endpoint-only, no `server.rs`); this only adds the `component` wiring.

**Scope — in**

- New `web/src/tags/component.rs` (wasm-only) holding `TagInput`, verbatim move.
- `web/src/tags/mod.rs` grows the `#[cfg(target_arch = "wasm32")]` component
  wiring.
- `web/src/posts/component.rs`: delete the moved `TagInput`, add
  `use crate::tags::TagInput;`, trim the now-unused `TagLabel` import.
- `web/src/posts/mod.rs`: drop `TagInput` from the wasm re-export list.
- Collapse three `Signal::derive(move || …)` → bare `move || …` on the tag
  pages.

**Scope — out** (per spec): `SiteTagPage`/`UserTagPage` stay in `posts/`;
`taglist/`, `TagCtx`, and any `tags/server.rs` are untouched/uncreated.

**Tasks**

1. Relocate `TagInput` → `tags/component.rs`; wire `tags/mod.rs`; excise from
   `posts/` (fn + re-export + `TagLabel` import) and add the
   `crate::tags::TagInput` import. One atomic refactor (a half-move doesn't
   compile).
2. Collapse the three redundant `Signal::derive` `Topbar` prop wrappers on
   `SiteTagPage`/`UserTagPage`, in place.
3. Full verification: host `cargo xtask check`, wasm-clippy, then
   `cargo xtask validate` incl. the tags e2e flows.

**Key risks / decisions**

- `TagInput` is `#[component]` → moves **wasm → wasm** (both files are wasm-only
  by their `mod` declaration), so no new host-compile surface and **no cfg gates
  inside** `tags/component.rs` (ADR-0070 §1). The wiring-only scan gate must
  stay green.
- `TagLabel` is used _only_ by `TagInput` in `posts/component.rs` (line 910);
  after the move its import (line 37) is orphaned — trim
  `use common::tag::{Tag, TagLabel};` to `use common::tag::Tag;` or clippy
  `-D warnings` fails. `TagSummary` stays (used widely by posts).
- `list_tags` is called fully-qualified as `crate::tags::list_tags` inside
  `TagInput`; that path still resolves from `tags/component.rs` (re-exported at
  `tags/mod.rs`), so the call body moves verbatim.
- #643 concurrently rewrites `posts/component.rs`; these edits (delete
  `TagInput`, one import line, three `Topbar` lines) are small and localized.
  Keep the branch current with `main`; resolve any overlap at merge.

## Global constraints

- Rust; `web` crate. No `Co-Authored-By` trailer on commits (`jaunder-commit`).
- ADR-0070 idiom: `target_arch` cfg only on the `mod`/`pub use` wiring lines in
  `mod.rs`, **never inside a leaf file**.
- Wasm-only code is invisible to host `cargo check`/`build`; run wasm-clippy
  (`cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`) before
  committing, or the slow gate fails on `must_use_candidate`/etc.
- Pre-commit hook runs full `cargo xtask check`; run it first so it passes
  clean. No editing during a gated commit (Nix builds the working tree
  mid-commit).
- No new tests: this is a pure relocation + a behavior-identical closure tidy.
  Existing coverage — `visibility.spec.ts` / composer e2e exercise `TagInput`;
  the endpoint's `TagLabel`/casing unit tests stay in `tags/api.rs` untouched.

---

## Task 1 — Relocate `TagInput` into `tags/component.rs`

**Files**

- **New:** `web/src/tags/component.rs`
- **Edit:** `web/src/tags/mod.rs`, `web/src/posts/component.rs`,
  `web/src/posts/mod.rs`

**1a. Create `web/src/tags/component.rs`.** Move the entire `TagInput` item —
its doc comment, the `#[expect(clippy::too_many_lines, …)]` attribute,
`#[component]`, and the fn body (`posts/component.rs` lines ~818–1043) —
**verbatim**. Prepend the file's module doc + the imports `TagInput` actually
needs (only `TagSummary`, `TagLabel`, and the leptos prelude; `list_tags` stays
fully-qualified in the body, and `spawn_local`/`set_timeout`/`Duration` keep
their existing function-local `use`s):

```rust
//! The **tags** vertical's wasm-only UI (ADR-0070): the `TagInput` tag-entry
//! widget — a chip list plus a debounced autocomplete field backed by the
//! [`list_tags`](super::list_tags) endpoint. Declared
//! `#[cfg(target_arch = "wasm32")] mod component;` in `tags/mod.rs`, so this file
//! is wasm-only by its `mod` declaration and carries no cfg gates of its own.

use leptos::prelude::*;

use common::seed::TagSummary;
use common::tag::TagLabel;

// … TagInput moved here verbatim …
```

**1b. Wire `web/src/tags/mod.rs`.** Add the component declaration + gated
re-export, keeping `mod.rs` wiring-only:

```rust
//! Tag autocomplete: the `/list_tags` endpoint + its `TagSummary` wire DTO, and
//! the `TagInput` tag-entry widget.
mod api;

#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{list_tags, ListTags, DEFAULT_TAG_LIMIT, MAX_TAG_LIMIT};

#[cfg(target_arch = "wasm32")]
pub use component::TagInput;
```

**1c. Excise from `web/src/posts/component.rs`.**

- Delete the `TagInput` item (doc comment through closing `}` — the block
  currently at ~818–1043).
- Add `use crate::tags::TagInput;` to the import block (alongside the other
  `crate::…` UI imports, ~lines 14–29) so the three call sites still resolve.
- Trim the orphaned import at line 37: `use common::tag::{Tag, TagLabel};` →
  `use common::tag::Tag;`.
- The three call sites (`<TagInput tags=tags />` at ~576, ~701;
  `<TagInput tags=post_tags />` at ~1728) are unchanged — they resolve via the
  new import.

**1d. Update `web/src/posts/mod.rs` re-export.** Remove `TagInput` from the wasm
`pub use component::{…}` list (currently line ~70). Result (TagInput dropped):

```rust
#[cfg(target_arch = "wasm32")]
pub use component::{
    AudiencePicker, ComposerFields, CreatePostPage, DraftsPage, EditPostPage, InlineComposer,
    PostCard, PostCreateForm, PostDisplay, PostPage, SiteTagPage, UserTagPage, UserTimelinePage,
};
```

> Note: `posts/mod.rs`'s module doc lists "the tag input" among the widgets;
> update that prose to drop it (docs track the move —
> `feedback_docs_track_late_api_changes`). Same for `posts/component.rs`'s
> module doc ("…the audience picker, and the tag input.").

**Verify (compile-driven, no new test):**

- Host: `cargo xtask check --no-test` (fmt + clippy). Expected: clean — proves
  the `TagLabel` trim and re-export edits are consistent on the host build.
- Wasm: `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`.
  Expected: clean — proves `TagInput` and its three call sites compile in the
  only target that builds them.

**Commit** (after `cargo xtask check` passes clean):
`refactor(web/tags): co-locate TagInput into the tags vertical (#328)`.

---

## Task 2 — Collapse the redundant `Signal::derive` `Topbar` wrappers

**File:** `web/src/posts/component.rs` (the tag pages stay here; only the
wrappers change).

**2a. `SiteTagPage`** (~line 2078):

```rust
// before
<Topbar title=Signal::derive(move || format!("#{}", read_tag())) sub="Posts on this instance" />
// after
<Topbar title=move || format!("#{}", read_tag()) sub="Posts on this instance" />
```

**2b. `UserTagPage`** (~lines 2274–2275) — collapse **both**:

```rust
// before
<Topbar
    title=Signal::derive(move || format!("#{}", read_tag()))
    sub=Signal::derive(move || format!("Posts by ~{}", read_username()))
/>
// after
<Topbar
    title=move || format!("#{}", read_tag())
    sub=move || format!("Posts by ~{}", read_username())
/>
```

Behavior-identical (`Topbar`'s props are `leptos::TextProp`; a bare `move || …`
converts directly — exactly how `UserTimelinePage` at ~line 1478 already spells
it; neither form memoizes). Leptos `view!` formatting: run `leptosfmt` via the
gate; keep any intent comments outside the `view!` macro
(`project_leptosfmt_comment_relocation`).

**Verify:** `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`
clean (these lines only compile on wasm).

**Commit:**
`refactor(web/tags): collapse redundant Signal::derive Topbar wrappers on the tag pages (#328)`.

---

## Task 3 — Full verification

- `cargo xtask validate` (foreground, `timeout: 600000`;
  `project_gate_foreground_not_background`) — static + coverage + all four e2e
  combos. Confirms the tags autocomplete (composer) and tag-timeline browsing
  still pass, and the wiring-only scan gate is green.
- If `validate` e2e is too heavy locally, `cargo xtask e2e-local` on the
  composer/tag spec covers the moved widget; CI runs the full matrix on the PR.
- `git status --porcelain` after green — `cargo xtask check` auto-fixes fmt but
  doesn't commit it (`project_xtask_check_fmt_autofix_uncommitted`).

No follow-up issues to file: scope is self-contained; the out-of-scope items
(tag pages staying in `posts/`, `taglist/`, `TagCtx`) are deliberate end states,
not deferred work.
