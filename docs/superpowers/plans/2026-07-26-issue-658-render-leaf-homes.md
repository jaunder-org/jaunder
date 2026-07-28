# Issue #658 Render Leaf Homes Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate `web::render` by moving its seven residual pure helpers to
co-located host homes and deleting `web/src/render/`.

**Scope:** In: `escape_html`, `Icons`, `TagCtx`,
`render_hero`/`render_home_masthead`, `render_load_more`, `format_bytes`,
callsites/comments, tests, and final `web::render` deletion. Out: shell
projector already in `web::app::render`, projector architecture changes,
historical ADR/archive rewrites, markup/content changes.

**Task list:**

1. Move HTML escaping to `web::html`.
2. Move icon path data to `web::icon`.
3. Move tag-list context to `web::taglist`.
4. Move home masthead rendering to `web::home::render`.
5. Move timeline load-more placeholder to `web::timeline::render`.
6. Move media byte formatter to `web::media`.
7. Delete `web::render` and run the final checks.

**Key risks/decisions:** No compatibility shim is allowed; every caller must cut
over. Root `mod html;` is required for the new crate-internal helper. `TagCtx`
remains `pub` because wasm component props expose it; most other helpers are
`pub(crate)`. `mod.rs` files stay wiring-only.

**Tech Stack:** Rust, Leptos 0.8, `cargo nextest`, `cargo xtask`, ADR-0070
host/wasm file split.

## Global Constraints

- Work in `.claude/worktrees/issue-658-render-leaf-homes` on branch
  `worktree-issue-658-render-leaf-homes`.
- Use `devtool run -- <cmd>` for every build/test/gate command.
- Run `lsp references` before changing exported symbols (`Icons`, `TagCtx`) or
  module-level public paths.
- No new `target_arch` cfgs except existing wasm-only `component` module
  declarations/re-exports.
- No fake host stubs; moved pure helpers stay ungated and host-tested.
- No `#[allow(...)]` or `#[expect(...)]` additions except preserving the
  existing `format_bytes` `#[expect(clippy::cast_precision_loss, …)]` reason
  verbatim.
- No `web::render` shim/re-export may remain.
- Historical ADR/archive mentions of `web::render` may remain; active code
  comments/docs must describe current homes.
- Before each commit, run `devtool run -- cargo xtask check --no-test`; commit
  via `jaunder-commit` and do not add `Co-Authored-By`.
- Final PR handoff requires `devtool run -- cargo xtask validate` green.

---

### Task 1: Move HTML escaping to `web::html`

**Files:**

- Create/Test: `web/src/html.rs`
- Modify: `web/src/lib.rs`
- Modify: `web/src/app/render.rs`
- Modify: `web/src/avatar/markup.rs`
- Modify: `web/src/posts/render.rs`
- Modify: `web/src/taglist/markup.rs`
- Modify: `web/src/topbar/markup.rs`
- Modify: `web/src/render/mod.rs`

**Interfaces:**

- Consumes: current
  `crate::render::escape_html<S: AsRef<str>>(input: S) -> String`.
- Produces: `crate::html::escape_html<S: AsRef<str>>(input: S) -> String` with
  identical escaping for `&`, `<`, `>`, `"`, and `'`.

- [x] **Step 1: Write the destination test and module wiring first.**

In `web/src/html.rs`, add only the test module and the intended signature
reference:

```rust
#[cfg(test)]
mod tests {
    use super::escape_html;

    #[test]
    fn escape_replaces_markup_metacharacters() {
        assert_eq!(escape_html("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&#39;");
    }
}
```

In `web/src/lib.rs`, add `mod html;` near the other root module declarations.

- [x] **Step 2: Run the test, verify it fails.**

Run:
`devtool run -- cargo nextest run -p web escape_replaces_markup_metacharacters`

Expected: FAIL — `crate::html::escape_html` / `super::escape_html` is not
defined yet.

- [x] **Step 3: Move the implementation and repoint callers.**

Move the current `escape_html` body from `web/src/render/mod.rs` into
`web/src/html.rs` above the test module:

```rust
pub(crate) fn escape_html<S: AsRef<str>>(input: S) -> String
```

Repoint callers:

- `web/src/app/render.rs`: `use crate::html::escape_html;`
- `web/src/avatar/markup.rs`: `use crate::html::escape_html;`
- `web/src/posts/render.rs`: import `escape_html` from `crate::html`, leaving
  remaining `crate::render` imports for later tasks.
- `web/src/taglist/markup.rs`: import `escape_html` from `crate::html`, leaving
  `TagCtx` on `crate::render` until Task 3.
- `web/src/topbar/markup.rs`: call or import `crate::html::escape_html` instead
  of `crate::render::escape_html`.

Delete the old `escape_html` function and its old test from
`web/src/render/mod.rs`.

- [x] **Step 4: Run focused checks, verify pass.**

Run:
`devtool run -- cargo nextest run -p web escape_replaces_markup_metacharacters`

Expected: PASS.

Run:
`devtool run -- cargo nextest run -p web topbar avatar tag_list post_content render_head`

Expected: PASS for the affected escaping consumers that have matching tests.

- [x] **Step 5: Gate and commit.**

Run: `devtool run -- cargo xtask check --no-test`

Expected: PASS.

Commit via `jaunder-commit` with message:
`refactor(web): move html escaping to leaf home (#658)`.

---

### Task 2: Move icon path data to `web::icon`

**Files:**

- Create/Test: `web/src/icon/paths.rs`
- Modify: `web/src/icon/mod.rs`
- Modify: `web/src/icon/markup.rs`
- Modify: `web/src/audiences/component.rs`
- Modify: `web/src/render/mod.rs`

**Interfaces:**

- Consumes: current public `crate::render::Icons` type and its associated
  `&'static str` glyph constants.
- Produces: public `crate::icon::Icons` type with the same constants and values.

- [x] **Step 1: Check references before moving the exported symbol.**

Run LSP `references` for `Icons` at `web/src/render/mod.rs` on the
`pub struct Icons;` line.

Expected: references include at least `icon`, `audiences`, and `sidebar`
consumers; no caller outside `web/src` depends on `crate::render::Icons`.

- [x] **Step 2: Create destination wiring that fails before the type moves.**

In `web/src/icon/mod.rs`, replace `pub use crate::render::Icons;` with:

```rust
mod paths;
pub use paths::Icons;
```

Create `web/src/icon/paths.rs` with an empty file or module comment only.

- [x] **Step 3: Run the icon test, verify it fails.**

Run:
`devtool run -- cargo nextest run -p web icon_matches_reactive_component_markup`

Expected: FAIL — unresolved import `paths::Icons`.

- [x] **Step 4: Move the `Icons` implementation and repoint direct imports.**

Move `pub struct Icons;` and the entire `impl Icons` block from
`web/src/render/mod.rs` to `web/src/icon/paths.rs`.

Update imports:

- `web/src/icon/markup.rs`: use `crate::icon::Icons` or `super::Icons` in tests.
- `web/src/audiences/component.rs`: `use crate::icon::Icons;`.

Existing sidebar imports already use `crate::icon::{Icon, Icons}` and should
compile unchanged.

- [x] **Step 5: Run focused checks, verify pass.**

Run:
`devtool run -- cargo nextest run -p web icon_matches_reactive_component_markup`

Expected: PASS.

Run: `devtool run -- cargo xtask check --no-test`

Expected: PASS, including wasm-clippy for `audiences`/`sidebar` icon callsites.

- [x] **Step 6: Commit.**

Commit via `jaunder-commit` with message:
`refactor(web): move icon paths to icon leaf (#658)`.

---

### Task 3: Move tag-list context to `web::taglist`

**Files:**

- Create/Test: `web/src/taglist/context.rs`
- Modify: `web/src/taglist/mod.rs`
- Modify: `web/src/taglist/component.rs`
- Modify: `web/src/taglist/markup.rs`
- Modify: `web/src/posts/component.rs`
- Modify: `web/src/posts/render.rs`
- Modify: `web/src/timeline/component.rs`
- Modify: `web/src/render/mod.rs`

**Interfaces:**

- Consumes: current public `crate::render::TagCtx`.
- Produces: public `crate::taglist::TagCtx`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagCtx {
    SiteWide,
    ForUser(common::username::Username),
}
```

- [ ] **Step 1: Check references before moving the exported symbol.**

Run LSP `references` for `TagCtx` at `web/src/render/mod.rs` on the
`pub enum TagCtx` line.

Expected: references include `posts::{component,render}`, `timeline::component`,
`taglist::{component,markup}`, and taglist/posts tests.

- [ ] **Step 2: Create destination wiring that fails before the type moves.**

In `web/src/taglist/mod.rs`, add:

```rust
mod context;
pub use context::TagCtx;
```

Create `web/src/taglist/context.rs` with no `TagCtx` definition yet.

- [ ] **Step 3: Run a taglist test, verify it fails.**

Run:
`devtool run -- cargo nextest run -p web tag_list_site_wide_has_hash_chip_and_no_here_link`

Expected: FAIL — unresolved import `context::TagCtx` or unresolved
`crate::taglist::TagCtx` after repointing the test import.

- [ ] **Step 4: Move the enum and repoint callers.**

Move the `TagCtx` enum and its `use common::username::Username;` dependency from
`web/src/render/mod.rs` to `web/src/taglist/context.rs`.

Repoint imports:

- `web/src/taglist/component.rs`: `use crate::taglist::TagCtx;`
- `web/src/taglist/markup.rs`: `use crate::taglist::TagCtx;` and
  `use crate::html::escape_html;`
- `web/src/posts/component.rs`: `use crate::taglist::TagCtx as TagContext;`
- `web/src/posts/render.rs`: import `TagCtx` from `crate::taglist`, not
  `crate::render`.
- `web/src/timeline/component.rs`: `use crate::taglist::TagCtx as TagContext;`

Delete the old enum from `web/src/render/mod.rs`.

- [ ] **Step 5: Run focused checks, verify pass.**

Run: `devtool run -- cargo nextest run -p web tag_list`

Expected: PASS.

Run: `devtool run -- cargo nextest run -p web post_article timeline`

Expected: PASS for tests matching those filters.

Run: `devtool run -- cargo xtask check --no-test`

Expected: PASS.

- [ ] **Step 6: Commit.**

Commit via `jaunder-commit` with message:
`refactor(web): move tag list context to taglist leaf (#658)`.

---

### Task 4: Move home masthead rendering to `web::home::render`

**Files:**

- Create/Test: `web/src/home/render.rs`
- Modify: `web/src/home/mod.rs`
- Modify: `web/src/home/component.rs`
- Modify: `web/src/posts/render.rs`
- Modify: `web/src/render/mod.rs`

**Interfaces:**

- Consumes: current `crate::render::render_home_masthead() -> String` and
  private `render_hero() -> String`.
- Produces: `crate::home::render::render_masthead() -> String`.
  `render_hero() -> String` remains private inside `home::render`.

- [ ] **Step 1: Create destination test and wiring first.**

In `web/src/home/mod.rs`, add `pub(crate) mod render;` beside the existing
component module declaration.

Create `web/src/home/render.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::render_masthead;

    #[test]
    fn home_masthead_has_topbar_hero_and_anon_only_cta() {
        let html = render_masthead();
        assert!(html.contains("<h1>jaunder.local</h1>"), "{html}");
        assert!(
            html.contains("<a href=\"/login\" class=\"j-btn j-anon-only\">Sign in</a>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<a href=\"/register\" class=\"j-btn is-primary j-anon-only\">Register</a>"
            ),
            "{html}"
        );
        assert!(html.contains("<div class=\"j-hero\">"), "{html}");
    }
}
```

- [ ] **Step 2: Run the test, verify it fails.**

Run:
`devtool run -- cargo nextest run -p web home_masthead_has_topbar_hero_and_anon_only_cta`

Expected: FAIL — `render_masthead` is not defined yet.

- [ ] **Step 3: Move implementation and repoint callers.**

Move `render_hero` and `render_home_masthead` from `web/src/render/mod.rs` to
`web/src/home/render.rs` above the tests.

Rename `render_home_masthead` to:

```rust
pub(crate) fn render_masthead() -> String
```

Keep `render_hero` private. Keep the same topbar call and copy exactly.

Repoint callers:

- `web/src/home/component.rs`:
  `let masthead = crate::home::render::render_masthead();`
- `web/src/posts/render.rs`: site timeline branch calls
  `crate::home::render::render_masthead()`.

Delete the old home masthead functions and old test from
`web/src/render/mod.rs`.

- [ ] **Step 4: Run focused checks, verify pass.**

Run:
`devtool run -- cargo nextest run -p web home_masthead_has_topbar_hero_and_anon_only_cta`

Expected: PASS.

Run: `devtool run -- cargo nextest run -p web site_timeline load_more`

Expected: PASS for the projector/body tests matching those filters.

Run: `devtool run -- cargo xtask check --no-test`

Expected: PASS.

- [ ] **Step 5: Commit.**

Commit via `jaunder-commit` with message:
`refactor(web): move home masthead renderer to home leaf (#658)`.

---

### Task 5: Move timeline load-more placeholder to `web::timeline::render`

**Files:**

- Create/Test: `web/src/timeline/render.rs`
- Modify: `web/src/timeline/mod.rs`
- Modify: `web/src/posts/render.rs`
- Modify: `web/src/render/mod.rs`

**Interfaces:**

- Consumes: current `crate::render::render_load_more(has_more: bool) -> String`.
- Produces:
  `crate::timeline::render::render_load_more(has_more: bool) -> String`.

- [ ] **Step 1: Create destination tests and wiring first.**

In `web/src/timeline/mod.rs`, add `pub(crate) mod render;`.

Create `web/src/timeline/render.rs` with only tests:

```rust
#[cfg(test)]
mod tests {
    use super::render_load_more;

    #[test]
    fn load_more_placeholder_renders_when_more_rows_exist() {
        assert_eq!(render_load_more(true), "<button>Load more</button>");
    }

    #[test]
    fn load_more_placeholder_renders_empty_without_next_page() {
        assert_eq!(render_load_more(false), "");
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail.**

Run: `devtool run -- cargo nextest run -p web load_more_placeholder`

Expected: FAIL — `render_load_more` is not defined yet.

- [ ] **Step 3: Move implementation and repoint callers.**

Move the existing `render_load_more` body from `web/src/render/mod.rs` to
`web/src/timeline/render.rs` above the tests with signature:

```rust
pub(crate) fn render_load_more(has_more: bool) -> String
```

Update `web/src/posts/render.rs` to call/import
`crate::timeline::render::render_load_more`.

Delete the old function from `web/src/render/mod.rs`. Keep the existing
`posts::render` composed body test that asserts the button appears/disappears.

- [ ] **Step 4: Run focused checks, verify pass.**

Run: `devtool run -- cargo nextest run -p web load_more_placeholder`

Expected: PASS.

Run:
`devtool run -- cargo nextest run -p web load_more_button_matches_has_more_state`

Expected: PASS.

Run: `devtool run -- cargo xtask check --no-test`

Expected: PASS.

- [ ] **Step 5: Commit.**

Commit via `jaunder-commit` with message:
`refactor(web): move load more renderer to timeline leaf (#658)`.

---

### Task 6: Move media byte formatter to `web::media`

**Files:**

- Create/Test: `web/src/media/format.rs`
- Modify: `web/src/media/mod.rs`
- Modify: `web/src/media/component.rs`
- Modify: `web/src/render/mod.rs`

**Interfaces:**

- Consumes: current
  `crate::render::format_bytes(bytes: impl Into<i64>) -> String`.
- Produces: `crate::media::format_bytes(bytes: impl Into<i64>) -> String`,
  re-exported crate-internally from `media/mod.rs`.

- [ ] **Step 1: Create destination tests and wiring first.**

In `web/src/media/mod.rs`, add:

```rust
mod format;
pub(crate) use format::format_bytes;
```

Create `web/src/media/format.rs` with only these tests:

```rust
#[cfg(test)]
mod tests {
    use super::format_bytes;

    #[test]
    fn format_bytes_displays_bytes_below_kb() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
    }

    #[test]
    fn format_bytes_displays_kb_range() {
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
    }

    #[test]
    fn format_bytes_displays_mb_range() {
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 2), "2.0 MB");
    }

    #[test]
    fn format_bytes_displays_gb_range() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail.**

Run: `devtool run -- cargo nextest run -p web format_bytes`

Expected: FAIL — `format_bytes` is not defined in `media::format` yet.

- [ ] **Step 3: Move implementation and repoint callers.**

Move `format_bytes` and its existing
`#[expect(clippy::cast_precision_loss, reason = ...)]` from
`web/src/render/mod.rs` to `web/src/media/format.rs` above the tests. Use
signature:

```rust
pub(crate) fn format_bytes(bytes: impl Into<i64>) -> String
```

Update `web/src/media/component.rs` to import/use `super::format_bytes` instead
of `crate::render::format_bytes`.

Delete the old function and old tests from `web/src/render/mod.rs`.

- [ ] **Step 4: Run focused checks, verify pass.**

Run: `devtool run -- cargo nextest run -p web format_bytes`

Expected: PASS.

Run: `devtool run -- cargo xtask check --no-test`

Expected: PASS, including clippy preserving the existing cast-expect reason.

- [ ] **Step 5: Commit.**

Commit via `jaunder-commit` with message:
`refactor(web): move byte formatter to media leaf (#658)`.

---

### Task 7: Delete `web::render` and run final checks

**Files:**

- Delete: `web/src/render/mod.rs`
- Remove directory: `web/src/render/` if empty
- Modify: `web/src/lib.rs`
- Modify: active code comments/docs under `web/src` and non-archive docs that
  still describe live code as `web::render`
- Verify: whole worktree

**Interfaces:**

- Consumes: all task outputs above.
- Produces: no `web::render` module; live callsites use co-located homes only.

- [ ] **Step 1: Remove the module and root export.**

Delete `web/src/render/mod.rs` and the now-empty `web/src/render/` directory.

Remove `pub mod render;` from `web/src/lib.rs`.

- [ ] **Step 2: Search for stale live references and update comments.**

Run source searches with the Grep tool, not shell pipelines:

- Pattern: `crate::render|web::render|render::Icons|render::TagCtx` under
  `web/src`
- Pattern: `web/src/render|web::render|crate::render` under active docs,
  excluding `docs/archive/` and preserving historical ADR language unless it
  claims current architecture.

Expected: no live code references to the deleted module. Remaining source
`render` references must point to co-located modules such as
`crate::posts::render`, `crate::app::render`, `crate::home::render`, or
`crate::timeline::render`.

- [ ] **Step 3: Run static gate.**

Run: `devtool run -- cargo xtask check --no-test`

Expected: PASS.

- [ ] **Step 4: Run full local gate.**

Run: `devtool run -- cargo xtask validate`

Expected: PASS, including the e2e matrix that renders projector
shell/home/timeline/sidebar/post/tag/media surfaces.

- [ ] **Step 5: Final conformance check.**

Confirm each acceptance criterion from
`docs/superpowers/specs/2026-07-26-issue-658-render-leaf-homes.md`:

- `web/src/render/` absent and `web/src/lib.rs` has no `pub mod render;`.
- helper homes match AC3.
- no `target_arch` cfgs added beyond existing module wiring.
- no `web::render` shim exists.
- tests that moved with helpers are present at their new homes.

- [ ] **Step 6: Commit final deletion if Task 7 changed files after Task 6.**

Run: `devtool run -- cargo xtask check --no-test`

Expected: PASS.

Commit via `jaunder-commit` with message:
`refactor(web): delete residual render module (#658)`.

- [ ] **Step 7: Handoff to shipping.**

After all commits are present and `validate` is green, invoke `jaunder-ship`:
whole-branch review, conformance review, archive spec/plan, final rebase, push,
open PR, monitor CI, then halt before merge.
