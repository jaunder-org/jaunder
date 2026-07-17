# Spec — `Filename` newtype for media filenames (#482)

- Issue: [#482](https://github.com/jaunder-org/jaunder/issues/482)
- Milestone: 13 — Domain-value type safety (newtypes)
- Date: 2026-07-17
- Governing decision:
  [ADR-0063](../../adr/0063-domain-value-newtype-convention.md) §1 (invariant +
  trust/safety boundary), §4 (boundary rule), §5 (pervasiveness).

## Problem

A media `filename` crosses every media layer as a bare `String`, even though it
carries a real **path-traversal / header-injection safety invariant** enforced
by `common::media::sanitize_filename` — a guarantee currently upheld by
_discipline at each boundary_ (sanitize on upload; a strict
`sanitize_filename(x) != x` re-check on serve) rather than by a type. An
**unsanitized** filename is a representable state everywhere the field is
`String`. Per ADR-0063 the value earns a newtype on two axes (invariant + trust
boundary), and under §5 the type must then be threaded through every surface
that carries it.

## Decisions (resolved in the design interview)

### D1 — Two construction doors (mirrors `ContentHash`)

`Filename` is a str-newtype in `common::media` (`#[derive(StrNewtype)]`, private
`String` field) with **two** doors, exactly paralleling `ContentHash`'s
`FromStr` + `from_digest`:

- **Door A — validating `FromStr` (canonical-only).** Accepts a string **iff**
  `!s.is_empty() && sanitize_filename(s) == s`; otherwise returns
  `InvalidFilename`. It is _pure and idempotent_ — it **rejects** a
  non-canonical name, never normalizes it. This is the door for untrusted
  strings that must exactly match a stored value: serve/atompub-member URL
  segments, the DB read-back, and the serde/wire surface (the `StrNewtype`
  derive routes `Deserialize` through this `FromStr`, so a non-canonical
  filename is rejected on the wire). Its predicate is the retired serve-time
  re-check **plus a non-empty guard**: the old guard `sanitize_filename(x) != x`
  _accepts_ `x == ""` (since `sanitize_filename("") == ""`), whereas Door A
  rejects empty — so Door A is strictly stricter. The difference is unobservable
  on the routes (an empty `{filename}` segment can't match the axum serve or
  atompub-member routes), so the empty case stays a 404 there regardless.
- **Door B — normalizing producer
  `Filename::sanitized(raw: &str) -> Result<Filename, InvalidFilename>`.** Runs
  `sanitize_filename`, rejects an empty result. This is the **upload intake**
  door, where a client's arbitrary name is _meant_ to be normalized to a safe
  leaf. It preserves today's upload behavior verbatim.

`sanitize_filename` stays the single source of truth for "safe leaf"; both doors
are defined in terms of it, and the private field means those two doors are the
only ways to construct a `Filename`. No change to `sanitize_filename`'s logic
(non-goal).

Rationale for two doors over a single normalizing `FromStr`: a normalizing
`FromStr` would make serve/member/wire inputs _silently normalize_ rather than
reject, so the strict serve-time equality guard could not be cleanly retired and
a non-canonical URL segment would resolve-then-404 instead of being rejected as
invalid. The validating door makes "unsanitized filename" unrepresentable and
the re-check redundant by construction.

### D2 — `content_type`/MIME spun out

The secondary `ContentType`/MIME newtype is **out of scope**; it is filed as a
separate type-safety issue (the plan's first task). Rationale: `content_type` is
effectively free-form (the atompub POST path passes the client's raw
`Content-Type` header through unmodified), so its invariant and the
enum-vs-newtype question the issue flags need their own design — folding it in
would enlarge this diff against ADR-0063's "each value class is its own
reviewable change." `MediaRecord.content_type` and the other four content_type
fields stay `String` in this issue.

### D3 — Rejection status differs by surface: who the client is, and where the URL came from

The choice of **400 (typed extractor, automatic)** vs **404 (in-handler parse)**
for a malformed segment is _not_ a safety question for the filename —
`sanitize_filename` is total and a path-join of a leaf can't panic, so there is
no DoS/"panic-before-slice" concern here (that concern is real only for the
_hash_, which the serve route slices `hash[..2]`/`[2..4]`). It is a plain
REST-semantics choice, and it splits by surface:

- **Authenticated atompub member routes → automatic 400 (typed extractor).**
  `GET`/`DELETE /atompub/{username}/media/{sha}/{filename}` are authenticated
  and the URL is one _we_ minted in `render_media_link_entry`. A malformed
  segment is the caller's fault, so we parse at the boundary and let axum
  reject: `member_get`/`member_delete` take
  `Path<(Username, ContentHash, Filename)>`. This is the ADR-0063 §4 ideal
  (parse at the outermost boundary), deletes the manual in-handler
  `.parse().map_err(...)` for **both** segments, and yields a uniform **400**
  for any malformed segment. **Consequence:** a malformed _hash_ on these two
  routes flips from today's in-handler `.parse()→404` to **400**. That reverses
  the earlier deliberate 404 choice (and its explanatory comment) on these
  routes only; any atompub test asserting a malformed-hash→404 on the member
  routes is updated to expect 400.
- **Public serve route → friendly 404 (in-handler parse).**
  `GET /media/{source}/{p1}/{p2}/{hash}/{filename}` is unauthenticated
  content-serving hit by browsers, crawlers, and stale/hotlinked URLs, where a
  404 ("no such asset") is the natural response and a 400 reads oddly. It also
  needs an in-handler `p1`/`p2`-vs-`hash` consistency check regardless, so it
  can't collapse to a pure typed extractor. So `ServeParams.filename` **stays
  `String`**, and `validate_serve_params` parses it into `Filename` (→ 404 on
  failure), which **replaces** the
  `sanitize_filename(&params.filename) != params.filename` guard.
  `validate_serve_params`'s return tuple grows from `(MediaSource, ContentHash)`
  to `(MediaSource, ContentHash, Filename)`, and `resolve_media_path` joins the
  validated `filename.as_ref()` (mirroring how it already joins the parsed
  `hash.as_ref()`). The serve route's hash keeps its existing
  `String`-then-in-handler-parse→404 treatment unchanged.

Note: on the serve path `content_disposition(filename: &str)` and
`detect_content_type(&str)` keep receiving `params.filename` (the raw `String`).
That is not a §5 gap — the value is byte-identical to the Door-A-validated
`Filename` (canonical), and these are read-only header/table lookups at the
external HTTP boundary; passing `&str` there is the same `Deref` read-out §5's
carve-out sanctions.

### D4 — No new ADR

Applying ADR-0063 to a new value class needs no new ADR; the two-door shape is
already precedented by `ContentHash` (`FromStr` + `from_digest`). This spec is
the decision record. (dev-cycle-ship backstops if a reviewer disagrees.)

### D5 — `list_media` degrades gracefully on an undecodable row (added in review)

Added during the pre-merge code review (owner-approved scope addition). Because
`media_record_from_row` now validates the `filename` column on read (like `sha256`
and `source`), a single corrupt/tampered row would fail the whole `list_media`
query with a 500 and hide **all** of a user's media. So `list_media` **skips** an
undecodable row (logging a `warn!`) and returns the rest; the single-row lookups
(`get_media`, `find_by_hash`) stay **strict** (a direct request for a specific
corrupt row still surfaces the error). This robustness applies to all three parsed
columns, not just filename. Tested dual-backend by raw-SQL-inserting a
non-canonical filename (bypassing the validating `create_media`) and asserting
`list_media` returns only the valid row while `find_by_hash` of the corrupt row
errors.

## Scope — pervasiveness sweep (§5)

`Filename` (owned) or `&Filename` (borrowed) replaces the bare filename
`String`/`&str` at **every** surface below. `source` stays `MediaSource`; the DB
column stays `TEXT`.

**`common`**

- `common/src/media.rs` — new `Filename` type, `InvalidFilename` error,
  `FromStr`, `sanitized()`, unit tests. `sanitize_filename` unchanged.
- `common/src/test_support.rs` — new `parse_filename(&str) -> Filename` helper
  (beside `parse_content_hash`).
- `common/src/atompub/entry.rs` — `MediaLinkEntry.title: Filename` (our type;
  the field narrows to filenames-only, which is accurate — it is only ever built
  from `record.filename`). `render_media_link_entry` reads it out via
  `Deref`/`Display` **at the `write_text_element`/`quick_xml` write** — that
  write into a foreign type is the §5 external-type carve-out; `MediaLinkEntry`
  itself holds the `Filename`.

**`storage`**

- `storage/src/media.rs` — `MediaRecord.filename: Filename`;
  `MediaStorage::get_media` / `delete_media` take `filename: &Filename`;
  `MediaDialect::delete_media_row` takes `&Filename`; binds use
  `filename.as_ref()`.
- `storage/src/{sqlite,postgres}/media.rs` —
  `delete_media_row(filename: &Filename)`, `.bind(filename.as_ref())`.
- `storage/src/helpers.rs` — `media_record_from_row` parses the `filename`
  column into `Filename` via the validating `FromStr` (→ `sqlx::Error::Decode`
  on a corrupt value), exactly like the adjacent `sha256` parse.

**`server`**

- `server/src/media_manager.rs` — `UploadMetadata.filename: Filename`;
  `validate_filename(Option<&str>) -> anyhow::Result<Filename>` is the **Door
  B** intake, retained for the multipart `upload` path. **`upload_bytes` takes
  `filename: &Filename`** (already-validated) and **drops its internal
  `validate_filename` re-sanitize** — removing today's double-sanitization;
  `get_content_type(Some(ct), filename)` reads the name via `Deref`.
  `register_in_db` builds `MediaRecord` with the owned `Filename`.
- `server/src/media.rs` — `UploadResponse.filename: Filename`;
  `ServeParams.filename` stays `String`; `validate_serve_params` returns
  `(MediaSource, ContentHash, Filename)` and `resolve_media_path` joins
  `filename.as_ref()`, per D3.
- `server/src/atompub/media.rs` — `collection_post` builds the filename via
  **Door B** (`Filename::sanitized(raw_name).map_err(|_| BadRequest)?`) _before_
  the existence check, and passes `&Filename` to `upload_bytes`;
  `member_get`/`member_delete` use the typed
  `Path<(Username, ContentHash, Filename)>` extractor (**400** on any malformed
  segment, per D3); `media_link_entry` sets `title` from `record.filename`
  (direct, no conversion).

**`web`**

- `web/src/media/mod.rs` — `MediaItem.filename: Filename`; `delete_media`
  server-fn arg becomes `filename: Filename` (typed wire arg, ADR-0065 shape —
  the hash there is already `ContentHash`; ActionForm submits the canonical
  string, deserialized via Door A).
- `web/src/pages/media.rs` — `render_media_row` renders/binds the filename via
  `.to_string()` for the `<a>` text and the hidden `<input value=…>` (mirroring
  the existing `item.sha256.to_string()` treatment).

**Tests** — `parse_filename` used at every `MediaRecord`/`MediaRow` fixture and
`&str` filename test helper (storage/src, storage/helpers tests,
server/tests/{web,misc,storage,atompub}).

## Acceptance criteria (observable)

1. **Type exists, single chokepoint.** `common::media::Filename` exists with a
   private field; the only constructors are the validating `FromStr` (Door A)
   and `sanitized()` (Door B), both routing through `sanitize_filename`. A
   compile-fail doc-test proves the field is private and a bare `String` cannot
   masquerade as a `Filename` (as `ContentHash` does).
2. **Invariant enforced (unit).** Door A: `"foo.txt".parse::<Filename>()`
   succeeds; `"".parse()`, `"..".parse()`, `"a/b".parse()`, `"../x".parse()`,
   `"foo\0".parse()` all return `Err(InvalidFilename)` (a non-canonical or empty
   name is rejected, not normalized). Door B:
   `Filename::sanitized("../../etc/passwd")` == `"passwd"`;
   `Filename::sanitized("..")` and `Filename::sanitized("")` return `Err`.
3. **Serde rejects on the wire (unit).** A `Filename` serializes as a plain JSON
   string; deserializing a non-canonical string (`"../x"`) is an error.
4. **Unsanitized filename unrepresentable.** `MediaRecord.filename`,
   `MediaLinkEntry.title`, `UploadResponse.filename`, `UploadMetadata.filename`,
   `web::MediaItem.filename`, and the atompub member handlers all hold
   `Filename`; there is no code path that puts a raw,
   un-run-through-`sanitize_filename` string into any of them (verified by the
   type + the sweep).
5. **Serve-time re-check retired.** The
   `sanitize_filename(&params.filename) != params.filename` comparison in
   `server/src/media.rs` is gone; a non-canonical serve-path filename is
   rejected because it fails to parse into `Filename`. Existing serve-path
   traversal tests
   (`resolve_media_path_rejects_filename_with_traversal_or_separators`) still
   pass (→ 404).
6. **Rejection status by surface (regression-tested at the handler level).**
   - Serve route `GET /media/.../{filename}`: a malformed filename → **404**
     (unchanged); the hash's existing malformed→404 also unchanged.
   - atompub `member_get`/`member_delete`: with the typed
     `Path<(Username, ContentHash, Filename)>` extractor, a malformed **filename
     or hash** segment → **400**. Any existing atompub test asserting a
     malformed-hash→404 on these routes is updated to expect 400.
7. **DB read-back accepts canonical, rejects corrupt.** `media_record_from_row`
   parses the `filename` column via Door A; a valid (canonical) column value
   round-trips, and a non-canonical one yields `sqlx::Error::Decode` — covered
   by a new `media_record_from_row_rejects_invalid_filename` test mirroring the
   existing `_rejects_invalid_sha256`.
8. **Idempotence pinned.** A regression test asserts that any non-empty
   `sanitize_filename(x)` output re-parses through Door A
   (`sanitize_filename(x).parse::<Filename>().is_ok()`) over the sanitize test
   corpus — so a future change to `sanitize_filename` that broke idempotence
   (silently turning valid DB rows into `Decode` errors) fails loudly here.
9. **Upload behavior unchanged.** atompub POST and the multipart `upload` path
   still normalize a client name to a safe leaf (Door B) and reject an
   empty-after-sanitize name as before; `upload_bytes` no longer
   double-sanitizes (`validate_filename` tests preserved, now returning
   `Filename`).
10. **content_type out of scope, filed.** A new GitHub issue for the
    `ContentType`/MIME newtype exists (linked from #482); no `content_type`
    field changes type in this branch.
11. **Test convention.** `common::test_support::parse_filename` exists and is
    used at media test fixtures instead of inline `.parse().unwrap()`.
12. **Gate green.** `cargo xtask validate --no-e2e` passes with no coverage
    regression.

## Non-goals

- No change to `sanitize_filename` / `detect_content_type` logic.
- No `ContentType`/MIME newtype (spun out).
- No change to the `media` table schema, the on-disk layout, or the URL shapes.
- No change to `MediaSource` typing or the `source` argument surfaces.
