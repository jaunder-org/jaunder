# Spec — `RootRelativeUrl` for the media serve-URL chain (#675)

- Status: awaiting approval
- Issue: #675 (milestone #13, Domain-value type safety)

## Problem

The media serve URL is a bare `String` end to end, while `RootRelativeUrl`
(`common/src/root_relative_url.rs`) already validates exactly its shape and
already types five post-DTO fields. `MediaItem`'s other five fields are all
`common` newtypes; `url` is the lone primitive. Neither DTO derives
`reactive_stores::Patch` (verified), so the #587 carve-out has never applied —
the gap is simply un-swept.

## What the survey found

The issue's Direction rests on a premise that does not hold, and the survey
turned up a latent defect that the newtype adoption forces us to confront.

1. **`media_url` interpolates the filename raw** and is untyped on both string
   arguments —
   `pub fn media_url(source: &str, sha256: &ContentHash, filename: &str) -> String`
   at `common/src/media.rs:276-278`, delegating to `media_path` (`:268-272`).
2. **`Filename` permits whitespace.** Its oracle `sanitize_filename`
   (`common/src/media.rs:190-199`) only strips path components and maps NUL →
   `_`. A test at `common/src/media.rs:900-921` explicitly asserts `"a b.txt"`
   re-parses as a valid `Filename`.
3. **`RootRelativeUrl` rejects whitespace and control characters**
   (`common/src/root_relative_url.rs:37-42`).

So for a legitimate upload named `a b.txt`, `media_url` returns
`/media/upload/…/a b.txt`, which **cannot be a `RootRelativeUrl`**. The issue's
"builds from a known-valid template … so it is a trusted producer" is false: a
trusted door here would mint a value violating the type's own invariant, which
is worse than leaving the field a `String`.

4. **Worse, and not caught by the type: `RootRelativeUrl` _allows_ `?`** (it
   permits an optional query). A file named `what?.png` yields
   `/media/…/what?.png`, which **validates cleanly** but whose path truncates at
   the `?` — it addresses the wrong file. `#` is the same class of defect.
   Validation alone cannot fix this; only encoding can. This is the decisive
   argument for D1.
5. **The raw value reaches an Atom feed** as an `href` via
   `server/src/atompub/media.rs:30-33`. Browsers tolerate a raw space; XML
   consumers need not.
6. **The serve route already tolerates encoding.** `server/src/media.rs:58-66`
   extracts `filename: SoftPath<Filename>` through axum's `Path`, which stores
   params as `PercentDecodedStr` and decodes via
   `percent_encoding::percent_decode(…).decode_utf8()` (verified in
   `axum-0.8.9/src/util.rs:20-23`). `%20` arrives as a literal space. **No route
   change is required.**
7. **`MediaRecord.source_url` is a dead field.** Every construction site sets it
   to `None`, production included (`storage/src/media_manager.rs:252`,
   `storage/src/media.rs:390,526,571`); it is only read back through the sqlx
   row tuple (`storage/src/helpers.rs:287,306`) and bound in the INSERT
   (`storage/src/media.rs:192`). The remote-caching ingest that would populate
   it does not exist yet.
8. **No test covers a filename with a space** — which is why the defect
   survived. Existing coverage uses clean names:
   `end2end/tests/media.spec.ts:26,29`, `end2end/tests/atompub.spec.ts:175`,
   `elisp/test/jaunder-test.el:432-437`. All keep passing under encoding; none
   would have caught the bug.
9. **`RootRelativeUrl` has no trusted door**, and `from_trusted` is a globally
   policed leaf name (`xtask/src/steps/rendered_html_from_trusted_check.rs`,
   `EXEMPT_QUALIFIERS = ["ContentType"]`).
10. **The on-disk layout is built in two independent places.** `media_path`
    (`common/src/media.rs:268-272`) builds it for writes; the serve route
    hand-rolls the same layout with `.join()` calls instead of calling it
    (`server/src/media.rs:241-250`). Nothing enforces that the two agree. See
    D2a.

## Decisions

**D1 — the filename segment is percent-encoded (see D2 for where), and
`media_url` returns `RootRelativeUrl`.** This is the load-bearing decision. It
makes the output satisfy the invariant by construction, and it fixes the `?`/`#`
mis-addressing (finding 4) that no amount of validation would catch. Rejected
alternatives: tighten `Filename` to forbid whitespace (rejects or mangles
legitimate uploads like `my photo.jpg` — user-hostile, and changes upload
semantics); add a trusted door that bypasses validation (the newtype would lie,
and the `?` bug would persist); leave `url` a `String` (abandons the issue and
keeps the bug).

**D2 — `media_path` encodes too, so the URL path and the on-disk path stay
byte-identical.** The encoding lives in `media_path`; `media_url` remains the
thin `format!("/media/{}", media_path(…))` wrapper. The file on disk is named
`my%20photo.jpg`.

Chosen over encoding only in `media_url` (the first draft of this spec) on an
**operational** ground: being able to paste the tail of a URL and find the file
on disk is a real troubleshooting affordance, and a URL/disk divergence destroys
it. Encoding both preserves it — and is strictly better than today's raw names,
for two further reasons:

- **Encoded names never need shell quoting.** `cat my%20photo.jpg` works where
  `cat my photo.jpg` does not.
- **It eliminates Unicode-normalization mismatch.** `café.png` becomes
  pure-ASCII `caf%C3%A9.png`, so a tree written NFC cannot fail to match when
  read through a filesystem that normalizes to NFD.

The DB `filename` column keeps the **raw** name — it is the display name shown
in the media list. So one derivation remains (DB → disk requires encoding), but
the troubleshooting path that matters, URL → disk, is identity.

**D2a — prerequisite: `resolve_media_path` must route through `media_path`.**
The serve route does not currently use `media_path`; it hand-rolls the same
layout with `.join()` calls (`server/src/media.rs:241-250`), so the disk layout
is constructed in two independent places with nothing enforcing agreement.
Encoding in `media_path` alone would therefore break reads: writes would encode,
reads would not. Unifying them is required for correctness here and removes a
duplication that could have drifted anyway.

**D2b — accepted risk: encoding lowers the filename-length ceiling.** `Filename`
has no length bound, and percent-encoding expands a name up to 3× (9× for
multi-byte UTF-8), so a ~90-character name of mostly spaces can exceed the
255-byte per-component filesystem limit and fail to write where it previously
succeeded. This is a _narrowing of an existing_ failure mode, not a new class —
a 300-character filename already fails today — so it is accepted and documented
rather than fixed by capping `Filename`, which would change upload semantics and
belongs to its own issue.

Backup/restore is unaffected: it mirrors the media tree by directory traversal
(`mirror_media_entries` / `restore_media_entries` in `storage/src/backup.rs`),
never reconstructing names from the database, so it copies whatever names exist.

**D3 — encode with a named `AsciiSet`: `NON_ALPHANUMERIC` minus the RFC 3986
_unreserved_ marks.** Add `percent-encoding = { workspace = true }` to `common`
(already a workspace dep at `Cargo.toml:92`, so the vendored copy is reused — no
new third-party code). The set is

```rust
const MEDIA_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-').remove(b'.').remove(b'_').remove(b'~');
```

**Bare `NON_ALPHANUMERIC` is wrong here** even though `content_disposition`
(`server/src/media.rs:263-264`) uses it: it encodes `.`, `-` and `_` too, so
`my-photo.jpg` would become `my%2Dphoto%2Ejpg`. That is correct-but-unreadable
for an RFC 5987 header and actively defeats D2's operational goal — every
ordinary filename would be mangled on disk. Keeping the four unreserved marks
means **the common case is byte-identical**: `photo.jpg`, `my-photo_2.png`,
`a~b.txt` all pass through untouched, and only genuinely troublesome characters
encode.

The set still covers everything that matters: controls and whitespace (which
`RootRelativeUrl` rejects), `%` (else a pre-existing `%XX` in a filename is
misread on decode), the URL-structural `?` and `#` (finding 4), `/` and `\`, and
all non-ASCII (D2's normalization win). It deliberately over-encodes rather than
enumerating a minimal set by hand: _which_ characters are encoded is the
security-relevant part, and a hand-rolled set is where such a decision goes
wrong later.

**D4 — no new trusted door; follow the `AbsoluteUrl::compose` idiom.** Because
D1 makes the output valid by construction, `media_url` stays infallible via the
established in-tree pattern — `let Ok(url) = … else { unreachable!(…) }`,
exactly as `common/src/absolute_url.rs:91-97` (`compose`) and `:71-74`
(`with_query_pairs`) already do — with the encoding argument recorded in the
comment. This **contradicts the issue's Direction**, which anticipated needing a
trusted door: adding one would mean widening a security gate's exemption list to
buy something the encoding already guarantees. The #398 gate and its
`EXEMPT_QUALIFIERS` are left untouched.

**D5 — tighten `media_url`'s arguments to `&MediaSource` and `&Filename`.**
Today both are `&str` and adjacent, so they are silently transposable — a
newtype criterion in its own right. Callers already hold typed values
(`web/src/media/api.rs:88` passes `r.source.as_ref()` and `&r.filename`); the
literal `"upload"` sites (`storage/src/media_manager.rs:314`,
`server/src/atompub/media.rs:30`) become `MediaSource::Upload`.

**D6 — `UploadResponse.url` and `MediaItem.url` become `RootRelativeUrl`.**
Their doc comments lose the "plain derived string … carried verbatim" note,
which recorded the flatten without justifying it (ADR-0063 §5 is explicit that
"it is only a DTO surface" is not a reason).

**D7 — `web/src/media/component.rs` holds `RwSignal<Option<RootRelativeUrl>>`**
(`:40`, set at `:72`, read at `:94`) and stringifies at the view site, per the
existing idiom. A newtype is not `IntoRender`.

**D8 — `server/src/atompub/media.rs` holds the newtype up to the
`atom_syndication` call and reads it out there** via `Deref`/`Display` — the
ADR-0063 sanctioned external-type boundary — rather than keeping the field
primitive to suit the external crate.

**D9 — `MediaRecord.source_url` becomes `Option<AbsoluteUrl>`.** `AbsoluteUrl`'s
invariant is exactly right (`http(s)` only, `url`-crate normalized) and
`#[derive(StrNewtype)]` already generates a validating sqlx bridge, so no bridge
work is needed and #686 (numeric `Id` newtypes) cannot collide. Because the
field is dead today (finding 7) this costs nothing now; its value is that it
becomes **the specification for a path not yet written** — whoever builds
remote-media caching is forced to supply a validated URL rather than whatever a
feed handed them. An unparseable `source_url` is useless by definition (we cache
by fetching it), so rejecting at that future ingest is strictly better than
storing a value no code can act on.

## Acceptance criteria

1. `media_path` percent-encodes the filename segment; `media_url` stays its thin
   `/media/` wrapper and returns `RootRelativeUrl`. Both take `&MediaSource` and
   `&Filename`. Both carry the D2 doc note explaining that the encoded name is
   what is on disk, and why.
2. `resolve_media_path` builds its path **via `media_path`**, not by hand — the
   round-trip (write path == read path) is what makes encoding safe, and a test
   pins agreement between them for a filename that actually needs encoding.
3. `UploadResponse.url`, `MediaItem.url` are `RootRelativeUrl`; the component
   signal is `Option<RootRelativeUrl>`.
4. `MediaRecord.source_url` is `Option<AbsoluteUrl>`, round-tripping through
   both backends.
5. No `.parse().expect()` and no new `from_trusted` door; `EXEMPT_QUALIFIERS`
   unchanged.
6. **Regression coverage for the defects this uncovered**, not merely for the
   refactor. Each of the first three must fail before the fix:
   - `media_url` for `a b.txt` is a valid `RootRelativeUrl` and contains no raw
     space;
   - `what?.png` and `a#b.png` encode, so the served path is not truncated — the
     finding-4 mis-addressing bug;
   - a filename containing a literal `%` round-trips without double-decoding;
   - **ordinary names are untouched** — `photo.jpg`, `my-photo_2.png`, `a~b.txt`
     encode to themselves, byte-for-byte (pins D3's unreserved-mark carve-out,
     without which every filename on disk would be mangled);
   - a **write-then-serve** test for a name needing encoding: the file lands at
     the encoded path and the serve route resolves it (this is what would catch
     a D2a regression);
   - an end-to-end path (upload → serve → Atom output) with a spaced filename.
7. `cargo xtask validate --no-e2e` clean, plus wasm-clippy (the media component
   is wasm-only).

## Out of scope

- Building the remote-media caching ingest that would populate `source_url` — D9
  types the field; it does not implement the path.
- #692 (`ContentType`/`Filename` through the media path) — blocked on this
  issue, done next.
- Any `Filename` invariant change; D1 deliberately leaves upload semantics
  alone. In particular, capping `Filename`'s length — which D2b's accepted risk
  would argue for — is deferred rather than smuggled in here.
