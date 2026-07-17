# `Filename` Newtype Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful — Task 4's mechanical fixture sweep is a good
> candidate). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make an unsanitized media filename unrepresentable by introducing a
`common::media::Filename` newtype and threading it through storage, the DTOs,
the handlers, and the web boundary.

**Architecture:** A `#[derive(StrNewtype)]` str-newtype modeled on the sibling
`ContentHash`, with two construction doors — a validating `FromStr` (Door A,
canonical-only) and a normalizing producer `Filename::sanitized` (Door B). Both
are defined by the existing `sanitize_filename`. Rejection status splits by
surface: authenticated atompub member routes get a typed extractor (400); the
public serve route keeps a friendly 404.

**Tech Stack:** Rust, `macros::StrNewtype`, `sqlx` (dual backend), `axum`,
Leptos server functions.

**Spec:**
[`2026-07-17-issue-482-filename-newtype.md`](../specs/2026-07-17-issue-482-filename-newtype.md)
— this plan is "how"; the spec is "what/why". Read D1–D4 and the acceptance
criteria there.

## Review header

**Scope — in:** `Filename` type + `parse_filename` helper (Task 2); serve-route
re-check retirement → 404 (Task 3); the full §5 pervasiveness sweep across
storage/server/web/tests, incl. atompub member routes → typed-extractor 400
(Task 4).

**Scope — out:** `content_type`/MIME newtype (Task 1 files it as a separate
issue); any change to `sanitize_filename`/`detect_content_type` logic, the
`media` schema, on-disk layout, URL shapes, or `MediaSource` typing.

**Tasks:**

1. File the `content_type`/MIME spin-out issue (tracker only, no code).
2. `Filename` newtype in `common::media` + `parse_filename` test helper + unit
   tests.
3. Retire the serve-route `sanitize_filename(x) != x` re-check (→ parse into
   `Filename`, 404).
4. Pervasiveness sweep: thread `Filename` through storage + server + web + all
   test fixtures (one atomic commit — a Rust type change on
   `MediaRecord.filename` forces every consumer).

**Key risks / decisions:**

- **Task 4 is necessarily one large atomic commit.** Flipping
  `MediaRecord.filename: Filename` breaks every downstream consumer (server,
  web, tests) until updated; the crate compile cascade admits no smaller green
  intermediate. It is mechanical; delegate the fixture bulk.
- **DB read-back safety** rests on `sanitize_filename` idempotence (proven in
  the spec, pinned by a test in Task 2 and exercised by the reject test in Task
  4).
- **Behavior change:** malformed hash _or_ filename on the atompub member routes
  → **400** (was 404 for the hash); the serve route stays 404. Update the
  affected atompub tests.

## Global Constraints

- **ADR-0063 §4/§5:** hold the newtype on every surface we define up to an
  external-type boundary; read it out via `Deref`/`Display`/`Serialize` only at
  that boundary (e.g. the `quick_xml` write, a `HeaderValue`, a SQL `.bind`). No
  `.to_string()`/`String::from` re-derive of a newtype-sourced value onto a
  `String` field.
- **Backend parity (CONTRIBUTING.md):** storage tests are dual-backend
  (`#[apply(backends)]`); a bare `#[tokio::test]` that should be dual-backend
  fails the `test-backend-pattern` guard. Do **not** add tests to the ADR-0019
  per-backend dialect files (`storage/src/{sqlite,postgres}/media.rs`).
- **Test fixtures** build newtype values via `common::test_support::parse_*` —
  add and use `parse_filename`, never inline `.parse().unwrap()` (memory:
  newtype test-helper convention).
- **Commit gate:** run `cargo xtask check` foreground (`timeout: 600000`) before
  each commit — storage changes trigger a coverage rebuild (~2 min) that gets
  killed in background mode. **No `Co-Authored-By` trailer.**

---

### Task 1: File the `content_type`/MIME spin-out issue

**Files:** none (GitHub tracker action via **jaunder-issues**).

**Interfaces:**

- Produces: a new open issue number, linked from #482, satisfying spec
  acceptance criterion 10.

- [x] **Step 1: Create the issue** (per **jaunder-issues** — `type-safety`
      label, milestone 13 "Domain-value type safety (newtypes)", priority via
      Project #1 field). Body captures:
  - The five `content_type` surfaces (`MediaRecord.content_type`,
    `UploadMetadata`, `UploadResponse`, `web::MediaItem`,
    `MediaLinkEntry.content_type`) + the
    `detect_content_type`/`should_inline`/`content_disposition`/`HeaderValue`
    sites (cite the spec's §3 inventory).
  - The **unresolved design question** the issue must settle: enum (fixed set,
    `PostFormat` style) vs. str-newtype (free-form `type/subtype`), given that
    the atompub POST path (`server/src/atompub/media.rs:73`) passes the client's
    raw `Content-Type` header through — so the invariant is "valid HTTP header
    value", not a closed set.
  - Note `detect_content_type` returns `&'static str` from a fixed table → the
    natural producer door for a newtype.
- [x] **Step 2: Cross-link** — filed **#495** (blocked-by #482, milestone 13,
      P3, project #1); commented on #482 linking it. Ship references #495 for
      criterion 10.

---

### Task 2: `Filename` newtype in `common::media`

**Files:**

- Modify: `common/src/media.rs` (add type near `ContentHash`, ~`:61`; tests in
  the existing `#[cfg(test)] mod tests`).

> **Note (deviation):** `parse_filename` in `common/src/test_support.rs` moved
> to **Task 4**, its first caller. Added here it has no user until Task 4, so
> the coverage gate flags it uncovered — it lands with its fixtures instead.

**Interfaces:**

- Consumes: `sanitize_filename` (`common/src/media.rs:106`),
  `macros::StrNewtype`.
- Produces:
  - `pub struct Filename(String)`
    `#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]`.
  - `pub struct InvalidFilename` (`thiserror::Error`).
  - `impl FromStr for Filename { type Err = InvalidFilename; }` — **Door A**
    (canonical-only).
  - `impl Filename { pub fn sanitized(raw: &str) -> Result<Filename, InvalidFilename> }`
    — **Door B**.

- [x] **Step 1: Write the failing tests** (in `common/src/media.rs` `mod tests`)

```rust
// --- Door A: validating FromStr (canonical-only) ---
#[test]
fn filename_parses_a_canonical_leaf() {
    let f: Filename = "photo.jpg".parse().unwrap();
    assert_eq!(f, "photo.jpg");
}
#[test]
fn filename_rejects_non_canonical_and_empty() {
    // Non-canonical (would be normalized by Door B) and empty are rejected, not normalized.
    for bad in ["", "..", ".", "a/b", "../x", "sub/file.txt", "C:\\x\\y.txt", "foo\0"] {
        assert!(bad.parse::<Filename>().is_err(), "must reject {bad:?}");
    }
}
#[test]
fn filename_display_and_deref_read_the_leaf() {
    let f: Filename = "a.txt".parse().unwrap();
    assert_eq!(f.to_string(), "a.txt");
    assert_eq!(&f[..1], "a"); // Deref<str>
}

// --- Door B: normalizing producer ---
#[test]
fn sanitized_normalizes_to_a_safe_leaf() {
    assert_eq!(Filename::sanitized("../../etc/passwd").unwrap(), "passwd");
    assert_eq!(Filename::sanitized("foo/bar/baz.txt").unwrap(), "baz.txt");
    assert_eq!(Filename::sanitized("file\0name.txt").unwrap(), "file_name.txt");
}
#[test]
fn sanitized_rejects_empty_after_normalization() {
    for bad in ["", ".", ".."] {
        assert!(Filename::sanitized(bad).is_err(), "must reject {bad:?}");
    }
}

// --- serde bridge (wire rejection) ---
#[test]
fn filename_serde_serializes_as_plain_string_and_validates_on_deserialize() {
    let f: Filename = "photo.jpg".parse().unwrap();
    assert_eq!(serde_json::to_string(&f).unwrap(), "\"photo.jpg\"");
    assert_eq!(serde_json::from_str::<Filename>("\"photo.jpg\"").unwrap(), f);
    assert!(serde_json::from_str::<Filename>("\"../x\"").is_err());
}

// --- idempotence pin (spec criterion 8): any non-empty sanitize output re-parses via Door A ---
#[test]
fn sanitize_filename_output_always_reparses_as_filename() {
    for raw in [
        "photo.jpg", "../../etc/passwd", "foo/bar/baz.txt", "C:\\Users\\file.txt",
        "file\0name.txt", "a b.txt", ".hidden", "no-ext",
    ] {
        let s = sanitize_filename(raw);
        if !s.is_empty() {
            assert!(s.parse::<Filename>().is_ok(), "sanitize({raw:?})={s:?} must re-parse");
        }
    }
}
```

Add the two compile-fail doctests on the `Filename` doc comment (mirror
`ContentHash` `common/src/media.rs:53-59`): private field, and `String` is not a
`Filename`.

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p common media::tests::filename` Expected: FAIL —
`Filename` / `InvalidFilename` / `sanitized` not defined.

- [x] **Step 3: Implement against the tests**

Add to `common/src/media.rs`, following the `ContentHash` template exactly (doc
comment with the compile-fail doctests, `#[derive(StrNewtype)]`, hand-written
`FromStr`):

````rust
/// A validated media filename: a single safe path leaf — the canonical form produced
/// by [`sanitize_filename`] (no path components, no `.`/`..`, no `\0`, non-empty). The
/// newtype makes an un-sanitized filename unrepresentable where a filename is expected
/// (a path-traversal / header-injection hazard, ADR-0063 §1 invariant + trust boundary).
///
/// Two doors, mirroring [`ContentHash`]'s `FromStr` + `from_digest`:
/// - [`FromStr`] validates — it accepts a string **iff** it is already a canonical leaf
///   (`sanitize_filename(s) == s`, non-empty), *rejecting* a non-canonical name rather
///   than normalizing it. This is the door for untrusted URL/wire/DB values that must
///   match a stored filename exactly; `#[derive(StrNewtype)]` routes `Deserialize`
///   through it, so a non-canonical filename is rejected on the wire.
/// - [`sanitized`][Filename::sanitized] normalizes — the upload-intake door.
///
/// The rest of the ADR-0063 string-newtype trailer is generated by `#[derive(StrNewtype)]`.
/// ```compile_fail
/// let _ = common::media::Filename("a".to_string()); // private field
/// ```
/// ```compile_fail
/// fn takes(_: common::media::Filename) {}
/// takes("a".to_string()); // a String is not a Filename
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct Filename(String);

/// Error returned when a string is not a canonical media filename leaf.
#[derive(Debug, Error)]
#[error("filename must be a non-empty safe path leaf (no path components, `.`/`..`, or null bytes)")]
pub struct InvalidFilename;

impl FromStr for Filename {
    type Err = InvalidFilename;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Door A: accept only an already-canonical, non-empty leaf. `sanitize_filename`
        // is the oracle; an empty input passes `sanitize("") == ""` so it is guarded
        // explicitly (a filename is never empty).
        if !s.is_empty() && sanitize_filename(s) == s {
            Ok(Filename(s.to_owned()))
        } else {
            Err(InvalidFilename)
        }
    }
}

impl Filename {
    /// Builds a [`Filename`] by **normalizing** `raw` to a safe leaf via
    /// [`sanitize_filename`], rejecting an empty result. This is the trusted upload-intake
    /// door (the atompub `Slug` header, a multipart `file_name`), where a client's
    /// arbitrary name is meant to be reduced to a leaf.
    ///
    /// # Errors
    /// Returns [`InvalidFilename`] when `raw` sanitizes to the empty string (`""`, `.`, `..`).
    pub fn sanitized(raw: &str) -> Result<Self, InvalidFilename> {
        let s = sanitize_filename(raw);
        if s.is_empty() {
            Err(InvalidFilename)
        } else {
            Ok(Filename(s))
        }
    }
}
````

Add `parse_filename` to `common/src/test_support.rs` (import
`crate::media::Filename`):

```rust
/// Parse `name` into a valid [`Filename`] for tests — the single place a test filename
/// literal is parsed, so a malformed fixture fails loudly and the parse isn't re-spelled.
///
/// # Panics
/// Panics if `name` is not a canonical safe leaf.
#[must_use]
pub fn parse_filename(name: &str) -> Filename {
    name.parse().expect("valid test filename")
}
```

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p common media` then `cargo test -p common --doc media`
Expected: PASS (incl. the two `compile_fail` doctests).

- [x] **Step 5: Commit**

```bash
git add common/src/media.rs common/src/test_support.rs
git commit -m "feat(common): add Filename newtype with validating + sanitizing doors (#482)"
```

Run `cargo xtask check` foreground first (**jaunder-commit**).

---

### Task 3: Retire the serve-route re-check (→ parse into `Filename`, 404)

**Files:**

- Modify: `server/src/media.rs` — `validate_serve_params` (`:243`),
  `resolve_media_path` (`:265`), and the in-file `#[cfg(test)] mod tests`.

**Interfaces:**

- Consumes: `common::media::Filename` (Task 2).
- Produces:
  `validate_serve_params(&ServeParams) -> Result<(MediaSource, ContentHash, Filename), StatusCode>`.
  `ServeParams.filename` **stays `String`** (keeps the public content route at
  404, not axum's 400). `resolve_media_path`'s external signature is unchanged
  (`-> (MediaSource, ContentHash, PathBuf)`).

- [x] **Step 1: Write / adjust the failing tests** (in `server/src/media.rs`
      `mod tests`)

The existing `resolve_media_path_rejects_filename_with_traversal_or_separators`
(`:421`) already asserts `..`, `.`, `../../etc/passwd`, `a/b`, `sub/file.txt` →
`NOT_FOUND`; it must keep passing through the new parse path. Add a positive
assertion that a canonical filename is validated and carried:

```rust
#[test]
fn validate_serve_params_returns_typed_filename_for_canonical_name() {
    let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let p = params("upload", "e3", "b0", hash, "photo.jpg");
    let (source, h, filename) = validate_serve_params(&p).expect("valid");
    assert_eq!(source, MediaSource::Upload);
    assert_eq!(h, hash);
    assert_eq!(filename, "photo.jpg"); // Filename: PartialEq<str>
}
```

- [x] **Step 2: Run, verify it fails** — the server crate's package is `jaunder`
      (dir `server/`), so `-p jaunder` not `-p server`; the arity mismatch
      confirmed FAIL.

Run: `cargo nextest run -p jaunder --lib media::tests::validate_serve_params`

- [x] **Step 3: Implement against the tests**

In `validate_serve_params`, **replace** the string re-check block

```rust
    if common::media::sanitize_filename(&params.filename) != params.filename {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok((source, hash))
```

with a parse into `Filename` (mapped to the same DoS-safe 404) and return it:

```rust
    // Parse (not re-check) the attacker-influenced filename into the newtype here — a
    // non-canonical leaf fails Door A and yields the same 404, and the typed value flows
    // to the path join. Keeping `ServeParams.filename: String` avoids axum turning a bad
    // segment into a pre-handler 400 (this public content route answers 404).
    let filename: Filename = params.filename.parse().map_err(|_| StatusCode::NOT_FOUND)?;

    Ok((source, hash, filename))
```

Update `resolve_media_path` to bind the third element and join
`filename.as_ref()` in place of `&params.filename` (`:279`):

```rust
    let (source, hash, filename) = validate_serve_params(params)?;
    let file_path = storage_path
        .join("media").join(source.as_str()).join(&params.p1).join(&params.p2)
        .join(hash.as_ref()).join(filename.as_ref());
    Ok((source, hash, file_path))
```

Add `use common::media::Filename;` to the existing `common::media` import
(`:14`).

- [x] **Step 4: Run the tests, verify they pass**
      (`-p jaunder --lib media::tests`; full `cargo xtask check` green with
      PostgreSQL provisioned)

- [x] **Step 5: Commit** (09afa2e1)

```bash
git add server/src/media.rs
git commit -m "refactor(server): retire serve-time filename re-check via Filename parse (#482)"
```

Run `cargo xtask check` foreground first.

---

### Task 4: Pervasiveness sweep — thread `Filename` through storage, server, web, and tests

**One atomic commit.** The linchpin is `MediaRecord.filename: Filename`
(`storage/src/media.rs:72`) — flipping it breaks every construction/read site in
server, web, and tests until updated together, and the crate compile cascade
admits no smaller _green_ intermediate. (Strictly, a few sub-edits would compile
alone via `&Filename → &str` deref coercion — the member-route extractor swap,
and the read-only `media_path`/`media_url`/`get_content_type` and multipart
`upload`-path sites, which _flow_ through coercion rather than needing edits —
but they aren't worth separate commits.) Mechanical — a good
**jaunder-dispatch** delegation (especially the test-fixture bulk). Sequence the
edits so the first `cargo check` after they're all applied is green.

**Files:**

- Modify: `storage/src/media.rs`, `storage/src/helpers.rs`,
  `storage/src/sqlite/media.rs`, `storage/src/postgres/media.rs`.
- Modify: `common/src/atompub/entry.rs` (`MediaLinkEntry.title`).
- Modify: `server/src/media_manager.rs`, `server/src/media.rs`
  (`UploadResponse`), `server/src/atompub/media.rs`.
- Modify: `web/src/media/mod.rs`, `web/src/pages/media.rs`.
- Modify (tests): `storage/src/media.rs` (dual-backend),
  `storage/src/helpers.rs`, `server/tests/misc/backup_fixture.rs`,
  `server/tests/web/web_media.rs`, `server/tests/storage/mod.rs`,
  `server/tests/atompub/atompub_media.rs`, `server/src/media_manager.rs` tests.

**Interfaces:**

- Consumes: `common::media::Filename`, `common::test_support::parse_filename`
  (Task 2).
- Produces (the new signatures every edit must agree on):
  - `MediaRecord.filename: Filename`.
  - `MediaStorage::get_media(&self, user_id, sha256: &ContentHash, filename: &Filename, source: &MediaSource)`.
  - `MediaStorage::delete_media(&self, user_id, sha256: &ContentHash, filename: &Filename, source: &MediaSource)`.
  - `MediaDialect::delete_media_row(pool, user_id, sha256: &ContentHash, filename: &Filename, source: &str)`.
  - `UploadResponse.filename: Filename`, `UploadMetadata.filename: Filename`,
    `web::MediaItem.filename: Filename`, `MediaLinkEntry.title: Filename`.
  - `MediaManager::validate_filename(Option<&str>) -> anyhow::Result<Filename>`
    (Door B).
  - `MediaManager::upload_bytes(&self, auth, filename: &Filename, content_type: &str, bytes: &[u8])`.
  - `web::delete_media(sha256: ContentHash, filename: Filename, source: String, force: Option<bool>)`.

- [x] **Step 1: Write the failing tests** (the two new tests; fixture
      conversions land in Step 3)

New storage reject test — mirror `media_record_from_row_rejects_invalid_sha256`
(`storage/src/helpers.rs:757`), in the same `#[cfg(test)] mod`:

```rust
#[test]
fn media_record_from_row_rejects_invalid_filename() {
    let row: MediaRow = (
        1,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
        "../escape".to_string(), // non-canonical filename column value
        "upload".to_string(),
        "image/jpeg".to_string(),
        1024,
        None,
        Utc::now(),
    );
    let err = media_record_from_row(row).unwrap_err();
    assert!(matches!(err, sqlx::Error::Decode(_)));
}
```

Adjust the existing `media_record_from_row_accepts_valid_source` (`:775`) — its
filename tuple element must be a canonical leaf (e.g. `"file.txt"`), which it
already is; assert the parsed `record.filename == "file.txt"`.

- [x] **Step 2: Run, verify the reject test fails**

Run:
`cargo nextest run -p storage helpers::tests::media_record_from_row_rejects_invalid_filename`
Expected: FAIL — filename column not yet parsed (no `Decode` error path).

- [x] **Step 3: Apply the sweep** (mechanical fixture conversions delegated to a
      subagent; atompub 400/403/404 test changes done directly)

**storage/src/media.rs**

- `MediaRecord.filename: Filename` (`:72`);
  `use common::media::{ContentHash, Filename};` (`:5`).
- Trait `MediaStorage` (`:127`, `:151`) and impl (`:263`, `:336`):
  `filename: &Filename`.
- INSERT bind (`:233`): `.bind(record.filename.as_ref())`.
- `get_media` SQL bind (`:273`): `.bind(filename.as_ref())`.
- `delete_media` (`:339`):
  `DB::delete_media_row(&self.pool, user_id, sha256, filename, source.as_str())`
  — `filename` is now `&Filename`.
- `MediaDialect::delete_media_row` (`:189`): `filename: &Filename`.

**storage/src/{sqlite,postgres}/media.rs** —
`delete_media_row(..., filename: &Filename, ...)`, `.bind(filename.as_ref())`
(`:28`/`:36` in each); `use common::media::{ContentHash, Filename};`.

**storage/src/helpers.rs** — `media_record_from_row` (`:307`): parse the
filename column via Door A, mirroring the `sha256` parse (`:315`):

```rust
    let filename: Filename = filename
        .parse()
        .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
```

Add `Filename` to the `common::media` import. (`MediaRow` tuple stays
all-`String` — it is the raw DB shape.)

**common/src/atompub/entry.rs** — `MediaLinkEntry.title: Filename` (`:592`);
import `common::media::Filename`. `render_media_link_entry` (`:620`) already
does `write_text_element(&mut writer, "title", &entry.title)` — `&entry.title`
Deref-coerces to `&str` at that external `quick_xml` write (§5 carve-out); no
change to the call. Update the in-file test (`:1031`)
`title: "pic.png".to_string()` → `title: parse_filename("pic.png")` (add
`common::test_support` dev-dep import if not present).

**server/src/media_manager.rs**

- `UploadMetadata.filename: Filename` (`:42`).
- `validate_filename` (`:105`) returns `anyhow::Result<Filename>` via Door B:
  ```rust
  pub fn validate_filename(file_name: Option<&str>) -> anyhow::Result<Filename> {
      let raw_name = file_name.unwrap_or("upload");
      Filename::sanitized(raw_name)
          .map_err(|_| anyhow::anyhow!(MediaError::BadRequest("Invalid filename".to_owned())))
  }
  ```
- `upload_bytes` (`:301`) takes `filename: &Filename`; **drop** the internal
  `let filename = Self::validate_filename(Some(filename))?;` (`:309`); pass
  `filename` (a `&Filename`) to `get_content_type(Some(content_type), filename)`
  (Deref) and into the `UploadMetadata { filename: filename.clone(), .. }`.
- `register_in_db` (`:206`): `filename: &Filename`,
  `record.filename = filename.clone()`.
- Import `common::media::Filename` (`:10`).

**server/src/media.rs** — `UploadResponse.filename: Filename` (`:43`).
(Serve-route parse landed in Task 3.)

**server/src/atompub/media.rs**

- `collection_post` (`:65-70`): build via Door B _before_ the existence check —
  ```rust
  let raw_name = headers.get("slug").and_then(|v| v.to_str().ok()).unwrap_or("upload");
  let filename = Filename::sanitized(raw_name).map_err(|_| HandlerError::BadRequest)?;
  ```
  Then `get_media(auth_user.user_id, &sha, &filename, &MediaSource::Upload)` and
  `manager.upload_bytes(&auth_user, &filename, &content_type, &body)`.
- `member_get` (`:131`) and `member_delete` (`:156`): typed extractor
  `Path((username, sha, filename)): Path<(Username, ContentHash, Filename)>`;
  **delete** the in-handler
  `let sha: ContentHash = sha.parse().map_err(|_| HandlerError::NotFound)?;` —
  the extractor now rejects a malformed hash or filename with **400**.
  `get_media`/`delete_media` receive `&sha`, `&filename` (both typed).
- `media_link_entry` (`:24`): `title: record.filename.clone()` — now a
  `Filename`, direct.
- Drop the now-unused `sanitize_filename` import (`:14`); keep `ContentHash`,
  add `Filename`.

**web/src/media/mod.rs**

- `MediaItem.filename: Filename` (`:24`); import `common::media::Filename`.
- `list_my_media` (`:75-78`):
  `media_url(r.source.as_str(), &r.sha256, &r.filename)` Deref-coerces;
  `MediaItem { filename: r.filename, .. }` direct.
- `delete_media` (`:125`): `filename: Filename`; the body's
  `media_url(source_enum.as_str(), &sha256, &filename)` and
  `delete_media(auth.user_id, &sha256, &filename, &source_enum)` take
  `&Filename` unchanged.

**web/src/pages/media.rs** — `render_media_row` (`:158`, `:171`, `:181`): render
the filename via `.to_string()` for the `<a>` text and the hidden
`<input value=…>` (a `Filename` implements neither Leptos `IntoView` nor
`IntoAttributeValue`), mirroring `item.sha256.to_string()` (`:160`):

```rust
    let filename = item.filename.to_string();
```

(then `{filename.clone()}` for the link text and `value=filename` for the hidden
input).

**Add `parse_filename` (moved from Task 2)** — in `common/src/test_support.rs`,
beside `parse_content_hash`:
`pub fn parse_filename(name: &str) -> Filename { name.parse().expect("valid test filename") }`
(import `crate::media::Filename`). Its callers below are what make it covered.

**Test fixtures → `parse_filename`** (all `MediaRecord`/`MediaRow` filename
literals; assertions use `Filename: PartialEq<str>` so
`assert_eq!(x.filename, "test.jpg")` still holds):

- `storage/src/media.rs` tests: `filename: "test.jpg".to_string()` (the
  `MediaRecord` literal ~`:406`) → `parse_filename("test.jpg")`; the closed-pool
  `get_media` (`:424`) and `delete_media` (`:450`) calls `"test.jpg"` →
  `&parse_filename("test.jpg")`.
- `storage/src/helpers.rs` tests: the accept-case filename element stays a
  canonical literal (`"file.txt"`); assert `record.filename == "file.txt"`.
- `server/tests/misc/backup_fixture.rs`: the `MediaRecord` fixture filename
  (`:167`) → `parse_filename(...)`, **and** the
  `state.media.get_media(..., "photo.jpg", ...)` caller at **`:259`** →
  `&parse_filename("photo.jpg")`.
- `server/tests/web/web_media.rs` (`MediaRecord` fixtures `:128,181,237,301`) →
  `parse_filename`.
- `server/tests/storage/mod.rs`: the `make_media_record` helper (`:6855`,
  `filename: &str` param builds via `parse_filename`) **and** the direct calls
  that pass string literals — `get_media` (`:6893` `"test.jpg"`, `:6959`
  `"del.jpg"`), `delete_media` (`:6953` `"del.jpg"`, `:6985` `"ghost.jpg"`) →
  each `&parse_filename(...)`.
- `server/src/media_manager.rs` tests:
  `validate_filename(Some("test.jpg")).unwrap()` now yields a `Filename` —
  `assert_eq!(..., "test.jpg")` via `PartialEq<str>`; `upload_bytes`/`upload`
  paths pass `&parse_filename("pic.png")` / assert
  `first.filename == "pic.png"`.
- `server/tests/atompub/atompub_media.rs` — the two tests using the placeholder
  malformed hash `deadbeef` now hit the typed extractor and get a **pre-handler
  400**, so each must be repointed to a well-formed 64-hex hash to still
  exercise its intended path:
  - `get_unknown_media_returns_404` (uri `.../deadbeef/none.png`, `:171`,
    asserts 404 at `:179`): change `deadbeef` → a well-formed-but-absent hash
    (e.g. the canonical `e3b0…b855`); the handler's `NotFound` for a valid,
    unmatched hash keeps this **404** and preserves the GET-member absent→404
    coverage.
  - `member_forbids_other_user` (GET+DELETE matrix, uri
    `/atompub/bob/media/deadbeef/pic.png`, `:333`, asserts **403** at `:341`):
    change `deadbeef` → a well-formed hash so the extractor passes and
    `require_user_match` still returns **403** (the cross-user path this test
    targets).
  - **Add** `member_rejects_malformed_segment_returns_400` (authenticated as
    alice): assert `/atompub/alice/media/deadbeef/pic.png` → **400** (malformed
    hash) and `/atompub/alice/media/{valid-64-hex}/a%5Cb.png` → **400** (the
    filename decodes to `a\b.png`, non-canonical). This is spec criterion 6's
    malformed-segment→400 handler-level coverage.
- `server/tests/misc/media_handlers.rs:18` `multipart_body(filename: &str, ...)`
  stays `&str` — it builds an HTTP request body, not a `MediaRecord`.

Note: `mockall::automock` regenerates `MockMediaStorage` with the `&Filename`
signatures; mock call sites (`.expect_get_media()`, etc.) update to the new arg
type automatically.

- [x] **Step 4: Run the tests, verify they pass** (full `cargo xtask check`
      green — workspace compile, dual-backend tests incl. PostgreSQL, coverage
      clean)

Run, in order:

```
cargo nextest run -p common
cargo nextest run -p storage
cargo nextest run -p server
cargo nextest run -p web
```

Expected: PASS across all four (dual-backend storage tests included). If a
`#[server]`-gated web path is involved, also
`cargo check -p web --all-features --all-targets` (memory: default check skips
server-gated web code).

- [x] **Step 5: Commit** (56621695)

```bash
git add common/ storage/ server/ web/
git commit -m "refactor: thread Filename through storage, DTOs, handlers, and web boundary (#482)"
```

Run `cargo xtask check` foreground (`timeout: 600000`) first — storage change
forces a coverage rebuild. Verify `git status` is clean before committing
(git-add hook may auto-stage edits).

---

## Self-review

- **Spec coverage:** criterion 1 → Task 2 Step 1/3 (doctests + private field); 2
  → Task 2 (Door A/B tests); 3 → Task 2 (serde test); 4 → Task 4 (all fields
  typed); 5 → Task 3 (re-check gone, traversal tests 404); 6 → Task 3
  (serve 404) + Task 4 (atompub 400 test updates); 7 → Task 4
  (`media_record_from_row_rejects_invalid_filename`); 8 → Task 2 (idempotence
  pin); 9 → Task 4 (`upload_bytes` drops double-sanitize; `validate_filename`
  tests); 10 → Task 1 (spun-out issue); 11 → Task 2 (`parse_filename`) + Task 4
  (fixtures); 12 → each task's `cargo xtask check` gate.
- **Type consistency:** `Filename` / `InvalidFilename` / `sanitized` /
  `parse_filename` and every threaded signature in Task 4's Interfaces block
  match the producer names in Task 2.
- **No placeholders:** every implementation step carries real tests +
  signatures.
