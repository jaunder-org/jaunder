# `ETag` newtype (#634) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Spec:**
[`docs/superpowers/specs/2026-07-24-issue-634-etag-newtype.md`](../specs/2026-07-24-issue-634-etag-newtype.md)
— read it; this plan is the "how," the spec is the "what/why."

**Goal:** Introduce `common::etag::ETag`, a `StrNewtype` that owns the whole
`digest → 64-hex → "sha256-" → quotes` pipeline, and thread it through all five
ETag producers, the stored `FeedCacheRow.etag`, and the comparison sites.

**Architecture:** `ETag` is a `common` string-newtype (the
`ContentType`/`ContentHash` precedent). Three trusted producer doors —
`sha256_of(&[u8])`, `from_sha256([u8;32])`, `from_content_hash(&ContentHash)` —
all yield `"sha256-<64hex>"` and funnel into one private quote+prefix helper;
`from_sha256` reuses `ContentHash::from_digest` for the hex, `sha256_of` reuses
`from_sha256`. A validating `FromStr` (general RFC-7232 strong quoted tag) backs
the derived serde/sqlx bridges, so the stored column re-validates on read-back.

**Tech Stack:** Rust, `sha2` (already a `common` dep), `sqlx` (feature-gated
bridge from `#[derive(StrNewtype)]`), `thiserror`, `rstest` dual-backend storage
tests.

## Global Constraints

- **No `Co-Authored-By` trailer** on commits.
- **Per-commit gate:** the pre-commit hook runs the full `cargo xtask check`
  (fmt + clippy + Nix coverage/tests). Run `cargo xtask check` before each
  commit so it passes clean (**jaunder-commit**).
- **DB-test env:** storage/server tests that touch Postgres only pass under the
  env `cargo xtask check` sets up; bare `cargo nextest` ConnectionRefused's on
  the Postgres backend (project convention). For those, `cargo xtask check` is
  the authoritative verification; a targeted
  `cargo nextest run -p <crate> <filter>` still gives compile + the in-process
  SQLite path for fast feedback.
- **`#[must_use]`** on every door (returns `ETag`; matches
  `ContentHash::from_digest`).
- **The `sha256-` prefix and the `"` may appear in exactly one place** —
  `common/src/etag.rs`. No `format!("\"…\"")`, manual quote `push`, `sha256-`
  literal, or `{:x}` on any ETag path outside `etag.rs` (grep-checked in AC3).
- **No new ADR, no new xtask gate** (spec Decision 5).

**Value changes (intentional — no production cache clients):** `site.rs` and
`media.rs` ETags gain the `sha256-` prefix; `feed_etag` goes 32→64 hex.
Producers unaffected in value: `projector`, `atompub`.

---

## Review header

**Scope — in:** the `ETag` type + three doors + `FromStr`/`InvalidETag`;
`parse_etag` test helper; five producers re-expressed to hand bytes/digest/hash;
`FeedCacheRow.etag: ETag` with validating decode; the compile-forced
consumer/test edits.

**Scope — out:** weak ETags (`W/"…"`); a non-SHA-256 / opaque-string door;
changing which bytes each producer hashes; the `#[server]`/wire boundary; #417
and other milestone-#13 issues.

**Tasks:**

1. `ETag` type, three doors, `FromStr`/`InvalidETag`, `parse_etag` helper
   (`common`).
2. `site.rs` — delete `etag_for`, mint via `from_sha256([u8;32])`.
3. `projector/mod.rs` — mint via `sha256_of(body.as_bytes())`.
4. `media.rs` — mint via `from_content_hash(&hash)`, flip the comparison.
5. `atompub/posts.rs` — `etag_for -> ETag` via `sha256_of(&bytes)`.
6. Feed ETag vertical (atomic, cross-crate): `feed_etag -> ETag`,
   `FeedCacheRow.etag: ETag`, `regenerate`/`handlers` consumers, storage
   decode-error test, three integration fixtures.

**Key risks / decisions:**

- **Task 6 is one atomic cross-crate commit** (`common` → `storage` → `server`):
  typing `feed_etag`'s return and `FeedCacheRow.etag` are compile-coupled
  through `regenerate.rs`, so they cannot be split without leaving the workspace
  uncompilable at the gate.
- **Deleting `site::etag_for`** removes its two unit tests; the hex+quote
  behavior they pinned is re-pinned by the `ETag::from_sha256` door tests in
  Task 1.
- **Doors are trusted** (no `FromStr` round-trip) — structurally valid by
  construction; pinned by re-parse tests in Task 1.

---

## Task 1 — ✅ DONE (commit e0024b1b): `ETag` type, doors (`parse_etag` deferred to Task 6)

**Files:**

- Create: `common/src/etag.rs`
- Modify: `common/src/lib.rs` (add `pub mod etag;`, alphabetical with the other
  modules)
- (`parse_etag` in `common/src/test_support.rs` is deferred to Task 6 — see the
  note below.)

**Interfaces:**

- Consumes: `common::media::ContentHash` (has
  `pub fn from_digest([u8; 32]) -> Self` and `AsRef<str>` yielding 64-hex);
  `sha2::{Digest, Sha256}` (already a `common` dep).
- Produces (relied on by Tasks 2–6):
  - `common::etag::ETag` —
    `#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]`; trailer gives
    `Display`/`AsRef<str>`/`Borrow<str>`/`Deref<Target=str>`/
    `PartialEq<str>`/`PartialEq<&str>`/owned-`String` conversions + validating
    serde & (feature-gated) sqlx bridges.
  - `ETag::sha256_of(bytes: impl AsRef<[u8]>) -> ETag`
  - `ETag::from_sha256(digest: [u8; 32]) -> ETag`
  - `ETag::from_content_hash(hash: &common::media::ContentHash) -> ETag`
  - `common::etag::InvalidETag` (`#[derive(Debug, thiserror::Error)]`)
  - (`common::test_support::parse_etag` — added in Task 6, its first user)

- [ ] **Step 1: Write the failing tests** in `common/src/etag.rs`
      (`#[cfg(test)] mod tests`).

```rust
use super::*;
use crate::media::ContentHash;

// A realistic lowercase sha256 hex digest (of the empty input) and its ETag form.
const HASH64: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[test]
fn from_sha256_produces_quoted_prefixed_lowercase_hex() {
    // 32 zero bytes → 64 '0' hex, prefixed and quoted.
    assert_eq!(ETag::from_sha256([0u8; 32]), format!("\"sha256-{}\"", "0".repeat(64)));
}

#[test]
fn sha256_of_hashes_then_formats_and_equals_from_sha256() {
    // sha256_of(b"") == from_sha256(digest of "") == "sha256-<HASH64>"
    assert_eq!(ETag::sha256_of(b""), format!("\"sha256-{HASH64}\""));
    assert_eq!(ETag::sha256_of(b""), ETag::from_sha256(sha2::Sha256::digest(b"").into()));
}

#[test]
fn from_content_hash_prefixes_and_quotes_the_hex() {
    let h: ContentHash = HASH64.parse().unwrap();
    assert_eq!(ETag::from_content_hash(&h), format!("\"sha256-{HASH64}\""));
    // The three doors agree for the same digest.
    assert_eq!(ETag::from_content_hash(&h), ETag::sha256_of(b""));
}

#[test]
fn every_door_output_reparses_via_fromstr() {
    for e in [ETag::from_sha256([0xab; 32]), ETag::sha256_of(b"hello"),
              ETag::from_content_hash(&(HASH64.parse::<ContentHash>().unwrap()))] {
        assert!(e.as_ref().parse::<ETag>().is_ok(), "door output must re-parse: {e}");
    }
}

#[test]
fn fromstr_accepts_a_canonical_quoted_tag() {
    assert_eq!("\"sha256-abc\"".parse::<ETag>().unwrap(), "\"sha256-abc\"");
    assert_eq!("\"known-etag\"".parse::<ETag>().unwrap(), "\"known-etag\""); // non-sha256 body ok
}

#[test]
fn fromstr_rejects_malformed_and_weak_forms() {
    for bad in [
        "",              // empty string
        "unquoted",      // no quotes
        "\"",            // single quote
        "\"missing",     // no closing quote
        "abc\"",         // no opening quote
        "\"a\"b\"",      // interior double-quote (not etagc)
        "\"a b\"",       // interior space (not etagc)
        "\"a\tb\"",      // interior control byte
        "W/\"abc\"",     // weak validator
    ] {
        let err = bad.parse::<ETag>().unwrap_err();
        assert!(err.to_string().contains("quoted"), "msg for {bad:?}: {err}");
    }
}

#[test]
fn serde_serializes_as_the_raw_quoted_string_and_validates_on_deserialize() {
    let e = ETag::from_sha256([0u8; 32]);
    let json = serde_json::to_string(&e).unwrap();
    assert_eq!(json, format!("\"\\\"sha256-{}\\\"\"", "0".repeat(64)));
    assert_eq!(serde_json::from_str::<ETag>(&json).unwrap(), e);
    assert!(serde_json::from_str::<ETag>("\"unquoted\"").is_err());
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p common etag` Expected: FAIL — `common::etag` / `ETag`
not defined.

- [ ] **Step 3: Implement `common/src/etag.rs`** to the interface above.

Write the module-doc, the type, error, `FromStr`, and the three doors.
Signatures are fixed by the Interfaces block; each door's body and the validator
are pinned by the Step 1 tests (door outputs, the accept/reject matrix, serde).
Two bodies the tests can't fully pin are given in full:

```rust
//! HTTP `ETag` values: the RFC 7232 strong quoted-tag invariant, owned by a newtype.
//!
//! Every ETag in jaunder is SHA-256-based, so `ETag` owns the whole
//! `digest → 64-lowercase-hex → "sha256-" prefix → surrounding quotes` pipeline: producers
//! hand it content bytes, a precomputed digest, or a `ContentHash` and never spell hex or
//! quotes. `FromStr` (the one validating chokepoint, backing the derived serde/sqlx
//! bridges) validates the *general* strong quoted format, so a stored `feed_cache.etag`
//! re-validates on read-back. The rest of the ADR-0063 trailer is `#[derive(StrNewtype)]`.

use std::str::FromStr;

use macros::StrNewtype;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::media::ContentHash;

#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct ETag(String);

/// Error returned when a string is not a valid strong HTTP `ETag`.
#[derive(Debug, Error)]
#[error("ETag must be a double-quoted strong opaque-tag, e.g. \"sha256-<hex>\"")]
pub struct InvalidETag;

impl FromStr for ETag {
    type Err = InvalidETag;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if is_valid_etag(s) { Ok(ETag(s.to_owned())) } else { Err(InvalidETag) }
    }
}

/// RFC 7232 strong `opaque-tag`: `DQUOTE *etagc DQUOTE`, where `etagc` is `%x21`,
/// `%x23-7E`, or `%x80-FF` (excludes the `"` byte, controls, and space). A leading
/// `W/` (weak) fails the opening-`"` check and is rejected.
fn is_valid_etag(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2
        && b[0] == b'"'
        && b[b.len() - 1] == b'"'
        && b[1..b.len() - 1]
            .iter()
            .all(|&c| c == 0x21 || (0x23..=0x7e).contains(&c) || c >= 0x80)
}

impl ETag {
    /// Compute the ETag of `bytes`: `"sha256-<hex of Sha256::digest(bytes)>"`.
    #[must_use]
    pub fn sha256_of(bytes: impl AsRef<[u8]>) -> ETag {
        Self::from_sha256(Sha256::digest(bytes.as_ref()).into())
    }

    /// Build the ETag of a precomputed 32-byte SHA-256 digest. Reuses
    /// `ContentHash::from_digest` for the lowercase-hex encoding.
    #[must_use]
    pub fn from_sha256(digest: [u8; 32]) -> ETag {
        Self::from_content_hash(&ContentHash::from_digest(digest))
    }

    /// Build the ETag of an already-validated content hash: `"sha256-<hex>"`. The one
    /// place the `sha256-` prefix and the quotes are written.
    #[must_use]
    pub fn from_content_hash(hash: &ContentHash) -> ETag {
        let hex = hash.as_ref();
        let mut s = String::with_capacity(hex.len() + "\"sha256-\"".len());
        s.push('"');
        s.push_str("sha256-");
        s.push_str(hex);
        s.push('"');
        ETag(s)
    }
}
```

> **Note:** `parse_etag` is deferred to **Task 6** (where the fixtures first use
> it) — a test helper with no caller reads as uncovered and fails the coverage
> gate at commit.

- [ ] **Step 4: Register the module** — add `pub mod etag;` to
      `common/src/lib.rs`.

- [ ] **Step 5: Run the tests, verify they pass**

Run: `cargo nextest run -p common etag` Expected: PASS (7 tests).

- [ ] **Step 6: Commit**

```bash
git add common/src/etag.rs common/src/lib.rs
git commit -m "feat(common): ETag newtype owning the sha256 quoted-tag pipeline (#634)"
```

Run `cargo xtask check` first (clean).

---

## Task 2: `site.rs` producer + consumer

**Files:**

- Modify: `server/src/site.rs` — delete `etag_for` (110–120); `build_response`
  (158–183); tests (309–412).

**Interfaces:**

- Consumes: `common::etag::ETag`, `ETag::from_sha256([u8; 32])`.
- Produces: nothing new (internal).

- [ ] **Step 1: Update the failing tests** in `server/src/site.rs` `mod tests`.

- **Delete** `etag_is_quoted_hex_and_stable` (309–316) and
  `etag_differs_for_different_bytes` (318–321) — they test the deleted
  `etag_for`; the hex+quote behavior is now pinned by `ETag::from_sha256` in
  Task 1.
- **`build_response_identity_sets_type_body_and_no_encoding`** (357) and
  **`build_response_brotli_sets_content_encoding_and_logical_type`** (378):
  change the `sha256` arg from a byte slice to a `[u8; 32]` — `&[1, 2, 3]` →
  `[1u8; 32]`, `&[9]` → `[9u8; 32]`.
- **`build_response_304_empty_body_when_if_none_match_matches`** (396): rewrite
  to

```rust
#[tokio::test]
async fn build_response_304_empty_body_when_if_none_match_matches() {
    let sha = [0xabu8; 32];
    let etag = common::etag::ETag::from_sha256(sha);
    let resp = build_response(
        "pkg/jaunder.wasm",
        Bytes::from_static(b"ignored"),
        sha,
        Encoding::Br,
        Some(etag.as_ref()),
    );
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(resp.headers().get(header::ETAG).unwrap(), etag.as_ref());
    assert_eq!(resp.headers().get(header::VARY).unwrap(), "Accept-Encoding");
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert!(body.is_empty());
}
```

- **`not_modified_*` tests** (323–337): `not_modified`'s `etag` param becomes
  `&ETag` (spec §Consumers), so build the etag inline (arbitrary non-sha256 tags
  → via `FromStr`; `.unwrap()` is allowed in `#[test]`):

```rust
#[test]
fn not_modified_true_on_exact_match() {
    assert!(not_modified(Some("\"abc\""), &"\"abc\"".parse().unwrap()));
}
#[test]
fn not_modified_true_when_present_in_list() {
    assert!(not_modified(Some("\"other\", \"abc\""), &"\"abc\"".parse().unwrap()));
}
#[test]
fn not_modified_false_on_mismatch_or_absent() {
    let etag: common::etag::ETag = "\"abc\"".parse().unwrap();
    assert!(!not_modified(Some("\"xyz\""), &etag));
    assert!(!not_modified(None, &etag));
}
```

- `serve_site_*` (414–476) stay string-based → **unchanged**.

- [ ] **Step 2: Run, verify fail** — `cargo nextest run -p jaunder site`
      Expected: FAIL (build error: `etag_for` gone / arg type).

- [ ] **Step 3: Implement.**

- Delete `etag_for` (110–120) and its `use std::fmt::Write` if now unused.
- `build_response`: change the parameter `sha256: &[u8]` → `sha256: [u8; 32]`,
  and its body head:

```rust
let etag = common::etag::ETag::from_sha256(sha256);
let mut headers = HeaderMap::new();
headers.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
insert_etag(&mut headers, etag.as_ref());

if not_modified(if_none_match, &etag) {
    return (StatusCode::NOT_MODIFIED, headers).into_response();
}
```

- Caller `serve_site` (220):
  `build_response(&logical, body, hash, encoding, if_none_match)` — pass `hash`
  (`[u8; 32]` from `file.metadata.sha256_hash()`) directly, dropping
  `hash.as_ref()`.
- Change `not_modified` to take `etag: &ETag` (spec §Consumers): body
  `if_none_match.is_some_and(|inm| inm.split(',').any(|tag| *etag == tag.trim()))`
  (via the derived `ETag: PartialEq<&str>`). Leave `insert_etag(etag: &str)`
  unchanged — it takes the already-string header value, and `build_response`
  passes `etag.as_ref()`.

- [ ] **Step 4: Run, verify pass** — `cargo nextest run -p jaunder site` → PASS.

- [ ] **Step 5: Commit**

```bash
git add server/src/site.rs
git commit -m "refactor(site): mint the asset ETag via common::etag::ETag (#634)"
```

Run `cargo xtask check` first.

---

## Task 3: `projector/mod.rs` producer + consumer

**Files:**

- Modify: `server/src/projector/mod.rs` — `cacheable` (~105); any inline etag
  unit test.

**Interfaces:**

- Consumes: `ETag::sha256_of(bytes)`.
- Produces: nothing new.

- [ ] **Step 1: Update/confirm the tests.** The projector 304 tests live in
      `server/tests/projector/mod.rs` (`permalink_if_none_match_returns_304`
      sends the returned ETag back;
      `permalink_stale_if_none_match_serves_full_200` sends
      `"\"sha256-stale\""`). Projector's ETag value is **unchanged**
      (`"sha256-<64hex>"`), so no integration-test value edits. If a
      `cacheable`-local unit test asserts the etag string, update it to compare
      via the `str` view; otherwise none.

- [ ] **Step 2: Run, verify fail** (after Step 3 edit compiles) —
      `cargo nextest run -p jaunder projector`. Expected initially FAIL only if
      a unit test referenced the old inline `format!`; otherwise this task is a
      pure refactor verified by the unchanged 304 tests under
      `cargo xtask check`.

- [ ] **Step 3: Implement** in `cacheable` (105):

```rust
let etag = common::etag::ETag::sha256_of(body.as_bytes());
```

and the conditional (107–110):

```rust
if let Some(inm) = headers.get(header::IF_NONE_MATCH) {
    if inm.to_str().ok() == Some(etag.as_ref()) {
        return StatusCode::NOT_MODIFIED.into_response();
    }
}
```

and the response `ETAG` header insert uses `etag.as_ref()` (was `&etag`). Drop
the now-unused `use sha2::…` / `Sha256` import if nothing else in the file uses
it.

- [ ] **Step 4: Run, verify pass** — `cargo nextest run -p jaunder projector`,
      and the projector integration 304/stale-200 tests via `cargo xtask check`.

- [ ] **Step 5: Commit**

```bash
git add server/src/projector/mod.rs
git commit -m "refactor(projector): mint the page ETag via ETag::sha256_of (#634)"
```

Run `cargo xtask check` first.

---

## Task 4: `media.rs` producer + consumer

**Files:**

- Modify: `server/src/media.rs` — `serve_response` (109–163); tests (509–530).

**Interfaces:**

- Consumes: `ETag::from_content_hash(&ContentHash)` (`hash` from
  `resolve_media_path` is a `common::media::ContentHash`).
- Produces: nothing new.

- [ ] **Step 1: Update the failing test.**
      `serve_response_returns_304_on_matching_if_none_match` (510) sends the
      ETag the producer emits — now `sha256-`-prefixed. Derive it from the door
      so the test can't drift:

```rust
let etag = common::etag::ETag::from_content_hash(&common::test_support::parse_content_hash(SAMPLE_HASH));
let mut headers = axum::http::HeaderMap::new();
headers.insert(
    axum::http::header::IF_NONE_MATCH,
    HeaderValue::from_str(etag.as_ref()).unwrap(),
);
```

`serve_response_serves_body_when_if_none_match_does_not_match` (534) sends a
deliberate non-match (`"\"not-the-hash\""`) → still 200; **unchanged**.

- [ ] **Step 2: Run, verify fail** — `cargo nextest run -p jaunder media`
      Expected: FAIL (304 test: old bare-hash ETag no longer matches the
      prefixed producer value).

- [ ] **Step 3: Implement** in `serve_response`:

```rust
// ETag / If-None-Match check.
let etag = common::etag::ETag::from_content_hash(&hash);
if let Some(if_none_match) = req_headers.get(axum::http::header::IF_NONE_MATCH) {
    if etag == if_none_match.to_str().unwrap_or("") {   // flipped: ETag: PartialEq<&str>
        return Ok(StatusCode::NOT_MODIFIED.into_response());
    }
}
```

and the header insert (153–156):

```rust
headers.insert(
    axum::http::header::ETAG,
    HeaderValue::from_str(etag.as_ref()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
);
```

- [ ] **Step 4: Run, verify pass** — `cargo nextest run -p jaunder media` → PASS
      (both the 304 and the non-match tests use mock storage,
      `guard:no-backend`).

- [ ] **Step 5: Commit**

```bash
git add server/src/media.rs
git commit -m "refactor(media): mint the media ETag via ETag::from_content_hash (#634)"
```

Run `cargo xtask check` first.

---

## Task 5: `atompub/posts.rs` producer + consumer

**Files:**

- Modify: `server/src/atompub/posts.rs` — `etag_for` (75–100);
  `if_match_satisfied` (105–110); the `ETAG` header sites (271, 448, 543) and
  `if_match_satisfied` callers (326, 489); `mod etag_tests` (551–638).

**Interfaces:**

- Consumes: `ETag::sha256_of(&[u8])`.
- Produces: `etag_for(post: &PostRecord) -> ETag` (value-identical to today —
  `"sha256-<64hex>"`).

- [ ] **Step 1: Update the tests** in `mod etag_tests`. Value is unchanged, so
      assertions hold via `ETag`'s `Deref<str>`/`PartialEq`; adjust only where a
      method needs the `str` view:
  - `etag_for_is_quoted_sha256` (posts.rs:591) uses `.strip_prefix("\"sha256-")`
    / `.strip_suffix("\"")` — these resolve through `Deref<Target = str>` on
    `&ETag`, so they compile unchanged; if a binding needs an explicit `&str`,
    use `etag_for(&post).as_ref()`.
  - `etag_for_is_deterministic` / `_ignores_identity_and_timestamps` /
    `_changes_on_each_content_field` compare `etag_for(a) == etag_for(b)` —
    `ETag: PartialEq` holds; unchanged.

- [ ] **Step 2: Run, verify fail** — `cargo nextest run -p jaunder etag_tests`
      Expected: FAIL (return type / any `String`-only method).

- [ ] **Step 3: Implement.**

- `etag_for`: change return type to `ETag`; replace the tail
  `format!("\"sha256-{:x}\"", Sha256::digest(&bytes))` with:

```rust
common::etag::ETag::sha256_of(&bytes)
```

Drop the now-unused `Sha256`/`Digest` import if nothing else in the file uses
it.

- `if_match_satisfied(headers: &HeaderMap, etag: &ETag) -> bool` (spec
  §Consumers) — keep the `*` wildcard arm; body
  `if_match == "*" || *etag == if_match` (via `ETag: PartialEq<&str>`,
  `if_match: &str`). At its callers (326, 489), bind
  `let etag = etag_for(&post);` once and pass `&etag` here and
  `etag.to_string()` to the header tuple (below).
- The response `ETAG` header tuples (271, 448, 543) —
  `(header::ETAG, etag_for(&post))` sit in arrays alongside sibling `String`
  entries, so convert at the boundary:
  `(header::ETAG, etag_for(&post).to_string())`.

- [ ] **Step 4: Run, verify pass** — `cargo nextest run -p jaunder etag_tests`;
      the AtomPub If-Match 412/200 integration suite
      (`server/tests/atompub/atompub_posts.rs`) via `cargo xtask check`
      (value-identical, so no integration edits expected).

- [ ] **Step 5: Commit**

```bash
git add server/src/atompub/posts.rs
git commit -m "refactor(atompub): type the post ETag as common::etag::ETag (#634)"
```

Run `cargo xtask check` first.

---

## Task 6: Feed ETag vertical — `common` + `storage` + `server` (atomic)

This task must land as **one commit**: typing `feed_etag`'s return and
`FeedCacheRow.etag` are compile-coupled through `regenerate.rs`.

**Files:**

- Modify: `common/src/feed/metadata.rs` — `feed_etag` + its tests (~41,
  ~94–123).
- Modify: `storage/src/feed_cache.rs` — `FeedCacheRow.etag` (22), `CacheTuple`
  (42–49), `upsert` bind (132), `sample` fixture (167), + new decode-error test.
- Modify: `server/src/feed/handlers.rs` — the `If-None-Match` compare (62) +
  `ETAG` header (82–84).
- Modify: `server/tests/feed/feed_worker.rs` (etag fixture),
  `server/tests/storage/mod.rs` (etag fixture),
  `server/tests/feed/feed_handlers.rs` (three fixtures + the paired 304 header).
- (`server/src/feed/regenerate.rs` — compiles unchanged once the types line up.)

**Interfaces:**

- Consumes: `common::etag::ETag`, `ETag::from_sha256([u8; 32])`,
  `common::test_support::parse_etag`.
- Produces:
  `feed_etag(items: &[FeedItem], generated_at: DateTime<Utc>) -> ETag`;
  `storage::feed_cache::FeedCacheRow.etag: common::etag::ETag`.

- [ ] **Step 1: Update `feed_etag`'s tests** (`common/src/feed/metadata.rs`).
      They assert stability/inequality of `feed_etag(...)` outputs — hold
      unchanged under `ETag: PartialEq`. No hex-length assertion exists, so
      dropping the 16-byte truncation needs no test change. (If any test binds
      the result as `String`, change it to compare `ETag`s directly.)

- [ ] **Step 2: Write the storage decode-error test** in
      `storage/src/feed_cache.rs` `mod tests`, mirroring
      `get_surfaces_a_column_decode_error_for_a_malformed_content_type`
      (dual-backend, `#[apply(backends)]`):

```rust
#[apply(backends)]
#[tokio::test]
async fn get_surfaces_a_column_decode_error_for_a_malformed_etag(#[case] backend: Backend) {
    let env = backend.setup().await;
    env.state.feed_cache.upsert(sample("/feed.rss")).await.unwrap();
    // Tamper the stored etag to an unquoted value — only reachable via DB tampering; the
    // validating ETag `Decode` bridge rejects it on read-back.
    env.base
        .pool()
        .execute("UPDATE feed_cache SET etag = 'not-a-quoted-etag' WHERE feed_url = '/feed.rss'")
        .await
        .unwrap();
    let err = env.state.feed_cache.get(&fp("/feed.rss")).await.unwrap_err();
    assert!(
        matches!(err, FeedCacheError::Db(sqlx::Error::ColumnDecode { .. })),
        "expected a column-decode error, got: {err:?}"
    );
}
```

- [ ] **Step 3: Run, verify fail** — `cargo nextest run -p common feed` (compile
      fail: `feed_etag` still `String`) and note the storage test won't compile
      until Step 4. Expected: FAIL.

- [ ] **Step 4: Implement the vertical.**

- **`common/src/feed/metadata.rs`** — change `feed_etag`'s return type to
  `common::etag::ETag`; keep every `hasher.update(...)` call; replace the
  `digest.iter().take(16)…` + `format!("\"sha256-{hex}\"")` tail with:

```rust
common::etag::ETag::from_sha256(hasher.finalize().into())
```

Remove the now-unused `std::fmt::Write` import; keep `sha2::{Digest, Sha256}`.
**Update the `feed_etag` doc comment** (metadata.rs:~39) that reads
`Format: "sha256-<hex32>"` — it is now the full 64-hex digest (docs track the
API change).

- **`common/src/test_support.rs`** — add the `parse_etag` helper here (its first
  users are this task's fixtures, so it is covered from the moment it lands):
  add `use crate::etag::ETag;` to the imports (after `email`), and near
  `parse_content_hash`/`parse_content_type`:

```rust
/// Parse `s` into a valid [`ETag`] for tests.
///
/// # Panics
///
/// Panics if `s` is not a valid double-quoted strong `ETag`.
#[must_use]
pub fn parse_etag(s: &str) -> ETag {
    s.parse().expect("valid test ETag")
}
```

- **`storage/src/feed_cache.rs`** —
  - add `ETag` to the import:
    `use common::{etag::ETag, feed::FeedPath, media::ContentType};`
  - field (18–22): replace the `String` + carve-out comment with
    `/// The stored strong `ETag`(validated on decode via the`ETag` sqlx bridge, #634).`
    then `pub etag: ETag,`.
  - `CacheTuple` (42–49): 3rd element `String` → `ETag`.
  - `upsert` (132): `.bind(row.etag.as_str())` → `.bind(&row.etag)` (typed
    bind).
  - `sample` fixture (167): `etag: "\"sha256-deadbeef\"".into()` →
    `etag: common::test_support::parse_etag("\"sha256-deadbeef\"")` (add the
    import in `mod tests`; the crate already imports
    `common::test_support::parse_content_type`).
- **`server/src/feed/handlers.rs`** —
  - compare (62): `Some(row.etag.as_str())` → `Some(row.etag.as_ref())`.
  - header (82–84): `HeaderValue::from_str(&row.etag)` →
    `HeaderValue::from_str(row.etag.as_ref())`.
- **Test fixtures** (build via `common::test_support::parse_etag`, add the
  import per file):
  - `server/tests/feed/feed_worker.rs`: `etag: "etag".to_string()` →
    `etag: parse_etag("\"etag\"")`.
  - `server/tests/storage/mod.rs`: `etag: "etag".to_string()` →
    `etag: parse_etag("\"etag\"")`.
  - `server/tests/feed/feed_handlers.rs`: the three `FeedCacheRow` fixtures →
    `parse_etag("\"known-etag\"")`, `parse_etag("\"test-etag-123\"")`,
    `parse_etag("\"test-etag\"")`. In `handler_if_none_match_returns_304`, the
    `IF_NONE_MATCH` header (211) must send the **same quoted string** as the
    stored fixture — set both from one binding, e.g.
    `let etag = parse_etag("\"test-etag-123\"");` for the row and
    `.header(header::IF_NONE_MATCH, etag.as_ref())` for the request, so the 304
    comparison matches.

- [ ] **Step 5: Run, verify pass.** `cargo nextest run -p common feed` (PASS),
      then the full gate for the DB-backed storage/server tests:

Run: `cargo xtask check` Expected: PASS — including the new dual-backend
`…malformed_etag` decode test and the feed 304 integration tests.

- [ ] **Step 6: Commit**

```bash
git add common/src/test_support.rs common/src/feed/metadata.rs \
        storage/src/feed_cache.rs server/src/feed/handlers.rs \
        server/tests/feed/feed_worker.rs server/tests/storage/mod.rs server/tests/feed/feed_handlers.rs
git commit -m "refactor(feed,storage): type the feed ETag end-to-end as common::etag::ETag (#634)"
```

`cargo xtask check` already run in Step 5.

---

## Final verification (after Task 6)

- [ ] **AC2/AC3 grep-check** — no quoting/hex/prefix spelling survives outside
      `etag.rs`. Expected: **no matches**.

```bash
rg -n 'format!\("\\"|"sha256-"|\{:x\}' \
   common/src server/src storage/src --glob '!common/src/etag.rs'
```

(A hit means a producer still spells the format — route it through a door
instead.)

- [ ] **AC6 gate** — run the pre-push gate the spec names:

Run: `cargo xtask validate --no-e2e` Expected: PASS — static + clippy +
coverage + `sqlx-newtype-bind` + the (unchanged) `rendered-html-from-trusted`
gate. This is the authoritative green for the branch.

---

## Self-review

- **Spec coverage:** AC1 → Task 1 (type/validation/serde tests). AC2 → Task 1
  (door tests + the single-quoting-site grep, enforced in AC3). AC3 → Tasks 2–6
  (each producer mints via a door; grep for `format!("\"`, `sha256-`, `{:x}`
  outside `etag.rs`). AC4 → Task 6 (field + `CacheTuple` + typed bind +
  `…malformed_etag` decode test). AC5 → Tasks 2–6 (comparison mechanism
  preserved; site/media/atompub/feed tests pass, with compile-forced call-site
  edits). AC6 → every task's `cargo xtask check`.
- **Placeholder scan:** none — each implement step has its signature + the tests
  that pin it, and the two bodies tests can't pin (`is_valid_etag`,
  `from_content_hash`) are written out.
- **Type consistency:** `ETag`,
  `ETag::{sha256_of, from_sha256, from_content_hash}`, `InvalidETag`,
  `parse_etag`, `FeedCacheRow.etag: ETag`, `feed_etag -> ETag` are used
  identically across tasks.

## Execution handoff

Plan complete and saved to
`docs/superpowers/plans/2026-07-24-issue-634-etag-newtype.md`. Execution is
driven by **jaunder-iterate** — task-by-task, `cargo xtask check` per commit,
ticking checkboxes in real time — after the plan-approval HALT.
