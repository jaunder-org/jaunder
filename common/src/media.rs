//! Pure helpers for jaunder's content-addressed media storage, shared by the
//! web media upload/serve handlers and the `AtomPub` media collection (both in
//! the `server` crate). Nothing here touches the filesystem or database — these
//! are deterministic string/path computations and small classification tables,
//! so they are cheap to unit-test and safe to call from any layer.
//!
//! # Storage layout
//!
//! A stored object is addressed by its `SHA-256` content hash and laid out as
//! `<source>/<p1>/<p2>/<sha256>/<filename>` (see [`media_path`]), served under
//! `/media/` (see [`media_url`]). `p1`/`p2` are the first two byte-pairs of the
//! hex digest — a two-level fan-out that keeps any single directory small.
//! `source` distinguishes provenance (e.g. `upload` vs a remote cache).
//!
//! The `<filename>` segment is **percent-encoded**, so the URL path and the on-disk path are
//! byte-identical: paste the tail of a serve URL and you have the path to the file. Both come
//! from [`media_path`], which is the only place the layout is spelled — so a new consumer must
//! call it rather than re-deriving, or the two spellings drift apart (#675).
//!
//! That encoding is not something [`media_path`] *does* — a [`Filename`] already **is**
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
//! reaches [`media_path`]: the type is what guarantees the `sha256[..2]`/`[2..4]`
//! slicing — unguarded, and panicking on a short or non-`UTF-8`-boundary value —
//! only ever sees a canonical 64-hex string.
//!
//! # Content type
//!
//! [`detect_content_type`] maps a filename extension to a `MIME` type (falling
//! back to `application/octet-stream`), and [`should_inline`] decides whether a
//! type is served inline or as an attachment (the `Content-Disposition`).

use std::borrow::Cow;
use std::path::Path;
use std::str::FromStr;

use macros::{NumNewtype, StrNewtype};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::root_relative_url::RootRelativeUrl;
/// A closed classification of the URL form that named a media entry.
///
/// The storage key retains this form because local URLs are intrinsically local,
/// while absolute and scheme-relative URLs require a later live ownership check.
/// `legacy` is deliberately not a parser value; it belongs only to migrated rows.
#[macros::text_enum(
    sqlx,
    error = InvalidMediaReferenceKind,
    message = "media reference kind must be \"local\", \"absolute\", or \"scheme_relative\""
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[strum(serialize_all = "snake_case")]
pub enum MediaReferenceKind {
    /// A relative or root-relative URL, which always names this instance.
    Local,
    /// An HTTP(S) URL with its own scheme and authority.
    Absolute,
    /// An authority-relative URL beginning with `//`.
    SchemeRelative,
}

/// A validated media content hash: exactly 64 lowercase hex characters
/// (`[0-9a-f]{64}`), the canonical `format!("{digest:x}")` form of a SHA-256
/// digest. Introducing the type means an arbitrary string can no longer be passed
/// where a media content hash is expected (a transposition hazard, ADR-0063 §1).
///
/// [`FromStr`] is the single validating chokepoint — it delegates to
/// [`is_valid_content_hash`], the one source of truth for "canonical content
/// hash". The rest of the ADR-0063 string-newtype trailer (`Display`, `AsRef<str>`,
/// `Borrow<str>`, `Deref<Target = str>`, owned `String` conversions,
/// `PartialEq<str>`, and the validating serde bridge) is generated by
/// `#[derive(StrNewtype)]`, so a `ContentHash` serializes as a plain string and
/// rejects invalid input on the wire — safe to use as a (de)serialized DTO field.
///
/// The wrapped `String` is private, so the only way in is a validating parse or
/// the trusted [`from_digest`][ContentHash::from_digest] door — an arbitrary
/// `String` cannot masquerade as a content hash.
///
/// The positive companion shows the identical fixture compiles — the path resolves
/// and the validating door accepts a canonical 64-hex digest — so each
/// `compile_fail` below fails for the private field, not for a moved path.
/// (Fixture lines are hidden with `#`.)
///
/// ```
/// use common::media::ContentHash;
/// use std::str::FromStr;
/// let h = ContentHash::from_str(
///     "0000000000000000000000000000000000000000000000000000000000000000",
/// )
/// .unwrap();
/// let _read: &str = h.as_ref();
/// ```
///
/// No public constructor:
/// ```compile_fail
/// # use common::media::ContentHash;
/// # use std::str::FromStr;
/// let _ = ContentHash("abc".to_string()); // private field
/// ```
///
/// A `String` does not convert to a `ContentHash`. Asserted on `.into()`, not on a
/// function argument: an argument is never coerced through `From`, so
/// `takes_hash(String)` fails for any two distinct types and would keep failing
/// even if the `From<String>` impl this forbids were added.
/// ```compile_fail
/// # use common::media::ContentHash;
/// # use std::str::FromStr;
/// let _h: ContentHash = "abc".to_string().into();
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct ContentHash(String);

/// Error returned when a string cannot be parsed as a [`ContentHash`].
#[derive(Debug, Error)]
#[error("content hash must be 64 lowercase hex characters ([0-9a-f]{{64}})")]
pub struct InvalidContentHash;

impl FromStr for ContentHash {
    type Err = InvalidContentHash;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if is_valid_content_hash(s) {
            Ok(ContentHash(s.to_owned()))
        } else {
            Err(InvalidContentHash)
        }
    }
}

impl ContentHash {
    /// Builds a [`ContentHash`] from the raw 32 bytes of a SHA-256 digest,
    /// lowercase-hex-encoding them here so the caller never spells the format.
    /// This is the trusted producer door — validity is **structural**: 32 bytes
    /// always encode to exactly 64 lowercase hex characters, so an invalid hash
    /// cannot be constructed (unlike a byte-slice or string door). A digest from
    /// `sha2` reaches it directly with `.into()`:
    /// `ContentHash::from_digest(Sha256::digest(bytes).into())`.
    ///
    /// A hash arriving as a **string** — a URL path segment, a `#[server]` wire
    /// arg, or a value read back from the `sha256` column — is not a digest here;
    /// it goes through [`FromStr`]/`TryFrom`, which validate the canonical form.
    #[must_use]
    pub fn from_digest(digest: [u8; 32]) -> Self {
        use std::fmt::Write as _;
        let mut hex = String::with_capacity(64);
        for byte in digest {
            // Infallible: writing to a String never errors.
            let _ = write!(hex, "{byte:02x}");
        }
        ContentHash(hex)
    }
}

/// A media filename in its **canonical spelling**: the percent-encoded safe leaf that is
/// simultaneously the database value, the on-disk name, and the URL segment — one value, no
/// derivation between them (#720). [`decoded`][Filename::decoded] is the display view.
///
/// The newtype makes an un-sanitized filename unrepresentable where a filename is expected:
/// the value feeds a filesystem path and a `Content-Disposition` header, so an un-sanitized
/// one is a path-traversal / header-injection hazard (ADR-0063 §1: invariant + trust/safety
/// boundary).
///
/// It is also bounded by [`MAX_FILENAME_ENCODED_BYTES`], so a value that passes validation
/// can always be written to disk. Because the stored value already *is* the encoded form,
/// that bound is a plain byte length — the encode-set coupling #708 recorded now lives only
/// in [`sanitized`][Filename::sanitized]'s intake budget.
///
/// Three construction doors:
/// - [`FromStr`] **validates** the canonical spelling — the door for untrusted URL / wire /
///   DB values that must match a stored filename exactly. It accepts `s` only when, **in
///   this order**: `s` is non-empty and neither `.` nor `..`; `sanitize_filename` fixes the
///   *decoded* form; `s` equals the encoder's output for that form; and `s.len()` is within
///   budget. `#[derive(StrNewtype)]` routes both `Deserialize` and the `sqlx` `Decode`
///   through it, so every wire value and every column read passes here.
///
///   Two subtleties, each load-bearing. The safe-leaf oracle runs on `decode(s)`, **never**
///   on `s`: `sanitize_filename("a%2Fb.jpg")` is `"a%2Fb.jpg"`, so checking the encoded form
///   passes vacuously and would admit a separator or a NUL. And the leaf check precedes the
///   canonicity check because a value like `a/b.txt` fails both — the caller needs to hear
///   about the leaf.
///
///   It *rejects* rather than truncating: a silently shortened value would match the wrong
///   file, or nothing (#708).
/// - [`sanitized`][Filename::sanitized] **normalizes** — the upload-intake door, where a
///   client's arbitrary name is reduced to a safe leaf and then encoded. It **truncates** an
///   over-long name (keeping the extension) instead of failing, so an upload is never lost
///   for a cosmetic reason. This is the one intricate step in the whole arrangement;
///   everything else is dumb by design.
/// - [`from_decoded_segment`][Filename::from_decoded_segment] **re-encodes** — the inbound
///   URL-segment door for text axum has already percent-decoded. It exists because encoding
///   is not idempotent, so one [`FromStr`] cannot serve both a decoded and an
///   already-canonical input. It checks but never repairs: an over-budget encoded spelling
///   cannot exist on disk, and truncating a lookup key could match the wrong file.
///
/// Their contract: `sanitized`'s output always satisfies `FromStr`, pinned by
/// `sanitized_output_always_reparses_as_filename`; and `from_decoded_segment` returns a
/// [`Filename`] directly, so no public decoded-intermediate type can leak past extraction.
///
/// The rest of the ADR-0063 string-newtype trailer (`Display`, `AsRef<str>`,
/// `Borrow<str>`, `Deref<Target = str>`, owned `String` conversions,
/// `PartialEq<str>`, and the validating serde bridge) is generated by
/// `#[derive(StrNewtype)]`. The wrapped `String` is private, so the only ways in
/// are the three validating doors — an arbitrary `String` cannot masquerade as a
/// filename.
///
/// The positive companion shows the identical fixture compiles — the path resolves
/// and the validating door accepts a canonical safe leaf — so each `compile_fail`
/// below fails for the private field, not for a moved path. (Fixture lines are
/// hidden with `#`.)
///
/// ```
/// use common::media::Filename;
/// use std::str::FromStr;
/// let f = Filename::from_str("a.png").unwrap();
/// let _read: &str = f.as_ref();
/// ```
///
/// No public constructor:
/// ```compile_fail
/// # use common::media::Filename;
/// # use std::str::FromStr;
/// let _ = Filename("a".to_string()); // private field
/// ```
///
/// A `String` does not convert to a `Filename` (asserted on `.into()`, so the proof
/// still fails if a `From<String>` impl is added — a function argument would not):
/// ```compile_fail
/// # use common::media::Filename;
/// # use std::str::FromStr;
/// let _f: Filename = "a".to_string().into();
/// ```
///
/// The decoded-segment door returns a `Filename` directly:
/// ```
/// # use common::media::Filename;
/// let f = Filename::from_decoded_segment("my photo.jpg").unwrap();
/// assert_eq!(f.as_ref(), "my%20photo.jpg");
/// ```
///
/// There is no public decoded-intermediate filename type:
/// ```compile_fail
/// # use common::media::Filename;
/// use common::media::ProfferedFilename;
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct Filename(String);

/// Error returned when a string is not a usable media filename leaf.
#[derive(Debug, Error)]
pub enum InvalidFilename {
    /// Not a canonical single path component.
    #[error(
        "filename must be a non-empty safe path leaf (no path components, `.`/`..`, or null bytes)"
    )]
    NotASafeLeaf,
    /// A safe leaf, but not in the canonical percent-encoded spelling.
    ///
    /// Distinct from [`NotASafeLeaf`][Self::NotASafeLeaf] because the likeliest cause is a
    /// *raw* name on the wire — a perfectly good leaf that simply was not encoded — and
    /// telling that caller their filename "is not a safe path leaf" misdirects (#720).
    #[error(
        "filename must be in canonical percent-encoded form (this is the stored spelling; \
         encode it once at the boundary, and decode only for display)"
    )]
    NotCanonical,
    /// Longer than the filesystem can hold once percent-encoded.
    ///
    /// The message states the *encoded* length and why it differs from what the user typed:
    /// "my 90-character name was rejected" is otherwise baffling (#708).
    #[error(
        "filename is too long: {encoded} bytes once percent-encoded, limit \
         {MAX_FILENAME_ENCODED_BYTES} (encoding expands each unsafe byte to `%XX`, so the \
         limit applies to the encoded form, not the characters you typed)"
    )]
    TooLong {
        /// The candidate's encoded byte length.
        encoded: usize,
    },
}

impl FromStr for Filename {
    type Err = InvalidFilename;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Door A: accept only the canonical stored spelling. The check order below is
        // load-bearing, not incidental — see each step.
        let decoded = percent_decode_str(s).decode_utf8_lossy();

        // The safe-leaf rule, applied to the DECODED form (#720). Canonicity does *not*
        // imply a safe leaf: `a%2Fb.jpg`, `a%00b.jpg` and `a%5Cb.jpg` are all canonical yet
        // decode to a separator, a NUL and a backslash. This is the path-traversal /
        // header-injection guard the type exists for, so losing it here would hollow out
        // `Filename` while every other check still looked present.
        //
        // Run on `decoded`, never on `s`: `sanitize_filename("a%2Fb.jpg")` is
        // `"a%2Fb.jpg"`, so testing the encoded form passes vacuously.
        //
        // Note what this does *not* catch: `sanitize_filename` does not touch CR/LF, so
        // a canonical `a%0D%0Ab.jpg` is accepted — see
        // `from_str_accepts_a_canonical_name_carrying_control_characters` for why that
        // is safe and deliberate rather than an oversight.
        //
        // Ordered BEFORE canonicity because a separator value is *both* an unsafe leaf and
        // non-canonical, and the failure a caller needs to hear about is the leaf.
        if !is_safe_leaf(&decoded) {
            return Err(InvalidFilename::NotASafeLeaf);
        }

        // Canonicity: the stored spelling is exactly what the encoder produces. This is
        // what makes "the column holds the encoded form" a checked fact rather than a
        // convention — a raw name from a hand-edited row, a restored backup or a wire
        // argument is rejected here, instead of becoming a file no URL can address.
        if utf8_percent_encode(&decoded, MEDIA_SEGMENT_ENCODE_SET).to_string() != s {
            return Err(InvalidFilename::NotCanonical);
        }

        // The value already *is* the encoded form, so the budget is a plain byte length —
        // no encode-set reference (#708's coupling now lives only in the intake door).
        // This door *rejects* rather than truncating: its values must match a stored name
        // exactly, and a silently-shortened one would match the wrong file, or nothing.
        if s.len() > MAX_FILENAME_ENCODED_BYTES {
            return Err(InvalidFilename::TooLong { encoded: s.len() });
        }
        Ok(Filename(s.to_owned()))
    }
}

impl Filename {
    /// Builds a [`Filename`] by **normalizing** `raw` to a safe leaf via
    /// [`sanitize_filename`], rejecting an empty result. This is the trusted
    /// upload-intake door (the `AtomPub` `Slug` header, a multipart `file_name`),
    /// where a client's arbitrary name is meant to be reduced to a single leaf —
    /// as distinct from [`FromStr`], which rejects a non-canonical name outright.
    ///
    /// An over-long name is **truncated, not rejected** — see [`truncate_to_budget`]. This is
    /// the intake door: failing an upload because the name is long would lose the file for a
    /// cosmetic reason, and shortening it is exactly the "reduce to a usable leaf" job this
    /// door already has (#708).
    ///
    /// # Errors
    ///
    /// Returns [`InvalidFilename::NotASafeLeaf`] when `raw` sanitizes — or truncates — to a
    /// name that is not one: `""`, `.`, or `..`.
    pub fn sanitized(raw: &str) -> Result<Self, InvalidFilename> {
        let s = truncate_to_budget(sanitize_filename(raw));
        // Truncation can leave a degenerate leaf (an empty stem with no extension), and
        // `sanitize_filename` already maps `.`/`..` to empty. Re-checked here — before the
        // encode, since `.`/`..` encode to themselves — so this door's output always
        // satisfies `FromStr`. The oracle half is redundant after `sanitize_filename`, but
        // stating one rule at all three doors is what stops them drifting apart.
        if !is_safe_leaf(&s) {
            return Err(InvalidFilename::NotASafeLeaf);
        }
        // Encode last (#720). The order `sanitize → truncate → encode` is deliberate:
        // truncating in *encoded* space would mean never splitting a `%XX` escape, never
        // splitting the escape run of one multi-byte character (`ä` is `%C3%A4`, and a cut
        // after `%C3` decodes to invalid UTF-8), and still never splitting a grapheme
        // cluster — strictly harder than measuring raw graphemes by their encoded cost,
        // for no gain. `truncate_to_budget` already bounds `encoded_len(s) <= MAX`, and
        // the encoded output's `len()` *is* that number, so the result is in budget by
        // construction.
        Ok(Filename(
            utf8_percent_encode(&s, MEDIA_SEGMENT_ENCODE_SET).to_string(),
        ))
    }

    /// Builds a [`Filename`] from one URL path segment that has already been
    /// percent-decoded by the router.
    ///
    /// This is the extractor/helper door for route parameters whose raw URL spelling is
    /// unavailable: `decoded` is checked as a safe leaf, percent-encoded exactly once with
    /// the media segment encode set, and rejected if that canonical encoded spelling would
    /// exceed [`MAX_FILENAME_ENCODED_BYTES`]. It checks but never repairs, because this is a
    /// lookup key: truncating would risk naming a different stored file.
    ///
    /// Use [`FromStr`] for values that are already in canonical encoded form, and
    /// [`sanitized`][Self::sanitized] for upload intake that should normalize arbitrary
    /// user text.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidFilename::NotASafeLeaf`] when `decoded` is not a safe path leaf, or
    /// [`InvalidFilename::TooLong`] when encoding it would exceed the filesystem component
    /// budget.
    pub fn from_decoded_segment(decoded: &str) -> Result<Self, InvalidFilename> {
        if !is_safe_leaf(decoded) {
            return Err(InvalidFilename::NotASafeLeaf);
        }

        let encoded = utf8_percent_encode(decoded, MEDIA_SEGMENT_ENCODE_SET).to_string();
        if encoded.len() > MAX_FILENAME_ENCODED_BYTES {
            return Err(InvalidFilename::TooLong {
                encoded: encoded.len(),
            });
        }

        Ok(Filename(encoded))
    }

    /// The name as a human should read it — the stored value with its percent-escapes
    /// undone. The **display view**, and the only place a `Filename` is transformed on
    /// the way out (#720).
    ///
    /// Every other consumer — [`media_path`], the URL builders, the `sqlx` bind, a
    /// reference comparison — wants the stored bytes and gets them from the ADR-0063
    /// trailer (`Display`, `Deref<str>`, `AsRef<str>`) with no call. That asymmetry is
    /// deliberate: a missed decode here is cosmetic, whereas a missed *encode* on a path
    /// would be a 404 or, with a name collision, the wrong file. The fragile direction is
    /// the one that must not need remembering.
    ///
    /// Returns [`Cow`] so the common nothing-to-decode case allocates nothing.
    ///
    /// Decoding is **lossy**, and cannot lose anything: a `Filename`'s escapes were
    /// produced by encoding valid UTF-8, and a lone invalid byte such as `%FF` cannot be
    /// stored — it fails the canonicity check, since decoding it yields U+FFFD, which
    /// re-encodes to `%EF%BF%BD` and so differs from the input. The substitution arm is
    /// therefore unreachable on a value of this type.
    #[must_use]
    pub fn decoded(&self) -> Cow<'_, str> {
        percent_decode_str(&self.0).decode_utf8_lossy()
    }
}

/// Shortens `name` until its percent-encoded form fits [`MAX_FILENAME_ENCODED_BYTES`],
/// keeping the extension and never splitting a grapheme cluster. A name already within
/// budget is returned unchanged.
///
/// The extension is kept because [`detect_content_type`] is the **only** content-type source
/// when a client sends no `Content-Type`, and it runs on the already-sanitized name — so
/// dropping the extension here would store `application/octet-stream` permanently, with no
/// way back but a re-upload. (Serving itself reads the stored column, so it is unaffected.)
/// It also keeps the `Content-Disposition` name openable on the user's machine.
///
/// [`Path`]'s stem/extension split is used rather than a last-dot search so a dotfile
/// survives: `.hiddenfile` has no extension, so it is truncated as one piece instead of
/// having its "stem" emptied.
fn truncate_to_budget(name: String) -> String {
    if encoded_len(&name) <= MAX_FILENAME_ENCODED_BYTES {
        return name;
    }
    let path = Path::new(&name);
    let split = path
        .file_stem()
        .and_then(|s| s.to_str())
        .zip(path.extension().and_then(|e| e.to_str()));
    let (stem, extension) = match split {
        Some((stem, extension)) => (stem, format!(".{extension}")),
        None => (name.as_str(), String::new()),
    };
    // Reserve room for the extension **and a minimal stem**. Reserving only the extension
    // would let a name whose first grapheme cluster alone busts the budget — a base character
    // with dozens of combining marks — truncate to bare `.jpg`. That is a *dotfile*, whose
    // `Path::extension()` is `None`, so `detect_content_type` would answer
    // `application/octet-stream` and store it permanently: precisely the loss keeping the
    // extension is meant to prevent. An extension too large to leave that room is beyond
    // saving — drop it and truncate the whole name, accepting the degraded content type.
    let reserved = encoded_len(&extension) + encoded_len(TRUNCATED_STEM);
    let (stem, extension) = if reserved > MAX_FILENAME_ENCODED_BYTES {
        (name.as_str(), String::new())
    } else {
        (stem, extension)
    };

    let budget = MAX_FILENAME_ENCODED_BYTES - encoded_len(&extension);
    let truncated = crate::text::truncate_by_graphemes(stem, budget, encoded_len);
    // Nothing usable survived — or what did is not a name (`.`/`..`). Substitute the same
    // placeholder a *missing* filename gets, so the result is always a real leaf carrying its
    // extension. This is what keeps truncation from ever producing a value `FromStr` would
    // reject, which `sanitized` relies on because it constructs `Filename` directly.
    let mut out = if truncated.is_empty() || truncated == "." || truncated == ".." {
        TRUNCATED_STEM.to_owned()
    } else {
        truncated
    };
    out.push_str(&extension);
    out
}

/// Whether `candidate` is a usable safe leaf: non-empty, not `.` or `..`, and already
/// exactly what [`sanitize_filename`] would make of it (so no path components and no NUL).
///
/// The whole rule in one place. All three of [`Filename`]'s doors need it, and before this
/// existed each spelled its own share of it — which is how a fourth door would come to
/// rediscover the rule rather than reuse it.
///
/// The `is_empty` check is explicit because `sanitize_filename("")` is `""`, so the oracle
/// alone would accept the empty string; `.` and `..` are named because they survive
/// percent-encoding unchanged (`.` is unreserved), so they cannot be caught downstream.
fn is_safe_leaf(candidate: &str) -> bool {
    !candidate.is_empty()
        && candidate != "."
        && candidate != ".."
        && sanitize_filename(candidate) == candidate
}

/// Strip path components, replace null bytes, reject `.`, `..`, and empty results.
#[must_use]
pub fn sanitize_filename(name: &str) -> String {
    let normalized = name.replace('\\', "/");
    let path = Path::new(&normalized);
    let Some(file_name) = path.file_name() else {
        return String::new();
    };
    let s = file_name.to_string_lossy();
    s.replace('\0', "_")
}

/// Returns true if `hash` is a canonical content hash: exactly 64 lowercase
/// hex characters (`[0-9a-f]{64}`), the form produced by `format!("{digest:x}")`
/// for a SHA-256 digest.
///
/// Callers that accept a hash from an untrusted source (e.g. a URL path
/// segment) must check this before slicing or joining it into a path:
/// [`media_path`] slices `sha256[..2]`/`[2..4]` unguarded, which panics on a
/// shorter string or one whose byte index 2 is not a UTF-8 char boundary.
#[must_use]
pub fn is_valid_content_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Source of a media record — the provenance segment of the storage layout
/// (`upload` vs a remote `cached` file). A closed string enum (`#[text_enum]`,
/// ADR-0075 as amended by #746): `serialize_all = "snake_case"` gives the wire/DB
/// token (`upload`/`cached`), and the attribute generates the named
/// `InvalidMediaSource` parse error, the serde bridge — so a bad wire value surfaces
/// the domain error at the `#[server]` media DTO/wire args (#577) — and, via `sqlx`,
/// the typed bind/decode for the stored TEXT token.
#[macros::text_enum(
    sqlx,
    error = InvalidMediaSource,
    message = "media source must be \"upload\" or \"cached\""
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[strum(serialize_all = "snake_case")]
pub enum MediaSource {
    /// File uploaded directly by a local user.
    Upload,
    /// Remote file cached locally by the system.
    Cached,
}

/// The percent-encode set for the filename segment of a media path: everything
/// [`NON_ALPHANUMERIC`] encodes, minus the RFC 3986 *unreserved* marks `-._~`.
///
/// Keeping those four unencoded is what makes an ordinary name round-trip byte-identical —
/// `photo.jpg` stays `photo.jpg` — which is the point of encoding here at all: the on-disk
/// name has to stay greppable and paste-able from a URL. Bare [`NON_ALPHANUMERIC`] would
/// yield `my%2Dphoto%2Ejpg` and make every stored file unreadable. (`content_disposition`
/// in the `server` crate *does* use the bare set; correct there, wrong here.)
const MEDIA_SEGMENT_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// The filesystem's limit on a single path component, in bytes. ext4/XFS/btrfs, APFS and NTFS
/// all cap one name at 255, and the media layout puts the whole filename in a single
/// component, so this is the entire budget.
///
/// It is measured against the **percent-encoded** form, because that is what lands on disk
/// (ADR-0080) — a name can be well under 255 characters and still not fit. That makes this
/// bound depend on [`MEDIA_SEGMENT_ENCODE_SET`]: widening that set shrinks the set of
/// representable names, so the two must be revisited together (#708).
pub const MAX_FILENAME_ENCODED_BYTES: usize = 255;

/// The stem substituted when truncation leaves nothing usable — the same placeholder a
/// *missing* upload filename already gets (`MediaManager::validate_filename`), so operators
/// see one recognizable name for "we had to invent this" rather than two.
const TRUNCATED_STEM: &str = "upload";

/// The encoded byte length of `s` as a media path segment — what the name costs on disk.
///
/// Sums the encoder's output chunks rather than collecting a `String`, so the per-grapheme
/// loop in [`truncate_to_budget`] allocates nothing. The single place the budget is measured,
/// so both `Filename` doors agree.
fn encoded_len(s: &str) -> usize {
    utf8_percent_encode(s, MEDIA_SEGMENT_ENCODE_SET)
        .map(str::len)
        .sum()
}

/// Returns `"<source>/<p1>/<p2>/<full-sha256>/<filename>"`, the content-
/// addressed layout described in the module docs — the **single** definition of that
/// layout, for both the on-disk path and the serve URL.
///
/// The filename segment is interpolated verbatim: a [`Filename`] **is** the canonical
/// percent-encoded path segment (#720), so this is what the file is named on disk, what the
/// URL carries, and what the database column holds — one spelling, no derivation. Callers
/// must not re-derive the layout: the read path and the write path agreeing is exactly what
/// makes the encoding safe.
///
/// Takes a [`ContentHash`] rather than a bare `&str`, so the `sha256[..2]`/`[2..4]`
/// slicing below can never see a short or non-`UTF-8`-boundary value — the type is
/// the guard (its `FromStr`/`from_digest` are the only ways to build one). `p1`/`p2`
/// index through `Deref<Target = str>`. [`MediaSource`] and [`Filename`] are typed rather
/// than `&str` because two adjacent string parameters are silently transposable.
#[must_use]
pub fn media_path(source: &MediaSource, sha256: &ContentHash, filename: &Filename) -> String {
    let p1 = &sha256[..2];
    let p2 = &sha256[2..4];
    let source = source.as_ref();
    format!("{source}/{p1}/{p2}/{sha256}/{filename}")
}

/// Returns `"/media/<source>/<2-hex-p1>/<2-hex-p2>/<full-sha256>/<filename>"` — the
/// [`media_path`] layout under the serve prefix.
///
/// The filename segment is already percent-encoded — a [`Filename`] *is* the canonical
/// segment (#720) — so this URL's tail **is** the path to the file on disk, byte for byte,
/// with nothing transformed on the way. Do not re-derive either one; see [`media_path`] for
/// why the two must not drift.
///
/// Infallible by construction, so it returns the newtype rather than a `Result`: see the
/// body for why the parse cannot fail.
#[must_use]
pub fn media_url(
    source: &MediaSource,
    sha256: &ContentHash,
    filename: &Filename,
) -> RootRelativeUrl {
    let path = format!("/media/{}", media_path(source, sha256, filename));
    let Ok(url) = path.parse() else {
        // Unreachable: the string always starts with a single `/media/`, and the only
        // caller-influenced segment is a `Filename`, whose invariant is that it is already
        // percent-encoded — so no whitespace, `?` or `#` can survive into it. (Nothing
        // encodes here any more; the guarantee comes from the type, not from a transform.)
        // The hash and source segments are a hex digest and a bounded enum token. Same
        // shape as `tagged_url::compose`, and the reason no trusted door is needed here.
        unreachable!("media_url builds a valid root-relative path");
    };
    url
}

/// The triple a media URL names: one stored entry, and one directory entry on disk.
///
/// A bare [`ContentHash`] would be too coarse — identical bytes stored under two names are
/// two distinct (hard-linked) entries — so the filename is part of the identity, and
/// [`MediaSource`] is too because it selects the storage root.
///
/// Carries no `user_id`: a URL does not name an owner, and the same entry can be referenced
/// from any post. Consumers that need ownership join through the referencing post (#711).
///
/// Ordered so a set of references has one deterministic serialization — extraction collects
/// into a `BTreeSet`, which gives dedup and a stable row order in one move, so callers
/// writing those rows get a byte-identical result for a byte-identical body. The order is
/// the derived one: field by field, in declaration order, each member ordering as its inner
/// value. The newtype members get ordering from the standard newtype trailer (ADR-0063 §2,
/// #761). [`MediaSource`] derives its own: it is a `text_enum`, not a newtype, so it has no
/// inner value to delegate to and orders by variant declaration order instead.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MediaRef {
    /// Which storage root the entry lives under.
    pub source: MediaSource,
    /// Content hash of the stored bytes.
    pub sha256: ContentHash,
    /// The canonical, percent-encoded filename segment (ADR-0080, #720).
    pub filename: Filename,
}

/// The parser-canonical, fragment-free URL form retained as media-reference evidence.
///
/// A value is valid only when [`parse_media_url`] recognizes it and emits the exact
/// same form: relative and scheme-relative forms retain their authored spelling,
/// while absolute URLs use `url`'s canonical serialization. Consequently a fragment,
/// unsupported scheme, malformed URL, non-media path, or merely non-canonical absolute
/// form cannot enter persisted evidence. [`FromStr`] is the validating door used by
/// serde and the `SQLx` bridge generated by [`StrNewtype`].
///
/// The wrapped [`String`] is private. Parser output constructs it directly; all other
/// callers must parse an exact canonical form.
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct MediaReferenceForm(String);

/// Error returned when a string is not an exact parser-canonical media reference form.
#[derive(Debug, Error)]
#[error("media reference form must be an exact parser-canonical media URL without a fragment")]
pub struct InvalidMediaReferenceForm;

impl FromStr for MediaReferenceForm {
    type Err = InvalidMediaReferenceForm;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let Some(reference) = parse_media_url(s) else {
            return Err(InvalidMediaReferenceForm);
        };
        if reference.reference_form.as_ref() == s {
            Ok(reference.reference_form)
        } else {
            Err(InvalidMediaReferenceForm)
        }
    }
}

/// A stored-media identity together with the complete URL form that named it.
///
/// The fields are private so a caller cannot manufacture a reference with a
/// media identity and URL evidence that the parser did not establish. The form
/// is retained exactly for local and scheme-relative input (apart from a
/// fragment, which browsers never send); absolute input uses `url`'s canonical
/// serialization.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MediaReference {
    media: MediaRef,
    kind: MediaReferenceKind,
    reference_form: MediaReferenceForm,
}

impl MediaReference {
    /// The canonical stored entry this URL names.
    #[must_use]
    pub fn media(&self) -> &MediaRef {
        &self.media
    }

    /// The syntactic URL form that named this entry.
    #[must_use]
    pub fn kind(&self) -> MediaReferenceKind {
        self.kind
    }

    /// The fragment-free form storage persists and a live ownership resolver probes.
    #[must_use]
    pub fn reference_form(&self) -> &MediaReferenceForm {
        &self.reference_form
    }
}

/// Parses a media URL into the [`MediaReference`] it names, or `None` if it names none.
///
/// Relative and root-relative input is local. Absolute input must be HTTP(S), has a
/// canonical `url` serialization, and rejects userinfo. Scheme-relative input retains its
/// authored authority, explicit port, path, and query, while dropping its fragment. The path
/// remains the exact serve or AtomPub-member layout and filename validation is unchanged.
#[must_use]
pub fn parse_media_url(input: &str) -> Option<MediaReference> {
    let fragment_free = input.split_once('#').map_or(input, |(form, _)| form);

    if fragment_free.starts_with("//") {
        let url = Url::parse(&format!("http:{fragment_free}")).ok()?;
        if has_userinfo(fragment_free, &url) {
            return None;
        }
        return parse_media_path(url.path()).map(|media| MediaReference {
            media,
            kind: MediaReferenceKind::SchemeRelative,
            reference_form: MediaReferenceForm(fragment_free.to_owned()),
        });
    }

    if has_leading_url_scheme(fragment_free) {
        let mut url = Url::parse(fragment_free).ok()?;
        if !matches!(url.scheme(), "http" | "https") || has_userinfo(fragment_free, &url) {
            return None;
        }
        url.set_fragment(None);
        return parse_media_path(url.path()).map(|media| MediaReference {
            media,
            kind: MediaReferenceKind::Absolute,
            reference_form: MediaReferenceForm(url.into()),
        });
    }

    parse_media_path(
        fragment_free
            .split_once('?')
            .map_or(fragment_free, |(path, _)| path),
    )
    .map(|media| MediaReference {
        media,
        kind: MediaReferenceKind::Local,
        reference_form: MediaReferenceForm(fragment_free.to_owned()),
    })
}

/// Whether the input's authority carries userinfo.
///
/// `url` canonicalizes an empty `@` username away, so inspect only the original
/// authority delimiter as well as the parsed username/password.
fn has_userinfo(input: &str, url: &Url) -> bool {
    let authority = input
        .strip_prefix("//")
        .or_else(|| input.split_once("://").map(|(_, authority)| authority));
    !url.username().is_empty()
        || url.password().is_some()
        || authority.is_some_and(|authority| {
            authority
                .split_once(['/', '?'])
                .map_or(authority, |(authority, _)| authority)
                .contains('@')
        })
}

/// Whether `input` syntactically starts with an RFC 3986 scheme.
///
/// A colon in a relative path, query, or fragment does not make the input absolute.
fn has_leading_url_scheme(input: &str) -> bool {
    let Some((scheme, _)) = input.split_once(':') else {
        return false;
    };
    let mut characters = scheme.bytes();
    matches!(characters.next(), Some(b'a'..=b'z' | b'A'..=b'Z'))
        && characters.all(|character| {
            matches!(
                character,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'+' | b'-' | b'.'
            )
        })
}

fn parse_media_path(path: &str) -> Option<MediaRef> {
    let mut segments = path.trim_start_matches('/').split('/');
    let first = segments.next()?;
    let (source, hash, filename) = match first {
        "media" => {
            let source = segments.next()?.parse().ok()?;
            let p1 = segments.next()?;
            let p2 = segments.next()?;
            let hash: ContentHash = segments.next()?.parse().ok()?;
            let filename = segments.next()?;
            if segments.next().is_some() || p1 != &hash[..2] || p2 != &hash[2..4] {
                return None;
            }
            (source, hash, filename)
        }
        "atompub" => {
            let _user = segments.next()?;
            if segments.next()? != "media" {
                return None;
            }
            let hash = segments.next()?.parse().ok()?;
            let filename = segments.next()?;
            if segments.next().is_some() {
                return None;
            }
            (MediaSource::Upload, hash, filename)
        }
        _ => return None,
    };

    // Decode, then re-encode through the decoded-segment door. That door is also the
    // arbiter of whether the name could be stored at all.
    let decoded = percent_decode_str(filename).decode_utf8().ok()?;
    let filename = Filename::from_decoded_segment(&decoded).ok()?;
    Some(MediaRef {
        source,
        sha256: hash,
        filename,
    })
}

/// A media `Content-Type` header value — a `type/subtype` media type with optional
/// `;`-separated parameters (e.g. `image/png`, `text/html; charset=utf-8`). Introducing
/// the type means an arbitrary string can no longer stand in for a media content type
/// (ADR-0063 §1), and every accepted value is a valid HTTP header / Atom `type=` value.
///
/// [`FromStr`] is the single validating chokepoint — it delegates to the private
/// `is_valid_content_type`. The rest of the ADR-0063 string-newtype trailer (`Display`,
/// `AsRef<str>`, `Borrow<str>`, `Deref<Target = str>`, owned `String` conversions,
/// `PartialEq<str>`, and the validating serde + sqlx bridges) is generated by
/// `#[derive(StrNewtype)]`. The wrapped `String` is private, so the only way in is the
/// validating door — an arbitrary `String` cannot masquerade as a content type:
///
/// The positive companion shows the identical fixture compiles — the path resolves
/// and the validating door accepts a real media type — so each `compile_fail`
/// below fails for the private field, not for a moved path. (Fixture lines are
/// hidden with `#`.)
///
/// ```
/// use common::media::ContentType;
/// use std::str::FromStr;
/// let t = ContentType::from_str("image/png").unwrap();
/// let _read: &str = t.as_ref();
/// ```
///
/// No public constructor:
/// ```compile_fail
/// # use common::media::ContentType;
/// # use std::str::FromStr;
/// let _ = ContentType("x".to_string()); // private field
/// ```
///
/// A `String` does not convert to a `ContentType` (asserted on `.into()`, so the
/// proof still fails if a `From<String>` impl is added — a function argument would
/// not):
/// ```compile_fail
/// # use common::media::ContentType;
/// # use std::str::FromStr;
/// let _t: ContentType = "image/png".to_string().into();
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct ContentType(String);

impl ContentType {
    /// Mint a `ContentType` from a string the caller asserts is a valid media type —
    /// a fixed `&'static` literal or other known-valid source — bypassing the
    /// [`FromStr`] check. The trusted-producer door, `pub(crate)` so outside this crate
    /// the only way in stays the validating `FromStr`.
    ///
    /// Grepping `ContentType::from_trusted` enumerates every mint site, and each is
    /// pinned by a test that the value is valid (`detect_content_type_outputs_are_valid`,
    /// `feed_path::…::format_content_types`). That is a **convention backed by those
    /// tests**, not a build-time guarantee: nothing fails if a new mint site arrives
    /// without one — the `rendered-html-from-trusted` gate reads the qualifier (#790),
    /// so this door is outside its population.
    #[must_use]
    pub(crate) fn from_trusted(content_type: impl Into<String>) -> Self {
        Self(content_type.into())
    }

    /// PNG image media type.
    #[must_use]
    pub fn image_png() -> Self {
        Self::from_trusted("image/png")
    }
}

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

/// An RFC 7230 `tchar` (the token characters a `type`/`subtype` may use).
fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// Whether `s` is a valid media `Content-Type`: every byte a valid `HeaderValue` byte
/// (`VCHAR` / SP / HTAB), and the essence (before the first `;`) is `token "/" token` with
/// non-empty, `tchar`-only halves. Parameters (after `;`) need only be header-safe — so
/// every accepted value is `HeaderValue::from_str`-constructible. The single source of
/// truth for [`ContentType`]'s invariant.
fn is_valid_content_type(s: &str) -> bool {
    if !s.bytes().all(|b| b == b'\t' || (0x20..=0x7e).contains(&b)) {
        return false;
    }
    // The essence is everything before the first `;` (or the whole string when there is no
    // parameter list); RFC 7231 permits optional whitespace around it, so trim for the check.
    let essence = s.split_once(';').map_or(s, |(before, _)| before).trim();
    let Some((ty, sub)) = essence.split_once('/') else {
        return false;
    };
    !ty.is_empty() && !sub.is_empty() && ty.bytes().all(is_tchar) && sub.bytes().all(is_tchar)
}

/// Returns true if the content type should be served inline rather than as an attachment.
#[must_use]
pub fn should_inline(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/jpeg"
            | "image/png"
            | "image/gif"
            | "image/webp"
            | "image/svg+xml"
            | "audio/mpeg"
            | "audio/ogg"
            | "audio/flac"
            | "audio/wav"
            | "video/mp4"
            | "video/webm"
            | "application/pdf"
    )
}

/// Extension-based content type detection. Falls back to `application/octet-stream`.
/// Mints the [`ContentType`] via [`ContentType::from_trusted`] from its canonical
/// `&'static str` table (all valid `type/subtype` literals) — no `FromStr` round-trip,
/// since the table is fixed and known-valid (pinned by
/// `detect_content_type_outputs_are_valid`).
#[must_use]
pub fn detect_content_type(filename: &Filename) -> ContentType {
    static EXTENSIONS: [(&[&str], &str); 12] = [
        (&["jpg", "jpeg"], "image/jpeg"),
        (&["png"], "image/png"),
        (&["gif"], "image/gif"),
        (&["webp"], "image/webp"),
        (&["svg"], "image/svg+xml"),
        (&["mp3"], "audio/mpeg"),
        (&["ogg", "oga"], "audio/ogg"),
        (&["flac"], "audio/flac"),
        (&["wav"], "audio/wav"),
        (&["mp4"], "video/mp4"),
        (&["webm"], "video/webm"),
        (&["pdf"], "application/pdf"),
    ];

    let ext = Path::new(filename.decoded().as_ref())
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    for (extensions, content_type) in EXTENSIONS {
        if extensions.contains(&ext.as_str()) {
            return ContentType::from_trusted(content_type);
        }
    }
    ContentType::from_trusted("application/octet-stream")
}

/// The maximum accepted upload size, in bytes (site config `media.max_file_size_bytes`).
/// A positive `i64` — a zero/negative limit is nonsensical — enforced by the
/// `NumNewtype`-generated validating `FromStr`/serde. Default 50 MiB.
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(
    inner = i64,
    min = 1,
    default = 52_428_800, // 50 MiB
    error = "media max file size must be a positive number of bytes"
)]
pub struct MaxFileSize(i64);

/// The per-user upload quota, in bytes (site config `media.user_quota_bytes`).
/// A positive `i64`, like [`MaxFileSize`]; a distinct type so a per-file limit and a
/// per-user quota can't be transposed. Default 1 GiB.
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(
    inner = i64,
    min = 1,
    default = 1_073_741_824, // 1 GiB
    error = "media user quota must be a positive number of bytes"
)]
pub struct UserQuota(i64);

/// A non-negative count of bytes — a *measured/stored* size (a media file's byte length,
/// a user's total upload usage), the actual-value counterpart to the [`MaxFileSize`] /
/// [`UserQuota`] *limits*. `min = 0` (an empty object is 0 bytes) and no `default` (it is
/// measured, never a config fallback). Unlike the limits — which are only ever built from
/// config strings — a `ByteSize` is built from a runtime `i64` (a DB column, a `SUM`), so it
/// relies on the `NumNewtype` validating `TryFrom<i64>` door.
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(
    inner = i64,
    min = 0,
    error = "byte size must be a non-negative number of bytes"
)]
pub struct ByteSize(i64);

/// Metadata for a successfully stored upload — the server-fn wire value (#517), living
/// here (not in `server`) so it is nameable on the wasm client. `storage`'s
/// `MediaManager` and `web`'s `media::upload` return it directly; `AtomPub` consumes its
/// identity to load and serialize the stored record. Every field is a validated `common`
/// newtype, so each re-validates on deserialize — including `url`, the derived serve path,
/// which is a
/// [`RootRelativeUrl`][crate::root_relative_url::RootRelativeUrl] because being *derived*
/// is not a reason to leave it stringly (ADR-0063 §5), and because the derivation is only
/// well-formed thanks to [`media_path`]'s encoding, which the type is what pins.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadedMedia {
    pub sha256: ContentHash,
    pub filename: Filename,
    pub content_type: ContentType,
    pub size_bytes: ByteSize,
    pub url: RootRelativeUrl,
}

#[cfg(test)]
mod tests {
    use crate::test_support::{MEDIA_TEST_SHA256, parse_content_hash};
    use rstest::rstest;
    use sha2::{Digest, Sha256};

    use super::*;

    /// Drives the whole `NumNewtype`-generated surface of a positive-`i64` (min-1) byte
    /// newtype `T`: parse accept/trim, reject `0`/negative/non-integer with the domain
    /// message, `Default`, `Display` round-trip, `From<Self> for i64`, and the
    /// transparent-`i64` serde bridge (round-trip + wire-rejection of `0`). Both byte-limit
    /// types share this shape, so one generic assertion replaces two near-identical tests.
    /// The DTO does not serde these on the host build, so this is the reachability for that
    /// generated code. Written via `From`/`.ok()`/`.err()` (no `unwrap`), so it needs no
    /// lint exception.
    fn assert_positive_byte_newtype<T>(default: i64, err_prefix: &str)
    where
        T: ::core::str::FromStr
            + ::core::default::Default
            + ::core::fmt::Display
            + ::core::fmt::Debug
            + ::core::marker::Copy
            + ::core::cmp::PartialEq
            + ::serde::Serialize
            + ::serde::de::DeserializeOwned,
        T::Err: ::core::fmt::Display,
        i64: ::core::convert::From<T>,
    {
        // parse accepts and trims
        assert_eq!("5".parse::<T>().map(i64::from).ok(), Some(5));
        assert_eq!("  100  ".parse::<T>().map(i64::from).ok(), Some(100));
        // parse rejects 0, negatives, and non-integers...
        for bad in ["0", "-1", "abc", "1.5"] {
            assert!(bad.parse::<T>().is_err(), "{bad} should reject");
        }
        // ...with the domain message
        assert!(
            "0".parse::<T>()
                .err()
                .is_some_and(|e| e.to_string().starts_with(err_prefix))
        );
        // Default, and From<Self> for i64
        let d = T::default();
        assert_eq!(i64::from(d), default);
        // Display round-trips through FromStr
        assert_eq!(d.to_string().parse::<T>().ok(), Some(d));
        // serde: bare integer, round-trip, wire-rejection of 0
        assert_eq!(serde_json::to_string(&d).ok(), Some(default.to_string()));
        assert_eq!(
            serde_json::from_str::<T>("42").map(i64::from).ok(),
            Some(42)
        );
        assert!(serde_json::from_str::<T>("0").is_err());
    }

    #[test]
    fn max_file_size_surface() {
        assert_positive_byte_newtype::<MaxFileSize>(52_428_800, "media max file size");
    }

    #[test]
    fn user_quota_surface() {
        assert_positive_byte_newtype::<UserQuota>(1_073_741_824, "media user quota");
    }

    #[test]
    fn byte_size_surface() {
        // `ByteSize` has its own test — it is min-0 (accepts `0`) and has no `default`, so it
        // cannot use `assert_positive_byte_newtype` (min-1, `Default`-requiring). Drives every
        // generated branch for coverage.
        assert_eq!("0".parse::<ByteSize>().map(i64::from).ok(), Some(0));
        assert_eq!(
            "  2048  ".parse::<ByteSize>().map(i64::from).ok(),
            Some(2048)
        );
        for bad in ["-1", "abc", "1.5"] {
            assert!(bad.parse::<ByteSize>().is_err(), "{bad} should reject");
        }
        assert!(
            "-1".parse::<ByteSize>()
                .err()
                .is_some_and(|e| e.to_string().starts_with("byte size"))
        );
        // Display round-trips through FromStr
        let b = "4096".parse::<ByteSize>().unwrap();
        assert_eq!(b.to_string().parse::<ByteSize>().ok(), Some(b));
        // From<Self> for i64
        assert_eq!(i64::from(b), 4096);
        // serde: transparent integer, round-trip, and wire-rejection of a *negative* (the
        // deserialize min-guard arm — `0` is accepted, so a negative is what reaches it)
        assert_eq!(serde_json::to_string(&b).ok(), Some("4096".to_string()));
        assert_eq!(
            serde_json::from_str::<ByteSize>("11").map(i64::from).ok(),
            Some(11)
        );
        assert!(serde_json::from_str::<ByteSize>("-1").is_err());
        // the new validating `TryFrom<i64>` door — accept 0, reject negative
        assert_eq!(ByteSize::try_from(0i64).map(i64::from).ok(), Some(0));
        assert!(ByteSize::try_from(-1i64).is_err());
    }

    #[test]
    fn sanitize_strips_path_components() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("foo/bar/baz.txt"), "baz.txt");
        assert_eq!(sanitize_filename("C:\\Users\\file.txt"), "file.txt");
    }

    #[test]
    fn sanitize_replaces_unsafe_chars() {
        assert_eq!(sanitize_filename("file\0name.txt"), "file_name.txt");
        assert_eq!(sanitize_filename("\0"), "_");
    }

    #[test]
    fn sanitize_rejects_empty() {
        assert!(sanitize_filename("").is_empty());
        assert!(sanitize_filename(".").is_empty());
        assert!(sanitize_filename("..").is_empty());
    }

    /// A validated filename built through the same intake door as uploaded names.
    fn filename(name: &str) -> Filename {
        Filename::sanitized(name).expect("a media test name is a valid leaf")
    }

    /// Detect a type from an inbound filename, preserving the production boundary shape.
    fn detected_content_type(name: &str) -> ContentType {
        detect_content_type(&filename(name))
    }

    /// The canonical hash and a [`Filename`], the two typed arguments every layout test
    /// needs. `name` is the name a *user would type*, built through the intake door —
    /// since #720 the strict door takes the canonical encoded spelling, so parsing a raw
    /// name here would reject it. Callers keep passing what a person types; the helper
    /// yields what gets stored.
    fn layout_args(name: &str) -> (ContentHash, Filename) {
        (parse_content_hash(MEDIA_TEST_SHA256), filename(name))
    }

    #[test]
    fn media_path_computation() {
        let (hash, filename) = layout_args("photo.jpg");
        let path = media_path(&MediaSource::Upload, &hash, &filename);
        assert_eq!(path, format!("upload/e3/b0/{MEDIA_TEST_SHA256}/photo.jpg"));
    }

    #[test]
    fn media_url_computation() {
        let (hash, filename) = layout_args("photo.jpg");
        let url = media_url(&MediaSource::Upload, &hash, &filename);
        assert_eq!(
            url,
            format!("/media/upload/e3/b0/{MEDIA_TEST_SHA256}/photo.jpg").as_str()
        );
    }

    /// What `media_url` adds over [`media_path`] is the **type**: the exact encoding of each
    /// name is pinned once, by `media_path`'s own tests. So these assert only that a
    /// `RootRelativeUrl` exists at all for names that could not be one, and that no URL
    /// delimiter survives into it.
    #[test]
    fn media_url_is_representable_for_names_the_newtype_would_otherwise_reject() {
        // A space makes the value unrepresentable — `RootRelativeUrl` rejects whitespace —
        // which is what blocked typing the serve URL in the first place. `?`/`#` are the
        // failure the newtype *cannot* catch: it accepts a query, so an unencoded
        // `what?.png` would validate while addressing a different file.
        for raw in ["a b.txt", "what?.png", "a#b.png"] {
            let (hash, filename) = layout_args(raw);
            let url = media_url(&MediaSource::Upload, &hash, &filename);
            assert!(
                !url.contains(' ') && !url.contains('?') && !url.contains('#'),
                "{raw} must not carry whitespace or a URL delimiter: {url}"
            );
            assert!(url.starts_with("/media/upload/"), "{raw} → {url}");
        }
    }

    #[test]
    fn media_path_leaves_ordinary_names_byte_identical() {
        // Pins `MEDIA_SEGMENT_ENCODE_SET`'s unreserved-mark carve-out. With bare NON_ALPHANUMERIC
        // these become `my%2Dphoto%2Ejpg` and every file on disk is unreadable.
        for name in ["photo.jpg", "my-photo_2.png", "a~b.txt", "IMG1234.JPEG"] {
            let (hash, filename) = layout_args(name);
            let path = media_path(&MediaSource::Upload, &hash, &filename);
            assert_eq!(
                path,
                format!("upload/e3/b0/{MEDIA_TEST_SHA256}/{name}"),
                "{name} must survive encoding unchanged"
            );
        }
    }

    #[test]
    fn media_path_interpolates_the_already_encoded_name() {
        // Encoding happens once, at intake (#720) — `media_path` only interpolates. So
        // this pins two things at once: that the intake door produces the right
        // spelling for each hazard, and that the path is byte-identical to it. A space
        // makes the URL unrepresentable as `RootRelativeUrl`; `?`/`#` are worse — they
        // pass its validation while truncating the path, addressing another file. `%`
        // must encode too, or a pre-existing escape is double-decoded on the way back.
        for (raw, encoded) in [
            ("a b.txt", "a%20b.txt"),
            ("what?.png", "what%3F.png"),
            ("a#b.png", "a%23b.png"),
            ("50%.png", "50%25.png"),
            ("café.png", "caf%C3%A9.png"),
        ] {
            let (hash, filename) = layout_args(raw);
            let path = media_path(&MediaSource::Upload, &hash, &filename);
            assert_eq!(filename, encoded, "{raw} must be stored as {encoded}");
            assert_eq!(
                path,
                format!("upload/e3/b0/{MEDIA_TEST_SHA256}/{encoded}"),
                "{raw} must encode to {encoded}"
            );
        }
    }

    #[rstest]
    #[case::jpeg("image/jpeg", true)]
    #[case::png("image/png", true)]
    #[case::gif("image/gif", true)]
    #[case::webp("image/webp", true)]
    #[case::svg("image/svg+xml", true)]
    #[case::mpeg("audio/mpeg", true)]
    #[case::mp4("video/mp4", true)]
    #[case::pdf("application/pdf", true)]
    #[case::zip("application/zip", false)]
    #[case::text("text/plain", false)]
    #[case::octet_stream("application/octet-stream", false)]
    fn should_inline_classifies_content_types(#[case] content_type: &str, #[case] expected: bool) {
        assert_eq!(should_inline(content_type), expected);
    }

    #[rstest]
    #[case::jpeg("photo.jpg", "image/jpeg")]
    #[case::jpeg_alias("photo.jpeg", "image/jpeg")]
    #[case::png("image.png", "image/png")]
    #[case::gif("anim.gif", "image/gif")]
    #[case::webp("photo.webp", "image/webp")]
    #[case::svg("icon.svg", "image/svg+xml")]
    #[case::mpeg("track.mp3", "audio/mpeg")]
    #[case::ogg("track.ogg", "audio/ogg")]
    #[case::oga("track.oga", "audio/ogg")]
    #[case::flac("track.flac", "audio/flac")]
    #[case::wav("track.wav", "audio/wav")]
    #[case::mp4("video.mp4", "video/mp4")]
    #[case::webm("clip.webm", "video/webm")]
    #[case::pdf("doc.pdf", "application/pdf")]
    #[case::unknown_extension("file.xyz", "application/octet-stream")]
    #[case::no_extension("noext", "application/octet-stream")]
    fn detect_content_type_classifies_filenames(#[case] filename: &str, #[case] expected: &str) {
        assert_eq!(detected_content_type(filename), expected);
    }

    #[test]
    fn content_type_accepts_valid() {
        for s in [
            "image/png",
            "application/pdf",
            "image/svg+xml",
            "text/html; charset=utf-8",
            "application/octet-stream",
        ] {
            assert!(s.parse::<ContentType>().is_ok(), "must accept {s:?}");
        }
    }

    #[test]
    fn content_type_rejects_malformed() {
        for s in [
            "",
            "garbage",             // no slash
            "a/",                  // empty subtype
            "/b",                  // empty type
            "image /png",          // space in type (not a tchar)
            "text/plain; x=\u{1}", // control byte inside a parameter
            "im\u{1}age/png",      // control byte in the type
        ] {
            assert!(s.parse::<ContentType>().is_err(), "must reject {s:?}");
        }
    }

    #[test]
    fn detect_content_type_outputs_are_valid() {
        // The in-module mint is honest: every canonical literal + the fallback re-parses
        // through the validating `FromStr`, so `detect` never fabricates an invalid value.
        for f in [
            "a.jpg",
            "a.png",
            "a.gif",
            "a.webp",
            "a.svg",
            "a.mp3",
            "a.ogg",
            "a.flac",
            "a.wav",
            "a.mp4",
            "a.webm",
            "a.pdf",
            "a.unknownext",
        ] {
            assert!(
                detected_content_type(f)
                    .as_ref()
                    .parse::<ContentType>()
                    .is_ok()
            );
        }
    }

    #[test]
    fn valid_content_hash_accepts_64_lowercase_hex() {
        let hash = "a".repeat(64);
        assert!(is_valid_content_hash(&hash));
        // A realistic lowercase sha256 hex digest.
        assert!(is_valid_content_hash(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
    }

    #[test]
    fn valid_content_hash_rejects_short_input() {
        // A single byte cannot be sliced at [2..]; short input must reject, not panic.
        assert!(!is_valid_content_hash("a"));
        assert!(!is_valid_content_hash(""));
        assert!(!is_valid_content_hash(&"a".repeat(63)));
    }

    #[test]
    fn valid_content_hash_rejects_long_input() {
        assert!(!is_valid_content_hash(&"a".repeat(65)));
    }

    #[test]
    fn valid_content_hash_rejects_uppercase_hex() {
        // Stored digests are lowercase (`format!("{digest:x}")`); uppercase is not canonical.
        assert!(!is_valid_content_hash(&"A".repeat(64)));
    }

    #[test]
    fn valid_content_hash_rejects_non_hex_chars() {
        // 64 chars but contains a non-hex letter.
        assert!(!is_valid_content_hash(&format!("g{}", "a".repeat(63))));
        // 64 chars but contains a path separator.
        assert!(!is_valid_content_hash(&format!("/{}", "a".repeat(63))));
    }

    #[test]
    fn valid_content_hash_rejects_non_ascii_off_boundary() {
        // A multi-byte char makes byte index 2 land off a UTF-8 boundary — the
        // other off-boundary slice hazard: build a 64-byte string whose char
        // boundaries do not align with byte 2. It must reject, not panic.
        let hash = format!("é{}", "a".repeat(62));
        assert!(!is_valid_content_hash(&hash));
    }

    #[test]
    fn content_hash_parses_canonical_digest() {
        let h: ContentHash = MEDIA_TEST_SHA256.parse().unwrap();
        assert_eq!(h, MEDIA_TEST_SHA256);
    }

    #[test]
    fn content_hash_rejects_non_canonical_forms() {
        // Reuses the `is_valid_content_hash` invariants: short, long, uppercase,
        // non-hex, off-boundary.
        assert!("a".parse::<ContentHash>().is_err());
        assert!("a".repeat(63).parse::<ContentHash>().is_err());
        assert!("a".repeat(65).parse::<ContentHash>().is_err());
        assert!("A".repeat(64).parse::<ContentHash>().is_err());
        assert!(
            format!("g{}", "a".repeat(63))
                .parse::<ContentHash>()
                .is_err()
        );
        assert!(
            format!("é{}", "a".repeat(62))
                .parse::<ContentHash>()
                .is_err()
        );
    }

    #[test]
    fn content_hash_display_produces_the_canonical_string() {
        let h: ContentHash = MEDIA_TEST_SHA256.parse().unwrap();
        assert_eq!(h.to_string(), MEDIA_TEST_SHA256);
    }

    #[test]
    fn content_hash_serde_serializes_as_plain_string_and_validates_on_deserialize() {
        let h: ContentHash = MEDIA_TEST_SHA256.parse().unwrap();
        assert_eq!(
            serde_json::to_string(&h).unwrap(),
            format!("\"{MEDIA_TEST_SHA256}\"")
        );

        // Deserialize routes through the validating parse.
        assert_eq!(
            serde_json::from_str::<ContentHash>(&format!("\"{MEDIA_TEST_SHA256}\"")).unwrap(),
            h
        );
        // Invalid input is rejected at deserialize time (wire rejection).
        assert!(serde_json::from_str::<ContentHash>("\"not-a-hash\"").is_err());
    }

    #[test]
    fn content_hash_from_digest_hex_encodes_the_32_digest_bytes() {
        // The producer door hex-encodes exactly 32 bytes into the canonical
        // 64-char lowercase form — always a valid ContentHash by construction.
        assert_eq!(ContentHash::from_digest([0u8; 32]), "0".repeat(64).as_str());
        assert_eq!(
            ContentHash::from_digest([0xab; 32]),
            "ab".repeat(32).as_str()
        );
        // A real digest round-trips: its hex equals the canonical string form.
        let digest: [u8; 32] = Sha256::digest(b"").into();
        assert_eq!(ContentHash::from_digest(digest), MEDIA_TEST_SHA256);
    }

    #[test]
    fn content_hash_invalid_error_displays_the_rule() {
        let msg = "bad".parse::<ContentHash>().unwrap_err().to_string();
        assert!(msg.contains("64 lowercase hex"), "{msg}");
    }

    // --- Filename: Door A (validating FromStr, canonical-only) ---

    #[test]
    fn filename_parses_a_canonical_leaf() {
        let f: Filename = "photo.jpg".parse().unwrap();
        assert_eq!(f, "photo.jpg");
    }

    #[test]
    fn filename_rejects_non_canonical_and_empty() {
        // Non-canonical names (which Door B *would* normalize) and empty are
        // rejected here, not normalized — Door A only accepts an already-safe leaf.
        for bad in [
            "",
            "..",
            ".",
            "a/b",
            "../x",
            "sub/file.txt",
            "C:\\x\\y.txt",
            "foo\0",
        ] {
            assert!(bad.parse::<Filename>().is_err(), "must reject {bad:?}");
        }
    }

    #[test]
    fn filename_display_and_deref_read_the_leaf() {
        let f: Filename = "a.txt".parse().unwrap();
        assert_eq!(f.to_string(), "a.txt");
        assert_eq!(&f[..1], "a"); // Deref<Target = str>
    }

    // --- Filename: the display view (#720) ---

    #[test]
    fn decoded_is_identity_for_a_name_with_nothing_encoded() {
        let f = Filename::sanitized("photo.jpg").expect("valid leaf");
        assert_eq!(f.decoded(), "photo.jpg");
    }

    #[test]
    fn decoded_borrows_when_there_is_nothing_to_decode() {
        let f = Filename::sanitized("photo.jpg").expect("valid leaf");
        assert!(matches!(f.decoded(), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn decoded_undoes_percent_escapes() {
        // Constructed through `FromStr` so this states the intended post-#720
        // relationship directly, independent of what `sanitized` currently stores.
        let f: Filename = "my%20photo.jpg".parse().expect("a safe leaf today");
        assert_eq!(f.decoded(), "my photo.jpg");
    }

    #[test]
    fn decoded_recovers_a_literal_percent() {
        let f: Filename = "50%25.jpg".parse().expect("a safe leaf today");
        assert_eq!(f.decoded(), "50%.jpg");
    }

    // --- Filename: Door B (normalizing producer) ---

    #[test]
    fn sanitized_normalizes_to_a_safe_leaf() {
        assert_eq!(Filename::sanitized("../../etc/passwd").unwrap(), "passwd");
        assert_eq!(Filename::sanitized("foo/bar/baz.txt").unwrap(), "baz.txt");
        assert_eq!(
            Filename::sanitized("file\0name.txt").unwrap(),
            "file_name.txt"
        );
    }

    #[test]
    fn sanitized_rejects_empty_after_normalization() {
        for bad in ["", ".", ".."] {
            assert!(Filename::sanitized(bad).is_err(), "must reject {bad:?}");
        }
    }

    #[test]
    fn filename_serde_serializes_as_plain_string_and_validates_on_deserialize() {
        let f: Filename = "photo.jpg".parse().unwrap();
        assert_eq!(serde_json::to_string(&f).unwrap(), "\"photo.jpg\"");
        assert_eq!(
            serde_json::from_str::<Filename>("\"photo.jpg\"").unwrap(),
            f
        );
        // A non-canonical name is rejected at deserialize time (wire rejection).
        assert!(serde_json::from_str::<Filename>("\"../x\"").is_err());
    }

    #[test]
    fn filename_invalid_error_displays_the_rule() {
        let msg = "..".parse::<Filename>().unwrap_err().to_string();
        assert!(msg.contains("safe path leaf"), "{msg}");
    }

    // --- #708: the length bound is on the ENCODED form ---

    // #708's original case now lives at the decoded-segment door — see
    // `decoded_segment_rejects_a_name_whose_encoded_form_exceeds_the_budget`. Since #720 a
    // `Filename` holds the already-encoded value, so `"ä".repeat(100)` is non-canonical
    // and fails that check before length is ever reached; the strict door's own bound is
    // a plain byte count, pinned by `from_str_rejects_an_over_long_canonical_name`.

    #[test]
    fn from_str_accepts_a_name_exactly_at_the_budget() {
        // Boundary: `<=` not `<`. Plain ASCII encodes 1:1, so 255 chars is 255 bytes.
        let raw = "a".repeat(MAX_FILENAME_ENCODED_BYTES);
        assert!(raw.parse::<Filename>().is_ok());
        let over = "a".repeat(MAX_FILENAME_ENCODED_BYTES + 1);
        assert!(matches!(
            over.parse::<Filename>().expect_err("one over must fail"),
            InvalidFilename::TooLong { .. }
        ));
    }

    #[test]
    fn from_str_still_reports_a_bad_leaf_as_such() {
        // The two failures stay distinguishable — the length check runs after the leaf check.
        assert!(matches!(
            "../escape"
                .parse::<Filename>()
                .expect_err("traversal must fail"),
            InvalidFilename::NotASafeLeaf
        ));
    }

    #[test]
    fn sanitized_truncates_instead_of_failing_and_keeps_the_extension() {
        let long = format!("{}.jpg", "a".repeat(400));
        let f = Filename::sanitized(&long).expect("the intake door truncates, never fails here");
        assert!(f.ends_with(".jpg"), "extension must survive: {f}");
        assert!(f.len() <= MAX_FILENAME_ENCODED_BYTES, "{f}");
        // The property the extension is kept *for*: the stored content type remains JPEG.
        // Asserting merely "ends_with(.jpg)" would pass on a mangled extension.
        assert_eq!(detect_content_type(&f), "image/jpeg");
    }

    #[test]
    fn sanitized_truncation_is_measured_in_encoded_bytes_not_characters() {
        // ~3× expansion. Truncating to 255 *characters* would leave ~765 encoded bytes —
        // still unwritable. This is the assertion that pins D1's choice.
        let long = format!("{}.png", "ä".repeat(300));
        let f = Filename::sanitized(&long).expect("must truncate");
        assert!(f.len() <= MAX_FILENAME_ENCODED_BYTES, "{f}");
        assert!(
            f.chars().count() < 255,
            "must cut well short of 255 chars: {f}"
        );
        assert_eq!(detect_content_type(&f), "image/png");
    }

    #[test]
    fn sanitized_never_splits_a_grapheme_cluster() {
        // Devanagari base + combining vowel sign: cutting between them corrupts the
        // character. Re-parsing through the strict door proves the result is a valid,
        // in-budget leaf.
        let long = format!("{}.txt", "कि".repeat(200));
        let f = Filename::sanitized(&long).expect("must truncate");
        assert!(f.len() <= MAX_FILENAME_ENCODED_BYTES, "{f}");
        assert!(f.as_ref().parse::<Filename>().is_ok(), "{f}");
        // No lone combining mark left at the cut.
        assert!(!f.starts_with('\u{093F}'), "{f}");
    }

    #[test]
    fn sanitized_preserves_a_dotfile_rather_than_treating_it_as_all_extension() {
        // `Path::extension()` is `None` for a dotfile, so it truncates as one piece. A
        // manual last-dot split would empty the "stem" and destroy the name.
        assert_eq!(
            Filename::sanitized(".hiddenfile").expect("valid leaf"),
            ".hiddenfile"
        );
        let long = format!(".{}", "a".repeat(400));
        let f = Filename::sanitized(&long).expect("must truncate");
        assert!(f.starts_with('.'), "leading dot must survive: {f}");
        assert!(f.len() <= MAX_FILENAME_ENCODED_BYTES, "{f}");
    }

    #[test]
    fn sanitized_truncates_the_whole_name_when_the_extension_alone_is_over_budget() {
        let f = Filename::sanitized(&format!("x.{}", "ä".repeat(300))).expect("must truncate");
        assert!(f.len() <= MAX_FILENAME_ENCODED_BYTES, "{f}");
        assert!(f.as_ref().parse::<Filename>().is_ok(), "{f}");
    }

    #[test]
    fn sanitized_rejects_input_that_reduces_to_a_degenerate_name() {
        for bad in ["", ".", ".."] {
            assert!(matches!(
                Filename::sanitized(bad).expect_err("degenerate names are not filenames"),
                InvalidFilename::NotASafeLeaf
            ));
        }
    }

    /// A stem that is a **single** grapheme cluster too large for the budget: one base
    /// character carrying ~80 combining marks. Nothing of the stem fits, so a naive
    /// implementation emits bare `.jpg` — a *dotfile*, whose `Path::extension()` is `None`,
    /// so `detect_content_type` answers `application/octet-stream` and stores it forever.
    /// That is the exact loss keeping the extension is supposed to prevent, so it is pinned.
    #[test]
    fn sanitized_substitutes_a_stem_when_no_cluster_of_the_original_fits() {
        let zalgo = format!("a{}.jpg", "\u{0301}".repeat(80));
        let f = Filename::sanitized(&zalgo).expect("must truncate, not fail");

        assert!(f.len() <= MAX_FILENAME_ENCODED_BYTES, "{f}");
        assert!(f.ends_with(".jpg"), "{f}");
        // The property that matters: still detected as an image, not octet-stream.
        assert_eq!(detect_content_type(&f), "image/jpeg");
        assert_ne!(f, ".jpg", "a bare extension is a dotfile, not a filename");
        // And it is a value the strict door accepts — `sanitized` builds `Filename` directly,
        // so nothing else enforces that.
        assert!(f.as_ref().parse::<Filename>().is_ok(), "{f}");
    }

    #[test]
    fn sanitized_never_reports_a_length_failure_as_a_bad_leaf() {
        // Truncation must always yield a usable name, so a merely-long input is never an
        // error — the `NotASafeLeaf` guard is only for what `sanitize_filename` itself
        // empties (`""`, `.`, `..`). A regression here would surface as a baffling
        // "must be a non-empty safe path leaf" for a name that is one.
        for long in [
            format!("a{}.jpg", "\u{0301}".repeat(80)), // no cluster of the stem fits
            format!("a{}", "\u{0301}".repeat(80)),     // ditto, and no extension to keep
            format!(".{}", "\u{0301}".repeat(80)),     // dotfile whose only cluster busts it
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}".repeat(40), // ZWJ family emoji
        ] {
            assert!(
                Filename::sanitized(&long).is_ok(),
                "a long-but-legal name must truncate, not fail: {long:?}"
            );
        }
    }

    // Idempotence pin: whatever the intake door (B) emits must re-parse through the strict
    // door (A). Otherwise a stored, B-written filename becomes a `Decode` error on read-back
    // — this fails loudly here instead.
    //
    // Asserted on `Filename::sanitized`, not on `sanitize_filename`: the bare oracle
    // does not truncate (#708), so its output for a long name is over budget and Door A
    // rightly rejects it. The claim that has to hold is about the door callers
    // actually use.
    #[test]
    fn sanitized_output_always_reparses_as_filename() {
        for raw in [
            "photo.jpg",
            "../../etc/passwd",
            "foo/bar/baz.txt",
            "C:\\Users\\file.txt",
            "file\0name.txt",
            "a b.txt",
            ".hidden",
            "no-ext",
            // Over budget once encoded — the cases that motivated #708.
            &"ä".repeat(300),
            &format!("{}.jpg", "a".repeat(400)),
        ] {
            // Asserted as `Ok` first, deliberately: allowing `Err` to satisfy the claim
            // would hide `sanitized` regressing to *rejecting* long names (#708). None
            // of these inputs is degenerate, so every one must succeed.
            let f = Filename::sanitized(raw)
                .unwrap_or_else(|e| panic!("sanitized({raw:?}) must succeed, got {e}"));
            assert!(
                f.as_ref().parse::<Filename>().is_ok(),
                "sanitized({raw:?}) = {f:?} must re-parse as Filename"
            );
        }
    }

    // --- Filename: Door C (decoded URL segment conversion) ---

    #[test]
    fn decoded_segment_accepts_a_safe_leaf_with_space() {
        let f = Filename::from_decoded_segment("my photo.jpg").expect("a safe decoded leaf");
        assert_eq!(f, "my%20photo.jpg");
        assert_eq!(f.decoded(), "my photo.jpg");
    }

    #[test]
    fn decoded_segment_rejects_unsafe_decoded_leaves() {
        // The decoded segment axum hands us. `a\b.png` is not a leaf, and the member
        // route answers 400 for it — pinned here so #720's re-encode cannot silently
        // turn that into a 404.
        for bad in ["a\\b.png", "a/b.png", "..", ".", "", "a\0b.png"] {
            assert!(
                Filename::from_decoded_segment(bad).is_err(),
                "must reject {bad:?}"
            );
        }
    }

    #[test]
    fn decoded_segment_rejects_an_over_long_name_rather_than_truncating() {
        // Reject, never repair: a shortened lookup key would match the wrong file.
        let long = format!("{}.jpg", "a".repeat(MAX_FILENAME_ENCODED_BYTES));
        assert!(Filename::from_decoded_segment(&long).is_err());
    }

    #[test]
    fn decoded_segment_conversion_returns_filename_directly() {
        // The door returns `Filename`, not a public decoded-intermediate trailer.
        let f: Filename = Filename::from_decoded_segment("photo.jpg").expect("a safe decoded leaf");
        assert_eq!(f, "photo.jpg");
    }

    // --- #720: the encoded form is canonical ---

    #[test]
    fn sanitized_stores_the_encoded_form() {
        let f = Filename::sanitized("my photo.jpg").expect("valid leaf");
        assert_eq!(f, "my%20photo.jpg");
        assert_eq!(f.decoded(), "my photo.jpg");
    }

    #[test]
    fn from_str_rejects_a_non_canonical_value() {
        // Raw (unencoded) and a lowercase escape are both non-canonical.
        assert!("my photo.jpg".parse::<Filename>().is_err());
        assert!("my%2fphoto.jpg".parse::<Filename>().is_err());
        assert!("my%20photo.jpg".parse::<Filename>().is_ok());
    }

    #[test]
    fn from_str_rejects_canonical_but_unsafe_values() {
        // The test that fails if the decoded-form safe-leaf guard is dropped. Each of
        // these is canonical, non-empty, neither `.` nor `..`, and under the length
        // bound — so only a check run on `decode(s)` rejects it. Running the oracle on
        // the encoded form would pass vacuously.
        for bad in ["a%2Fb.jpg", "a%00b.jpg", "a%5Cb.jpg"] {
            assert!(
                bad.parse::<Filename>().is_err(),
                "canonical-but-unsafe value must be rejected: {bad:?}"
            );
        }
    }

    #[test]
    fn from_str_accepts_a_canonical_name_carrying_control_characters() {
        // CR/LF in a canonical name is accepted — decided, not missed (#720):
        // `sanitize_filename` normalizes backslashes, strips path components and maps
        // NUL, but does not reject CR/LF. No live hazard follows: `content_disposition`
        // drops control characters from its `filename=` fallback and percent-encodes
        // `filename*=`, the stored spelling holds no literal control byte, and CR/LF
        // are legal XML in the Atom `<title>` (NUL, which is not, is caught above).
        // Tightening this would change what uploads are accepted — a separate decision.
        assert!("a%0D%0Ab.jpg".parse::<Filename>().is_ok());
    }

    #[test]
    fn from_str_distinguishes_non_canonical_from_a_bad_leaf() {
        // Check order matters: `a/b.txt` is BOTH an unsafe leaf and non-canonical, and
        // must still report the leaf failure. A merely-unencoded name reports the new
        // variant.
        assert!(matches!(
            "a/b.txt".parse::<Filename>().expect_err("not a leaf"),
            InvalidFilename::NotASafeLeaf
        ));
        assert!(matches!(
            "my photo.jpg"
                .parse::<Filename>()
                .expect_err("not canonical"),
            InvalidFilename::NotCanonical
        ));
    }

    #[test]
    fn media_path_interpolates_without_encoding() {
        let f = Filename::sanitized("my photo.jpg").expect("valid leaf");
        let hash = ContentHash::from_digest(Sha256::digest(b"x").into());
        let path = media_path(&MediaSource::Upload, &hash, &f);
        assert!(path.ends_with("/my%20photo.jpg"), "{path}");
        // The stored value IS the path segment — byte identity, not a derivation.
        assert!(path.ends_with(&format!("/{f}")), "{path}");
    }

    #[test]
    fn a_literal_percent_round_trips() {
        // The case that exposes a double-encode or double-decode.
        let f = Filename::sanitized("50%.jpg").expect("valid leaf");
        assert_eq!(f, "50%25.jpg");
        assert_eq!(f.decoded(), "50%.jpg");
        assert!(
            f.as_ref().parse::<Filename>().is_ok(),
            "a canonical value must re-parse"
        );
    }

    #[test]
    fn a_user_typed_escape_does_not_materialize_a_separator() {
        // `a%2Fb.jpg` typed literally must store double-encoded, so no `/` appears in
        // any derived path segment — the traversal this arrangement must never permit.
        let f = Filename::sanitized("a%2Fb.jpg").expect("valid leaf");
        assert_eq!(f, "a%252Fb.jpg");
        assert_eq!(f.decoded(), "a%2Fb.jpg");
        let hash = ContentHash::from_digest(Sha256::digest(b"x").into());
        let path = media_path(&MediaSource::Upload, &hash, &f);
        let segment = path.rsplit('/').next().expect("a trailing segment");
        assert_eq!(segment, "a%252Fb.jpg");
    }

    #[test]
    fn decoded_segment_re_encodes_the_decoded_segment() {
        // The serve door: axum hands us the decoded name; the stored form must come back.
        assert_eq!(
            Filename::from_decoded_segment("my photo.jpg").expect("a safe decoded leaf"),
            "my%20photo.jpg"
        );
    }

    #[test]
    fn decoded_segment_output_always_satisfies_filename() {
        for raw in [
            "photo.jpg",
            "my photo.jpg",
            "50%.jpg",
            "résumé.pdf",
            ".hiddenfile",
        ] {
            let f = Filename::from_decoded_segment(raw).expect("a safe decoded leaf");
            assert!(
                f.as_ref().parse::<Filename>().is_ok(),
                "must re-parse: {raw:?}"
            );
        }
    }

    #[test]
    fn from_str_rejects_an_over_long_canonical_name() {
        // At `FromStr` the value is already encoded, so the bound is a plain byte count.
        let over = "a".repeat(MAX_FILENAME_ENCODED_BYTES + 1);
        assert!(matches!(
            over.parse::<Filename>().expect_err("over budget"),
            InvalidFilename::TooLong { .. }
        ));
    }

    #[test]
    fn decoded_segment_rejects_a_name_whose_encoded_form_exceeds_the_budget() {
        // #708's original case, relocated: the decoded-segment door receives the decoded
        // name, so this is where "100 chars, 200 raw bytes, 600 encoded" is still the
        // hazard a char-count bound would miss.
        let raw = "ä".repeat(100);
        let err =
            Filename::from_decoded_segment(&raw).expect_err("an over-budget name must be rejected");
        assert!(matches!(err, InvalidFilename::TooLong { .. }), "{err}");
        let msg = err.to_string();
        assert!(msg.contains("percent-encoded"), "{msg}");
        assert!(msg.contains("255"), "{msg}");
    }

    #[test]
    fn media_source_tokens_parse_and_round_trip() {
        assert_eq!(
            "upload".parse::<MediaSource>().unwrap(),
            MediaSource::Upload
        );
        assert_eq!(
            "cached".parse::<MediaSource>().unwrap(),
            MediaSource::Cached
        );
        assert_eq!(MediaSource::Upload.as_ref(), "upload");
        assert_eq!(MediaSource::Cached.to_string(), "cached");
    }

    #[test]
    fn media_source_unknown_token_is_rejected_with_message() {
        let err = "bogus".parse::<MediaSource>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "media source must be \"upload\" or \"cached\""
        );
    }

    #[test]
    fn media_source_serde_round_trips_the_token() {
        assert_eq!(
            serde_json::to_string(&MediaSource::Cached).unwrap(),
            "\"cached\""
        );
        assert_eq!(
            serde_json::from_str::<MediaSource>("\"upload\"").unwrap(),
            MediaSource::Upload
        );
    }

    // -----------------------------------------------------------------------
    // parse_media_url — the inverse of `media_url` (#711)
    // -----------------------------------------------------------------------

    /// The canonical `Filename` for a raw (undecoded) name, via the decoded-segment door.
    fn canonical(raw: &str) -> Filename {
        Filename::from_decoded_segment(raw).expect("a legal filename")
    }

    #[test]
    fn parse_media_url_retains_local_path_and_root_relative_forms_without_fragments() {
        for form in [
            format!("media/upload/e3/b0/{MEDIA_TEST_SHA256}/photo.jpg?download=1#preview"),
            format!("/media/upload/e3/b0/{MEDIA_TEST_SHA256}/photo.jpg?download=1#preview"),
        ] {
            let reference = parse_media_url(&form).expect("local media URL parses");
            assert_eq!(reference.kind(), MediaReferenceKind::Local);
            assert_eq!(
                reference.reference_form(),
                form.split_once('#').map_or(form.as_str(), |(form, _)| form)
            );
        }
    }

    #[test]
    fn parse_media_url_round_trips_every_source_and_encoded_names_as_local() {
        for source in [MediaSource::Upload, MediaSource::Cached] {
            for raw in ["photo.jpg", "my photo.jpg", "ünïcode nàme.png", "100%.jpg"] {
                let filename = canonical(raw);
                let hash: ContentHash = MEDIA_TEST_SHA256.parse().unwrap();
                let url = media_url(&source, &hash, &filename);
                let reference = parse_media_url(&url).expect("relative media URL parses");
                assert_eq!(
                    reference.media(),
                    &MediaRef {
                        source,
                        sha256: hash,
                        filename,
                    },
                    "round trip failed for {source:?} / {raw}"
                );
                assert_eq!(reference.kind(), MediaReferenceKind::Local);
                assert_eq!(reference.reference_form(), url.as_ref());
            }
        }
    }

    #[test]
    fn parse_media_url_canonicalises_absolute_form_and_removes_fragment() {
        let path = format!("/media/upload/e3/b0/{MEDIA_TEST_SHA256}/photo.jpg");
        let reference =
            parse_media_url(&format!("HTTPS://Example.COM:443{path}?download=1#preview"))
                .expect("absolute URL parses");
        assert_eq!(reference.kind(), MediaReferenceKind::Absolute);
        assert_eq!(
            reference.reference_form().as_ref(),
            format!("https://example.com{path}?download=1")
        );
    }

    #[test]
    fn media_reference_form_accepts_only_exact_parser_output() {
        let path = format!("/media/upload/e3/b0/{MEDIA_TEST_SHA256}/photo.jpg");
        let canonical = format!("https://example.com{path}?download=1");
        let form: MediaReferenceForm = canonical.parse().expect("canonical form parses");
        assert_eq!(form.as_ref(), canonical);

        for invalid in [
            format!("HTTPS://Example.COM:443{path}?download=1"),
            format!("{canonical}#preview"),
            format!("ftp://example.com{path}"),
            "/not-media.jpg".to_owned(),
        ] {
            assert!(
                invalid.parse::<MediaReferenceForm>().is_err(),
                "{invalid} must not enter persisted evidence"
            );
        }
    }

    #[test]
    fn parse_media_url_preserves_scheme_relative_authority_port_path_and_query() {
        let path = format!("/media/upload/e3/b0/{MEDIA_TEST_SHA256}/photo.jpg");
        let reference = parse_media_url(&format!("//Example.COM:8443{path}?download=1#preview"))
            .expect("scheme-relative URL parses");
        assert_eq!(reference.kind(), MediaReferenceKind::SchemeRelative);
        assert_eq!(
            reference.reference_form().as_ref(),
            format!("//Example.COM:8443{path}?download=1")
        );
    }

    #[test]
    fn parse_media_url_accepts_the_atompub_member_layout_as_upload() {
        let url = format!("/atompub/alice/media/{MEDIA_TEST_SHA256}/photo.jpg");
        let reference = parse_media_url(&url).expect("AtomPub media member URL parses");
        assert_eq!(
            reference.media(),
            &MediaRef {
                source: MediaSource::Upload,
                sha256: MEDIA_TEST_SHA256.parse().unwrap(),
                filename: canonical("photo.jpg"),
            }
        );
        assert_eq!(reference.kind(), MediaReferenceKind::Local);
        assert_eq!(reference.reference_form().as_ref(), url);
    }

    #[test]
    fn parse_media_url_canonicalises_a_raw_filename_to_the_stored_spelling() {
        let raw = format!("/media/upload/e3/b0/{MEDIA_TEST_SHA256}/my photo.jpg");
        let encoded = format!("/media/upload/e3/b0/{MEDIA_TEST_SHA256}/my%20photo.jpg");
        let raw_reference = parse_media_url(&raw).expect("raw spelling parses");
        let encoded_reference = parse_media_url(&encoded).expect("encoded spelling parses");
        assert_eq!(raw_reference.media(), encoded_reference.media());
        assert_ne!(
            raw_reference.reference_form(),
            encoded_reference.reference_form()
        );
        assert_eq!(raw_reference.media().filename.as_ref(), "my%20photo.jpg");
    }

    #[test]
    fn parse_media_url_rejects_non_exact_shard_prefixes() {
        for (p1, p2) in [("ff", "b0"), ("e3", "ff"), ("e", "b0"), ("e3", "b")] {
            assert_eq!(
                parse_media_url(&format!(
                    "/media/upload/{p1}/{p2}/{MEDIA_TEST_SHA256}/photo.jpg"
                )),
                None
            );
        }
    }

    #[test]
    fn parse_media_url_rejects_userinfo_malformed_and_non_http_absolute_urls() {
        let path = format!("/media/upload/e3/b0/{MEDIA_TEST_SHA256}/photo.jpg");
        for url in [
            format!("https://user:password@example.com{path}"),
            format!("https://@example.com{path}"),
            format!("https://{path}"),
            format!("https://example.com:bad{path}"),
            format!("ftp://example.com{path}"),
            format!("mailto:media@example.com{path}"),
            format!("//user:password@example.com{path}"),
            // This begins with a letter but contains a forbidden scheme byte, so
            // it must remain a relative form rather than reaching URL parsing.
            format!("http^s:{path}"),
        ] {
            assert_eq!(parse_media_url(&url), None, "{url} must not be a reference");
        }
    }

    #[test]
    fn parse_media_url_rejects_non_media_paths() {
        assert_eq!(parse_media_url("/posts/hello"), None);
        assert_eq!(parse_media_url("/media/upload/e3/b0/short/photo.jpg"), None);
        assert_eq!(
            parse_media_url(&format!(
                "/media/bogus-source/e3/b0/{MEDIA_TEST_SHA256}/photo.jpg"
            )),
            None,
            "an unknown source token is not a media URL"
        );
        assert_eq!(parse_media_url(""), None);
        assert_eq!(parse_media_url("/media/upload/e3/b0"), None);
        assert_eq!(
            parse_media_url(&format!("/media/upload/e3/b0/{MEDIA_TEST_SHA256}/a/b.jpg")),
            None,
            "the filename is one segment, so a deeper path is not this layout"
        );
        assert_eq!(
            parse_media_url(&format!(
                "/atompub/alice/not-media/{MEDIA_TEST_SHA256}/photo.jpg"
            )),
            None,
            "the AtomPub member layout requires its media segment"
        );
        assert_eq!(
            parse_media_url(&format!(
                "/atompub/alice/media/{MEDIA_TEST_SHA256}/photo.jpg/extra"
            )),
            None,
            "the AtomPub filename is the final path segment"
        );
    }

    #[test]
    fn media_refs_order_by_source_then_hash_then_filename() {
        // The ordering exists so a set of references serializes one way for one body:
        // extraction collects into a `BTreeSet`, so this is what makes the written rows
        // deterministic rather than hash-order.
        let hash: ContentHash = MEDIA_TEST_SHA256.parse().unwrap();
        let make = |source, name| MediaRef {
            source,
            sha256: hash.clone(),
            filename: canonical(name),
        };

        // Same hash: the filename breaks the tie.
        assert!(
            make(MediaSource::Upload, "a.jpg") < make(MediaSource::Upload, "b.jpg"),
            "filename orders last"
        );
        // The source dominates the filename. Which source sorts first is the *derived*
        // order — by variant declaration, so `Upload` before `Cached` — not the
        // lexicographic order of their tokens, which would put `cached` first. Nothing
        // depends on the direction; the ordering exists only so one body yields one
        // byte-identical set of rows.
        assert!(
            make(MediaSource::Upload, "z.jpg") < make(MediaSource::Cached, "a.jpg"),
            "source orders first"
        );

        let mut sorted = [
            make(MediaSource::Cached, "z.jpg"),
            make(MediaSource::Upload, "b.jpg"),
            make(MediaSource::Upload, "a.jpg"),
        ];
        sorted.sort();
        let names: Vec<&str> = sorted.iter().map(|r| r.filename.as_ref()).collect();
        // Both `Upload`s first (source dominates), `a` before `b` within them, and the
        // `Cached` one last regardless of its filename sorting first.
        assert_eq!(names, ["a.jpg", "b.jpg", "z.jpg"]);
    }

    #[test]
    fn media_refs_deduplicate_in_a_btree_set() {
        // A post embedding the same image twice must yield one row, without needing
        // dialect-divergent conflict handling at the insert.
        let hash: ContentHash = MEDIA_TEST_SHA256.parse().unwrap();
        let one = MediaRef {
            source: MediaSource::Upload,
            sha256: hash,
            filename: canonical("photo.jpg"),
        };
        let set: std::collections::BTreeSet<MediaRef> =
            [one.clone(), one.clone(), one].into_iter().collect();
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn parse_media_url_rejects_a_filename_the_stored_door_would_reject() {
        // An over-budget or unsafe segment cannot name a stored entry, so it is not a
        // reference — the decoded-segment door is the single arbiter of that.
        let long = "a".repeat(MAX_FILENAME_ENCODED_BYTES + 1);
        assert_eq!(
            parse_media_url(&format!("/media/upload/e3/b0/{MEDIA_TEST_SHA256}/{long}")),
            None
        );
    }
}
