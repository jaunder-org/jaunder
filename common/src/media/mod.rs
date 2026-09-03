//! Pure helpers for jaunder's content-addressed media storage, shared by the
//! web media upload/serve handlers and the `AtomPub` media collection (both in
//! the `server` crate). Nothing here touches the filesystem or database — these
//! are deterministic string/path computations and small classification tables,
//! so they are cheap to unit-test and safe to call from any layer.
//!
//! # Storage layout
//!
//! A stored object is addressed by its `SHA-256` content hash and laid out as
//! `<source>/<p1>/<p2>/<sha256>/<filename>` (see [`path`]), served under
//! `/media/` (see [`url`]). `p1`/`p2` are the first two byte-pairs of the
//! hex digest — a two-level fan-out that keeps any single directory small.
//! `source` distinguishes provenance (e.g. `upload` vs a remote cache).
//!
//! The `<filename>` segment is **percent-encoded**, so the URL path and the on-disk path are
//! byte-identical: paste the tail of a serve URL and you have the path to the file. Both come
//! from [`path`], which is the only place the layout is spelled — so a new consumer must
//! call it rather than re-deriving, or the two spellings drift apart (#675).
//!
//! That encoding is not something [`path`] *does* — a [`Filename`] already **is**
//! the canonical encoded segment (#720). The database column, the on-disk name and the URL
//! all hold the same bytes, and display is the one place anything is transformed
//! ([`Filename::decoded`]).
//!
//! Encoding is not cosmetic. The name a user types may legally contain a space (which
//! [`crate::root_relative_url::RootRelativeUrl`] rejects) or a `?`/`#` (which it *accepts*,
//! silently truncating the path at the delimiter and addressing a different file).
//!
//! One canonical spelling plus a decoded display view, and why:
//! `docs/adr/0080-media-path-naming-correspondence.md` (as amended by
//! `docs/adr/0084-media-filename-encoded-canonical.md`).
//!
//! # Untrusted input
//!
//! Filenames and hashes round-trip through URLs, so they are attacker-
//! influenced. [`sanitize_filename`] reduces a name to a single safe path
//! component; [`Filename`] is the newtype that holds such a value — its
//! validating [`FromStr`] admits only a **canonically encoded** value whose
//! *decoded* form is a safe leaf, and its [`sanitized`][Filename::sanitized] door
//! normalizes an arbitrary name into one — so an un-sanitized filename is
//! unrepresentable past the boundary. A third door,
//! [`Filename::from_decoded_segment`], takes the decoded segment axum hands the
//! routes that carry a filename in their path, and re-encodes it to recover the
//! stored spelling. An externally
//! supplied hash must be parsed into a [`ContentHash`]
//! (via [`is_valid_content_hash`], its `FromStr`'s validating engine) before it
//! reaches [`path`]: the type is what guarantees the `sha256[..2]`/`[2..4]`
//! slicing — unguarded, and panicking on a short or non-`UTF-8`-boundary value —
//! only ever sees a canonical 64-hex string.
//!
//! # Content type
//!
//! [`detect_content_type`] maps a filename extension to a `MIME` type (falling
//! back to `application/octet-stream`), and [`should_inline`] decides whether a
//! type is served inline or as an attachment (the `Content-Disposition`).

mod filename;
mod hash;
mod mime;
mod references;
mod storage;
mod values;

pub use filename::{Filename, InvalidFilename, MAX_FILENAME_ENCODED_BYTES, sanitize_filename};
pub use hash::{ContentHash, InvalidContentHash, is_valid_content_hash};
pub use mime::{ContentType, InvalidContentType, detect_content_type, should_inline};
pub use references::{
    InvalidMediaReferenceForm, InvalidMediaReferenceKind, MediaReference, MediaReferenceForm,
    MediaReferenceKind, parse_media_url,
};
pub use storage::{InvalidMediaSource, MediaRef, MediaSource, path, url};
pub use values::{ByteSize, MaxFileSize, UploadedMedia, UserQuota};
