# Split `storage/src/render.rs` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the misplaced `storage/src/render.rs` so the pure content-rendering and title/metadata logic lives in `common`, while the storage-orchestration service layer stays in `storage` under an honestly-named module.

**Architecture:** `render.rs` currently mixes three concerns: (1) pure body rendering (Markdown/Org/HTML → HTML), (2) pure title/metadata extraction, and (3) a service layer that orchestrates the `PostStorage` trait (used by *both* `web` and `server/atompub`). Concerns (1)+(2) depend only on `PostFormat` (a plain enum, no sqlx) and `common::slug`, so they move to `common`. Concern (3) is coupled to the `PostStorage` trait/types (defined in `storage`) and is shared by two crates, so it stays in `storage`, renamed `post_service`. `PostFormat` moves to `common` (required, since `common` cannot depend on `storage`) and is re-exported from `storage` so no external imports change.

**Tech Stack:** Rust workspace (crates: `common`, `storage`, `server`, `web`, `hydrate`), `sqlx`, `cargo nextest`, `pulldown-cmark`, `orgize`. Coverage gate: `scripts/check-coverage` against `.coverage-manifest.json` / `.crap-manifest.json`.

---

## This is a refactor, not greenfield

The code being relocated is already implemented and already has a comprehensive test suite. The TDD safety net here is: **the existing tests move with the code and must stay green after each move.** There is no "write a failing test first" step; instead every task ends with `cargo nextest run` (and where relevant `scripts/check-coverage`) proving behavior is unchanged. Moves must be **verbatim** — do not rewrite logic while relocating it.

## Pre-flight facts (verified 2026-06-02, branch `simplify`)

- `common/Cargo.toml` **already** depends on `orgize` and `pulldown-cmark` — no dependency needs adding to `common`.
- `storage` uses `orgize`/`pulldown_cmark` **only** in `render.rs` — so `storage` can drop both deps after the move.
- `PostFormat` is a plain enum with `Display`/`FromStr` and **no sqlx derives**; DB mapping is string-based via those impls.
- External consumers of the orchestration layer (must keep working via crate-root re-export):
  - `web/src/posts/mod.rs:166,335` → `perform_post_creation`, `perform_post_update`; tests use `candidate_slug`.
  - `server/src/atompub/posts.rs:296,370` → `storage::perform_post_creation`, `storage::perform_post_update`.
- External consumers of `PostFormat`: `server/src/atompub/mapping.rs`, `web/src/pages/profile.rs`, `web/src/posts/{mod,server}.rs`, plus many `server/tests/*`. All reference it as `storage::PostFormat` (or `crate::PostFormat` inside storage) — the re-export keeps every one of these unchanged.
- `render()` and `derive_post_metadata()` have **no callers outside `render.rs`** today (confirmed by repo-wide grep); after the move their only caller is `storage::post_service`.

## File Structure (end state)

- **Create `common/src/render.rs`** — owns: `PostFormat`, `InvalidPostFormat`, `RenderError`, `render`, `render_markdown`, `render_org`, `DerivedPostMetadata`, `derive_post_metadata`, `extract_markdown_title`, `extract_org_title`, `fallback_label`, plus all their unit tests. One responsibility: pure post-body rendering and title/metadata derivation. (~200 LoC + tests.)
- **Modify `common/src/lib.rs`** — add `pub mod render;`.
- **Rename `storage/src/render.rs` → `storage/src/post_service.rs`** — keeps only the orchestration: `create_rendered_post`, `update_rendered_post`, `perform_post_creation`, `perform_post_update`, `candidate_slug`, and the error enums `CreateRenderedPostError`, `UpdateRenderedPostError`, `PerformUpdateError`, `PerformCreationError`, plus their DB-backed tests. One responsibility: post create/update orchestration over `PostStorage`.
- **Modify `storage/src/posts.rs`** — delete the `PostFormat` definition/impls and its unit tests; re-export `pub use common::render::{PostFormat, InvalidPostFormat};`.
- **Modify `storage/src/lib.rs`** — `mod render;`→`mod post_service;` and `pub use render::*;`→`pub use post_service::*;`.
- **Modify `storage/Cargo.toml`** — remove `orgize` and `pulldown-cmark`.
- **Modify `.coverage-manifest.json` / `.crap-manifest.json`** — drop `storage/src/render.rs`; add `common/src/render.rs` and `storage/src/post_service.rs` at measured values (REQUIRES USER APPROVAL — see Task 5).

No changes required in `web` or `server` source (re-exports preserve all paths).

---

### Task 1: Move `PostFormat` to `common::render` (with re-export)

**Files:**
- Create: `common/src/render.rs`
- Modify: `common/src/lib.rs:1-11` (module list)
- Modify: `storage/src/posts.rs:3` (imports), `:13-50` (PostFormat block), test fns `post_format_*`

- [ ] **Step 1: Create `common/src/render.rs` with `PostFormat` moved verbatim**

Move, **verbatim**, the `PostFormat` enum, the `InvalidPostFormat` error, and the `impl fmt::Display for PostFormat` / `impl FromStr for PostFormat` blocks currently at `storage/src/posts.rs:13-50`. Add the two `use` lines these need. The new file starts as:

```rust
//! Pure post-body rendering and title/metadata derivation.
//!
//! Format-driven transformation of post bodies to HTML plus extraction of
//! titles, slug seeds, and summary labels. No storage or database concerns.

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// The format/markup language used to author a post body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostFormat {
    /// CommonMark/GitHub-flavored Markdown.
    Markdown,
    /// Emacs Org-mode format.
    Org,
    /// Pre-rendered HTML.
    Html,
}

/// Error returned when a string cannot be parsed as a [`PostFormat`].
#[derive(Debug, Error)]
#[error("post format must be \"markdown\", \"org\", or \"html\"")]
pub struct InvalidPostFormat;

impl fmt::Display for PostFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PostFormat::Markdown => f.write_str("markdown"),
            PostFormat::Org => f.write_str("org"),
            PostFormat::Html => f.write_str("html"),
        }
    }
}

impl FromStr for PostFormat {
    type Err = InvalidPostFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "markdown" => Ok(PostFormat::Markdown),
            "org" => Ok(PostFormat::Org),
            "html" => Ok(PostFormat::Html),
            _ => Err(InvalidPostFormat),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}
```

- [ ] **Step 2: Move the `PostFormat` unit tests into `common/src/render.rs`**

Cut these test fns **verbatim** from `storage/src/posts.rs`'s `mod tests` and paste them into the `mod tests` of `common/src/render.rs`: `post_format_markdown_variant` (`:435`), `post_format_org_variant` (`:441`), `post_format_display_round_trips` (`:447`), `post_format_rejects_invalid_value` (`:458`), `post_format_debug` (`:467`), `post_format_html_roundtrips_via_display_and_from_str` (`:577`). They reference only `PostFormat`/`InvalidPostFormat`, satisfied by `use super::*;`.

- [ ] **Step 3: Register the module in `common/src/lib.rs`**

Insert into the alphabetical module list (after `pub mod password;`):

```rust
pub mod render;
```

- [ ] **Step 4: Replace the `PostFormat` definition in `storage/src/posts.rs` with a re-export**

Delete lines `13-50` (the `PostFormat` enum through the end of the `FromStr` impl) and the six `post_format_*` tests moved in Step 2. In their place, near the top of the file (after the existing `use` lines), add:

```rust
pub use common::render::{InvalidPostFormat, PostFormat};
```

- [ ] **Step 5: Fix now-unused imports in `storage/src/posts.rs`**

Line 3 is `use std::{fmt, str::FromStr};`. If `cargo build` reports `fmt` and/or `FromStr` unused (the `PostFormat` impls were their only users), remove the unused ones. If other code in `posts.rs` still uses them, leave those. Let the compiler decide — do not guess.

- [ ] **Step 6: Build the workspace**

Run: `cargo build`
Expected: compiles clean. `storage::PostFormat` now resolves through `posts.rs`'s re-export of `common::render::PostFormat`, so `web`/`server` see no change.

- [ ] **Step 7: Run the affected test suites**

Run: `cargo nextest run -p common -p storage`
Expected: PASS, including the migrated `post_format_*` tests now under `common`.

- [ ] **Step 8: Commit**

```bash
git add common/src/render.rs common/src/lib.rs storage/src/posts.rs
git commit -m "refactor(common): move PostFormat into common::render, re-export from storage

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Move pure rendering + metadata into `common::render`

**Files:**
- Modify: `common/src/render.rs` (add the pure functions + tests)
- Modify: `storage/src/render.rs` (remove the moved items; import them from `common::render`)

- [ ] **Step 1: Move the pure source items verbatim into `common/src/render.rs`**

Cut these items **verbatim** from `storage/src/render.rs` and paste them into `common/src/render.rs` (above its `mod tests`): `RenderError` (`:14-18`), `render` (`:24-35`), `DerivedPostMetadata` (`:38-43`), `derive_post_metadata` (`:45-88`), `fallback_label` (`:90-96`), `extract_markdown_title` (`:98-120`), `extract_org_title` (`:122-174`), `render_markdown` (`:176-190`), `render_org` (`:192-199`).

`render_markdown`/`render_org` carry their own `use pulldown_cmark::…` / `orgize::…` inside the fn bodies, so no module-level import is needed for them. These functions reference `PostFormat`, already present in this module.

- [ ] **Step 2: Move the pure tests verbatim into `common/src/render.rs`**

Cut these test fns **verbatim** from `storage/src/render.rs`'s `mod tests` into `common/src/render.rs`'s `mod tests`:
- Markdown: `markdown_headings`, `markdown_paragraph`, `markdown_bold_italic_strikethrough`, `markdown_code_block`, `markdown_links`, `markdown_ordered_list`, `markdown_unordered_list`, `markdown_table`, `markdown_empty_input`, `markdown_multiple_paragraphs`, `markdown_tasklist`
- Org: `org_headings`, `org_paragraph`, `org_bold_italic_code`, `org_list`, `org_code_block`, `org_link`, `org_empty_input`
- Dispatch/metadata: `render_dispatches_markdown`, `render_dispatches_org`, `derive_metadata_prefers_explicit_title`, `derive_metadata_extracts_markdown_h1`, `derive_metadata_extracts_org_title`, `derive_metadata_for_html_extracts_no_title_but_keeps_fallback_label`, `derive_metadata_allows_titleless_notes`, `derive_metadata_rejects_empty_posts`, `derive_metadata_extracts_org_level1_heading`
- Render error: `render_error_display`, `render_error_debug`
- Org title extraction: `extract_org_title_handles_level1_heading`, `extract_org_title_heading_after_kv_lines`, `extract_org_title_skips_blank_lines_inside_kv_block`, `extract_org_title_blank_line_after_kv_without_title_returns_none`, `extract_org_title_title_takes_precedence_over_heading`, `extract_org_title_heading_not_top_level_ignored`, `extract_org_title_heading_after_body_text_ignored`, `extract_org_title_empty_title_value_skipped_heading_used`, `extract_org_title_non_kv_colon_line_returns_none`, `extract_org_title_empty_heading_returns_none`
- HTML identity: `render_html_format_is_identity`

These reference only items now in `common::render` (`render`, `render_markdown`, `render_org`, `derive_post_metadata`, `extract_org_title`, `PostFormat`, `RenderError`) — all satisfied by `use super::*;`.

- [ ] **Step 3: Import the moved items in `storage/src/render.rs`**

The orchestration code remaining in `storage/src/render.rs` calls `render` and `derive_post_metadata` and constructs `RenderError`. Update its imports so the `use crate::{…}` block no longer lists the removed items, and add:

```rust
use common::render::{derive_post_metadata, render, RenderError};
```

Keep the existing `use common::slug::{slugify_title, Slug};` (still used by the orchestration). Keep the `use crate::{ CreatePostError, CreatePostInput, PostFormat, PostRecord, PostStorage, UpdatePostError, UpdatePostInput, };` block (these are still used; `PostFormat` resolves via the crate re-export). `DerivedPostMetadata` is only used as the return of `derive_post_metadata` via `.title`/`.slug_seed` fields — if the code names the type explicitly anywhere, add it to the `common::render` import; otherwise it is not needed.

- [ ] **Step 4: Build the workspace**

Run: `cargo build`
Expected: compiles clean.

- [ ] **Step 5: Run the affected test suites**

Run: `cargo nextest run -p common -p storage`
Expected: PASS. The pure tests now run under `common`; the orchestration tests (`perform_*`, `*_rendered_post_error_*`) still run under `storage`.

- [ ] **Step 6: Commit**

```bash
git add common/src/render.rs storage/src/render.rs
git commit -m "refactor(common): move pure post rendering and metadata into common::render

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Rename `storage/src/render.rs` → `storage/src/post_service.rs`

**Files:**
- Rename: `storage/src/render.rs` → `storage/src/post_service.rs`
- Modify: `storage/src/lib.rs:17,47`

- [ ] **Step 1: Rename the file with git**

Run:
```bash
git mv storage/src/render.rs storage/src/post_service.rs
```

- [ ] **Step 2: Update the module declaration and re-export in `storage/src/lib.rs`**

Change line 17 from `mod render;` to keep alphabetical order — remove the old `mod render;` line and add `mod post_service;` in alphabetical position (after `mod password;`). Change the re-export line 47 from `pub use render::*;` to `pub use post_service::*;` (place in alphabetical order after `pub use password::*;`).

Resulting lines (exact):
```rust
mod post_service;
```
```rust
pub use post_service::*;
```

- [ ] **Step 3: Update the module doc comment**

At the top of `storage/src/post_service.rs`, add/adjust the module doc to reflect its real responsibility:

```rust
//! Post create/update orchestration over the [`PostStorage`] trait.
//!
//! Validates input, derives titles/slugs (via `common::render`), renders the
//! body, and performs the storage write with slug-collision retry. Shared by
//! the `web` and `server` AtomPub front-ends.
```

- [ ] **Step 4: Build the workspace**

Run: `cargo build`
Expected: compiles clean. All public items (`perform_post_creation`, `perform_post_update`, `create_rendered_post`, `update_rendered_post`, `candidate_slug`, error enums) remain re-exported at the `storage` crate root, so `web::posts` and `server::atompub::posts` are unaffected.

- [ ] **Step 5: Run the full unit/integration suite**

Run: `cargo nextest run`
Expected: PASS (1000+ tests). Confirms `web` and `server` consumers still resolve every symbol.

- [ ] **Step 6: Commit**

```bash
git add storage/src/post_service.rs storage/src/lib.rs
git commit -m "refactor(storage): rename render module to post_service

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Drop now-unused `storage` dependencies

**Files:**
- Modify: `storage/Cargo.toml:18-19`

- [ ] **Step 1: Remove the render libraries from `storage/Cargo.toml`**

Delete these two lines (verified to be used only by the moved code):
```toml
orgize.workspace = true
pulldown-cmark.workspace = true
```

- [ ] **Step 2: Build to confirm they were unused**

Run: `cargo build -p storage`
Expected: compiles clean. If it fails with an unresolved `orgize`/`pulldown_cmark`, a usage was missed — restore the dep and investigate before proceeding (do not leave it half-done).

- [ ] **Step 3: Verify lockfile + workspace still build**

Run: `cargo build`
Expected: clean; `Cargo.lock` may update to drop the now-unneeded edges for `storage` (the crates remain in the lock because `common` still uses them).

- [ ] **Step 4: Commit**

```bash
git add storage/Cargo.toml Cargo.lock
git commit -m "refactor(storage): drop orgize and pulldown-cmark, now unused

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: Update coverage baselines and run full verification

**Files:**
- Modify: `.coverage-manifest.json`, `.crap-manifest.json`

> CONTRIBUTING requires baseline changes to be approved by the user and committed in the same commit as the code whose coverage changed. Because the relocation is a single logical change spanning Tasks 1–4, record the baseline updates here in one final commit.

- [ ] **Step 1: Measure current coverage**

Run: `scripts/check-coverage`
Expected: it will report a regression/new-file situation, because the `storage/src/render.rs` manifest entry (currently `98.8535`) now points at a non-existent file, and two new files (`common/src/render.rs`, `storage/src/post_service.rs`) default to an expected 100%. Note the measured percentages it prints for the two new files.

- [ ] **Step 2: Investigate any genuine gaps**

Run: `scripts/check-coverage --investigate`
Expected: `common/src/render.rs` should measure at/near 100% (pure, fully tested). `storage/src/post_service.rs` should measure at/near the old render.rs value. Confirm any sub-100% lines are pre-existing (e.g. DB-backed orchestration arms), not newly uncovered logic introduced by the move.

- [ ] **Step 3: Update `.coverage-manifest.json`**

Remove the `"storage/src/render.rs": 98.8535,` entry. Add, in alphabetical position, `"common/src/render.rs": <measured>,` and `"storage/src/post_service.rs": <measured>,` using the values from Step 1. **Get explicit user approval for these three baseline changes before committing.**

- [ ] **Step 4: Regenerate CRAP baseline if needed**

`scripts/check-coverage` rewrites `.crap-manifest.json` with the renamed/relocated function entries. Confirm via `git diff .crap-manifest.json` that the changes are only path/file relocations for the moved functions, not score regressions.

- [ ] **Step 5: Re-run the coverage gate to confirm green**

Run: `scripts/check-coverage`
Expected: `Coverage and CRAP OK (coverage at/above baseline, CRAP scores at/below baseline).`

- [ ] **Step 6: Run full verification**

Run: `scripts/verify`
Expected: fmt, build, tests, lint, coverage, and `nix flake check` all pass. (Note: the qemu-VM e2e checks are timing-sensitive under constrained environments; a green run on the user's machine is authoritative.)

- [ ] **Step 7: Commit**

```bash
git add .coverage-manifest.json .crap-manifest.json
git commit -m "test(coverage): rebaseline after splitting render module

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

- [ ] **Step 8: Close the bead**

Run:
```bash
bd close jaunder-pcmk
```

---

## Self-Review

**Spec coverage** (against the agreed assessment):
- Pure rendering → `common`: Tasks 1 (PostFormat) + 2 (render/metadata). ✓
- Orchestration stays in `storage`, honestly named: Task 3 (`post_service`). ✓
- `PostFormat` movable (no sqlx) and re-exported so external code is untouched: Task 1, Steps 4 + 6. ✓
- Drop deps storage no longer needs: Task 4. ✓
- Coverage manifest split (render.rs → common/render.rs + post_service.rs), with approval: Task 5. ✓
- Both consumers (`web`, `server/atompub`) preserved via crate-root re-exports: verified by `cargo nextest run` in Task 3, Step 5. ✓

**Placeholder scan:** No `TBD`/`handle edge cases`/"write tests for the above" — tests are named explicitly and moved verbatim. The only intentionally-deferred values are the measured coverage percentages in Task 5 (cannot be known until the move is done) and the compiler-driven unused-import decision in Task 1 Step 5 (correct to defer to the compiler rather than guess).

**Type/name consistency:** `PostFormat`, `InvalidPostFormat`, `RenderError`, `DerivedPostMetadata`, `derive_post_metadata`, `render`, `render_markdown`, `render_org`, `perform_post_creation`, `perform_post_update`, `candidate_slug` are used consistently across tasks. New module is `common::render`; storage module renamed to `post_service`; both re-exported at the `storage` crate root.

## Risks / watch-items

- **`PostFormat` re-export is load-bearing.** Every `storage::PostFormat` / `crate::PostFormat` reference relies on the re-export in `posts.rs`. The `cargo nextest run` in Task 3 Step 5 (which compiles `web` + `server`) is the proof it holds.
- **Verbatim moves.** Do not "improve" logic mid-move; behavior changes must be a separate follow-up so the existing tests remain a valid safety net.
- **Coverage manifest churn.** The path rename means a stale entry + two new entries; this is the one step needing user sign-off.
