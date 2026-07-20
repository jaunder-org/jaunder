# Plan — Issue #495: `ContentType` newtype for media

Spec:
[`docs/superpowers/specs/2026-07-20-issue-495-content-type-newtype.md`](../specs/2026-07-20-issue-495-content-type-newtype.md)
(the "what/why" — decisions D1–D6, acceptance AC1–AC10). This is the "how".

## Review header

**Goal.** Introduce a validating `ContentType` str-newtype (mirroring
`Filename`) and thread it through all five media `content_type` fields + the
producer/consumer path per ADR-0063 §5, so an invalid media content type is not
representable.

**Scope.**

- In: `common/src/media.rs` (type + `detect_content_type`),
  `common/src/test_support.rs`, `storage/src/{media.rs,helpers.rs}`,
  `server/src/{media.rs,media_manager.rs}`, `server/src/atompub/media.rs`,
  `common/src/atompub/entry.rs` (**only** `MediaLinkEntry`),
  `web/src/media/{api.rs,component.rs}`.
- Out (spec D5): the Atom **content-element** type (`AtomContent`/`content_type`
  = `"text"/"html"/"xhtml"` in `entry.rs`); **feed** content types
  (`FeedCacheRow`, `server/src/feed/*`). No `should_inline`/`detect`
  classification-logic change.

**Tasks (one line each).**

1. Define `ContentType` + validating `FromStr` + unit tests; add
   `parse_content_type` helper.
2. `detect_content_type -> ContentType` (mint in-module); pin table validity by
   test.
3. Storage: `MediaRecord.content_type` + `MediaRow` decode + typed bind +
   fixtures.
4. Server `media_manager`: `UploadMetadata` + the single
   `get_content_type -> Result` door.
5. Server `media`: `UploadResponse` + serve-path unify + dead-fallback removal +
   `:596` test.
6. atompub: `MediaLinkEntry` + Atom `type=` read-out + client-header door +
   reject test.
7. web: `MediaItem` (`api.rs`) + the Leptos view cell read-out (`component.rs`).
8. Full gate + coverage; single atomic commit.

**Key risks / decisions.**

- **Atomic sweep = one commit.** Changing `detect`'s return type and the five
  field types ripples across crates; the tree compiles only when the whole
  thread is done, and coverage gates the working tree — so tasks 1–7 build to
  one green tree and land as **one commit** (the newtype-vertical precedent,
  #482). Task 1's type is introduced _with_ its exercising thread, not before
  it, so its derive-generated serde/sqlx lines aren't left uncovered.
- **Behavior change (spec D3/AC4):** `get_content_type` becomes the single
  validating door; a malformed _present_ client `Content-Type` is now rejected
  on **both** the multipart and atompub paths. Absent/non-UTF-8 atompub default
  (octet-stream) preserved.
- **Dead fallback (spec D4/AC7):** the `media.rs:178` octet-stream fallback
  becomes unreachable once `ContentType` guarantees header-validity — remove it,
  don't leave it live.
- **Scope trap:** two different `content_type`s live in `entry.rs`; touch only
  `MediaLinkEntry` (`:601/:626/:1037`), never `AtomContent` (`:136/:319/:497`).

**For agentic workers.** Drive with `jaunder-iterate`; delegate a task via
`jaunder-dispatch` if useful. Tasks 3–7 are one compile-unit — verify by
`cargo check`-ing the workspace after the sweep, commit once at task 8.

## Global constraints

- No `Co-Authored-By` trailer. Commit only on a green gate (`jaunder-commit`);
  the pre-commit hook runs full `cargo xtask check`.
- `unwrap_used`/`expect_used` are **deny** in production (`Cargo.toml:110-111`)
  — `detect` mints in-module (no expect); `expect` is fine only in
  `#[cfg(test)]`/`test_support`.
- Storage media round-trip tests are already dual-backend
  (`test-backend-pattern`); we edit fixtures, not add bare `#[tokio::test]`s.
- `common` has the `sqlx` feature (host-only, wasm-excluded); `ContentType`'s
  derive sqlx bridge rides it exactly like `Filename`.

---

## Task 1 — Define `ContentType` + tests + helper

**Files:** `common/src/media.rs` (beside `Filename`),
`common/src/test_support.rs`.

Add (mirroring `Filename` at `media.rs:138`):

```rust
/// A media `Content-Type` header value (`type/subtype` with optional `;` parameters).
/// The ADR-0063 trailer (Display/AsRef/Borrow/Deref<str>/From<Self> for String/PartialEq,
/// validating serde + sqlx bridges) is `#[derive(StrNewtype)]`; the sole door is the
/// validating `FromStr` below, so an arbitrary `String` cannot become a `ContentType`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct ContentType(String);

/// Error returned when a string is not a valid media `Content-Type` value.
#[derive(Debug, Error)]
#[error("content type must be a `type/subtype` media type, e.g. `image/png`")]
pub struct InvalidContentType;

impl FromStr for ContentType {
    type Err = InvalidContentType;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if is_valid_content_type(s) {
            Ok(ContentType(s.to_owned()))
        } else {
            Err(InvalidContentType)
        }
    }
}

/// RFC 7230 `tchar`.
fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// A valid media type: every byte a valid `HeaderValue` byte (VCHAR / SP / HTAB), and the
/// essence (before the first `;`) is `token "/" token` with non-empty, `tchar`-only halves.
/// Parameters (after `;`) need only be header-safe — matching `HeaderValue::from_str`'s own
/// acceptance, so every `ContentType` is header-constructible (spec D2/D4).
fn is_valid_content_type(s: &str) -> bool {
    if !s.bytes().all(|b| b == b'\t' || (0x20..=0x7e).contains(&b)) {
        return false;
    }
    let essence = s.split(';').next().unwrap_or("").trim();
    let Some((ty, sub)) = essence.split_once('/') else {
        return false;
    };
    !ty.is_empty() && !sub.is_empty() && ty.bytes().all(is_tchar) && sub.bytes().all(is_tchar)
}
```

**Tests** (in `media.rs` `#[cfg(test)] mod tests`), AC1:

```rust
#[test]
fn content_type_accepts_valid() {
    for s in ["image/png", "application/pdf", "image/svg+xml", "text/html; charset=utf-8"] {
        assert!(s.parse::<ContentType>().is_ok(), "must accept {s:?}");
    }
}
#[test]
fn content_type_rejects_malformed() {
    for s in ["", "garbage", "a/", "/b", "image /png", "text/plain; x=\u{1}", "im\u{1}age/png"] {
        assert!(s.parse::<ContentType>().is_err(), "must reject {s:?}");
    }
}
```

The AC1 **header-constructible** assertion (the D4 invariant against the _real_
`HeaderValue::from_str` oracle) lives in the **server** crate, not `common` —
`axum` is a server dependency, and a structural re-check in `common` would
merely restate `is_valid_content_type`. It is added in Task 5
(`server/src/media.rs` tests, where `axum` and the header read-out already
live).

Add to `common/src/test_support.rs` (mirror `parse_filename` at `:151`), AC9:

```rust
/// Parse `s` into a [`ContentType`] for tests — the single place a test content-type
/// literal is parsed. # Panics if `s` is not a valid media type.
#[must_use]
pub fn parse_content_type(s: &str) -> ContentType {
    s.parse().expect("valid test content type")
}
```

**Run:** `cargo nextest run -p common media::tests::content_type` — expect the
new tests PASS (after task 2 the module compiles; if run standalone, the type +
tests compile alone).

## Task 2 — `detect_content_type -> ContentType`

**Files:** `common/src/media.rs`.

Change the signature to
`pub fn detect_content_type(filename: &str) -> ContentType` and mint in-module —
the matched table entry and the fallback both wrap directly:

```rust
for (extensions, content_type) in EXTENSIONS {
    if extensions.contains(&ext.as_str()) {
        return ContentType(content_type.to_owned()); // in-module mint (private field)
    }
}
ContentType("application/octet-stream".to_owned())
```

The `EXTENSIONS` table (`&'static str` values) and `should_inline` are
**unchanged** (AC5). `should_inline(&str)`/`content_disposition(&str, …)` keep
`&str` params — a `&ContentType` deref-coerces at call sites, no `.as_ref()`.

**Test** (AC3): every canonical literal is genuinely valid, so the in-module
mint is honest:

```rust
#[test]
fn detect_content_type_outputs_are_valid() {
    for f in ["a.jpg","a.png","a.gif","a.webp","a.svg","a.mp3","a.ogg","a.flac","a.wav",
              "a.mp4","a.webm","a.pdf","a.unknown"] {
        assert!(detect_content_type(f).as_ref().parse::<ContentType>().is_ok());
    }
}
```

Existing `detect_content_type_*` tests compare against string literals
(`== "image/jpeg"`) — `PartialEq<str>` from the trailer keeps them compiling.

## Task 3 — Storage threading

**Files:** `storage/src/media.rs`, `storage/src/helpers.rs`.

- `MediaRecord.content_type: String` → `ContentType` (`media.rs:76`); import
  `ContentType` from `common::media`.
- Bind: `media.rs:241` `.bind(record.content_type.as_str())` →
  `.bind(&record.content_type)`.
- `helpers.rs:307` `MediaRow` element 5 `String` → `ContentType`;
  `media_record_from_row` (`:314/:328`) moves the decoded `content_type` (the
  column now decodes straight into `ContentType` via the validating `Decode` — a
  corrupt column is a `ColumnDecode` error; step stays fallible for `source`).
- Fixtures `media.rs:434/500/545` and `helpers.rs` media rows:
  `content_type: "image/jpeg".to_string()` → `parse_content_type("image/jpeg")`.
  The raw-SQL INSERT fixtures (`media.rs:471/515`) bind a literal string —
  unchanged (that's SQL text, not a `ContentType`).

**Verify:** contributes to the sweep; `cargo check -p storage` after tasks 1–3.

## Task 4 — Server `media_manager` (the single door)

**Files:** `server/src/media_manager.rs`.

- `UploadMetadata.content_type: String` → `ContentType` (`:41`).
- `get_content_type(content_type: Option<&str>, filename: &str) -> Result<ContentType, InvalidContentType>`
  (`:111`):

```rust
pub fn get_content_type(client: Option<&str>, filename: &str) -> Result<ContentType, InvalidContentType> {
    match client {
        Some(c) => c.parse(),
        None => Ok(detect_content_type(filename)),
    }
}
```

- Callers propagate: `upload` (`:79`) and `upload_bytes` (`:301`) —
  `let content_type = Self::get_content_type(…)?;` — mapping
  `InvalidContentType` into the handler's error type (match the existing
  upload-error enum; a malformed type → a 4xx, not a 500).
- `store_metadata`/`UploadMetadata { content_type, … }` (`:89/:209/:276/:315`)
  now carry `ContentType`.
- Test `:592` `assert_eq!(first.content_type, "image/png")` — `PartialEq<str>`
  keeps it.

## Task 5 — Server `media` (serve + DTO + dead fallback)

**Files:** `server/src/media.rs`.

- `UploadResponse.content_type: String` → `ContentType` (`:44`).
- Serve unify (`:161`):
  `map_or_else(|| detect_content_type(&params.filename), |r| r.content_type)` —
  both arms are `ContentType`; **drop `.to_owned()`**.
- Header read-out + dead-fallback removal (`:178`): `ContentType` guarantees a
  valid header value, so replace
  `HeaderValue::from_str(&content_type).unwrap_or_else(|_| from_static(…))` with
  a `.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?` read-out matching the
  sibling etag/disposition lines (`:187/:191`), and `// cov:ignore` the
  never-hit `Err` arm (the invariant makes it unreachable; a `warn!`-free single
  line). `content_disposition(&content_type, …)` and
  `should_inline(&content_type)` deref-coerce — no signature change.
- Test `:596`: `let expected = detect_content_type("photo.jpg");` compared
  against an `Option<&str>` header → `Some(expected.as_ref())`.
- **AC1 header-oracle test** (here, where `axum` lives): assert every
  representative `ContentType` is header-constructible —
  `for s in ["image/png","text/html; charset=utf-8","application/octet-stream"] { assert!(HeaderValue::from_str(common::media::ContentType::from_str(s).unwrap().as_ref()).is_ok()); }`.
  This observes the D4 invariant the `:178` fallback removal relies on.

## Task 6 — atompub threading + reject test

**Files:** `server/src/atompub/media.rs`, `common/src/atompub/entry.rs`.

- `MediaLinkEntry.content_type: String` → `ContentType` (`entry.rs:601`). Atom
  emission `entry.rs:626` `("type", entry.content_type.as_str())` → `.as_ref()`.
  Fixture `:1037` `content_type: "image/png".to_string()` →
  `parse_content_type("image/png")`.
- **Leave `AtomContent`/content-element `content_type`
  (`entry.rs:136/319/497/645`) untouched** (spec D5/AC8).
- `server/src/atompub/media.rs:40` `content_type: record.content_type.clone()` —
  both `ContentType`, clone stays. `:71` keeps the absent/non-UTF-8 →
  `"application/octet-stream"` `&str` default and passes it as `Some(&str)` to
  `upload_bytes` (`:87`) → the single door validates it (no separate parse
  here).
- **Reject test (AC4):** a `#[test]` on
  `get_content_type(Some("garbage"), "x.png")` is `Err` (cheap, direct on the
  door). If a server integration test for the atompub HTTP 4xx path is low-cost
  (`server/tests/web/web_media.rs`), add it; otherwise the unit test suffices
  and is noted.

## Task 7 — web threading

**Files:** `web/src/media/api.rs`, `web/src/media/component.rs`.

- `MediaItem.content_type: String` → `ContentType` (`api.rs:31`); the producer
  `api.rs:111` `content_type: r.content_type` unifies once both are
  `ContentType`.
- View cell `component.rs:355` `<td>{item.content_type.clone()}</td>` —
  `ContentType` is not `IntoView`; read out via
  `<td>{item.content_type.to_string()}</td>` (or `.as_ref().to_owned()`).
- Confirm `common/sqlx` is **not** pulled into the wasm build — `MediaItem` is a
  wire DTO; `ContentType`'s serde bridge is wasm-safe (sqlx is feature-gated off
  for wasm), same as `Filename` already in this struct.

## Task 8 — Full gate + coverage; commit

**Run:** `devtool run -- cargo xtask check` (foreground, `timeout: 600000`).
Expect green; inspect the coverage sidecar for any uncovered `ContentType`
trailer/bridge line. Encode/ Decode/serde covered by media storage + wire
round-trips; `Display`/`Deref`/`PartialEq` by the read-outs + AC1 tests. **If**
a trailer line is flagged uncovered, add a targeted unit test (e.g. exercise
`String::from(content_type)` / `Borrow`) — do not leave it uncovered.

Then `devtool run -- cargo xtask validate --no-e2e` (AC10). Commit once
(`jaunder-commit`):
`types: ContentType newtype for media content types, threaded per ADR-0063 §5 (#495)`.

## Self-review

- Every AC maps: AC1→T1 (accept/reject) + T5 (header-oracle); AC2→T3–T7; AC3→T2;
  AC4→T4/T6; AC5→T2; AC6→T3; AC7→T5; AC8→T6; AC9→T1; AC10→T8.
- No out-of-scope work (Atom content-element type / feed content types untouched
  — T6 note).
- No separable concern surfaced (single vertical) → no first-task issue filing.
- Tasks 3–7 are one compile-unit landing as one commit; T1/T2 precede (the
  type + producer the thread depends on).
