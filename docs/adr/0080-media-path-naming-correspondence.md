# ADR-0080: Media path naming — one layout, percent-encoded, URL-identical on disk

- Status: accepted
- Date: 2026-07-29
- Issue: [#675](https://github.com/jaunder-org/jaunder/issues/675)

## Context

A stored media object is content-addressed and laid out as
`<source>/<p1>/<p2>/<sha256>/<filename>`, served under `/media/`. Four of those
five segments are machine-generated and safe. The fifth, `<filename>`, is **user
input**: `Filename`'s canonicalization oracle (`sanitize_filename`) strips path
components and maps NUL to `_`, but deliberately permits spaces, `?`, `#`, `%`,
and arbitrary non-ASCII — a filename is a display name, and mangling one is
user-hostile.

Interpolating that raw into the path produced two distinct failures, which #675
surfaced while adopting `RootRelativeUrl` for the serve URL:

1. **Unrepresentable.** `RootRelativeUrl` rejects whitespace, so `a b.txt`
   yielded a serve URL that could not be typed at all. This is the benign
   failure: it is loud.
2. **Silently wrong.** `RootRelativeUrl` _accepts_ `?` (it permits a query), so
   `what?.png` yielded a URL that **validates cleanly** while its path truncates
   at the delimiter, addressing a different file. `#` is the same class. No
   amount of validation catches this — the value is well-formed, just pointing
   somewhere else. It was also emitted into Atom feeds as an `href`, and into
   the AtomPub media member URL, which is an entry's permanent `atom:id`.

Compounding it, the layout was spelled in **two** independent places:
`media_path` for writes, and a hand-rolled `.join()` chain in the serve route
for reads. Nothing made them agree, so any change to one silently broke the
other.

## Decision

**The filename segment is percent-encoded, and `media_path` is the single
definition of the layout.**

- The encode set is `NON_ALPHANUMERIC` minus the RFC 3986 **unreserved** marks
  `-._~`, defined once as a private const in `common::media` and reached only
  through `media_path`/`media_url`/`encode_filename_segment`. The bare set is
  wrong here even though `content_disposition` correctly uses it for an RFC 5987
  header: it encodes `.`, `-` and `_`, so `my-photo.jpg` would become
  `my%2Dphoto%2Ejpg`. Keeping the four unreserved marks means **ordinary names
  are byte-identical** and only troublesome characters encode.
- **The encoding lives in `media_path`, not only in `media_url`**, so the
  on-disk path and the URL path are the same bytes. Every consumer of the
  _storage_ layout — the upload write, the serve read, the AtomPub entry's
  content link — calls `media_path` or `media_url`; the serve route no longer
  re-derives it.
- The AtomPub **media member** URL
  (`/atompub/{username}/media/{sha}/{filename}`) is a different layout — the
  collection's, not the content-addressed store's — so it is built at its own
  call site. It shares only `encode_filename_segment`, which is public for
  exactly that reason. This is the one place a filename becomes a path segment
  outside `media_path`, and it is typed `RootRelativeUrl` too, so the two cannot
  drift in kind.
  - **Amended by `docs/adr/0084-media-filename-encoded-canonical.md` (#720):**
    as of #720 a `Filename` _was_ the canonical percent-encoded segment, so
    neither site encoded — both interpolated — and `encode_filename_segment` was
    deleted, returning the encode set to a private const with no public escape
    hatch.
- **The database `filename` column keeps the raw name.** It is the display name
  shown in the media list and returned as `UploadResponse.filename`.
  - **Reversed by `docs/adr/0084-media-filename-encoded-canonical.md` (#720).**
    As of #720 the column held the **encoded** form, byte-identical to the
    on-disk name and the URL segment; display surfaces decode. The motivating
    reason is #711's post→media reference table, whose comparison against names
    extracted from rendered HTML becomes byte equality instead of a transform at
    a comparison point.
- `media_url` returns `RootRelativeUrl` **infallibly**. Every segment is a hex
  digest, a bounded enum token, or percent-encoded, so the parse cannot fail;
  the `unreachable!` arm follows `AbsoluteUrl::compose`. **No trusted-minting
  door is added** — the encoding provides by construction what a `from_trusted`
  door would merely have asserted, so the `rendered-html-from-trusted` gate has
  nothing new to exempt. (That gate's `EXEMPT_QUALIFIERS` list, named here
  before #778, no longer exists: exemptions are in-source markers now, and a
  door added here would have cost one.)

So there are three spellings of a filename, and exactly one derivation between
them:

| Where                 | Spelling | Why                                             |
| --------------------- | -------- | ----------------------------------------------- |
| Database `filename`   | raw      | it is the display name                          |
| On disk               | encoded  | identical to the URL, so a URL locates the file |
| URL (serve + AtomPub) | encoded  | must be a well-formed URL reference             |

**URL → disk is identity. DB → disk requires encoding.**

**Superseded by `docs/adr/0084-media-filename-encoded-canonical.md` (#720):**
there is now **one** spelling — database, disk and URL are byte-identical — plus
a decoded _view_ for display. The remaining derivations are the display decode
(cosmetic if missed) and a re-encode at the three inbound URL doors, where axum
has already percent-decoded the segment. That re-encode is dumb and its only
failure is a 404: percent-encoding under a fixed set is injective, so a
mis-encoded lookup can miss but can never resolve to a _different_ file.

## Consequences

**Chosen deliberately over encoding only the URL**, which was the first draft of
#675's spec. That would have left the disk name raw and the URL encoded — and
destroyed the ability to paste the tail of a serve URL and find the file, which
is a real troubleshooting affordance. Encoding both preserves it, and is better
than the pre-#675 raw names in two further ways: encoded names never need shell
quoting (`cat my%20photo.jpg` works where `cat my photo.jpg` does not), and disk
names become pure ASCII, so a tree written NFC cannot fail to match when read
through a filesystem that normalizes to NFD.

**Also rejected: tightening `Filename` to forbid whitespace.** It would reject
or silently rename legitimate uploads like `my photo.jpg`, changing upload
semantics to serve a path-layout concern. The display name is not the path's
business.

**What this commits us to:**

- A new consumer of the media layout **must** call `media_path`/`media_url`.
  Re-deriving the layout reintroduces the read/write divergence this closes; the
  doc comments on both functions say so.
- The serve route's re-encode is **not** redundant. axum's `Path` extractor
  percent-_decodes_ path parameters before the handler sees them, so the handler
  holds the raw name and `media_path` re-encodes it to recover the stored
  spelling. It reads like something to simplify away, and doing so breaks
  serving for any name needing encoding.
  - **Relocated by `docs/adr/0084-media-filename-encoded-canonical.md` (#720):**
    the re-encode still exists and is still not redundant, but it now lives in
    `ProfferedFilename`'s door rather than in `media_path`, which only
    interpolates. There is no un-decoded extractor to avoid it with —
    `RawPathParams` is "raw" only in the sense of _undeserialized_; its values
    are `PercentDecodedStr` too.
- **The effective filename-length ceiling is lower, so `Filename` is now bounded
  by its encoded length.** Encoding expands a name up to 3× (9× for multi-byte
  UTF-8), so a name that validated could exceed the 255-byte per-component
  filesystem limit and fail the write with an IO error rather than a domain
  error. Resolved in [#708](https://github.com/jaunder-org/jaunder/issues/708):
  - The bound is on the **encoded** form (`MAX_FILENAME_ENCODED_BYTES`), not a
    character count — a char count cannot express this limit, since a safe one
    would be ~28 characters.
  - **Narrowed by `docs/adr/0084-media-filename-encoded-canonical.md` (#720).**
    This note used to say the bound makes `Filename`'s _invariant_ depend on the
    encode set. That is no longer true: the stored value already **is** the
    encoded form, so the type's own bound is a plain `len() <= 255` with no
    encode-set reference.

    The coupling itself survives, relocated to the **intake budget**. Intake
    still runs `sanitize → truncate → encode`, with `truncate_to_budget` walking
    raw graphemes and measuring each by its encoded cost — truncating in encoded
    space would mean never splitting a `%XX` escape, never splitting the escape
    run of one multi-byte character (`ä` is `%C3%A4`; a cut after `%C3` decodes
    to invalid UTF-8), and still never splitting a grapheme cluster, which is
    strictly harder for no gain. So **widening the encode set still shrinks the
    set of typed names that survive intake intact, and the two must still be
    revisited together** — it is now a property of `Filename::sanitized`, not of
    the type's invariant.

    Strict canonicity does put a new encode-set reference into `FromStr`, but it
    is a better dependency: about _which spelling is canonical_, not _how much
    fits_.

  - It is enforced in `Filename`'s own doors, which is the _earliest_ point in
    the upload pipeline — a name's length is known before the stream opens,
    unlike a file's size.
  - The two doors differ, matching their roles: `FromStr` **rejects** (its
    values must match a stored name exactly, so shortening one would match the
    wrong file), while the upload-intake door `Filename::sanitized`
    **truncates**, keeping the extension. So a merely-long name is no longer an
    error at all.
  - The extension is kept because `detect_content_type` is the only content-type
    source when a client sends none, and it runs on the sanitized name —
    dropping the extension would store `application/octet-stream` permanently.
    Serving is unaffected; it reads the stored column.

- Backup/restore is unaffected: it mirrors the media tree by directory
  traversal, never reconstructing names from the database, so it carries
  whatever names exist.
  - Since #720 the `filename` **column** has an invariant a backup could
    violate, and restore does not check it — it binds every cell as text, never
    constructing the newtype, so a bad value surfaces later as a `Decode` error
    on read rather than at the restore boundary. Tracked as
    [#725](https://github.com/jaunder-org/jaunder/issues/725); the gap is
    generic to every typed column, not specific to media.
- This was adopted with **no legacy data**. Had there been stored media,
  changing the on-disk spelling would have required a migration, and changing
  the AtomPub member URL would have changed existing entries' `atom:id`.

Related: ADR-0063 (domain-value newtype convention — §5 pervasiveness, and that
"it is only a DTO surface" is not a reason to leave a field stringly), ADR-0073
(`url` crate for absolute-URL normalization).
