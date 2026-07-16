# Plan — #459: `ContentHash` newtype for the media content hash

Spec:
[`docs/superpowers/specs/2026-07-16-issue-459-content-hash-newtype.md`](../specs/2026-07-16-issue-459-content-hash-newtype.md).
Read it for the _what/why_ (incl. the door-choice, trusted-read, ServeParams,
and ADR-0065 rationales, and the enumerated test surface); this plan is the
_how_.

## Review header

**Goal.** Make the media content hash a type: `common::media::ContentHash`, an
ADR-0063 default (fallible) `#[derive(StrNewtype)]` newtype whose `FromStr`
validates 64-lowercase-hex (reusing `is_valid_content_hash`) and whose trusted
`from_digest(impl Into<String>)` door (mirroring `TokenHash`, #458) serves the
compute and trusted-DB-read paths. Thread it **pervasively** through every
media-hash site — storage record/trait/impls/dialect/row-helper, the server
compute/serve/AtomPub paths, and the web wire-DTO/`#[server]`-arg/form path.
Behavior-preserving except two accepted defense-in-depth error paths.

**Scope — in:**

- `common::media` — `ContentHash`, `InvalidContentHash`, `from_digest`;
  `media_path`/`media_url` params → `&ContentHash`. `is_valid_content_hash`
  stays `&str` (the validation primitive).
- `common::test_support` — `parse_content_hash`.
- `storage` — `MediaRecord.sha256: ContentHash`; `MediaStorage` + `MediaDialect`
  signatures → `&ContentHash`; impls bind via `.as_ref()`;
  `media_record_from_row` trusted-wraps via `from_digest`.
- `server` — `media_manager` compute sites + `UploadMetadata`/`register_in_db`;
  `UploadResponse.sha256`; `validate_serve_params` parses to `ContentHash`
  (keeps `ServeParams.hash: String`); AtomPub compute + member-route parses.
- `web` — `MediaItem.sha256: ContentHash`;
  `#[server] delete_media(sha256: ContentHash)` (ADR-0065 typed arg); the
  hidden-field render.
- All test fixtures (enumerated in the spec): `PartialEq<String>` → `.as_ref()`,
  64-char hashes where they now decode, the auth-test keeps a **valid** hash.

**Scope — out:** `user_id` (#471), `filename`, `MediaSource`-as-wire-`String`
(`delete_media`'s `source` arg, `delete_media_row`'s `source: &str`); the
NOT-IN-SCOPE non-media hashes (feed/post/page ETags, backup integrity hashes,
session `TokenHash`); no schema migration; no new ADR; no `xtask` gate (not a
security newtype).

**Tasks:**

1. `ContentHash` type + `from_digest` + tests + compile-fail doctest +
   `parse_content_hash` (`common`, standalone-green).
2. Pervasive threading across `common` helpers + `storage` + `server` + `web` +
   every test fixture — **one atomic cross-crate commit** (the
   `MediaRecord.sha256` and `media_path`/`media_url` changes cascade through all
   three crates at once).
3. Verify the `ActionForm` → typed-arg **form-urlencoded decode** path (unproven
   in-repo) with a `delete_media` integration test; then `cargo xtask check`
   (coverage incl. PostgreSQL).

Full `cargo xtask validate` (e2e) runs at ship (`jaunder-ship`).

**Key risks / decisions:**

- **Task 2 is atomic across crates.** `MediaRecord.sha256: ContentHash` breaks
  every reader (storage binds, `media_manager`, `UploadResponse`, `MediaItem`,
  tests) simultaneously; the whole-workspace gate only goes green once all sites
  move. Organize the edit in dependency order (common helpers → storage → server
  → web → tests) but commit once when `--all-features --all-targets` is green.
- **`PartialEq<String>` gap is a hard compile error, not a nicety.** The trailer
  gives `PartialEq<str>`/`<&str>` only. Every
  `assert_eq!(x.sha256, owned_string)` must go through `.as_ref()`. This is the
  easiest thing to miss.
- **The auth test must not degrade into a decode test.** `web_media.rs:61`
  proves the auth gate; give it a valid 64-char hash so decode passes and
  `require_auth` still fires. Only hash-rejection-intent tests switch to "assert
  non-OK".
- **Trusted-read, not validate-read.** `media_record_from_row` uses
  `ContentHash::from_digest(sha256)` (peer-consistent with `TokenHash` at
  `helpers.rs:93`); this also keeps the short-`"sha256"` row-helper test
  fixtures valid and adds no uncovered `Decode` branch.
- **ServeParams keeps `String`.** Typing the axum `Path` field would turn the
  documented DoS-safe 404-on-malformed into a pre-handler 400. Parse at the
  `validate_serve_params` chokepoint instead.
- **Form-decode is unproven.** No existing `#[server]` fn decodes a `common`
  newtype from `ActionForm`'s form-urlencoded POST. Task 3 verifies it
  explicitly rather than assuming the serde bridge round-trips through serde_qs.

**For agentic workers:** execute with `jaunder-iterate`, delegating the bulk
test sweep of Task 2 to a subagent via `jaunder-dispatch` when useful (keeps
file bulk out of the driver's context). Tick checkboxes in real time.

## Global constraints

- No `Co-Authored-By` trailer. Run `cargo xtask check` clean before each commit
  (`jaunder-commit`); serialize edit → gate → commit (no editing mid-gate).
- Storage tests follow the dual-backend template (`#[apply(backends)]`).
- Import discipline: import `ContentHash` / `parse_content_hash` at module tops;
  no fully-qualified `common::media::ContentHash` at call sites.
- Media-hash test fixtures use `common::test_support::parse_content_hash` (not
  inline `.parse().unwrap()` or a per-module helper).
- Verify web threading with `--all-features --all-targets` (default check skips
  `#[cfg(feature="server")]` web bodies, #397).

---

## Task 1 — `ContentHash` type (`common`), standalone-green

**Files:** `common/src/media.rs`, `common/src/test_support.rs`.

**How.** In `common/src/media.rs`, add (imports `std::str::FromStr`,
`macros::StrNewtype`, `thiserror::Error`):

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct ContentHash(String);

#[derive(Debug, Error)]
#[error("content hash must be 64 lowercase hex characters ([0-9a-f]{{64}})")]
pub struct InvalidContentHash;

impl FromStr for ContentHash {
    type Err = InvalidContentHash;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if is_valid_content_hash(s) { Ok(ContentHash(s.to_owned())) }
        else { Err(InvalidContentHash) }
    }
}

impl ContentHash {
    /// Trusted door for a hash we produced (a `format!("{digest:x}")` string or a
    /// value from our own `sha256` column). Untrusted input uses `FromStr`.
    #[must_use]
    pub fn from_digest(digest: impl Into<String>) -> Self { ContentHash(digest.into()) }
}
```

Document the type + `is_valid_content_hash` is now the constructor's engine. Add
`parse_content_hash(s: &str) -> ContentHash` to `test_support.rs` (import
`crate::media::ContentHash`; the module already has
`#![expect(clippy::expect_used)]`).

**Tests (`common/src/media.rs` `mod tests`, mirroring `email.rs`).** Reuse the
existing `is_valid_content_hash` vectors: valid 64-hex parses; rejects
short/long/ uppercase/non-hex/off-boundary; `Display` returns the canonical
string; serde serializes as a plain JSON string and `Deserialize` validates
(rejects a bad hash on the wire); `from_digest` trusted-wraps without
validating; `PartialEq<str>` holds. **Include an `InvalidContentHash` `Display`
test** (`"bad".parse::<ContentHash>().unwrap_err().to_string()` contains the
message) — without it the derive-generated `#[error]` `Display` is an uncovered
region and Task 1's own coverage gate fails (mirrors `email.rs:94-100`). Add a
`compile_fail` doctest proving an arbitrary `String` cannot be passed where
`ContentHash` is expected and that `ContentHash`/`TokenHash` don't interconvert
(mirror `common/src/token.rs`'s `compile_fail` blocks).

**Run:** `cargo nextest run -p common media` (and the doctest via
`cargo test -p common --doc`) → green. `cargo xtask check --no-test` +
`cargo nextest run -p common` → commit
(`feat(common): add ContentHash media-hash newtype (#459)`).

## Task 2 — Thread `ContentHash` pervasively (one atomic commit)

**Files:** `common/src/media.rs`; `storage/src/media.rs`,
`storage/src/{sqlite,postgres}/media.rs`, `storage/src/helpers.rs`;
`server/src/media.rs`, `server/src/media_manager.rs`,
`server/src/atompub/media.rs`; `web/src/media/mod.rs`, `web/src/pages/media.rs`;
and the test files enumerated in the spec's **Test surface** section.

**How — production, in dependency order:**

1. **`common`** — `media_path`/`media_url`: `sha256: &ContentHash` (slicing
   unchanged via `Deref`); update the module/fn docs (caller hands in a
   validated hash). `is_valid_content_hash` unchanged.
2. **`storage`** — `MediaRecord.sha256: ContentHash`;
   `MediaStorage::{get_media, delete_media, find_by_hash}` and
   `MediaDialect::delete_media_row` take `sha256: &ContentHash`; `create_media`
   bind `record.sha256.as_str()` → `.as_ref()` (**`.as_str()` is a hard error**
   — the trailer gives no inherent `as_str`), the query methods bind
   `sha256.as_ref()`; the two backend `delete_media_row` impls bind
   `sha256.as_ref()`; `media_record_from_row` →
   `sha256: ContentHash::from_digest(sha256)` (**add a `ContentHash` import** to
   `helpers.rs`, which currently imports only `TokenHash`).
   (`delete_media_row`'s `source: &str` unchanged.) SQL text and migrations
   unchanged.
3. **`server`** — `media_manager`: `UploadMetadata.sha256_hex: ContentHash`, the
   two compute sites `ContentHash::from_digest(format!("{digest:x}"))`,
   `stream_to_temp` returns `(ContentHash, i64)` (**`i64`**, not `u64` —
   `bytes_written`/`size_bytes` are `i64`, `media_manager.rs:341,358`),
   `register_in_db(sha256_hex: &ContentHash)`, `MediaRecord { sha256: … }` +
   `UploadResponse { sha256: … }` flow typed,
   `media_path("upload", &metadata.sha256_hex, …)`. `media.rs`:
   `UploadResponse.sha256: ContentHash`; `validate_serve_params` →
   `Result<(MediaSource, ContentHash), StatusCode>`, replacing the
   `is_valid_content_hash` check with
   `params.hash.parse::<ContentHash>() .map_err(|_| StatusCode::NOT_FOUND)?`,
   keeping the p1/p2 `starts_with` checks (on the parsed hash via `Deref`);
   **`resolve_media_path` returns `(MediaSource, ContentHash, PathBuf)`** and
   `serve_response` uses the typed hash for ETag/`find_by_hash`/path. Update the
   inline test `resolve_media_path_builds_path_for_valid_params` (~:365) to the
   3-tuple destructure. Keep `ServeParams.hash: String`. `atompub/media.rs`:
   idempotency compute → `ContentHash::from_digest(format!("{:x}", …))`;
   `member_get`/`member_delete` parse the `sha` path segment into `ContentHash`
   at entry, mapping a parse failure to the handler's existing not-found path
   (read the handler to confirm the exact status).
4. **`web`** — `MediaItem.sha256: ContentHash`; `list_my_media` maps `r.sha256`
   straight + `media_url(…, &r.sha256, …)`;
   `#[server] delete_media(sha256: ContentHash, …)` drops the body parse and
   uses `&sha256`; `render_media_row` uses `item.sha256.to_string()` for the
   hidden input `value`.

**How — tests (the enumerated surface; delegate the sweep if useful):**

- Replace media-hash literals that now decode with 64-char values built via
  `parse_content_hash`; `make_media_record` takes/returns a `ContentHash`.
- Fix `PartialEq<String>` comparisons — **site-specific, not a blanket
  `.as_ref()`** (see the spec's Test surface):
  - `server/tests/storage/mod.rs:6890,7184` — the `sha256` local becomes a
    `ContentHash` (it feeds `get_media`/`find_by_hash`), so keep
    `assert_eq!(fetched.sha256, sha256)` — `ContentHash == ContentHash`, **no
    `.as_ref()`** (which would not compile there).
  - `server/src/media_manager.rs:590` — `expected_sha` is a `format!` `String`;
    compare `&str`↔`&str`:
    `assert_eq!(first.sha256.as_ref(), expected_sha.as_str())`.
- Update `server/src/media.rs`'s inline `mod tests` (`resolve_media_path…`
  ~:365) for the 3-tuple destructure.
- **Auth test** (`server/tests/web/web_media.rs:61`): give the `delete_media`
  case a **valid 64-char** `sha256` so it still asserts `500`/`"unauthorized"`.
- `delete_media` success/reference tests (`:249,324`): valid 64-char hashes in
  **both** the `MediaRecord` fixture and the form body.
- Verify `backup_fixture.rs`/`media_handlers.rs` `SAMPLE_HASH`/`FIXTURE_*` are
  already 64-char (per the inventory) and route through `parse_content_hash`.

**Run:** `cargo xtask check` (or, while iterating,
`cargo check --all-features --all-targets` then `cargo nextest run`) → green →
commit
(`refactor(media): thread ContentHash newtype through storage/server/web (#459)`).

## Task 3 — Confirm the `ActionForm` decode path + full gate

**Files:** none new expected (see below).

**How.** The form-urlencoded decode of a typed `ContentHash` arg is **already
exercised** by `server/tests/web/web_media.rs`'s
`delete_media_succeeds_for_existing_item` (~:213), which posts
`sha256=…&filename=…&source=upload&force=false` via `post_form(...)` through the
real server-fn decode and asserts `OK`. Task 2 already upgrades that test's hash
to 64 chars — which makes it the decode proof, so **no new harness/test is
needed**; just confirm that test passes (it proves the `StrNewtype` `String` →
`FromStr` bridge, `str_newtype.rs:136-137`, round-trips through serde_qs for a
scalar field). **Only if** that test fails to decode: fall back to ADR-0065's
secret-exception shape (arg stays `String`, parse-in-body with client
pre-validation) and record the reason in the spec. Do not pre-emptively add a
duplicate decode test.

**Run:** `cargo xtask check` (static + clippy + coverage incl. PostgreSQL) →
green. Confirm the `xtask-done: … ok=true` sentinel.

## Definition of done

- `cargo xtask check` green; new `ContentHash` unit tests (incl. the
  error-`Display` test) + compile-fail doctest pass.
- **Type-anchored completeness** (a bare grep can't isolate the media hash from
  `MediaSource::as_str`/other newtypes' `.parse().unwrap()`):
  `MediaRecord.sha256`, `MediaItem.sha256`, `UploadResponse.sha256`,
  `UploadMetadata.sha256_hex` are `ContentHash`, and
  `get_media`/`delete_media`/`find_by_hash`/`delete_media_row` take
  `&ContentHash`. Once `MediaRecord.sha256: ContentHash`, **the compiler is the
  completeness gate** — any missed read/bind/call is a type error, not a lint.
  Spot-check leftover sub-64 literals with `rg 'sha256:\s*"[0-9a-f]{1,63}"'` and
  `rg 'sha256=[0-9a-f]{1,63}[&"]'` (form bodies).
- Every media-content-hash site uses `ContentHash` (no skipped location).
- `cargo xtask validate` (e2e, all four combos) green at ship.
