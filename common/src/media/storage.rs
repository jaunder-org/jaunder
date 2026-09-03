use crate::root_relative_url::RootRelativeUrl;

use super::{filename::Filename, hash::ContentHash};

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
pub fn path(source: &MediaSource, sha256: &ContentHash, filename: &Filename) -> String {
    let p1 = &sha256[..2];
    let p2 = &sha256[2..4];
    let source = source.as_ref();
    format!("{source}/{p1}/{p2}/{sha256}/{filename}")
}

/// Returns `"/media/<source>/<2-hex-p1>/<2-hex-p2>/<full-sha256>/<filename>"` — the
/// [`path`] layout under the serve prefix.
///
/// The filename segment is already percent-encoded — a [`Filename`] *is* the canonical
/// segment (#720) — so this URL's tail **is** the path to the file on disk, byte for byte,
/// with nothing transformed on the way. Do not re-derive either one; see [`path`] for
/// why the two must not drift.
///
/// Infallible by construction, so it returns the newtype rather than a `Result`: see the
/// body for why the parse cannot fail.
#[must_use]
pub fn url(source: &MediaSource, sha256: &ContentHash, filename: &Filename) -> RootRelativeUrl {
    let path = format!("/media/{}", path(source, sha256, filename));
    let Ok(url) = path.parse() else {
        // Unreachable: the string always starts with a single `/media/`, and the only
        // caller-influenced segment is a `Filename`, whose invariant is that it is already
        // percent-encoded — so no whitespace, `?` or `#` can survive into it. (Nothing
        // encodes here any more; the guarantee comes from the type, not from a transform.)
        // The hash and source segments are a hex digest and a bounded enum token. Same
        // shape as `tagged_url::compose`, and the reason no trusted door is needed here.
        unreachable!("media::url builds a valid root-relative path");
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

/// Recognizes the storage-owned /media/ layout emitted by the URL constructor.
pub(crate) fn parse_stored_media_path(path: &str) -> Option<MediaRef> {
    let mut segments = path.trim_start_matches('/').split('/');
    if segments.next()? != "media" {
        return None;
    }
    let source = segments.next()?.parse().ok()?;
    let p1 = segments.next()?;
    let p2 = segments.next()?;
    let hash: ContentHash = segments.next()?.parse().ok()?;
    let encoded_filename = segments.next()?;
    if segments.next().is_some() || p1 != &hash[..2] || p2 != &hash[2..4] {
        return None;
    }
    let decoded = percent_encoding::percent_decode_str(encoded_filename)
        .decode_utf8()
        .ok()?;
    let filename = Filename::from_decoded_segment(&decoded).ok()?;
    Some(MediaRef {
        source,
        sha256: hash,
        filename,
    })
}

#[cfg(test)]
mod tests {
    use crate::test_support::{MEDIA_TEST_SHA256, parse_content_hash};
    use sha2::{Digest, Sha256};

    use super::*;

    /// A validated filename built through the same intake door as uploaded names.
    fn filename(name: &str) -> Filename {
        Filename::sanitized(name).expect("a media test name is a valid leaf")
    }
    /// The canonical `Filename` for a raw (undecoded) name, via the decoded-segment door.
    fn canonical(raw: &str) -> Filename {
        Filename::from_decoded_segment(raw).expect("a legal filename")
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
    fn path_computation() {
        let (hash, filename) = layout_args("photo.jpg");
        let path = path(&MediaSource::Upload, &hash, &filename);
        assert_eq!(path, format!("upload/e3/b0/{MEDIA_TEST_SHA256}/photo.jpg"));
    }

    #[test]
    fn url_computation() {
        let (hash, filename) = layout_args("photo.jpg");
        let url = url(&MediaSource::Upload, &hash, &filename);
        assert_eq!(
            url,
            format!("/media/upload/e3/b0/{MEDIA_TEST_SHA256}/photo.jpg").as_str()
        );
    }

    /// What [`url`] adds over [`path`] is the **type**: the exact encoding of each
    /// name is pinned once, by `path`'s own tests. So these assert only that a
    /// `RootRelativeUrl` exists at all for names that could not be one, and that no URL
    /// delimiter survives into it.
    #[test]
    fn url_is_representable_for_names_the_newtype_would_otherwise_reject() {
        // A space makes the value unrepresentable — `RootRelativeUrl` rejects whitespace —
        // which is what blocked typing the serve URL in the first place. `?`/`#` are the
        // failure the newtype *cannot* catch: it accepts a query, so an unencoded
        // `what?.png` would validate while addressing a different file.
        for raw in ["a b.txt", "what?.png", "a#b.png"] {
            let (hash, filename) = layout_args(raw);
            let url = url(&MediaSource::Upload, &hash, &filename);
            assert!(
                !url.contains(' ') && !url.contains('?') && !url.contains('#'),
                "{raw} must not carry whitespace or a URL delimiter: {url}"
            );
            assert!(url.starts_with("/media/upload/"), "{raw} → {url}");
        }
    }

    #[test]
    fn path_leaves_ordinary_names_byte_identical() {
        // Pins `MEDIA_SEGMENT_ENCODE_SET`'s unreserved-mark carve-out. With bare NON_ALPHANUMERIC
        // these become `my%2Dphoto%2Ejpg` and every file on disk is unreadable.
        for name in ["photo.jpg", "my-photo_2.png", "a~b.txt", "IMG1234.JPEG"] {
            let (hash, filename) = layout_args(name);
            let path = path(&MediaSource::Upload, &hash, &filename);
            assert_eq!(
                path,
                format!("upload/e3/b0/{MEDIA_TEST_SHA256}/{name}"),
                "{name} must survive encoding unchanged"
            );
        }
    }

    #[test]
    fn path_interpolates_the_already_encoded_name() {
        // Encoding happens once, at intake (#720) — `path` only interpolates. So
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
            let path = path(&MediaSource::Upload, &hash, &filename);
            assert_eq!(filename, encoded, "{raw} must be stored as {encoded}");
            assert_eq!(
                path,
                format!("upload/e3/b0/{MEDIA_TEST_SHA256}/{encoded}"),
                "{raw} must encode to {encoded}"
            );
        }
    }

    #[test]
    fn path_interpolates_without_encoding() {
        let f = Filename::sanitized("my photo.jpg").expect("valid leaf");
        let hash = ContentHash::from_digest(Sha256::digest(b"x").into());
        let path = path(&MediaSource::Upload, &hash, &f);
        assert!(path.ends_with("/my%20photo.jpg"), "{path}");
        // The stored value IS the path segment — byte identity, not a derivation.
        assert!(path.ends_with(&format!("/{f}")), "{path}");
    }

    #[test]
    fn a_user_typed_escape_does_not_materialize_a_separator() {
        // `a%2Fb.jpg` typed literally must store double-encoded, so no `/` appears in
        // any derived path segment — the traversal this arrangement must never permit.
        let f = Filename::sanitized("a%2Fb.jpg").expect("valid leaf");
        assert_eq!(f, "a%252Fb.jpg");
        assert_eq!(f.decoded(), "a%2Fb.jpg");
        let hash = ContentHash::from_digest(Sha256::digest(b"x").into());
        let path = path(&MediaSource::Upload, &hash, &f);
        let segment = path.rsplit('/').next().expect("a trailing segment");
        assert_eq!(segment, "a%252Fb.jpg");
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

    // -----------------------------------------------------------------------
    // parse_media_url — the inverse of `url` (#711)
    // -----------------------------------------------------------------------
}
