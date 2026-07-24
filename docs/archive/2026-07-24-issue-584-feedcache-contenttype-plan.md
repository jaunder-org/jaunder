# Plan — #584: type the `FeedCacheRow` fields (ContentType; ETag decision)

Spec:
[`2026-07-24-issue-584-feedcache-contenttype.md`](../specs/2026-07-24-issue-584-feedcache-contenttype.md).
The plan is "how"; see the spec for "what/why" and the AC list.

## Review header

**Goal.** Stop erasing the existing `common::media::ContentType` (#495) to a raw
`String` on `FeedCacheRow`: type the stored field, type its producer
(`FeedFormat::content_type()`), and record the ETag decision (stays `String`;
repo-wide `ETag` newtype deferred to a filed follow-up).

**Scope.**

- _In:_ `FeedCacheRow.content_type: ContentType`;
  `FeedFormat::content_type() -> ContentType`; the coupled server callers
  (`regenerate.rs`, `handlers.rs` test); test helpers → `parse_content_type`; a
  decode-error test (AC6); the ETag deferral comment + follow-up issue.
- _Out:_ any `ETag` newtype / threading its ~5 producers (follow-up, task 1);
  `FeedCacheRow.body` (opaque payload); feed SQL/schema/endpoints/304 semantics.

**Tasks.**

- [x] 1. File the repo-wide `ETag`-newtype follow-up issue (separable concern) —
     yields the number AC4's comment must cite. → **#634** (blocked-by #584).
- [x] 2. `common`: `FeedFormat::content_type()` returns `ContentType` via a
     `pub(crate) ContentType::from_trusted` door (+ `detect_content_type` onto
     it), a pinning test, and the `#398` gate extended to exempt
     `ContentType::`.
- [x] 3. `storage` + `server`: `FeedCacheRow.content_type: ContentType` —
     struct, `CacheTuple`, bind, tests, the AC6 decode-error test, and the
     coupled `regenerate.rs` / `handlers.rs`-test updates + the `ETag` deferral
     comment. (Sweep also fixed 5 `FeedCacheRow` construction sites in
     `server/tests/` the plan-review grep missed — `src/`-only.)
- [x] 4. Full gate: `cargo xtask validate --no-e2e` clean.

**Key risks / decisions.**

- No new generic `sqlx` bounds: the `StrNewtype` bridge derives `Type`/`Encode`/
  `Decode` for `ContentType` delegating to `String`, so the existing
  `String: Encode`/`CacheTuple: FromRow` bounds on `feed_cache.rs` already cover
  bind and decode (verified against `str_newtype.rs:339-380` and the `media.rs`
  precedent). The bind changes from `.bind(row.content_type.as_str())` to
  `.bind(&row.content_type)` (no `.as_str()` on the newtype).
- Producer mints via a `pub(crate) ContentType::from_trusted` trusted door (a
  `FromStr` `.parse().expect(...)` is illegal here — the repo denies
  `clippy::expect_used`). This requires extending the `#398`
  `rendered-html-from-trusted` gate to exempt the `ContentType::` qualifier.
  (Spec Decision 2.)
- Task 2 leaves the tree green on its own (`regenerate.rs` keeps its
  `.to_string()` — `ContentType: Display`); task 3 removes it. Tasks 2 and 3 are
  separate commits by crate seam.

**For agentic workers.** Execute with **jaunder-iterate**, delegating a task to
a subagent via **jaunder-dispatch** where useful. Tick checkboxes in real time.

## Global constraints

- **Backend parity (ADR-0019):** storage tests are dual-backend via
  `#[apply(backends)]`; a bare `#[tokio::test]` that should be dual-backend
  trips the `test-backend-pattern` guard. No per-backend dialect files touched.
- **Newtype test helpers:** build `ContentType` in tests via
  `common::test_support::parse_content_type(...)`, never `"…".into()`.
- **Coverage:** the new decode-error path (AC6) is covered by task 3's test;
  `common`/macros are coverage-measured — the pinning test covers the producer.
- **Gate before commit:** run `cargo xtask check` clean first (the pre-commit
  hook runs the full `cargo xtask check`); see **jaunder-commit**. **No
  `Co-Authored-By` trailer.**

---

## Task 1 — File the repo-wide `ETag`-newtype follow-up issue

**No code.** Use **jaunder-issues**. File into milestone #13 (Domain-value type
safety), label `type-safety`. Title e.g. _"types: repo-wide `ETag` `StrNewtype`
for the quoted-format invariant"_. Body: the `"…"`-quoted invariant is enforced
nowhere; ~5 independent producers (`server/src/site.rs`,
`server/src/projector/mod.rs`, `server/src/media.rs`,
`common::feed::metadata::feed_etag`, `server/src/atompub/posts::etag_for`) each
`format!("\"…\"")` a `String`; only `feed_cache` stores one. A meaningful
newtype threads all producers — deferred from #584 (which typed only
`FeedCacheRow`). Reference #584 and ADR-0063.

**Done when:** the issue exists; record its number `<ETAG_ISSUE>` for task 3's
comment.

---

## Task 2 — `common`: `FeedFormat::content_type()` returns `ContentType`

**Files.** `common/src/feed/feed_path.rs` (impl + in-file `#[cfg(test)]`).

**Test (RED first).** **Update the existing** `format_content_types` test
(`feed_path.rs:252-263`) — don't add a parallel one (it currently asserts
against `&str` and would survive silently via the derived `PartialEq<&str>`,
becoming redundant). Retype its assertions to the newtype so it pins the parse:

```rust
#[test]
fn format_content_types() {
    use crate::test_support::parse_content_type;
    assert_eq!(
        FeedFormat::Rss.content_type(),
        parse_content_type("application/rss+xml; charset=utf-8")
    );
    assert_eq!(
        FeedFormat::Atom.content_type(),
        parse_content_type("application/atom+xml; charset=utf-8")
    );
    assert_eq!(
        FeedFormat::Json.content_type(),
        parse_content_type("application/feed+json")
    );
}
```

`cargo nextest run -p common format_content_types` → **FAIL** (returns
`&'static str`; won't compile against `ContentType`).

**Implement.** Three coupled edits (a `FromStr` `.parse().expect(...)` is
**not** usable — the repo denies `clippy::expect_used`/`missing_panics_doc`, so
it fails the gate; mint through a trusted door instead):

1. **`common/src/media.rs` — the door.** Add a `pub(crate)` trusted constructor
   on `ContentType` (mirrors `RenderedHtml::from_trusted`), and refactor
   `detect_content_type`'s two `ContentType(x.to_owned())` mints onto it (single
   trusted mint site):

   ```rust
   impl ContentType {
       /// Mint from a string the caller asserts is a valid media type … `pub(crate)`
       /// so outside this crate the only door stays the validating `FromStr`.
       #[must_use]
       pub(crate) fn from_trusted(content_type: impl Into<String>) -> Self {
           Self(content_type.into())
       }
   }
   ```

2. **`common/src/feed/feed_path.rs` — the producer.**

   ```rust
   use crate::{media::ContentType, tag::Tag, username::Username};
   // ...
   #[must_use]
   pub fn content_type(self) -> ContentType {
       let literal = match self {
           FeedFormat::Rss => "application/rss+xml; charset=utf-8",
           FeedFormat::Atom => "application/atom+xml; charset=utf-8",
           FeedFormat::Json => "application/feed+json",
       };
       ContentType::from_trusted(literal)
   }
   ```

3. **`xtask/src/steps/rendered_html_from_trusted_check.rs` — extend the gate.**
   The `#398` gate leaf-matches the `from_trusted` name for `RenderedHtml`'s XSS
   door, so `ContentType::from_trusted` trips it. Add an
   `EXEMPT_QUALIFIERS = ["ContentType"]` carve-out in `is_door` (skip when the
   qualifier segment left of the leaf is exempt) + module-doc update + tests:
   `ContentType::from_trusted` (direct + `.map()` reference) is exempt, an
   unrelated `Widget::from_trusted` still flagged.

**Verify.** `cargo nextest run -p common format_content_types` → **PASS**.
`cargo nextest run --manifest-path xtask/Cargo.toml rendered_html_from_trusted`
→ **PASS** (incl. the 3 new cases). `cargo xtask check --no-test` → all steps
`ok` (clippy + `rendered-html-from-trusted` + `xtask-tests`).
`regenerate.rs:112`'s `.to_string()` still compiles via `ContentType: Display`.

**Commit** (jaunder-commit):
`refactor(common): FeedFormat::content_type returns ContentType via a trusted door (#584)`
— includes the media door, producer, and gate extension.

---

## Task 3 — `storage` + `server`: `FeedCacheRow.content_type: ContentType`

**Files.** `storage/src/feed_cache.rs` (struct, `CacheTuple`, bind, tests);
`server/src/feed/regenerate.rs`; `server/src/feed/handlers.rs` (test helper).

**Test (RED first).** In `feed_cache.rs` tests, add the AC6 decode-error case —
backend-agnostic (no timestamp literals): upsert a valid row, corrupt the column
via raw SQL, assert the read errors. Mirrors
`media.rs::find_by_hash_surfaces_a_column_decode_error_for_a_malformed_filename`.

```rust
#[apply(backends)]
#[tokio::test]
async fn get_surfaces_a_column_decode_error_for_a_malformed_content_type(
    #[case] backend: Backend,
) {
    use sqlx::Executor;
    let env = backend.setup().await;
    env.state.feed_cache.upsert(sample("/feed.rss")).await.unwrap();
    // A non-media-type value bypasses `ContentType` validation — only reachable via
    // DB tampering. The key stays valid so the row is found; the validating bridge
    // `Decode` then rejects the `content_type` column on read.
    env.base
        .pool()
        .execute("UPDATE feed_cache SET content_type = 'not-a-content-type' WHERE feed_url = '/feed.rss'")
        .await
        .unwrap();
    let err = env.state.feed_cache.get(&fp("/feed.rss")).await.unwrap_err();
    assert!(
        matches!(err, FeedCacheError::Db(sqlx::Error::ColumnDecode { .. })),
        "expected a column-decode error, got: {err:?}"
    );
}
```

Also update `sample()` (line 162):
`content_type: parse_content_type("application/rss+xml"),` and add
`use common::test_support::parse_content_type;` to the test module.

`cargo nextest run -p storage feed_cache` → **FAIL** (field is `String`; the
corrupt value round-trips fine, so `get` returns `Ok`, and `sample` won't
compile against the new helper once the field flips — write the impl next).

**Implement — storage.**

- `use common::media::ContentType;`
- `FeedCacheRow.content_type: ContentType` (line 19). Add the ETag deferral
  comment on the sibling field:
  ```rust
  /// The stored ETag. Stays `String`: the `"…"`-quoted invariant is upheld by
  /// construction at every producer and only this copy is stored, so a
  /// stored-only newtype would enforce nothing. A repo-wide `ETag` newtype is
  /// tracked in #<ETAG_ISSUE>.
  pub etag: String,
  ```
- `CacheTuple` element 3 (line 41): `String` → `ContentType`. `row_from_tuple`
  is unchanged (assigns `t.3`); extend its existing header comment to note the
  `content_type` column now decodes into `ContentType` via the #438 bridge, same
  as `feed_url`.
- `upsert` bind (line 128): `.bind(row.content_type.as_str())` →
  `.bind(&row.content_type)`. No `where`-bound changes (existing
  `String: Encode` / `CacheTuple: FromRow` cover it).

**Implement — server.**

- `regenerate.rs:112`: `content_type: format.content_type().to_string()` →
  `content_type: format.content_type(),`.
- `handlers.rs` `sample_row` (line 199):
  `content_type: "application/rss+xml; charset=utf-8".to_owned()` →
  `content_type: parse_content_type("application/rss+xml; charset=utf-8"),` +
  `use common::test_support::parse_content_type;` in that test module. Line 79
  (`HeaderValue::from_str(&row.content_type)`) is unchanged — `&ContentType`
  coerces to `&str` via `Deref`.

**Verify.**

- `cargo nextest run -p storage feed_cache` → **PASS** (both backends, incl. the
  new decode-error test).
- `cargo nextest run -p jaunder feed::handlers` → **PASS** (served
  `Content-Type` header + 304 behavior unchanged — AC5). If the
  `parse_content_type` import fails to resolve, `server`'s dev-deps need
  `common` with `features = ["test-support"]` (storage already enables it; add
  it to `server/Cargo.toml` `[dev-dependencies]` if absent).
- `cargo check -p jaunder --all-features --all-targets` compiles.
- `cargo clippy -p storage -p jaunder --all-targets -- -D warnings` clean.

**Commit** (jaunder-commit):
`refactor(storage): type FeedCacheRow.content_type as ContentType (#584)`.

---

## Task 4 — Full local gate

Run `cargo xtask validate --no-e2e` (foreground, `timeout: 600000`). Resolve any
clippy (`must_use_candidate` etc.) or coverage findings; `cargo fmt` if the hook
reflowed anything (`git status --porcelain` after green — the check auto-fixes
fmt but doesn't commit). No behavior change to e2e-covered paths, so `--no-e2e`
is the gate here; **jaunder-ship** runs the full validate.

**Done when:** `validate --no-e2e` is green and the tree is clean (AC7).
