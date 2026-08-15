# #692/#1046 typed media boundaries and strict addresses Implementation Plan

> **For agentic workers:** Execute task-by-task with `jaunder-iterate`,
> delegating an individual task through `jaunder-dispatch` when useful. Tick
> each checkbox before its commit gate.

**Goal:** Make every post-intake media value typed and make malformed public
media addresses fail at extraction with HTTP 400.

**Architecture:** `ContentType` and `Filename` cross the storage seam, with
parsing/detection retained at the HTTP intake boundaries. The public
five-segment media route deserializes into one private validated address so the
handler sees no raw prefixes or optional parse state. The no-row serve fallback
remains file-first and derives a typed type from the filename.

**Tech Stack:** Rust; Axum extractors; `serde`; `cargo nextest`;
SQLite/PostgreSQL test matrix; `cargo xtask`.

## Review header

**Specification:**
[`2026-08-15-issue-692-media-value-types.md`](../specs/2026-08-15-issue-692-media-value-types.md)

**Scope**

- **In:** `ContentType`/`Filename` propagation through multipart, AtomPub, and
  storage; strict public-media route extraction; exact serve response
  preservation; served-metric removal; matching static gate and ADR projection.
- **Out:** projector and Syndication Feed `SoftPath` routes; filename
  encoding/storage layout; upload metric semantics; unrelated media route
  changes.

**Tasks**

1. Type the common/storage seam and parse its HTTP callers in one buildable
   cutover.
2. Introduce the strict public media-address extractor and its router tests.
3. Preserve response fallback/headers while removing served telemetry.
4. Align ADR, architecture projection, and static policy.

**Key decisions and risks**

- `ProfferedFilename` is accepted only in the strict extractor, then immediately
  rewrapped into canonical `Filename`; reusing it later double-encodes decoded
  Axum path input.
- Strict parsing changes only public-media malformed paths from 404 to 400;
  valid file absence remains 404 and an existing file without a row remains 200.
- `ContentType::from_trusted` remains only for fixed compile-owned values; every
  HTTP-supplied value is parsed before it reaches `MediaManager`.
- The front proxy owns HTTP outcome counts. Removing the in-process counter must
  not change `media_upload` or `media_upload_bytes`.

## Global constraints

- Follow ADR-0063 and `CONTRIBUTING.md`: external raw strings cross one
  validating boundary; existing storage-observing tests remain dual backend.
- Preserve the route layout `/media/{source}/{p1}/{p2}/{hash}/{filename}` and
  canonical encoded `Filename` storage/wire spelling per ADR-0084.
- Delete the obsolete soft-route validation, raw manager signatures, served
  metric, and broad static-gate allowance in the same cutover; no shims.
- Every task uses TDD: first run names the expected missing behavior/compile
  error; second run passes after implementation. Then run
  `devtool run -- cargo xtask check`, stage its mechanical changes, and commit
  without a `Co-Authored-By` trailer.

---

## File structure

| Responsibility          | Files                                                                                                                             |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Typed media values      | `common/src/media.rs`                                                                                                             |
| Upload persistence seam | `storage/src/media_manager.rs`                                                                                                    |
| HTTP intake             | `web/src/media/api.rs`, `server/src/atompub/media.rs`                                                                             |
| Public route            | `server/src/media.rs`                                                                                                             |
| Integration contracts   | `server/tests/web/web_media.rs`, `server/tests/atompub/atompub_media.rs`, `server/tests/misc/media_handlers.rs`                   |
| Metrics                 | `host/src/metrics.rs`                                                                                                             |
| Static policy           | `xtask/src/steps/proffered_filename_check.rs`                                                                                     |
| Decision projection     | `docs/adr/0140-strict-media-address-extraction.md`, `docs/adr/0084-media-filename-encoded-canonical.md`, `docs/ARCHITECTURE.md` |

## Task 1: Type the media seam and migrate HTTP callers

**Files:**

- Modify: `common/src/media.rs:913-966`
- Modify: `storage/src/media_manager.rs:76-164,373-426,536-549,786-890`
- Modify: `web/src/media/api.rs:244-281`
- Modify: `server/src/atompub/media.rs:68-119`
- Modify: `server/src/media.rs:181-195,322-349`
- Test: in-file common/storage/server tests plus `server/tests/web/web_media.rs`
  and `server/tests/atompub/atompub_media.rs` **Interfaces:**

- Produces `pub fn should_inline(content_type: &ContentType) -> bool` and
  `pub fn detect_content_type(filename: &Filename) -> ContentType`.
- Produces
  `pub async fn upload<S, E>(&self, user_id: UserId, filename: &Filename, content_type: Option<ContentType>, stream: S) -> anyhow::Result<UploadResponse> where S: Stream<Item = Result<Bytes, E>> + Unpin, E: std::error::Error + Send + Sync + 'static`.
- Produces
  `pub async fn upload_bytes(&self, user_id: UserId, filename: &Filename, content_type: ContentType, bytes: &[u8]) -> anyhow::Result<UploadResponse>`.
- Removes `MediaManager::get_content_type` and all `&str` content-type manager
  parameters. Multipart and AtomPub parse the raw values before calling these
  APIs, keeping the workspace buildable in the same cutover.

- [x] **Step 1: Write failing common/storage contract tests**

```rust
let filename: Filename = "photo.jpg".parse().unwrap();
assert_eq!(detect_content_type(&filename), "image/jpeg");
assert!(should_inline(&"image/png".parse::<ContentType>().unwrap()));
assert_eq!(
    manager
        .upload_bytes(user_id, &parse_filename("pic.png"), "image/png".parse().unwrap(), bytes)
        .await
        .unwrap()
        .content_type,
    "image/png"
);
```

Keep the existing streaming dual-backend test but pass
`Some("image/png".parse().unwrap())`; assert its response type is `image/png`.
Delete the old test of the removed raw-string parser.

- [x] **Step 2: Run the focused tests and verify RED**

Run: `devtool run -- cargo nextest run -p common -E 'test(media::tests)'`

Expected: FAIL because `detect_content_type` and `should_inline` still accept
`&str`.

Run:
`devtool run -- cargo nextest run -p storage -E 'test(media_manager::tests)'`

Expected: FAIL to compile because the manager still requires raw content-type
strings.

- [x] **Step 3: Implement the typed seam**

Change the two common function parameters to typed references; use
`filename.decoded()` only for extension extraction. Change manager public and
inner signatures to the listed interfaces, remove `get_content_type`, and store
the already supplied/detected `ContentType` in `UploadMetadata`. Migrate
`content_disposition` to typed inputs, decoding the filename only inside its
display-header construction. In multipart, parse a present MIME before moving
its field and detect an absent type from the typed filename. In AtomPub, parse a
present UTF-8 header, reject a non-UTF-8 or invalid header, and parse only the
fixed absent default. Do not parse or clone a typed content type merely to store
it.

- [x] **Step 4: Run focused tests and verify GREEN**

Run: `devtool run -- cargo nextest run -p common -E 'test(media::tests)'`

Expected: PASS.

Run:
`devtool run -- cargo nextest run -p storage -E 'test(media_manager::tests)'`

Expected: PASS for SQLite and PostgreSQL cases.

Run:
`devtool run -- cargo nextest run -p jaunder -E 'test(web_media::) + test(atompub_media::)'`

Expected: PASS for both backends.

- [x] **Step 5: Gate and commit**

Run: `devtool run -- cargo xtask check`

Expected: PASS.

```bash
git add common/src/media.rs storage/src/media_manager.rs web/src/media/api.rs server/src/atompub/media.rs server/src/media.rs server/tests/web/web_media.rs server/tests/atompub/atompub_media.rs
git commit -m "refactor(media): type intake and storage media values"
```

## Task 2: Extract a strict public media address

**Files:**

- Modify: `server/src/media.rs:1-21,103-223,260-325,374-579`
- Modify: `server/tests/misc/media_handlers.rs:724-762`
- Modify: `xtask/src/steps/proffered_filename_check.rs:1-401`

**Interfaces:**

- Produces private
  `ServeAddress { source: MediaSource, hash: ContentHash, filename: Filename }`
  and deserializes it from all five route segments.
- `serve_handler(media: Extension<Arc<dyn MediaStorage>>, storage_path: Extension<Arc<PathBuf>>, Path(address): Path<ServeAddress>, req_headers: HeaderMap) -> Result<Response, StatusCode>`
  and
  `serve_response(media: Extension<Arc<dyn MediaStorage>>, storage_path: Extension<Arc<PathBuf>>, address: ServeAddress, req_headers: HeaderMap) -> Result<Response, StatusCode>`
  consume only this value.
- Removes `ServeParams`, `validate_serve_params`, and `SoftPath` from this
  public route. Task 3 consumes the typed `ServeAddress` response path.

- [x] **Step 1: Write failing router and gate tests**

Replace direct invalid-`ServeParams` tests with router `oneshot(Request)` calls.
Parameterize the malformed public URLs below and assert
`StatusCode::BAD_REQUEST`:

```text
/media/not-a-source/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/file.txt
/media/upload/e3/b0/a/file.txt
/media/upload/zz/zz/zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz/file.txt
/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/a%2Fb.txt
/media/upload/ff/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/file.txt
/media/upload/e3/ff/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/file.txt
```

Add a valid never-materialized URL case asserting `NOT_FOUND`. Update
static-gate fixtures so `Path<ServeAddress>` is allowed,
`SoftPath<ProfferedFilename>` is rejected, and bare `ProfferedFilename`
fields/parameters/returns remain rejected.

- [x] **Step 2: Run focused tests and verify RED**

Run: `devtool run -- cargo nextest run -p jaunder -E 'test(media_handlers::)'`

Expected: FAIL because malformed route segments still soft-parse to 404.

Run:
`devtool run -- cargo nextest run -p xtask -E 'test(proffered_filename_check::tests)'`

Expected: FAIL because the gate still names `SoftPath` as an allowed shape.

- [x] **Step 3: Implement strict deserialization and narrow the gate**

Deserialize every route segment strictly, verify `p1 == hash[..2]` and
`p2 == hash[2..4]` only after `ContentHash` parsing, and rewrap decoded
`ProfferedFilename` with `Filename::from`. Resolve the file path from that
address alone. Delete all obsolete optional parsing/revalidation helpers. Make
the gate recognize solely the strict extractor's `ProfferedFilename` position,
not a general `SoftPath` wrapper.

- [x] **Step 4: Run focused tests and verify GREEN**

Run: `devtool run -- cargo nextest run -p jaunder -E 'test(media_handlers::)'`

Expected: PASS for every malformed 400 and valid-absent 404 case on both
backends.

Run:
`devtool run -- cargo nextest run -p xtask -E 'test(proffered_filename_check::tests)'`

Expected: PASS.

- [x] **Step 5: Gate and commit**

Run: `devtool run -- cargo xtask check`

Expected: PASS.

```bash
git add server/src/media.rs server/tests/misc/media_handlers.rs xtask/src/steps/proffered_filename_check.rs
git commit -m "feat(media): strictly extract serve addresses"
```

## Task 3: Keep serve wire behavior and remove served telemetry

**Files:**

- Modify: `server/src/media.rs:119-223,374-754`
- Modify: `server/tests/misc/media_handlers.rs`
- Modify: `host/src/metrics.rs:30-110,170-190,270-300,480-500`

**Interfaces:**

- Consumes Task 2's `ServeAddress`; `serve_response` retains
  `Result<Response, StatusCode>` for file/DB failures.
- Removes `ServeResult`, `media_served`, `Instruments::media_served`, and
  `serve_result`.
- Produces no replacement serve metric; upload metric emitters remain unchanged.

- [x] **Step 1: Write failing response/metric tests**

Add a dual-backend router test that creates only the on-disk file at a valid
address (no database row) and asserts `OK`, body bytes `file-bytes`,
`Content-Type: image/jpeg`,
`Cache-Control: public, max-age=31536000, immutable`, the SHA ETag, and a
`Content-Disposition` with decoded `filename="my photo.jpg"` plus its RFC 5987
spelling. Request the file through the canonical URL segment `my%20photo.jpg`
and assert it succeeds. Delete the metrics inventory/assertions that call the
obsolete served emitter so compilation is red until its implementation
disappears.

- [x] **Step 2: Run focused tests and verify RED**

Run: `devtool run -- cargo nextest run -p jaunder -E 'test(media_handlers::)'`

Expected: FAIL until the no-row test uses the typed filename path correctly.

Run: `devtool run -- cargo nextest run -p host -E 'test(metrics::tests)'`

Expected: FAIL because `ServeResult`/`media_served` are still present in the
test inventory.

- [x] **Step 3: Preserve response path; delete telemetry**

Keep the existing file-open-before-row-lookup order and derive fallback content
type with `detect_content_type(&address.filename)`. Change `content_disposition`
to take `&ContentType` and `&Filename`, decoding only inside it; pass typed
content type to `should_inline`. Remove the handler wrapper and all
served-counter declarations, emission, inventory calls, and enum string tests;
retain all upload instrumentation.

- [x] **Step 4: Run focused tests and verify GREEN**

Run: `devtool run -- cargo nextest run -p jaunder -E 'test(media_handlers::)'`

Expected: PASS for both backends, including exact fallback headers/body.

Run: `devtool run -- cargo nextest run -p host -E 'test(metrics::tests)'`

Expected: PASS.

- [x] **Step 5: Gate and commit**

Run: `devtool run -- cargo xtask check`

Expected: PASS.

```bash
git add server/src/media.rs server/tests/misc/media_handlers.rs host/src/metrics.rs
git commit -m "refactor(media): remove served outcome metric"
```

## Task 4: Project the accepted media-boundary decision

**Files:**

- Modify: `docs/adr/0140-strict-media-address-extraction.md`
- Modify: `docs/adr/0084-media-filename-encoded-canonical.md`
- Modify: `docs/ARCHITECTURE.md`
- Test: implementation and `xtask` checks from Tasks 3–4 are the executable
  policy proof

**Interfaces:**

- Records the route-specific supersession of #504 without changing its projector
  or Syndication Feed policy.
- Amends only ADR-0084's permitted `ProfferedFilename` extractor/gate shape;
  preserves canonical encoded filename storage/wire rules.

- [x] **Step 1: Write the decision/projection assertions as review criteria**

Verify the draft says malformed source/hash/filename/prefix mismatch is 400,
valid missing file is 404, present no-row file is extension-derived 200, and the
front proxy owns HTTP status accounting. Verify ADR-0084 and architecture text
name strict extraction and no longer permit the old `SoftPath` shape or claim
`jaunder.media.served` exists.

- [x] **Step 2: Verify the pre-edit policy mismatch**

Run:
`devtool run -- cargo nextest run -p xtask -E 'test(proffered_filename_check::tests)'`

Expected: the Task 2 gate tests are the failing proof before the new extractor
policy is implemented; after Task 2 they are the executable proof for these
records.

- [x] **Step 3: Align draft, amendment, and architecture text**

Update only the stated policy/projection sections. Do not renumber or promote
the draft; promotion happens at shipping. If execution exposes a factual
mismatch, correct the approved specification's affected sentence in the same
change.

- [x] **Step 4: Format and verify GREEN**

Run:
`devtool run -- prettier -w docs/adr/0140-strict-media-address-extraction.md docs/adr/0084-media-filename-encoded-canonical.md docs/ARCHITECTURE.md`

Expected: PASS.

Run: `devtool run -- cargo xtask check`

Expected: PASS, including `proffered-filename-position`.

- [x] **Step 5: Commit the projection**

```bash
git add docs/adr/0140-strict-media-address-extraction.md docs/adr/0084-media-filename-encoded-canonical.md docs/ARCHITECTURE.md
git commit -m "docs(adr): record strict media address extraction"
```

## Final verification

- [ ] Run `devtool run -- cargo xtask validate`; expected PASS.
- [ ] Review the final diff against the approved specification: no public-media
      `SoftPath`, no raw post-intake manager content type, no served metric, and
      no missing backend parameterization.
- [ ] Leave every task checkbox ticked only after its commit and gate exist.
