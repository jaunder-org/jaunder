# Spec — Issue #495: `ContentType` newtype for media content types

## Context

Media `content_type` crosses every media layer as a bare `String`, though it has
`type/subtype` structure and feeds the HTTP `Content-Type`/`Content-Disposition`
headers and the Atom media-link `type=` attribute. Per ADR-0063 §1 it is a
domain-value newtype candidate (invariant: a valid media-type header value);
under §5 the type must be threaded through every surface that carries it. Spun
out of #482 (`Filename`), which is the direct template — `ContentType` mirrors
it exactly.

## Decisions

### D1 — A validating **str-newtype**, not an enum.

The atompub POST path accepts an **arbitrary** client `Content-Type` header, so
a closed enum would reject valid uploads or need a catch-all. `ContentType` is a
`#[derive(…, StrNewtype)]` newtype (ADR-0063 default trailer:
`Display`/`AsRef`/`Borrow`/
`Deref<str>`/`From<Self> for String`/`PartialEq`/validating serde bridge/**sqlx
bridge**) plus a hand-written validating `FromStr` and a `thiserror`
`InvalidContentType`. It lives in `common/src/media.rs` beside
`Filename`/`detect_content_type`. Derives `Clone, Debug, PartialEq, Eq, Hash`
(matching `Filename`).

### D2 — Validation: structural media-type, parameters allowed, verbatim.

`FromStr` accepts a string iff it is a well-formed media type: a non-empty
`type` token `"/"` a non-empty `subtype` token (RFC 7230 token chars),
optionally followed by `;`-separated parameters. **Every** character of the
whole string — including the parameter portion after `;` — must be **visible
ASCII (0x21–0x7E) or space/tab**, i.e. a valid `HeaderValue` byte; a control
byte or non-ASCII byte anywhere (even inside a parameter value) is rejected. The
value is stored **verbatim** — no case-folding, no parameter normalization.
Rationale: this admits every real value (`image/png`,
`text/html; charset=utf-8`, the 13 `detect_content_type` outputs) while
rejecting the malformed (`""`, `garbage` (no slash), `a/` / `/b` (empty token),
a control byte anywhere). Because every accepted byte is a valid
`HeaderValue`/Atom-attribute byte and the value is non-empty, a `ContentType` is
**always** constructible as an HTTP header value and Atom `type=` value — the
invariant D4's dead-fallback removal rests on, and which AC1 tests directly via
`HeaderValue::from_str`.

### D3 — Doors: `detect_content_type` mints in-module; client input validates.

- `detect_content_type(filename) -> ContentType` mints the private tuple
  **directly** from its canonical `&'static str` table (and the
  `application/octet-stream` fallback) — the same in-module minting `render()`
  uses for `RenderedHtml`. No `expect`/`unwrap` (both `deny`), no public trusted
  door. A unit test asserts every table entry + the fallback also satisfies
  `FromStr`, proving the minted literals are genuinely valid.
- `MediaManager::get_content_type(client: Option<&str>, filename) -> Result<ContentType, InvalidContentType>`
  is the **single validating door**, shared by both intake paths: `Some(c)` →
  `c.parse()` (validating); `None` → `detect_content_type(filename)`
  (infallible). Its callers — the multipart `upload` (`media_manager.rs:79`) and
  `upload_bytes` (`:301`, the atompub path) — propagate the error with `?`.
- The atompub boundary (`server/src/atompub/media.rs:71`) keeps its existing
  default (an absent or non-UTF-8 request `Content-Type` → the `&str`
  `"application/octet-stream"`) and passes that `&str` as `Some(...)` to
  `upload_bytes`/`get_content_type` — it does **not** separately parse (no
  double-validation; `get_content_type` is the one door).
- **Deliberate behavior change (both paths):** a malformed _present_ client
  `Content-Type` (a UTF-8 header that is not a valid media type, e.g. `garbage`)
  is now **rejected** at the single door rather than stored — for the multipart
  `/media/upload` path as well as the atompub POST — satisfying "invalid not
  representable". (Absent/non-UTF-8 still defaults to octet-stream on the
  atompub path, unchanged.)

### D4 — §5 read-out carve-outs and the dead fallback.

External boundaries read the value out via `Deref`/`AsRef` (the sanctioned §5
carve-out); their signatures stay `&str` so `&ContentType` deref-coerces at the
call site with no `.as_ref()`:

- `should_inline(&str)` and `content_disposition(&str, …)` — unchanged
  signatures.
- `HeaderValue::from_str(content_type.as_ref())` and the Atom
  `("type", entry.content_type   .as_ref())` quick-xml emission. Because D2
  guarantees header-validity, the
  `HeaderValue::from_str(&content_type) .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"))`
  fallback at `server/src/media.rs:178` becomes **unreachable**. Replace it with
  an invariant-justified read-out (a single-line `warn!`-free construction); the
  exact form (a `cov:ignore`d error arm mapped to 500, matching the sibling
  `etag`/`disposition` lines at :187/:191) is a plan detail. Do **not** keep a
  live-but-dead fallback.

### D5 — Scope boundary: media content types only.

Only the **five media fields** and the media producer/consumer path are in
scope. Explicitly **out of scope**, though they share the name:

- The Atom **content element** `type` (`"text"`/`"html"`/`"xhtml"`) in
  `common/src/atompub/entry.rs` (`AtomContent`/`content_type` at lines
  ~136/319/497/645) — a different concept; untouched.
- **Feed** content types (`FeedCacheRow.content_type` = `application/rss+xml`
  etc. in `storage/src/feed_cache.rs`, `server/src/feed/*`) — a separate newtype
  candidate, not this issue.

### D6 — Test fixtures via a shared helper.

Add `common::test_support::parse_content_type(&str) -> ContentType` (mirroring
`parse_filename`/`parse_content_hash`) and use it at every `cfg(test)`/fixture
site that sets `content_type` (per the newtype test-helper convention), instead
of inline `.parse().unwrap()`.

## Surfaces (the §5 sweep)

**Five struct fields → `ContentType`:**

1. `MediaRecord.content_type` — `storage/src/media.rs:76`
2. `UploadMetadata.content_type` — `server/src/media_manager.rs:41`
3. `UploadResponse.content_type` — `server/src/media.rs:44`
4. `MediaItem.content_type` — `web/src/media/api.rs:27` (field `:31`) — **not**
   `mod.rs` (ADR-0070: mod.rs is wiring-only)
5. `MediaLinkEntry.content_type` — `common/src/atompub/entry.rs:601`

**Producer:** `detect_content_type` (`common/src/media.rs:256`, now returns
`ContentType`) — its callers must unify: `get_content_type`
(`media_manager.rs:112`) and the **serve path**
`map_or_else(|| detect_content_type(&params.filename).to_owned(), |r| r.content_type)`
(`server/src/media.rs:161`) — once both arms are `ContentType`, drop the
`.to_owned()`. atompub client header (`server/src/atompub/media.rs:71`, see D3).

**Consumer read-out (via `Deref`/`AsRef`):** `should_inline`
(`common/src/media.rs:236`); `content_disposition` + `HeaderValue::from_str`
(`server/src/media.rs:178,300`); Atom `type=`
(`common/src/atompub/entry.rs:626`); the Leptos view cell
`<td>{item.content_type…}</td>` (`web/src/media/component.rs:355`) —
`ContentType` is not `IntoView`, so read out via `.to_string()`/deref. Test
`server/src/media.rs:596` compares `detect_content_type(...)` against an
`Option<&str>` header — now needs `.as_ref()`.

**DB:** bind `storage/src/media.rs:241` (`.bind(record.content_type.as_str())` →
`.bind(&record.content_type)`); decode via `media_record_from_row`/`MediaRow`
(`storage/src/helpers.rs`) — the column decodes straight into `ContentType`
(validating `Decode`; a legacy row that fails the invariant becomes a
`ColumnDecode` error, the #438 tradeoff).

## Acceptance criteria

- **AC1** A `ContentType` newtype exists in `common/src/media.rs`
  (`#[derive(…, StrNewtype)]`
  - validating `FromStr` + `InvalidContentType`), with the D2 validation:
    unit-tested to accept `image/png`, `text/html; charset=utf-8`, and every
    `detect_content_type` output, and to reject `""`, `"garbage"` (no slash),
    `"a/"`/`"/b"` (empty token), a top-level control-char value, **and a
    control/non-ASCII byte inside the parameter portion** (e.g.
    `"text/plain; x=\x01"`). A further test asserts **every accepted value is
    `HeaderValue::from_str`-constructible** (the D4 invariant, observed not
    assumed).
- **AC2** An invalid content type is **not representable** in `MediaRecord`,
  `UploadMetadata`, `UploadResponse`, `MediaItem`, or `MediaLinkEntry` — all
  five fields are `ContentType`; the wire (serde) and DB (`Decode`) entry points
  route through the validating `FromStr` (the private field admits no arbitrary
  `String`).
- **AC3** `detect_content_type` returns `ContentType`, minted in-module, with no
  `expect`/`unwrap`; a test asserts every canonical literal + the fallback also
  re-parses via `FromStr`.
- **AC4** The single `get_content_type` door validates client-supplied
  `Content-Type` on **both** intake paths (atompub POST and multipart
  `/media/upload`): a malformed present header is rejected (a test asserts it
  does not produce a stored media row / surfaces the error), where it was
  previously stored. The atompub absent/non-UTF-8 → octet-stream default is
  preserved.
- **AC5** `detect_content_type`/`should_inline` classification logic is
  byte-unchanged (their match arms and the extension table are untouched; only
  `detect`'s return type changes).
- **AC6** DB bind is typed (`.bind(&record.content_type)`, no `.as_str()`); the
  `media.content_type` column decodes straight into `ContentType`. The
  `sqlx-newtype-bind` gate passes.
- **AC7** The `server/src/media.rs:178` octet-stream fallback is
  removed/`cov:ignore`d (no live dead branch); serve still emits the correct
  `Content-Type` header. The serve-path `detect` unify (`:161`, drop
  `.to_owned()`), the `:596` test (`expected.as_ref()`), and the
  `component.rs:355` view read-out are updated; the `serve_response_*` tests
  pass.
- **AC8** Out-of-scope `content_type`s (Atom content element type; feed content
  types) are untouched (not in the diff).
- **AC9** Test/fixture sites construct via
  `common::test_support::parse_content_type`.
- **AC10** `cargo xtask validate --no-e2e` clean — no coverage/CRAP regression.

## Verification

- Unit tests on `ContentType` (AC1/AC3) in `common/src/media.rs`.
- Storage round-trip (SQLite + Postgres) exercises bind/decode (AC6); an
  integration/unit test for the atompub reject path (AC4).
- `sqlx-newtype-bind` gate + `validate --no-e2e` (AC6/AC10).
