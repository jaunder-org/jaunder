# Spec — Move `PageSeed` + public-surface wire DTOs to `common::seed` (#610)

## Summary

`web::render::PageSeed` is the projector↔client seed contract: the server
projector (`server::projector`) serializes it into the `#jaunder-seed` DOM blob,
and the CSR client (`csr`) deserializes it on boot for a byte-identical first
paint (ADR-0041). It is a pure `Serialize`/`Deserialize` enum, and the four wire
DTOs it embeds (`TimelinePage`, `TimelinePostSummary`, `PostResponse`,
`TagSummary`) are equally pure — every field is already a `common` type, and
none carry a leptos / `web_sys` / storage dependency. Yet they live in `web`,
which #312 is dissolving. #610 relocates the six items to `common`, their honest
final home.

## Decision (resolved)

A new **`common::seed`** module holds all five wire types — `PageSeed`,
`TimelinePage`, `TimelinePostSummary`, `PostResponse`, `TagSummary` — as one
cohesive projector↔client contract. The `deserialize_rendered_html` helper
moves to **`common::render`**, next to `RenderedHtml`,
`RenderedHtml::from_trusted`, and the deliberate "no `Deserialize`" comment
(`render.rs:146`) that motivates it.

Rationale for a dedicated module over the `common::media` domain-module
precedent: the post DTOs are **composite** (no single domain-newtype module to
live in), and `PageSeed` deliberately spans domains — it is the seed contract
itself, not a member of one domain. Keeping them together aligns with the
shared-DTO / `jaunder-core` API-surface direction (#284). This is a **pure
relocation**: no change to any DTO's shape, derives, serde attributes, or wire
form.

## Design

### A. New `common::seed` module

Move verbatim (fields, derives
`Debug, Clone, Serialize, Deserialize, PartialEq, Eq`, and `#[serde(...)]` attrs
unchanged):

- `PageSeed` (enum) — from `web/src/render/mod.rs:57`
- `TimelinePage`, `TimelinePostSummary` — from `web/src/posts/api/listing.rs`
- `PostResponse` — from `web/src/posts/api.rs`
- `TagSummary` — from `web/src/tags/api.rs`

`common/src/lib.rs` gains `pub mod seed;`. Every field type is already in
`common` (`PostId`, `Username`, `PostTitle`, `PostSummary`, `PostBody`, `Slug`,
`PostFormat`, `RenderedHtml`, `UtcInstant`, `RootRelativeUrl`, `Tag`,
`TagLabel`), so `common::seed` imports only from within `common` — no new crate
dependency, no cycle (`common` is the leaf; `web`/`server`/`csr` depend on it,
not vice-versa). The module is **ungated** (no `#[cfg]`).

### B. `deserialize_rendered_html` → `common::render`

The helper (`String::deserialize().map(RenderedHtml::from_trusted)`) moves to
`common::render`, made at least `pub(crate)` so `common::seed` can reference it.
The two `#[serde(deserialize_with = "…")]` sites (on
`TimelinePostSummary.rendered_html` and `PostResponse.rendered_html`) resolve it
via its new path — either `use` it into `common::seed` and keep the bare name,
or spell the full `common::render::…` path in the attribute (implementer's
choice; behavior identical).

### C. Strangler move, then delete the shims

Land the types in `common::seed`, re-export from the old `web` paths
(`web::posts::{PostResponse, TimelinePage, TimelinePostSummary}`,
`web::tags::TagSummary`, `web::render::PageSeed`) so the ~40 call sites and the
integration tests keep compiling, then **repoint every consumer to
`common::seed` and delete the shims** (per the issue — no lingering re-exports
of the moved types). Dependency order that keeps each step compiling:
`TagSummary` → `TimelinePostSummary`/`TimelinePage` → `PostResponse` →
`PageSeed`.

### D. Repoint consumers

- **`server::projector`** (`server/src/projector/mod.rs`) — imports + all
  `PageSeed::…` constructions and the `TimelinePage`/`PostResponse` references,
  plus its tests.
- **`web::render`** renderers (`render_head`/`render_discovery`/`render_shell`
  in `render/mod.rs`, `render_body` in `posts/render.rs`) — take
  `common::seed::PageSeed`.
- **`csr` boot** (`csr/src/lib.rs`) — `serde_json::from_str::<PageSeed>` now
  names `common::seed::PageSeed` (a one-line import change, as the issue
  anticipates).
- **`#[server]` fns + components** — the `list_*` fns (return `TimelinePage`),
  `list_tags` (`Vec<TagSummary>`), `get_post`/`get_post_preview`
  (`PostResponse`), and the wasm components
  (`posts`/`home`/`timeline`/`taglist`) that read the seed from context or build
  the DTOs.
- **Server-fn registrar** — unaffected (registration is by endpoint string, not
  type path).
- **`web::posts`/`web::tags` re-export hubs** — drop the moved types from their
  `pub use` lists.

### E. Server logic stays in `web::posts`

The `#[cfg(feature = "server")]` query/assembly fns beside the DTOs —
`page_from_rows`, `fetch_user_posts`, `fetch_local_timeline`,
`fetch_posts_by_tag`, `fetch_user_posts_by_tag` (`posts/api/listing.rs`), and
`timeline_post_summary`, `post_tags_to_summaries`, `post_response`
(`posts/server.rs`) — **do not move**; they are server logic, not wire types.
They now import the DTOs they build from `common::seed`.

## In scope

Everything in Design A–E: the `common::seed` module, the
`deserialize_rendered_html` move, the strangler relocation with full repoint +
shim deletion, and the `csr` boot import update.

## Out of scope / non-goals

- **No DTO shape or wire-form change** — pure relocation; serialization is
  byte-identical (the projector seed and every `#[server]` response are
  unchanged).
- **Not the render _functions_** — `render_head`/`render_body`/etc. dissolve
  onto verticals under #312; #610 moves only the data types.
- **Not the server-side query/assembly fns** (Design E stays put).
- No other `web` DTOs beyond the five named.
- `UploadResponse` already lives in `common::media` (landed in #517) — not
  touched.

## Acceptance criteria

1. **AC1 — `common::seed` exists.** `common/src/seed.rs` defines `PageSeed`,
   `TimelinePage`, `TimelinePostSummary`, `PostResponse`, `TagSummary` (ungated,
   pure `Serialize`/`Deserialize`, derives + serde attrs unchanged);
   `common::lib.rs` exposes `pub mod seed;`. `deserialize_rendered_html` lives
   in `common::render`.
2. **AC2 — no wire change.** The five DTOs' fields, derives, and `#[serde(...)]`
   attributes are byte-for-byte the same; the `#jaunder-seed` blob and all
   `#[server]` response bodies serialize identically. No `common` dependency
   added.
3. **AC3 — fully repointed, shims gone.** Every consumer (projector,
   `web::render` renderers, `csr` boot, `#[server]` fns, wasm components,
   `web::posts`/`web::tags` hubs, server integration tests) imports the types
   from `common::seed`; the transitional `web::…` re-export shims for the moved
   types are deleted (a search finds no `web::posts`/ `web::tags`/`web::render`
   re-export of `PageSeed`/`PostResponse`/`TimelinePage`/
   `TimelinePostSummary`/`TagSummary`).
4. **AC4 — server logic stays.** `page_from_rows`, `fetch_*`,
   `timeline_post_summary`, `post_tags_to_summaries`, `post_response` remain in
   `web::posts`, importing their DTOs from `common::seed`.
5. **AC5 — `csr` boot.** `csr/src/lib.rs` deserializes `common::seed::PageSeed`.
6. **AC6 — the gate is green.** `cargo xtask validate` passes (static + clippy +
   coverage + e2e matrix, incl. `wasm-clippy`, `server-fn-registrar`,
   `rendered-html-from-trusted`); coverage is clean with no new
   `cov:ignore`/`crap:allow` and no regression. First paint is unchanged
   (byte-identical seed → the existing e2e matrix is the behavioral guard).
