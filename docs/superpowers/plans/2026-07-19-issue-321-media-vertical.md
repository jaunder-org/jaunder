# Plan — #321: converge the `media` vertical onto the file-level host/wasm split

**Spec:** `docs/superpowers/specs/2026-07-19-issue-321-media-vertical.md` (the
"what/why"; this plan is the "how"). **ADR:**
`docs/adr/0070-web-vertical-wasm-only-component-files.md`; layout template
`docs/web-style-guide.md` §8. **For agentic workers:** drive with
**`jaunder-iterate`**, delegating a task to a subagent via
**`jaunder-dispatch`** when useful; tick checkboxes in real time. Gate each
commit with **`jaunder-commit`**.

## Review header

**Goal.** Finish converging `media` (server fns already live; UI still in
`pages/`) into the ADR-0070 four-file layout —
`media/{mod.rs (wiring), api.rs (#[server] + DTOs), server.rs (only if a real host helper peels out), component.rs (wasm-only UI)}`
— settling the upload-glue question and consolidating the two upload widgets.

**Scope.**

- _In:_ split `mod.rs`→`api.rs`; move `MediaPage` + the (consolidated) upload
  widget into wasm-only `component.rs`; decide+land the upload glue (Option A
  multipart `#[server]`, spike-first, B fallback); extract the pure url sliver
  host-tested; delete `pages/media.rs` + `pages/upload.rs` and rewire; keep
  `web::media::…` registrar/DTO paths stable.
- _Out:_ `App`/Router relocation (#330); `pages/ui.rs`/`render` dissolution
  (#312); `/media/proxy` + storage/quota semantics; other verticals.

**Tasks.**

- [x] 1. Multipart spike — **decision gate** for Option A vs B. → **Option B**
     (see Spike outcome below).
- [ ] 2. Split `media/mod.rs` → `api.rs` + wiring-only `mod.rs` (paths stable).
- [ ] 3. Extract the pure response-`url` sliver into a host-tested ungated fn.
- [ ] 4. Create `media/component.rs`: move `MediaPage`; consolidate the two
     upload widgets into one; migrate both call sites.
- [ ] 5. Land the upload glue — **Option B**: relocate the `web_sys` glue into
     `component.rs` (axum route retained).
- [ ] 6. Delete `pages/media.rs` + `pages/upload.rs`; rewire `pages/mod.rs`, the
     router, `pages/ui.rs`, and the registrar.
- [ ] 7. Full `cargo xtask validate` (no ADR — Option B).

## Spike outcome (2026-07-19) — Option B (fallback)

Option A ("multipart `#[server]` in `api.rs`, delete the axum route") is **not
clean**, for a structural reason independent of leptos multipart mechanics: the
server-side upload engine is `server::media_manager::MediaManager` (~370 lines:
streaming, SHA-256 content-addressing, temp files, hard-link dedup, quota/size
enforcement, DB record, metrics), which lives in the **`server`** crate and is
**shared with AtomPub** (`server/src/atompub/media.rs:85` → `upload_bytes`). A
`#[server]` fn lives in **`web`**, but `server` depends on `web`
(`media_manager.rs:16`), so `web` cannot depend on `server`. Reaching the engine
from a web `#[server]` fn would require hoisting `MediaManager` (+
`UploadResponse`

- media metrics) into a new web-reachable crate and rewiring AtomPub — a
  cross-cutting refactor out of scope for this vertical convergence. **Decision
  (maintainer-confirmed): Option B.** The multipart-`#[server]` +
  `MediaManager`-hoist migration is filed as a separate follow-up issue. Task 5
  relocates the existing `web_sys` fetch glue into wasm-only `component.rs`; the
  axum route stays.

**Key risks / decisions.**

- **Multipart is first-of-kind** in this repo (leptos 0.8.2; no existing
  `MultipartData` usage). Task 1 de-risks it before any deletion; if not clean
  we take Option B and no server-crate churn happens.
- **Path stability is load-bearing:** `server/tests/helpers/mod.rs:70-72`
  registers `web::media::{ListMyMedia, MediaUsage, DeleteMedia}` by path; the
  `api.rs` split must re-export them from `mod.rs` unchanged (Task 2 verifies
  before moving on).
- **Widget consolidation** is a behavioral touch (not pure mechanical) — covered
  by the media e2e (upload → list → usage → delete).
- **wasm-clippy is gate surface** for the now-wasm-only UI (ADR-0070).

## Global constraints

- Rust, `cargo`. No `Co-Authored-By` trailer (repo policy).
- ADR-0070: `component.rs` is wasm-only **by its `mod` declaration** and carries
  **zero cfg gates inside**; components do **not** host-compile — no
  `cov:ignore`, no `#[component]`-exemption reliance. `target_arch = "wasm32"`
  appears only on `mod`/`pub use` lines. `mod.rs` is wiring only.
- ADR-0055: pure logic is extracted into ungated host-tested files _before_
  gating; **no fake-value host stub**.
- Verify default `check` skips server-gated web code — after any web threading
  change run `cargo xtask check` (which does the coverage build with
  `--all-features`), not a bare default `cargo check` (memory:
  default-check-skips-server-gated-web).
- The pre-commit hook runs full `cargo xtask check`; run it green _before_ each
  commit (`jaunder-commit`). Serialize edit→gate→commit; don't edit tracked
  files during a gated commit.
- Copy structure from the `audiences/` vertical (closest twin) and the `auth/`
  template.

---

## Task 1 — Multipart spike (decision gate: Option A vs B)

**Intent.** Prove or disprove that a leptos 0.8.2 `#[server]` fn can accept a
multipart file upload end-to-end against `MediaStorage`, preserving the auth +
quota + max-file-size enforcement the axum `upload_handler`
(`server/src/media.rs:61`) does today. Outcome selects the upload path for
Task 5.

**Approach.** Minimal real attempt (keep it if clean — it becomes Task 5A's
seed):

- Add a `#[server]` fn in `web/src/media/mod.rs` (temporary home; moves to
  `api.rs` in Task 2) using server_fn's multipart input codec, e.g.
  ```rust
  #[server(input = server_fn::codec::MultipartFormData)]
  pub async fn upload_media(data: server_fn::codec::MultipartData) -> WebResult<String> {
      // require_auth().await?; read the "file" field bytes + filename;
      // enforce quota/max-size via SiteConfigStorage keys as upload_handler does;
      // store via MediaStorage; return the "/media/…" url.
  }
  ```
  Reuse the exact quota/size/storage logic from `server/src/media.rs:61` (read
  it; factor shared logic rather than duplicate if clean).
- Wire the wasm side: a throwaway button that builds `web_sys::FormData` and
  calls the generated action / `upload_media`, or exercise via the e2e harness.

**Verify (the spike's pass condition):**

- `cargo xtask check` compiles host + wasm (`--all-features`); wasm-clippy
  clean.
- A real upload round-trips: either extend `end2end/tests/media.spec.ts` locally
  or drive the existing upload flow and confirm the file lands + the returned
  url is usable. Auth-required and over-quota/over-size are rejected.

**Decision & record.**

- **Clean → Option A.** Keep the fn; record "Option A confirmed" as a dated note
  at the top of this plan and in the spec's Decision 2; proceed. Task 5 hardens
  it.
- **Not clean** (multipart codec unsupported/awkward, hydration/CSR-only
  friction, or storage wiring can't be reached from a `#[server]` body cleanly)
  → **Option B.** Revert the spike fn, record "Option B (fallback)" + the
  blocking reason, and Task 5 relocates the existing `web_sys` glue into
  `component.rs`.

**Commit.** Spike code only if Option A (as the seed for Task 5); otherwise
revert and this task produces only the recorded decision (no commit).

---

## Task 2 — Split `media/mod.rs` → `api.rs` + wiring-only `mod.rs`

**Files.**

- New `web/src/media/api.rs`: move verbatim from `web/src/media/mod.rs` the wire
  DTOs (`MediaItem`, `MediaUsageData`, `DeleteMediaResult`), the three
  `#[server]` fns (`list_my_media`, `media_usage`, `delete_media`) + their
  generated action structs, and the single grouped
  `#[cfg(feature = "server")] use { … };` support block. Add an ADR-0070-style
  `//!` doc (mirror `web/src/auth/api.rs:1-4`).
- Rewrite `web/src/media/mod.rs` to **wiring only** (mirror
  `web/src/auth/mod.rs`): `//!` "wiring only (ADR-0070, amended #530)" doc,
  `mod api;`, and
  `pub use api::{MediaItem, MediaUsageData, DeleteMediaResult, ListMyMedia, MediaUsage, DeleteMedia, list_my_media, media_usage, delete_media};`.
  (Gated `mod component;` + `mod server;` lines land in Tasks 4/5.)

**Interfaces.** External paths
`web::media::{ListMyMedia, MediaUsage, DeleteMedia, MediaItem, MediaUsageData, DeleteMediaResult}`
and the fn paths must resolve unchanged (re-export). No behavior change.

**Test / Run.** No new test (pure move). Prove path stability:

- `cargo xtask check` green (host + wasm; `--all-features`).
- Confirm `server/tests/helpers/mod.rs:70-72` still compiles/registers
  (`web::media::ListMyMedia` etc.):
  `cargo nextest run -p jaunder --test integration` (or the media subset) builds
  and passes.

**Commit** (`jaunder-commit`):
`refactor(web): split media api.rs from mod.rs (wiring-only) (#321)`.

---

## Task 3 — Extract the pure response-`url` sliver, host-tested

**Files.** The response-JSON `url` extraction currently inline at
`web/src/pages/upload.rs:178-184` becomes an ungated, host-tested fn.

- Under **Option A** the typed `#[server]` return (`WebResult<String>`) already
  yields the url; the sliver dissolves into the typed boundary. If nothing pure
  remains to extract, record that and skip — do **not** manufacture a fn. (Still
  confirm no host stub is introduced.)
- Under **Option B**, add a pure fn, e.g. in `web/src/media/api.rs` (ungated) or
  a small pure leaf (mirror `auth/marker.rs` if a leaf is cleaner):
  ```rust
  pub(crate) fn extract_upload_url(body: &str) -> Result<String, String> {
      let parsed: serde_json::Value =
          serde_json::from_str(body).map_err(|_| "invalid JSON in response".to_string())?;
      parsed["url"].as_str().map(ToString::to_string)
          .ok_or_else(|| "response JSON missing 'url' field".to_string())
  }
  ```

**Test.** In-file `#[cfg(test)] mod tests`: valid `{"url":"/media/x"}` → `Ok`;
missing field → `Err`; non-JSON → `Err`.

- `cargo nextest run -p web extract_upload_url` — expect PASS.

**Commit** (Option B only):
`refactor(web): extract host-tested media upload url parse (#321)`. (Fold into
Task 5 if trivial.)

---

## Task 4 — `media/component.rs`: move `MediaPage`; consolidate the upload widget

**Files.**

- New `web/src/media/component.rs` (`//!` doc; **no cfg inside**): move
  `MediaPage`
  - `render_media_row` from `web/src/pages/media.rs`, and the upload widget(s)
    from `web/src/pages/upload.rs`. Repoint imports to `crate::media::{…}` (the
    api re-exports) and shared widgets to their `crate::<leaf>::` homes (e.g.
    `crate::topbar::Topbar`, `render::format_bytes`).
- `web/src/media/mod.rs`: add `#[cfg(target_arch = "wasm32")] mod component;`
  and
  `#[cfg(target_arch = "wasm32")] pub use component::{MediaPage, <UploadWidget>};`.
- **Consolidate** `MediaUploadButton` + `MediaPanel` into one widget (spec
  Decision 3): keep a single coherent component (a primitive picker with
  optional URL/error display, or one merged widget), and migrate **both** call
  sites — `MediaPage` and `web/src/pages/ui.rs:7` (the compose form). Preserve
  behavior (callback contract, uploaded-url display, error surface).

**Interfaces.** The consolidated widget is re-exported from `mod.rs` under the
wasm gate; `pages/ui.rs` imports it from `crate::media::…`.

**Test / Run.** Components are wasm-only (not host-tested). Type-check via
wasm-clippy: `cargo xtask check` (host + wasm, `--all-features`). Behavioral
proof is the media e2e in Task 7.

**Commit** (`jaunder-commit`):
`refactor(web): move media UI into wasm-only component.rs; consolidate upload widget (#321)`.

---

## Task 5 — Land the upload glue (per Task 1 outcome)

**Option A (multipart `#[server]`) — if the spike was clean:**

- Move the spike's `upload_media` `#[server]` fn into `web/src/media/api.rs`
  alongside the others; harden it (full quota/size/error parity with the old
  handler; `boundary!`/`require_auth` idiom like the sibling fns).
- Delete the axum route + handler: `server/src/media.rs` `/media/upload` route
  (`:27`) and `upload_handler` (`:61`) — keep `/media/proxy` untouched. Move or
  delete `server/tests/misc/media_handlers.rs`'s upload-handler tests; port
  their assertions (auth-required, over-quota, over-size, happy-path url) onto
  the `#[server]` fn's server-side integration test (mirror how
  `web::auth::login` is integration-tested; `cargo nextest run -p jaunder`).
- Register the new fn: add `web::media::UploadMedia` (generated struct name) to
  `server/tests/helpers/mod.rs`.
- `component.rs`: the widget calls the `upload_media` action/fn directly — **no
  `web_sys::fetch`, no `target_arch` cfg** remains for upload.

**Option B (relocate glue) — fallback:**

- Move `upload_file` (the `web_sys` fetch to `/media/upload`) verbatim into
  `web/src/media/component.rs` (whole file wasm-gated → drop the now-redundant
  `crap:allow`/cfg framing). The consolidated widget calls it +
  `extract_upload_url` (Task 3). Axum route + handler untouched.

**Test / Run.** `cargo xtask check` green; under A the ported server-fn tests
pass (`cargo nextest run -p jaunder`).

**Commit** (`jaunder-commit`): A →
`feat(web): media upload as multipart #[server] fn; drop axum route (#321)`; B →
`refactor(web): relocate media upload glue into component.rs (#321)`.

---

## Task 6 — Delete `pages/` media files and rewire

**Files.**

- Delete `web/src/pages/media.rs` and `web/src/pages/upload.rs`.
- `web/src/pages/mod.rs`: remove `pub mod media;` (`:5`), `pub mod upload;`
  (`:14`), `pub use upload::{MediaPanel, MediaUploadButton};` (`:16`), and the
  `use crate::pages::media::MediaPage;` (`:34`). The
  `<Route path="media" view=MediaPage/>` (`:143`) now imports `MediaPage` from
  `crate::media::MediaPage` — update the `use` (router lines otherwise
  unchanged, per spec out-of-scope for #330).
- `web/src/pages/ui.rs:7`: repoint the upload-widget import to
  `crate::media::<UploadWidget>`.
- Confirm no dangling references:
  `rg 'pages::(media|upload)|MediaUploadButton|MediaPanel'` returns only
  intended (updated) sites.

**Test / Run.** `cargo xtask check` green (host + wasm, `--all-features`);
registrar integration builds.

**Commit** (`jaunder-commit`):
`refactor(web): delete pages/media + pages/upload; rewire to media vertical (#321)`.

---

## Task 7 — Conditional ADR + full validate

**ADR (Option A only).** Landing multipart `#[server]` deletes a route and
changes the upload wire protocol, settling ADR-0056's open upload question for
media — record it via **`jaunder-adr`** (numberless draft in `docs/adr/drafts/`,
promoted at ship). Under Option B, a dated note in this plan + the spec
suffices; no ADR.

**Full gate.** `cargo xtask validate` (static + wasm-clippy + coverage + full
e2e matrix). Run foreground with a long timeout (memory: slow gates foreground).
The load-bearing behavioral checks are the media e2e
(`end2end/tests/media.spec.ts`): upload → appears in list + usage summary →
delete (incl. referenced-in-posts branch). Under Option A confirm the multipart
upload e2e is green across all `{sqlite,postgres}×{chromium,firefox}` combos.

**Done when:** spec's Target end state (acceptance floor) items 1–9 all hold and
`cargo xtask validate` is green.

## Self-review

- Every task ends green-gated and committed; no task leaves the tree broken.
- Task 1 gates the risky work before any deletion; Tasks 2–4 are
  direction-independent; Task 5 branches on the recorded decision.
- Registrar/path stability is verified in Task 2 (before the UI move) and
  re-checked in Task 6.
- No `mod.rs` items, no in-`component.rs` cfg, no host stub, no `cov:ignore` for
  UI (ADR-0070/0055) — enforced by the gate + spec floor.
