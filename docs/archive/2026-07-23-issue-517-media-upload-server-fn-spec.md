# Spec — Media upload as a multipart `#[server]` fn, via relocating `MediaManager` to `storage` (#517)

## Summary

`web`'s heaviest browser-API surface is the media upload glue: the client
(`web/src/media/component.rs`) assembles a `FormData` and does a raw browser
`fetch` POST to `/media/upload`, then parses the JSON `url`; the server
(`server/src/media.rs::upload_handler`) is a bespoke axum multipart handler that
streams the field to `server::MediaManager`. #517 resolves this to **Option 1 —
a multipart `#[server]` fn** (the "delete the bespoke handler + glue" branch).

**Enabling move:** a leptos `#[server]` fn must live in `web`, but
`MediaManager` (the streaming upload service) lives in `server`, and `web` does
**not** depend on `server` (the dependency is server→web; `MediaManager` even
imports `web::auth::AuthUser`). So Option 1 is unlocked by **relocating
`MediaManager` to `storage`** — where it belongs: its work (stream to a
content-addressed path, SHA-256, hard-link dedupe, quota, insert a
`MediaRecord`) is persistence, and its own deps are almost all `storage`'s.
`web` already depends on `storage`, so the `#[server]` fn can then construct
`storage::MediaManager` directly from context handles — the same way the sibling
media server fns get their storage — with **no DI trait seam**.

## Decision (resolved)

Option 1, realized by moving `MediaManager` to `storage`. The move is unblocked
by two mechanical decouplings verified against source: `MediaManager` reads only
`auth_user.user_id` (→ take `UserId`), and its streaming input is
`axum::…::Field` (→ generalize to a byte stream; `multer::Field`/axum's field
both `impl Stream`). The only genuinely server-bound piece is `map_error`
(`MediaError → StatusCode`), which stays at the HTTP boundary. Metrics move with
the service (`storage → host` already exists).

## Design

### A. Relocate `MediaManager` → `storage`

- Move `MediaManager`, its `MediaError` enum (made `pub` as
  `storage::…::MediaError`), and the streaming/finalize/dedupe logic from
  `server/src/media_manager.rs` into `storage` (a `storage` media-manager
  module). Its persistence deps
  (`sha2`/`tokio::fs`/`chrono`/`MediaStorage`/`SiteConfigStorage`/`MediaRecord`/`common`)
  are already present in `storage`.
- **Decouple auth:** `upload`/`upload_bytes` take `user_id: UserId` instead of
  `&AuthUser` (only `.user_id` is used). Removes the `web` coupling.
- **Generalize the stream:** `stream_to_temp` takes
  `impl Stream<Item = Result<Bytes, E>>` where
  `E: std::error::Error + Send + Sync + 'static` (satisfied by `multer::Error`;
  funnels into `anyhow` via `?`), driven by `futures_util::StreamExt::next`
  instead of `axum::…::Field` + `.chunk()`. `storage` gains a `bytes` dependency
  (add to the workspace; `multer`/axum already pull it transitively). Callers
  extract `file_name()`/`content_type()` off their `Field` **before** streaming
  and pass the validated `Filename`/`ContentType` + the byte stream; the pure
  `validate_filename`/`get_content_type`/`detect_content_type` helpers move with
  `MediaManager`.
- **Metrics move with the service, emitted exactly once.** `MediaManager`
  funnels every upload through a single outcome point that emits
  `host::metrics::media_upload*` on both success and failure (`storage → host`
  is allowed, no cycle); `upload_outcome` (`MediaError → UploadOutcome`) moves
  to `storage` with it. `map_error`'s current metric emission
  (`media_manager.rs:135`) is **removed** so neither path double-counts —
  `map_error` becomes a pure `MediaError → StatusCode` map. (Benign delta: a
  non-media `anyhow` error previously funneled through `map_error` no longer
  emits a spurious `media_upload(Error)`.)
- **Move the streaming tests** into `storage` using storage's own fixtures
  (`test_support` env / `seed_user`, mocks under `test-utils`), dropping the
  `AuthUser` test literals for a plain `UserId`.

### B. `map_error` stays at the boundary

`map_error` (`MediaError → StatusCode`) and its `test_map_error` test stay in
`server` (HTTP concern; `StatusCode` must not enter `storage`). `MediaError`
being `pub` lets the boundary `downcast_ref::<storage::…::MediaError>()`.
(`upload_outcome` and its test move to `storage` with `MediaManager` — it maps
to a metric outcome, not a status, and after the metric relocation `map_error`
no longer calls it.)

### C. `UploadResponse` → `common::media`

Move `UploadResponse` (currently `server/src/media.rs:45`, `Serialize`-only) to
`common::media`, deriving `Serialize + Deserialize` (the wire round-trip). It is
the `#[server]` fn's **return type**, so it must be nameable on the wasm client
build — where `storage` is **not** compiled (`storage` is a `server`-gated `web`
dep). `common` is ungated and reachable by `storage` + `web` (both targets) +
`server`, and every field is already a `common` type — so
`storage::MediaManager` returns `common::media::UploadResponse` directly,
`web`'s fn returns it, and AtomPub serializes it, with **no mapping layer**.
(`MediaError` stays in `storage` — it is only named inside the `server`-gated fn
body, so the wasm build never needs it.)

### D. The web `#[server]` fn

New `#[server(input = MultipartFormData)]` fn (e.g. `upload_media`) in
`web/src/media/api.rs`, `feature = "server"` body: authenticate via
`require_auth()`; `expect_context` the `MediaStorage`/`SiteConfigStorage`
handles; obtain `storage_path` (an `Arc<PathBuf>` that is only an axum
`Extension`, **not** in leptos context — a naive `expect_context` panics) via
`leptos_axum::extract::<Extension<Arc<PathBuf>>>()` **or** an added
`provide_context` in the `/api` `additional_context` closure
(`server/src/lib.rs`; if this route is chosen, the per-request closure must also
capture a clone of the `storage_path` extension, which it does not today);
construct `storage::MediaManager`; pull the file field from
`MultipartData::into_inner()` (`Option<multer::Multipart>`), extract its
metadata, call `upload(user_id, …, stream)`; return the `UploadResponse`,
mapping `storage::…::MediaError` → `WebError`. Its generated client stub
replaces `upload_file`.

### E. Delete the old glue; repoint AtomPub; fix e2e

- **Client:** delete `upload_file` (the `fetch`/`Response` block) and
  `extract_upload_url` from `web/src/media/`; `on_file_change` builds the
  `FormData` and calls `upload_media(form_data.into()).await`, reading
  `response.url`.
- **Server:** delete `upload_handler`, the `/media/upload` route, and the manual
  JSON assembly (the serve/proxy routes stay).
- **AtomPub:** repoint `server/src/atompub/media.rs` at `storage::MediaManager`,
  passing `auth_user.user_id`.
- **e2e:** rewrite the two `end2end/tests/media.spec.ts` tests that
  `page.request.post` the raw `/media/upload` (asserting 201 / 401) — the route
  is gone; drive the UI or the server-fn endpoint and assert the server-fn
  response / auth-rejection.

## In scope

Everything in Design A–E. Additionally:

- `storage` gains `bytes` (new workspace dep) and `uuid` (both currently absent
  from `storage`; `uuid` is used at `media_manager.rs:166`).
- **Enable `server_fn`'s `multipart` codec feature** (+ `multer`) for `web`'s
  `server` build — this is the repo's first multipart `#[server]` fn, so the
  feature is not yet on.
- **Sweep now-dead deps:** remove the unused `web-sys` features
  (`Request`/`RequestInit`/`RequestMode`/`Response`) from `web/Cargo.toml` once
  `upload_file` is deleted, and remove `uuid` from `server/Cargo.toml` (it
  leaves with `MediaManager`).
- **Test fixtures:** `storage`'s `test_support` gains the sqlite
  media/site-config/user fixtures the moved streaming tests need (or the tests
  adopt storage's `TestEnv`/`AppState` idiom — mind the ADR-0053
  whole-`TestEnv`-binding dual-backend hazard).

## Out of scope

- The content-addressing scheme, dedupe (hard-link), quota rules, and on-disk
  layout — preserved byte-for-byte; this is a relocation + transport swap.
- The media serve/proxy routes and the AtomPub _protocol_ handling (only the
  `MediaManager` construction call in AtomPub changes).
- The `FormData`/`File`/`HtmlInputElement` **assembly** in `component.rs` —
  inherent browser glue, stays (legit wasm-only under #526).
- `UploadResponse`'s eventual consolidation with the other wire DTOs
  (#610-style) — it lands in `storage` here; a later DTO sweep may relocate it.

## Acceptance criteria

Stated so ship-time conformance review can tell delivered from not.

1. **AC1 — `MediaManager` lives in `storage`, decoupled.**
   `server/src/media_manager.rs` is gone (or reduced to nothing);
   `MediaManager` + `MediaError` (`pub`) + the streaming/dedupe logic are in
   `storage`, and `UploadResponse` is in `common::media` (deriving
   `Serialize + Deserialize`). `MediaManager` no longer references `web`,
   `axum::…::Field`, or `axum::http::StatusCode`. `upload`/`upload_bytes` take
   `UserId`. A search confirms `storage` has no `web`/`server`/`axum`
   dependency.

2. **AC2 — streaming + limits preserved.** `MediaManager` still consumes the
   file as a chunked byte stream (never buffering the whole file), enforcing
   `max_file_size` mid-stream, content-addressing, hard-link dedupe, and quota —
   unchanged behavior. The streaming input is a generic
   `Stream<Item = Result<Bytes, _>>`.

3. **AC3 — the upload is a multipart `#[server]` fn.** `web/src/media/api.rs`
   defines a `#[server(input = MultipartFormData)]` fn that constructs
   `storage::MediaManager` from server-fn context (incl. `storage_path`) and
   streams the multipart field to it, returning `UploadResponse` and mapping
   `MediaError → WebError`. `web/src/media/` contains no
   `web_sys::Request`/`Response`/`RequestInit`/`fetch` (search returns no
   matches); `upload_file` and `extract_upload_url` are gone; the old
   `crap:allow` is gone.

4. **AC4 — the bespoke server handler + route are deleted.**
   `server/src/media.rs` no longer defines `upload_handler` or routes
   `POST /media/upload`; serve/proxy routes unchanged. `map_error`
   (`MediaError → StatusCode`) remains in `server` for the AtomPub boundary.

5. **AC5 — AtomPub still works.** `server/src/atompub/media.rs` constructs
   `storage::MediaManager` and calls `upload_bytes(user_id, …)`; the AtomPub
   media-upload e2e/behavior is unchanged.

6. **AC6 — auth preserved.** The `#[server]` fn authenticates via
   `require_auth()`; an unauthenticated upload is rejected (no anonymous
   upload), observable as the server fn's auth-error response (a serialized
   `WebError::Unauthorized`), not necessarily a bare HTTP 401. The rewritten
   unauthenticated-upload e2e asserts that rejection.

7. **AC7 — behavior preserved end-to-end.** Choosing a file in the UI uploads it
   and surfaces the returned URL, identical to today. The media e2e suite —
   including the two rewritten direct-POST tests — passes. Metric emission moves
   into `MediaManager` (emitted exactly once per upload, success or failure;
   `map_error` no longer emits) — verified by inspection, since no test asserts
   metric counts.

8. **AC8 — the gate is green.** `cargo xtask validate` (incl. the e2e matrix,
   `wasm-clippy`, and the `server-fn-registrar` / `rendered-html-from-trusted`
   guards) passes; coverage is clean with no new `cov:ignore`/`crap:allow` and
   no regression (the moved streaming tests keep `MediaManager` covered in its
   new home).

## Non-goals / explicitly not added

- No DI trait for the uploader — the `#[server]` fn constructs the concrete
  `storage::MediaManager`, mirroring how it already gets storage handles from
  context.
- No new upload _scenarios_ — only the two existing direct-POST e2e tests are
  adapted; no invented coverage. Identical user-observable behavior.
