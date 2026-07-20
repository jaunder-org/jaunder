# Spec — #321: converge the `media` vertical onto the file-level host/wasm split

**Status:** awaiting approval. **Parent:** #303 (umbrella), milestone #11.
**Decision record:** `docs/adr/0070-web-vertical-wasm-only-component-files.md`
(supersedes ADR-0056), with `docs/web-style-guide.md` §8 as the layout template.
**Retained principles:** ADR-0055 (no fake-value host stub; pure logic extracted
_before_ gating). **Coordinates with:** #517 (fate of the upload fetch glue)
and, downstream, #312 / #330 (which this issue does **not** touch).

## Problem

The `media` vertical is **half-converged**. The server half already moved, but
the UI still lives in the old wasm-only `pages/` home:

- `web/src/media/mod.rs` — the three `#[server]` fns (`list_my_media`,
  `media_usage`, `delete_media`) + their generated action structs (`ListMyMedia`
  / `MediaUsage` / `DeleteMedia`) + the wire DTOs (`MediaItem`,
  `MediaUsageData`, `DeleteMediaResult`). Already uses the single grouped
  `#[cfg(feature = "server")] use { … };` support block. Ungated
  `pub mod media;` at `web/src/lib.rs:32`.
- `web/src/pages/media.rs` — the `MediaPage` `#[component]` (usage summary +
  media list + per-row delete `ActionForm`) and the private `render_media_row`
  helper. The old wasm-only `pages/` home.
- `web/src/pages/upload.rs` — the upload widgets `MediaUploadButton`
  (`on_uploaded` / `on_error` callbacks, `web_sys::FormData` builder) and
  `MediaPanel` (button + uploaded-URL display + error), plus the private
  `upload_file(FormData) -> Result<String, String>` browser fetch glue that
  POSTs to the hand-rolled axum route `/media/upload` (`server/src/media.rs:27`)
  and inlines a pure response-JSON `url` extraction at `:178-184`.

The UI and the server fns of one feature live in separate homes — the split
ADR-0070 §Decision eliminates — and `mod.rs` mixes wiring with the endpoints and
DTOs that ADR-0070 (amended #530) says belong in `api.rs`.

## Decisions (interview-resolved)

1. **Four-file layout per ADR-0070.** Media ends as
   `web/src/media/{mod.rs, api.rs, server.rs?, component.rs}`:
   - `api.rs` ← the three `#[server]` fns + the wire DTOs + the one grouped
     `#[cfg(feature = "server")]` support block (moved verbatim out of
     `mod.rs`).
   - `server.rs` ← host-only support **only if** a genuine host helper peels out
     of the server-fn bodies. The current bodies call `storage::` and
     `crate::auth::require_auth` directly with little private logic; if nothing
     non-trivial peels out, **no `server.rs` is created** (don't manufacture an
     empty file). Decided during implementation, recorded in the plan.
   - `component.rs` ← `MediaPage` + the consolidated upload widget (Decision 3),
     declared `#[cfg(target_arch = "wasm32")] mod component;`. **Zero cfg gates
     inside the file**; it calls browser code / `client::` directly and does
     **not** host-compile (not dead-but-exempt; no `cov:ignore`, no
     `#[component]`-exemption reliance).
   - `mod.rs` → **wiring only**: `mod api;`, gated `mod component;` (and gated
     `mod server;` if created), plus re-exports preserving the stable
     `web::media::{…}` paths — `pub use api::{…}` and
     `#[cfg(target_arch = "wasm32")] pub use component::{…}`.

2. **Upload glue — RESOLVED to Option B (2026-07-19 spike).** _The spike found
   Option A structurally out of scope: the upload engine `MediaManager` lives in
   the `server` crate and is shared with AtomPub, and `server` depends on `web`,
   so a `#[server]` fn in `web` cannot reach it without hoisting `MediaManager`
   into a new web-reachable crate + rewiring AtomPub. Maintainer confirmed
   Option B; the multipart migration is a filed follow-up._ The genuine
   wasm-only fetch glue is the decision ADR-0056 assigned to this vertical.
   Target end state: convert `/media/upload` into a leptos `#[server]` fn
   accepting multipart form data, living in `media/api.rs` beside the other
   three media endpoints, and **delete** both the hand-rolled axum route
   (`server/src/media.rs`) and the `web_sys` fetch glue — the fully canonical,
   zero-`target_arch`-cfg shape. **The plan's first task is a small multipart
   spike** proving leptos server-fn multipart upload works cleanly end-to-end
   against `MediaStorage` (auth + quota + max-file-size enforcement preserved).
   **If the spike is not clean, fall back to Option B**: relocate the
   `upload_file` fetch glue verbatim into wasm-only `component.rs` (whole file
   is gated, so no internal cfg) and keep the axum route. The direction is
   settled by the spike outcome and recorded before the rest of the plan
   proceeds.

3. **Consolidate the upload widgets.** `MediaUploadButton` and `MediaPanel`
   overlap (both wrap a file picker; `MediaPanel` adds uploaded-URL/error
   display). Reconcile them into a single coherent upload widget in
   `component.rs`, migrating both call sites (`MediaPage` and `pages/ui.rs`'s
   compose form). Behavior preserved; the public surface re-exported from
   `mod.rs` is the consolidated widget.

4. **Extract the pure url-extraction sliver, host-tested and ungated.** The
   response-JSON `url` extraction (`pages/upload.rs:178-184`) is pure logic; per
   ADR-0055/0070 it is extracted into an **ungated, host-tested** fn (with
   `#[cfg(test)]` unit tests) _before_ the browser glue is gated — regardless of
   Option A vs B. Under Option A this parses the `#[server]` fn's typed return
   so the sliver may dissolve into the typed boundary; under Option B it stays a
   standalone pure fn the glue calls. Either way the pure logic keeps a
   host-compiled, coverage-measured home and no fake host stub is introduced.

5. **Registrar/path stability.** The
   `web::media::{ListMyMedia, MediaUsage, DeleteMedia}` paths that
   `server/tests/helpers/mod.rs:70-72` register, and the DTO/UI paths external
   code imports, are preserved via `mod.rs` re-exports. Any new upload
   `#[server]` fn (Option A) is registered there too.

## Target end state (acceptance floor)

1. `media`'s UI, `#[server]` fns, and wire types live under `web/src/media/`;
   **no `web/src/pages/media.rs` and no `web/src/pages/upload.rs` remain**, and
   their `pages/mod.rs` declarations (`pub mod media;`, `pub mod upload;`, the
   `pub use upload::{…}` re-export, the `use …::media::MediaPage`) and the
   `<Route path="media">` are updated/removed accordingly; `pages/ui.rs`'s
   upload import repoints to `crate::media::…`.
2. The `#[component]` UI lives in **wasm-only `component.rs`**, declared
   `#[cfg(target_arch = "wasm32")] mod component;` on the `mod` line only —
   **zero cfg gates inside the file**; the components **do not host-compile**
   and are **not** dead-but-exempt. No `cov:ignore` / `#[component]`-exemption
   is added to satisfy host compilation of UI.
3. `mod.rs` is **wiring only** (ADR-0070 amended #530): no `#[server]` fns,
   DTOs, or `#[component]`s of its own — only `mod` declarations + re-exports.
   The `#[server]` endpoints + wire DTOs live in `api.rs`.
4. `target_arch = "wasm32"` appears in the vertical **only on `mod`
   declarations** and their paired `pub use`, never on an item inside a leaf
   file.
5. The client/server split of the `#[server]` bodies is expressed only via
   `feature = "server"` + the `#[server]` macro; any host-only support is
   `#[cfg(feature = "server")] mod server`.
6. **Upload path:** either (A) `/media/upload` is a leptos `#[server]` multipart
   fn in `api.rs`, the axum route + `web_sys` glue are deleted, and no
   `target_arch` cfg remains for upload; **or** (B — fallback) the fetch glue
   lives in wasm-only `component.rs` with the axum route retained. The chosen
   half aligns with #517 (the other issue lands nothing conflicting).
7. Pure, host-testable logic (the url sliver, any signal/form-state helpers)
   stays in **ungated, host-tested** files, extracted _before_ any gate
   (ADR-0070 §6); **no fake-value host stub** is introduced (ADR-0055).
8. The two upload widgets are consolidated into one (Decision 3); both former
   call sites use it; behavior unchanged.
9. `cargo xtask validate` green, including the media e2e flows (upload, list,
   usage summary, delete — including the delete-referenced-in-posts path).

## Shape of the work

- **Multipart spike (first, gating the direction).** Prove/​disprove Option A.
- **Split `mod.rs` → `api.rs` + wiring.** Move the 3 `#[server]` fns, DTOs, and
  the grouped support block into `api.rs`; reduce `mod.rs` to declarations +
  re-exports; add the ADR-0070-style `//!` module doc.
- **Re-home the UI into `component.rs`.** `MediaPage` (+ `render_media_row`)
  from `pages/media.rs`; the consolidated upload widget from `pages/upload.rs`.
  Bodies move essentially unchanged (cut-paste-and-gate), then consolidate the
  widgets.
- **Upload glue** per the spike outcome (A: multipart `#[server]` + delete axum
  route/glue; B: relocate glue into `component.rs`). Extract the pure url sliver
  host-tested either way.
- **Rewire.** Remove `pages/media.rs` + `pages/upload.rs`; fix `pages/mod.rs`
  declarations, the router `use`/`<Route>`, and `pages/ui.rs`'s import; register
  any new upload `#[server]` fn in `server/tests/helpers/mod.rs`.
- **Decision record.** If Option A lands (deletes a route + changes the upload
  wire protocol, settling ADR-0056's open question for media), record it via
  `jaunder-adr` (draft-out-of-git). If Option B, a spec/plan note suffices — no
  ADR. Decided by the spike.

## Out of scope

- Moving `App`/Router out of `pages/mod.rs` to the app entry — that is **#330**.
- Dissolving `pages/ui.rs` / `web::render` — that is **#312**. This issue only
  stops _importing_ media's UI from `pages/` and repoints the one `pages/ui.rs`
  upload import; it does not remove the `pages/mod.rs` shim wholesale.
- Any change to `server/src/media.rs`'s `/media/proxy` route or storage/quota
  semantics beyond what the upload-glue decision forces.
- Other verticals' convergence (#317–#329) and the shared-UI omnibus.

## Verification

`cargo xtask validate` (static + wasm-clippy + coverage + full e2e matrix). The
load-bearing behavioral checks are the media e2e flows: upload a file, see it in
the list + usage summary, delete it (incl. the referenced-in-posts branch).
Because the UI is now wasm-only, **`wasm-clippy` is load-bearing gate surface**
for this vertical's UI type-checking (ADR-0070 §Consequences), not just host
clippy. Under Option A, the multipart `#[server]` upload flow is exercised by
the same upload e2e.
