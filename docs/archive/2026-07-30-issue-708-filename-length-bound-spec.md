# Spec — bound `Filename`'s length so a valid name cannot fail the encoded write (#708)

- Status: awaiting approval
- Issue: [#708](https://github.com/jaunder-org/jaunder/issues/708) (milestone
  #13, P1)
- Follows: #675 / ADR-0080, which established the percent-encoded media path and
  accepted this as documented risk D2b.

## Problem

`Filename` has no length bound. Since #675 the media path segment is
percent-encoded, so the **stored** name is up to 3× the accepted one for ASCII
specials and up to 9× for multi-byte UTF-8 (`café` → `caf%C3%A9`, a 4-byte emoji
→ 12 bytes). A name that passes validation can therefore exceed the filesystem's
255-byte per-component limit and fail the write — surfacing as an `anyhow` IO
error mapped to `MediaError::Internal` → a 500, with no statement of the actual
cause.

A ~40-character CJK name is ~120 raw bytes (fine before #675) and ~360 encoded
(fails now).

## What the survey found

1. **The house pattern cannot express this bound.** Every bounded string newtype
   here — `Slug`, `Bio`, `DisplayName`, `PostSummary`, `SessionLabel` — uses a
   `MAX_*_CHARS` const with `trimmed.chars().count()`. A char count cannot bound
   encoded bytes: set safely (255/9 ≈ 28 chars) it is absurd for ASCII; set
   generously it does not protect the write. **The budget must be the computed
   encoded byte length.**
2. **`slug.rs` is the precedent for the truncating half.** It pairs a strict
   `FromStr` that rejects over-length (`slug.rs:48`) with a normalizing
   producer, `slugify_title`, that truncates **on a grapheme-cluster boundary**
   so a base scalar is never split from its combining marks (`slug.rs:104-115`).
   `Filename` has the same two-door shape.
3. **Intake happens strictly before streaming.**
   `MediaManager::validate_filename` (`media_manager.rs:139-143`) runs
   `Filename::sanitized` on the multipart `file_name()` before any byte is read
   — its doc says so — and `upload_inner` receives an already-typed `&Filename`
   (`:105`). The `AtomPub` `Slug` header path is the same
   (`server/src/atompub/media.rs:93`).
4. **Correcting the issue's framing of the precedent.** #708's body implies the
   existing storage limits are enforced late. They are not: **max-file-size is
   enforced mid-stream** (`stream_to_temp` bails `PayloadTooLarge` as soon as
   `bytes_written` exceeds it, `:401`); only **quota** is in `finalize_upload`
   (`:277`). Both run before the file reaches its final content-addressed path,
   with the temp removed on failure — so the pipeline already ensures a
   violating upload cannot land.
5. **Which makes the placement decision easy rather than balanced.** File size
   is checked mid-stream because it _cannot be known earlier_. A filename's
   length can: it is known before the stream opens. So enforcing in `Filename`'s
   doors is not "a type invariant instead of a boundary check" — it is the
   **earliest** point in the pipeline, ahead of both existing checks. An
   over-long name never opens a stream.
6. **The intake errors already map to client errors.** `MediaError::BadRequest`
   on the multipart path (`:142`), `HandlerError::BadRequest` on `AtomPub`
   (`atompub/media.rs:93`). So nothing new is needed to make the failure
   client-facing — and under D2 below there is no failure at all for a
   merely-long name.

## Decisions

**D1 — the bound is on the encoded byte length, enforced in `Filename`'s
doors.** A named const in `common/src/media.rs`, beside
`MEDIA_SEGMENT_ENCODE_SET`:

```rust
/// The filesystem's per-path-component limit (bytes). ext4/XFS/btrfs, APFS, and NTFS all
/// cap a single name at 255; the media layout puts the filename in one component, so this
/// is the whole budget.
const MAX_FILENAME_ENCODED_BYTES: usize = 255;
```

Enforced against `encode_filename_segment(…)`'s output length, not the raw
length — the encoded form is what lands on disk (ADR-0080).

**Accepted coupling:** `Filename`'s invariant now depends on
`MEDIA_SEGMENT_ENCODE_SET`. A general "safe path leaf" type learning about
percent-encoding is a real cost, and it means changing the encode set silently
changes the invariant. Taken anyway, because the defect is that a
_valid-looking_ value cannot be stored, and a check positioned elsewhere can be
forgotten by a future writer. A comment on the const records the dependency so a
later encode-set change is prompted to revisit it.

**D2 — the two doors answer truncate-vs-reject differently, matching their
documented roles.**

- **`FromStr` rejects.** It is the strict door for values that must match a
  stored name exactly (URL segments, DB reads) and already "rejects a
  non-canonical name outright." An over-long URL segment cannot name an existing
  file, so rejecting is also the correct serve-path answer — a 404 via
  `SoftPath`, not a filesystem probe.
- **`Filename::sanitized` truncates.** It is the upload-intake door whose stated
  job is reducing "a client's arbitrary name … to a single leaf". Failing an
  upload because a name is long is a cosmetic reason to lose a file. Truncating
  means **the merely-long case stops being an error at all** — which is a better
  outcome than the issue asked for (it asked for a domain error at the
  boundary).

**D3 — truncation preserves the extension and never splits a grapheme cluster.**

**Not for the reason it first appears.** Serving does **not** depend on the
extension: the serve path reads the **stored** `content_type` column
(`find_by_hash(…).map_or_else(|| detect_content_type(&filename), |r| r.content_type)`,
`server/src/media.rs:128-132`) and the extension is only its no-DB-row fallback;
inline-vs-attachment is likewise `should_inline(content_type)`, not the name. A
normally uploaded file always has a row, so `Content-Type` on serve is safe
either way.

Two narrower reasons remain, and only the first is durable:

- **Upload-time detection.** `get_content_type(None, filename)` →
  `detect_content_type(filename)` (`media_manager.rs:152-162`) is the _only_
  content-type source when the client omits `Content-Type`, and it is called
  with the **already-sanitized** name. Truncation eating the extension would
  store `application/octet-stream` **permanently** — unrecoverable without a
  re-upload. A data consequence, not a presentational one.
- **The saved-as filename.** `Content-Disposition`'s `filename=`/`filename*=`
  (`server/src/media.rs:134`) is the name the browser writes to disk, so an
  extension-less name lands as something the user's OS will not open by
  double-click.

Rejected alternative: detect the content type from the _raw_ name before
truncating, which would make naive truncation safe. `get_content_type` is called
from `upload_inner`, which already holds a typed `&Filename`, so threading the
raw name through would give it two competing notions of "the name" — and it
would not fix the saved-as filename anyway.

Algorithm:

1. Split with **`Path::file_stem()` / `Path::extension()`**, not a manual
   last-dot split — a manual split misreads a dotfile (`.hiddenfile` would
   become an empty stem plus a `.hiddenfile` extension, and truncating the stem
   would destroy the name). `Path`'s rules give `extension() == None` there, and
   `media.rs` already imports `Path`.
2. Reserve the extension's _encoded_ length; if it alone exceeds the budget
   (pathological), ignore the split and truncate the whole name. Detection then
   degrades to `application/octet-stream` for a client that sent no
   `Content-Type` — accepted, because an _extension_ that alone exceeds 255
   encoded bytes is not a real case.
3. Fill the stem by walking **grapheme clusters** (as `slugify_title` does),
   accumulating each cluster's _encoded_ byte length, stopping before the budget
   is exceeded — so a base scalar is never separated from its combining marks
   and the result is always valid UTF-8.
4. Guard the degenerate outputs: if the result is empty, `.`, or `..`, it is an
   `InvalidFilename` — the same answer `sanitized` already gives for those.

**D4 — `InvalidFilename` becomes an enum.** It is currently a unit struct with
one fixed message ("must be a non-empty safe path leaf (no path components,
`.`/`..`, or null bytes)"), which would actively lie about a length failure. Two
variants — not-a-safe-leaf and too-long — with the too-long message naming the
budget and the fact that it applies to the _encoded_ form, since "my
100-character name was rejected" is otherwise baffling.

**D5 — the oracle relationship changes, and the existing test must follow.**
`sanitize_filename` (the free function) does **not** truncate, so once `FromStr`
bounds length, `sanitize_filename`'s output no longer always re-parses — which
is exactly what `sanitize_filename_output_always_reparses_as_filename`
(`common/src/media.rs:1047`) asserts today. The property that actually matters
is **`Filename::sanitized`'s output always satisfies `FromStr`** — the intake
door's output must pass the strict door. That test is re-pointed at `sanitized`,
which is a stronger claim than the one it replaces, and a long-input case is
added to it.

## Acceptance criteria

1. `Filename` cannot be constructed — by either door — such that
   `encode_filename_segment(&f)` exceeds `MAX_FILENAME_ENCODED_BYTES`.
2. `FromStr` rejects an over-long name with a distinct `InvalidFilename` variant
   whose message names the budget and says it applies to the encoded form.
3. A truncated name still yields the right stored `content_type` when the client
   sends none — i.e. `detect_content_type` on the truncated name matches the
   untruncated one. This is D3's durable reason and the assertion that actually
   protects it; testing only that "the extension is present" would pass on a
   name whose extension was mangled.
4. A dotfile (`.hiddenfile`) is not destroyed by truncation — pins the
   `Path::file_stem()`/`extension()` choice over a manual last-dot split.
5. `Filename::sanitized` truncates instead of failing, preserves the extension,
   and cuts only on grapheme boundaries. Tests must include: a long ASCII name;
   a long multi-byte name (where encoded length is ~3× raw, so raw-length
   bounding would have been wrong); a name whose extension must survive; a name
   whose combining marks must not be split; and the degenerate cases (extension
   alone over budget, result would be `.`/`..`/empty).
6. **A test proving the write no longer fails for a name that previously would
   have** — a name over the encoded budget goes through upload and lands, under
   its truncated name. This is the regression the issue is about; the unit tests
   alone do not demonstrate it.
7. `sanitize_filename_output_always_reparses_as_filename` re-pointed per D5,
   with a long input among its cases.
8. `cargo xtask validate --no-e2e` clean.

## Out of scope

- Changing `MEDIA_SEGMENT_ENCODE_SET` or the media layout (ADR-0080 owns those).
- The `tar` name-length question: a 255-byte component exceeds ustar's 100-byte
  name field and needs pax/GNU extensions in backup archives. Names that long
  are **already** possible today, so this is pre-existing and orthogonal — noted
  here so it is not mistaken for fallout of this change. Worth its own issue if
  the backup crate does not emit pax headers.
- #711 (`delete_media`'s substring matching), the other #675 follow-up, done
  next.
