# Plan — `RootRelativeUrl` for the media serve-URL chain (#675)

Spec:
[`docs/archive/2026-07-29-issue-675-rootrelativeurl-media-spec.md`](2026-07-29-issue-675-rootrelativeurl-media-spec.md)
(approved). The spec is "what/why"; this is "how". Decisions are referenced as
**D1**–**D9** and not restated.

## Review header

**Goal.** Type the media serve URL as `RootRelativeUrl` end to end, and fix the
latent malformed-URL defect that adoption exposes: filenames may contain
whitespace, `?` and `#`, so the derived URL is currently either unrepresentable
as the newtype or silently mis-addressed. Percent-encode the filename segment
inside `media_path` so the URL path and on-disk path stay byte-identical (D2).

**Scope — in:** `media_path`/`media_url` encoding + typed args;
`resolve_media_path` unification; `UploadResponse.url`, `MediaItem.url`, the web
component signal, the AtomPub boundary; `MediaRecord.source_url` →
`Option<AbsoluteUrl>`; regression tests; an ADR for the naming correspondence.

**Scope — out:** capping `Filename`'s length (D2b — filed as a follow-up in T1);
building the remote-media caching ingest that would populate `source_url` (D9
types the field only); #692 (`ContentType`/`Filename` through the media path),
which is blocked on this and comes next.

**Tasks.**

1. File the `Filename` length-cap follow-up (D2b).
2. `MEDIA_SEGMENT` encode set; `media_path` encodes; both fns take
   `&MediaSource`/`&Filename`.
3. Route `resolve_media_path` through `media_path` (D2a).
4. `media_url` → `RootRelativeUrl`, and the DTO/call-site cascade (one atomic
   commit).
5. Web component holds `Option<RootRelativeUrl>`; stringify at the view.
6. `MediaRecord.source_url` → `Option<AbsoluteUrl>` (dual-backend).
7. Write-then-serve + end-to-end coverage for names needing encoding.
8. ADR draft: the URL / on-disk / DB naming correspondence.

**Key risks and decisions.**

- **T2 and T3 must land together in effect** — encoding writes without encoding
  reads breaks serving. T2 changes the writer, T3 the reader; T2 alone leaves
  the tree green but functionally wrong for encoding-needing names, so T3's
  round-trip test is what proves the pair. Do not stop between them.
- **T4 is the one large commit.** Changing `media_url`'s return type cascades to
  every consumer; splitting it would need throwaway `.to_string()` scaffolding,
  which the no-placeholders rule forbids. It is enumerated file-by-file below.
- **`NON_ALPHANUMERIC` must not be used bare** (D3) — it encodes `.`/`-`/`_` and
  would mangle every ordinary filename. T2's first test pins that.
- Coverage: the `unreachable!` arm in T4 follows the `AbsoluteUrl::compose`
  idiom and may need a `cov:ignore` marker; check the coverage gate rather than
  inventing a test for an unreachable branch.

**For agentic workers.** Execute with **`jaunder-iterate`**, delegating
individual tasks via **`jaunder-dispatch`** where useful. Tick checkboxes in
this file as you go.

## Global constraints

- Run `cargo xtask check` before each commit (the pre-commit hook runs it
  anyway; running it first leaves a clean staged state). See
  **`jaunder-commit`**. **No `Co-Authored-By` trailer.**
- The media component is wasm-only, so the host `clippy` never sees it. Lint it
  with `cargo xtask check --no-test`, whose `wasm-clippy` step carries the right
  flags. **Corrected mid-execution:** an earlier draft of this line said
  `cargo clippy -p web --target wasm32-unknown-unknown --all-features`, which
  fails in `mio` — `--all-features` pulls the server/tokio-net feature set onto
  the wasm target. The real invocation is
  `-p web -p client -p csr --features csr --target wasm32-unknown-unknown` (see
  `xtask/src/steps/static_checks.rs:58-90`); prefer the xtask step over retyping
  it.
- Storage tests are dual-backend per ADR-0053 / `CONTRIBUTING.md`; a bare
  `#[tokio::test]` that should be dual-backend fails the `test-backend-pattern`
  guard.
- No `.parse().expect()`; no new `from_trusted` door; `EXEMPT_QUALIFIERS`
  unchanged (D4).

---

## T1 — File the `Filename` length-cap follow-up

- [x] Filed as **#708** (Task, `type-safety`, milestone 13, P1 — a valid name
      failing the write is a behavior bug per the triage rubric; blocked-by
      #675).

Per D2b, encoding lowers the effective filename-length ceiling (a ~90-character
name of mostly spaces can exceed the 255-byte per-component filesystem limit).
Out of scope here because capping `Filename` changes upload semantics.

Use **`jaunder-issues`**. Body must state: the expansion factor (3× ASCII, up to
9× for multi-byte UTF-8), that it is a narrowing of an existing failure mode
rather than a new one, that the write surfaces as an IO error at
`storage/src/media_manager.rs:283`, and that the fix is a length bound on
`Filename`'s `FromStr`/`sanitized` doors with a decision needed on
truncate-vs-reject. Label `type-safety`; milestone #13.

## T2 — `media_path` percent-encodes the filename; both fns take typed args

- [x] Done — `b8135f95`. Verified the encoding test genuinely goes red first
      (reverted the encode line, saw `a b.txt must encode to a%20b.txt` FAIL,
      restored). **Scope added:** `encode_filename_segment` is public, because
      `server/src/atompub/media.rs` interpolated the filename raw into the media
      member URL too — the same defect, and that URL is the entry's `atom:id`.
      Both now share one set.

**Files**

- `common/Cargo.toml` — add `percent-encoding = { workspace = true }`.
- `common/src/media.rs` — `MEDIA_SEGMENT`, `media_path`, `media_url` signatures,
  module docs.
- Test: in-file `#[cfg(test)] mod tests` (this file's existing convention).

**Interfaces**

```rust
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

/// The percent-encode set for a media path segment: everything `NON_ALPHANUMERIC`
/// encodes, minus the RFC 3986 *unreserved* marks.
///
/// Keeping `-._~` unencoded is what makes an ordinary name round-trip byte-identical
/// (`photo.jpg` stays `photo.jpg`), which is the whole point of D2 — the on-disk name
/// must stay greppable and paste-able from a URL. Bare `NON_ALPHANUMERIC` would yield
/// `my%2Dphoto%2Ejpg`.
const MEDIA_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub fn media_path(source: &MediaSource, sha256: &ContentHash, filename: &Filename) -> String;
pub fn media_url(source: &MediaSource, sha256: &ContentHash, filename: &Filename) -> String;
```

`media_path` becomes
`format!("{source}/{p1}/{p2}/{sha256}/{}", utf8_percent_encode(filename, MEDIA_SEGMENT))`
(`source` via `MediaSource`'s `Display`/`AsRef<str>`; `filename` derefs to
`str`). `media_url` is unchanged in body and still returns `String` — T4 changes
its type.

Update the module docs (`common/src/media.rs:10`, `:25`) and both fn docs to
state that the **filename segment is percent-encoded on disk and in the URL, and
the two are byte-identical**, while the DB `filename` column keeps the raw
display name. Say why, so a later reader does not "simplify" the encoding away.

**Callers to update** (mechanical, argument types only):
`storage/src/media_manager.rs:283` and `:314`, `server/src/atompub/media.rs:30`,
`web/src/media/api.rs:88` and `:141`. The literal `"upload"` sites become
`&MediaSource::Upload`; `web`'s `r.source.as_ref()` becomes `&r.source`.

**Tests** (in `common/src/media.rs`)

```rust
#[test]
fn media_path_leaves_ordinary_names_byte_identical() {
    // D3's unreserved-mark carve-out. Without it every filename on disk is mangled.
    for name in ["photo.jpg", "my-photo_2.png", "a~b.txt", "IMG1234.JPEG"] {
        let hash = parse_hash();
        let filename: Filename = name.parse().expect("canonical leaf");
        let path = media_path(&MediaSource::Upload, &hash, &filename);
        assert!(path.ends_with(name), "{name} must not be encoded: {path}");
    }
}

#[test]
fn media_path_encodes_whitespace_and_url_structural_characters() {
    // Space: unrepresentable as RootRelativeUrl. `?`/`#`: silently mis-addressing.
    let hash = parse_hash();
    for (raw, encoded) in [
        ("a b.txt", "a%20b.txt"),
        ("what?.png", "what%3F.png"),
        ("a#b.png", "a%23b.png"),
        ("50%.png", "50%25.png"),
        ("café.png", "caf%C3%A9.png"),
    ] {
        let filename: Filename = raw.parse().expect("canonical leaf");
        let path = media_path(&MediaSource::Upload, &hash, &filename);
        assert!(path.ends_with(encoded), "{raw} → {path}, wanted …{encoded}");
    }
}
```

**Run**

- `cargo nextest run -p common media_path` — the two new tests FAIL before the
  change, PASS after. The pre-existing `media_path_computation` /
  `media_url_computation` tests (`:585`, `:592`) need their argument types
  updated but must keep asserting the same output, since `photo.jpg` is
  unaffected.
- `cargo xtask check`, then commit.

## T3 — Route `resolve_media_path` through `media_path` (D2a)

- [x] Done — `189bc270`. All eight `resolve_media_path` tests pass, including
      the pre-existing traversal and `p1`/`p2`-mismatch rejections through the
      new path.

**Files**

- `server/src/media.rs` — `resolve_media_path` (`:236-253`) and its in-file
  tests.

**Interfaces**

Replace the hand-rolled join chain with the shared producer, so the read path
cannot disagree with the write path:

```rust
let file_path = storage_path
    .join("media")
    .join(media_path(&source, &hash, &filename));
```

`validate_serve_params` is unchanged — it still checks that the URL's `p1`/`p2`
match the hash (`:223`), which `media_path` cannot do because it derives them.
Keep the existing comment's intent: the path is built from the _parsed_ values,
not `params.*`.

Note for the implementer: axum has already percent-**decoded** the incoming
segment (finding 6), so `filename` here is the raw name and `media_path`
re-encodes it — that round-trip is exactly what makes read and write agree.

**Tests** (in `server/src/media.rs`)

```rust
#[test]
fn resolve_media_path_matches_the_writer_for_a_name_needing_encoding() {
    // The D2a regression: a hand-rolled read path would join the *decoded* name and
    // miss the file the writer stored under the encoded one.
    let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let p = params("upload", "e3", "b0", hash, "a b.txt");
    let (_source, _hash, _filename, path) =
        resolve_media_path(Path::new("/data"), &p).expect("valid params");
    assert!(
        path.ends_with("a%20b.txt"),
        "read path must match the encoded name on disk: {path:?}"
    );
}
```

`resolve_media_path_builds_path_for_valid_params` (`:365`) keeps passing
unchanged — `photo.jpg` encodes to itself.

**Run**

- `cargo nextest run -p jaunder resolve_media_path` — new test FAILS before,
  PASSES after.
- `cargo xtask check`, then commit.

## T4 — `media_url` → `RootRelativeUrl` and the consumer cascade

- [x] Done — `79bdddcd`, **together with T5**. The plan had them separate; that
      was wrong. `component.rs` is wasm-only, so the host compiler never
      type-checks its signal, callback prop, or view helpers — T4 alone would
      have left the wasm build broken and the host gate green. Only
      `wasm-clippy` catches it. Coverage needed no `cov:ignore` for the
      `unreachable!` arm after all.

The type change and its consumers must land together: an intermediate state
would need `.to_string()` scaffolding at every DTO boundary.

**Files**

- `common/src/media.rs` — `media_url` return type; `UploadResponse.url` (`:463`)
  and its doc.
- `storage/src/media_manager.rs:314` — the `UploadResponse` construction; `:713`
  test.
- `web/src/media/api.rs` — `MediaItem.url` (`:45`), construction at `:88`, and
  `:141`.
- `server/src/atompub/media.rs:30-33` — hold the newtype to the
  `atom_syndication` call and read it out there via `Deref`/`Display` (D8); do
  not store it flat.
- `server/tests/web/web_media.rs:221,244` — embeds the value in a markdown body.

**Interfaces**

```rust
/// Returns the root-relative serve URL, `"/media/<source>/<p1>/<p2>/<sha256>/<filename>"`,
/// with the filename segment percent-encoded (see [`media_path`]).
#[must_use]
pub fn media_url(
    source: &MediaSource,
    sha256: &ContentHash,
    filename: &Filename,
) -> RootRelativeUrl {
    let path = format!("/media/{}", media_path(source, sha256, filename));
    let Ok(url) = path.parse() else {
        // Infallible: `media_path` percent-encodes the only caller-influenced segment,
        // so the result is a single-slash, host-less path with no whitespace, control
        // characters, `?` or `#`. Same shape as `AbsoluteUrl::compose`.
        unreachable!("media_url builds a valid root-relative path");
    };
    url
}
```

D4: no `from_trusted` door, no `.expect()`. `UploadResponse.url` and
`MediaItem.url` become `RootRelativeUrl`; strike the "plain derived string …
carried verbatim" note from `UploadResponse`'s doc
(`common/src/media.rs:455-456`) — it recorded the flatten without justifying it.

**Tests**

```rust
#[test]
fn media_url_is_a_valid_root_relative_url_for_a_name_with_a_space() {
    let hash = parse_hash();
    let filename: Filename = "a b.txt".parse().expect("canonical leaf");
    let url = media_url(&MediaSource::Upload, &hash, &filename);
    assert!(!url.contains(' '), "no raw space may survive: {url}");
    assert!(url.starts_with("/media/upload/"), "{url}");
}

#[test]
fn media_url_does_not_truncate_at_a_query_or_fragment_character() {
    // finding 4: `?` passes RootRelativeUrl's validation, so only encoding prevents the
    // path from silently addressing a different file.
    let hash = parse_hash();
    for raw in ["what?.png", "a#b.png"] {
        let filename: Filename = raw.parse().expect("canonical leaf");
        let url = media_url(&MediaSource::Upload, &hash, &filename);
        assert!(!url.contains('?') && !url.contains('#'), "{raw} → {url}");
    }
}
```

**Run**

- `cargo nextest run -p common media_url` — new tests FAIL before, PASS after.
- `cargo nextest run -p storage`, `cargo nextest run -p web`,
  `cargo nextest run -p jaunder` — the cascade compiles and existing assertions
  hold.
- If the coverage gate flags the `unreachable!` arm, mark it per the repo's
  `cov:ignore` convention rather than writing a test for an unreachable branch.
- `cargo xtask check`, then commit.

## T5 — Web component holds `Option<RootRelativeUrl>`

- [x] Done — folded into `79bdddcd` (see T4). The callback prop became
      `Callback<RootRelativeUrl>` too: its only caller is in the same file and
      discards the value, so typing it cost nothing and keeps a future consumer
      from getting a stringly URL.

**Files**

- `web/src/media/component.rs` — signal at `:40`, set at `:72`, read at `:94`
  and `uploaded_url_view`.

`RwSignal::new(Option::<RootRelativeUrl>::None)`. A newtype is not `IntoRender`,
so stringify at the view site per the existing idiom in
`web/src/taglist/component.rs`.

**Run**

- `cargo nextest run -p web media`
- `cargo clippy -p web --target wasm32-unknown-unknown --all-features -- -D warnings`
  (wasm-only file — the host build will not catch its errors).
- `cargo xtask check`, then commit.

## T6 — `MediaRecord.source_url` → `Option<AbsoluteUrl>` (D9)

- [x] Done — `73bd38d`. **Two** dual-backend tests, not one: the planned
      normalization round-trip, plus a raw-SQL-inserted `'not a url'` that must
      fail to decode. The first alone would not have shown the bridge
      _validates_ — normalization happens at parse time, so a raw-text column
      would pass it too. The bind needed an explicit
      `for<'q> Option<AbsoluteUrl>: Encode + Type` bound (replacing
      `Option<String>`); the newtype's own impls follow from the existing
      `String` bounds, but the `Option` wrapper must be named.

**Files**

- `storage/src/media.rs` — the field (`:29`), the INSERT bind (`:192`), the
  SELECT column lists (`:222`, `:255`, `:269`, `:341`).
- `storage/src/helpers.rs:287,306` — the row-tuple destructure and `MediaRecord`
  build.
- `None`-literal construction sites need no change:
  `storage/src/media_manager.rs:252`, `storage/src/media.rs:390,526,571`, and
  the test fixtures.

`#[derive(StrNewtype)]` already provides the validating sqlx bridge, so the row
tuple's column type becomes `Option<AbsoluteUrl>` and the bind takes the
newtype. Update the field doc: still "for cached media, the original remote
URL", plus that the type is the ingest contract for the not-yet-written caching
path, and that an unparseable value is useless by definition because caching
means fetching it.

**Tests** — dual-backend per ADR-0053, in the storage integration suite
alongside the existing media coverage (`server/tests/storage/mod.rs`): insert a
`MediaRecord` with `Some("https://Example.COM:443/x.png".parse()?)` and assert
it reads back **normalized** to `https://example.com/x.png`, proving the bridge
validates and normalizes rather than storing the raw text. Follow the file's
existing dual-backend macro/template — do not write a bare `#[tokio::test]`.

**Run**

- `cargo nextest run -p jaunder --test integration source_url` (both backends).
- `cargo xtask check`, then commit.

## T7 — Write-then-serve and end-to-end coverage

- [x] Done — `dbfa184d`. `media.spec.ts` (7 passed) and `atompub.spec.ts` (3
      passed) run locally via `cargo xtask e2e-local`. The AtomPub assertion
      targets `href`/`src` attributes rather than the whole body: `<title>`
      legitimately carries the raw display name, so a blanket "no whitespace"
      check would have been wrong. Note: bare `cargo nextest` reports a false
      `ConnectionRefused` for the postgres cases — it does not provision the
      database; the xtask gate does.

Unit tests prove the functions agree; this proves the _bytes on disk_ agree with
the _URL served_, which is the property D2/D2a actually buy.

**Files**

- `server/tests/web/web_media.rs` — upload a file whose name needs encoding,
  then GET the returned `url` and assert `200` plus the body. This is the test
  that would catch a future re-divergence of the read and write paths.
- `end2end/tests/media.spec.ts` — add a spaced-filename case beside the existing
  upload assertions (`:26,29`), asserting the returned URL contains `%20` and
  that fetching it serves the bytes.
- `end2end/tests/atompub.spec.ts` — extend the media-entry assertion (`:175`) so
  the emitted `href` contains no raw space.

Existing assertions on `/media/upload/` keep passing; none used a name needing
encoding, which is finding 8 — the reason the defect survived.

**Run**

- `cargo nextest run -p jaunder --test integration web_media`
- `cargo xtask e2e-local media.spec.ts` (see the local-e2e note in
  `CONTRIBUTING.md`).
- `cargo xtask check`, then commit.

## T8 — ADR draft: media URL / on-disk / DB naming correspondence

- [x] Done — drafted at `docs/adr/0080-media-path-naming-correspondence.md`,
      referenced from `common/src/media.rs`'s module docs. The draft itself is
      **not committed**: the drafts pen is gitignored so a number is never
      picked by hand; `cargo xtask adr promote` numbers it and rewrites the
      reference at ship.

No existing ADR owns the media storage layout — it lives only in
`common/src/media.rs` module docs, and the three-way correspondence this issue
establishes is a durable constraint that future code must respect (anyone adding
a consumer of the layout has to go through `media_path`).

Use **`jaunder-adr`**: author a **numberless** draft in `docs/adr/drafts/`,
promoted by `cargo xtask adr promote` at ship (`jaunder-ship` step 6) — never
numbered by hand.

Record: the layout; that the filename segment is percent-encoded with the
unreserved marks preserved, so ordinary names are unchanged; that the URL path
and disk path are byte-identical **by construction** because both derive from
`media_path`, and why that matters operationally (paste a URL tail, find the
file; no shell quoting; no Unicode normalization mismatch); that the DB column
keeps the raw display name, so DB → disk is the one remaining derivation; and
D2b's accepted length-ceiling narrowing with a pointer to T1's follow-up issue.
Reference ADR-0063 (newtype convention) and ADR-0073 (`url` normalization) as
related, not superseded.

**Run** — `cargo xtask check` (the `adr-format` step validates draft structure),
then commit.

## Self-review

- Every task is one commit and leaves the tree green, except the T2/T3 pair
  noted in the review header, where T2 alone is green but functionally
  incomplete.
- No task introduces scaffolding a later task removes; that is why T4 is
  deliberately large.
- Spec coverage: D1 → T2/T4; D2 → T2; D2a → T3; D2b → T1 + T8; D3 → T2; D4 → T4;
  D5 → T2; D6 → T4; D7 → T5; D8 → T4; D9 → T6. Acceptance criteria 1–7 → T2, T3,
  T4, T6, T4, T2/T3/T4/T7, and the per-task gate runs.
