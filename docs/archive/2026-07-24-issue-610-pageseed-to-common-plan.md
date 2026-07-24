# Plan — Move `PageSeed` + wire DTOs to `common::seed` (#610)

Spec: `docs/superpowers/specs/2026-07-24-issue-610-pageseed-to-common.md` (the
what/why). This file is the **how**. Pure relocation — no DTO shape/wire change;
the existing integration tests (which deserialize the wire) + the e2e matrix are
the behavioral guard, so **no new tests** are added.

## Review header

**Goal.** Relocate `PageSeed`, `TimelinePage`, `TimelinePostSummary`,
`PostResponse`, `TagSummary` (+ the `deserialize_rendered_html` helper) from
`web` to a new `common::seed` module (helper → `common::render`), then repoint
every consumer and delete the transitional shims.

**Scope — in.** The new `common::seed` module; the helper move to
`common::render`; the strangler re-exports; the full repoint of ~40 call sites
across `web`/`server`/`csr` + integration tests; shim deletion. **Out.** Any
wire-shape change; the render _functions_ (#312); the `#[cfg(server)]`
query/assembly fns (they stay in `web::posts`, importing DTOs from `common`).

**Tasks.**

1. `refactor(common)`: create `common::seed` with the 5 DTOs (moved verbatim) +
   move `deserialize_rendered_html` to `common::render`; re-export the moved
   types from the old `web::posts`/`web::tags`/`web::render` paths so the tree
   still compiles. **The atomic move.**
2. `refactor(web)`: repoint every consumer (projector, `web::render` renderers,
   `csr` boot, `#[server]` fns, wasm components, server integration tests, and
   the `web::posts` server-logic fns) to `common::seed`; **delete the re-export
   shims**.
3. Full gate: `cargo xtask validate` green, `git status --porcelain` empty.

**Key risks / decisions.**

- **Byte-identical wire (AC2).** Move the DTOs verbatim — fields, derive list
  (`Debug, Clone, Serialize, Deserialize, PartialEq, Eq`), and `#[serde(...)]`
  attrs unchanged. Do NOT "tidy" anything. The `#jaunder-seed` blob and every
  `#[server]` response must serialize identically; the integration tests that
  `serde_json::from_str::<TimelinePage>`/`<PostResponse>` and the e2e
  first-paint matrix prove it.
- **`deserialize_with` path resolution.** The two attrs (on
  `TimelinePostSummary`/ `PostResponse`) resolve `deserialize_rendered_html` by
  path string. Once both structs and the helper are in `common`,
  `use crate::render::deserialize_rendered_html;` in `seed.rs` + the bare
  `"deserialize_rendered_html"` string works (same crate). The helper is
  `pub(crate)` in `common::render` — no `web` reference remains (web never
  deserializes these; it only _builds_ them server-side), so no transitional
  `pub`.
- **No cycle / no new dep.** `common::seed` imports only `common` types;
  `common` stays the leaf crate. Confirm `common/Cargo.toml` is unchanged.
- **Coverage neutral.** The DTOs are pure data (derive-generated codec,
  exercised by the same serialize/deserialize tests before and after) and
  `deserialize_rendered_html` is exercised by the integration tests that
  deserialize the two structs — coverage follows the types to `common`. No new
  `cov:ignore`/`crap:allow`.
- **Registrar unaffected.** Server-fn registration is by endpoint string, not
  type path (`server/tests/helpers/mod.rs`) — no change.

## Global Constraints

- **Fork point:** `wt-base-issue-610`. Review the whole branch with
  `git diff wt-base-issue-610..HEAD`.
- **Per-commit gate:** `cargo xtask check` (via
  `devtool run -- cargo xtask check`). **Full gate:** `cargo xtask validate`.
- Commit subjects: `type(scope): subject (#610)`; **no** `Co-Authored-By`
  trailer.
- Follow `CONTRIBUTING.md` (import discipline, coverage policy). Wasm-touching
  edits (`csr`, `web` components): run `cargo xtask check`'s `wasm-clippy` step
  (it lints `-p web -p csr --features csr`).
- No commit without explicit user approval; request review first.
- **Agentic workers:** dispatch via `jaunder-dispatch`; execute the per-task
  loop via `jaunder-iterate`.

---

## Task 1 — Create `common::seed`; move the helper; add strangler re-exports

**Files:** new `common/src/seed.rs`; `common/src/lib.rs`;
`common/src/render.rs`; `web/src/render/mod.rs`; `web/src/posts/api.rs`;
`web/src/posts/api/listing.rs`; `web/src/tags/api.rs`; `web/src/posts/mod.rs`;
`web/src/tags/mod.rs`.

### 1a. `deserialize_rendered_html` → `common::render`

Cut the helper from `web/src/posts/api.rs:126-131` and paste it into
`common/src/render.rs`, next to `RenderedHtml`/`from_trusted`, as `pub(crate)`:

```rust
/// Deserializes a wire `String` into a `RenderedHtml` via `from_trusted` — the
/// deserialize counterpart to `RenderedHtml`'s deliberate lack of a `Deserialize`
/// impl (it is server-rendered, trusted output; see the note above). Used by the
/// seed DTOs' `#[serde(deserialize_with = ...)]`.
pub(crate) fn deserialize_rendered_html<'de, D>(deserializer: D) -> Result<RenderedHtml, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer).map(RenderedHtml::from_trusted)
}
```

(`serde::Deserialize` is already used in `render.rs` for the sibling types; add
the `use` if needed.)

### 1b. New `common/src/seed.rs`

Add `pub mod seed;` to `common/src/lib.rs` (alphabetical: after
`pub mod root_relative_url;`, before `pub mod site;`). Create
`common/src/seed.rs` and move the five types **verbatim** from their current
locations (definitions confirmed in the spec's source map):

- `TagSummary` (from `web/src/tags/api.rs:26-30`)
- `TimelinePostSummary`, `TimelinePage` (from
  `web/src/posts/api/listing.rs:37-69`)
- `PostResponse` (from `web/src/posts/api.rs:134-154`)
- `PageSeed` (from `web/src/render/mod.rs:57-74`)

Module header + imports:

```rust
//! The projector↔client seed contract (#610, ADR-0041): `PageSeed` — the initial data
//! a public page renders from — and the public-surface wire DTOs it embeds. The server
//! projector serializes `PageSeed` into the `#jaunder-seed` DOM blob; the `csr` client
//! deserializes it on boot for a byte-identical first paint. These are also the return
//! types of the media/post/tag `#[server]` fns. Pure `Serialize`/`Deserialize` data —
//! every field is a `common` type, so this module has no leptos/web_sys/storage coupling.

use serde::{Deserialize, Serialize};

use crate::ids::PostId;
use crate::post_body::PostBody;
use crate::post_summary::PostSummary;
use crate::post_title::PostTitle;
use crate::render::{deserialize_rendered_html, PostFormat, RenderedHtml};
use crate::root_relative_url::RootRelativeUrl;
use crate::slug::Slug;
use crate::tag::{Tag, TagLabel};
use crate::time::UtcInstant;
use crate::username::Username;
```

The `#[serde(deserialize_with = "deserialize_rendered_html")]` bare-name attrs
on `TimelinePostSummary.rendered_html` and `PostResponse.rendered_html` resolve
against the `use crate::render::deserialize_rendered_html;` above — keep the
strings unchanged.

### 1c. Strangler re-exports (keep the tree compiling)

Delete the moved definitions from their web files and re-export the `common`
types from the old paths so the ~40 consumers and the server-logic fns still
resolve:

- `web/src/render/mod.rs`: remove the `PageSeed` enum + its now-unused imports;
  add `pub use common::seed::PageSeed;`. (The render _functions_ stay and now
  take `common::seed::PageSeed`.)
- `web/src/posts/api/listing.rs`: remove `TimelinePage`/`TimelinePostSummary` +
  the `use super::deserialize_rendered_html;`; the `fetch_*`/`page_from_rows`
  fns reference the DTOs via `super::…` (resolved through the mod re-export
  below).
- `web/src/posts/api.rs`: remove `PostResponse` + `deserialize_rendered_html`.
- `web/src/posts/mod.rs`: replace the removed names in the `pub use api::{…}`
  list with
  `pub use common::seed::{PostResponse, TimelinePage, TimelinePostSummary};`.
- `web/src/tags/api.rs`: remove `TagSummary`; `web/src/tags/mod.rs`:
  `pub use common::seed::TagSummary;`.
- `web/src/posts/server.rs` fns (`post_response`, `timeline_post_summary`,
  `post_tags_to_summaries`) keep referencing `super::…` — resolved via the
  `posts/mod.rs` re-export. (Direct-`common` repoint happens in Task 2.)

Trim any now-unused `use serde::{Deserialize, Serialize};` / `use common::…` in
the touched web files (compiler-forced).

### Verify (Task 1)

```
devtool run -- cargo xtask check
```

Expected: PASS. The re-exports keep every consumer compiling. Confirm the move
landed:

```
rg -n "pub enum PageSeed|pub struct (TimelinePage|TimelinePostSummary|PostResponse|TagSummary)" common/src/seed.rs
rg -n "pub enum PageSeed|pub struct PostResponse" web/src   # → no definitions, only re-exports
rg -n "^\\w" common/Cargo.toml                              # unchanged — no new dep
```

**Commit:**
`refactor(common): move PageSeed + seed wire DTOs to common::seed (#610)`

---

## Task 2 — Repoint every consumer to `common::seed`; delete the shims

**Files:** `server/src/projector/mod.rs`; `web/src/render/mod.rs`;
`web/src/posts/render.rs`; `csr/src/lib.rs`; `web/src/posts/api/listing.rs`,
`api.rs`, `server.rs`; `web/src/posts/component.rs`;
`web/src/home/component.rs`; `web/src/timeline/{state.rs,component.rs}`;
`web/src/taglist/{markup.rs,component.rs}`; `web/src/posts/mod.rs`;
`web/src/tags/mod.rs`; `server/tests/web/{web_posts.rs,web_tags.rs}`.

Repoint each site (import + usages) from the `web::…` path to `common::seed::…`.
Then **delete the re-export shims** added in Task 1c (`pub use common::seed::…`
in `web::posts`/`web::tags`/`web::render`). Order within the commit doesn't
matter — deleting a shim and repointing its consumers happen together, so the
tree only compiles once both are done (this is one commit).

Key sites (from the spec's source map):

- **`server::projector`** —
  `use web::render::{render_head, render_shell, PREPAINT_SCRIPT};`
  - `use common::seed::PageSeed;`; every `PageSeed::…` construction and
    `web::posts::TimelinePage` reference (incl. the `:356-414` tests) →
    `common::seed::…`.
- **`web::render` / `web::posts::render`** — the
  `render_head`/`render_discovery`/ `render_shell`/`render_body` signatures take
  `common::seed::PageSeed`; the test fixtures build
  `common::seed::{TimelinePostSummary, TagSummary, …}`.
- **`csr/src/lib.rs`** — `use common::seed::PageSeed;` (was
  `web::render::PageSeed`); `serde_json::from_str::<PageSeed>` unchanged
  otherwise.
- **`#[server]` fns / server-logic** — `list_*` (`listing.rs`), `list_tags`
  (`tags/api.rs`), `get_post*` (`api.rs`), and
  `post_response`/`timeline_post_summary`/ `post_tags_to_summaries`
  (`server.rs`), `fetch_*`/`page_from_rows` — import the DTOs from
  `common::seed` instead of `super::`.
- **wasm components** — `posts/component.rs`, `home/component.rs`, `timeline/*`,
  `taglist/*` — repoint `crate::posts::{…}` / `crate::tags::TagSummary` /
  `crate::render::PageSeed` to `common::seed::…`.
- **Integration tests** — `server/tests/web/web_posts.rs`, `web_tags.rs`: the
  `web::posts::PostResponse` / `web::tags::TagSummary` /
  `serde_json::from_str::<TimelinePage>` imports → `common::seed::…`.

### Verify (Task 2)

```
devtool run -- cargo xtask check
devtool run -- cargo clippy -p web -p csr --features csr --target wasm32-unknown-unknown -- -D warnings -A clippy::too_many_arguments
```

Expected: PASS. Confirm the shims are gone (AC3):

```
rg -n "pub use common::seed" web/src            # → empty (no lingering re-export)
rg -n "web::render::PageSeed|web::posts::(PostResponse|TimelinePage|TimelinePostSummary)|web::tags::TagSummary" .   # → empty
```

**Commit:**
`refactor(web): repoint seed DTO consumers to common::seed, drop shims (#610)`

---

## Task 3 — Full gate

**Files:** none (verification only).

```
devtool run -- cargo xtask validate
git status --porcelain
```

Expected: PASS — static + clippy + coverage + the full e2e matrix (the
byte-identical first-paint guard for AC2), incl. `wasm-clippy`,
`server-fn-registrar`, `rendered-html-from-trusted`. Coverage clean, no new
`cov:ignore`/`crap:allow`. Working tree empty.

**Commit:** none (gate only), unless a fmt tail must be folded back.

---

## Self-review — spec AC → task map

| AC      | Requirement                                                             | Satisfied by                                            |
| ------- | ----------------------------------------------------------------------- | ------------------------------------------------------- |
| **AC1** | `common::seed` with the 5 DTOs; helper in `common::render`              | Task 1 (1a, 1b)                                         |
| **AC2** | No wire change; byte-identical serialization; no new `common` dep       | Task 1 (verbatim move) + Task 3 (e2e/integration guard) |
| **AC3** | All consumers repointed; shims deleted                                  | Task 2 (repoint + delete + grep)                        |
| **AC4** | Server query/assembly fns stay in `web::posts`, importing from `common` | Task 1 (stay) + Task 2 (repoint imports)                |
| **AC5** | `csr` boot deserializes `common::seed::PageSeed`                        | Task 2                                                  |
| **AC6** | `cargo xtask validate` green; coverage clean                            | Task 3                                                  |
