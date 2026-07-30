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
- **The database `filename` column keeps the raw name.** It is the display name
  shown in the media list and returned as `UploadResponse.filename`.
- `media_url` returns `RootRelativeUrl` **infallibly**. Every segment is a hex
  digest, a bounded enum token, or percent-encoded, so the parse cannot fail;
  the `unreachable!` arm follows `AbsoluteUrl::compose`. **No trusted-minting
  door is added** — the encoding provides by construction what a `from_trusted`
  door would merely have asserted, so the `rendered-html-from-trusted` gate's
  `EXEMPT_QUALIFIERS` list is untouched.

So there are three spellings of a filename, and exactly one derivation between
them:

| Where                 | Spelling | Why                                             |
| --------------------- | -------- | ----------------------------------------------- |
| Database `filename`   | raw      | it is the display name                          |
| On disk               | encoded  | identical to the URL, so a URL locates the file |
| URL (serve + AtomPub) | encoded  | must be a well-formed URL reference             |

**URL → disk is identity. DB → disk requires encoding.**

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
- **The effective filename-length ceiling is lower, so `Filename` is now bounded
  by its encoded length.** Encoding expands a name up to 3× (9× for multi-byte
  UTF-8), so a name that validated could exceed the 255-byte per-component
  filesystem limit and fail the write with an IO error rather than a domain
  error. Resolved in [#708](https://github.com/jaunder-org/jaunder/issues/708):
  - The bound is on the **encoded** form (`MAX_FILENAME_ENCODED_BYTES`), not a
    character count — a char count cannot express this limit, since a safe one
    would be ~28 characters. This makes `Filename`'s invariant depend on the
    encode set above: **widening that set shrinks the set of representable
    names, so the two must be revisited together.**
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
- This was adopted with **no legacy data**. Had there been stored media,
  changing the on-disk spelling would have required a migration, and changing
  the AtomPub member URL would have changed existing entries' `atom:id`.

Related: ADR-0063 (domain-value newtype convention — §5 pervasiveness, and that
"it is only a DTO surface" is not a reason to leave a field stringly), ADR-0073
(`url` crate for absolute-URL normalization).
