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
//! # Untrusted input
//!
//! Filenames and hashes round-trip through URLs, so they are attacker-
//! influenced. [`sanitize_filename`] reduces a name to a single safe path
//! component, and [`is_valid_content_hash`] must gate any externally supplied
//! hash before it reaches [`media_path`], whose `sha256[..2]`/`[2..4]` slicing
//! is unguarded and panics on a short or non-`UTF-8`-boundary value.
//!
//! # Content type
//!
//! [`detect_content_type`] maps a filename extension to a `MIME` type (falling
//! back to `application/octet-stream`), and [`should_inline`] decides whether a
//! type is served inline or as an attachment (the `Content-Disposition`).

use std::path::Path;

/// Strip path components, replace null bytes, reject `.`, `..`, and empty results.
#[must_use]
pub fn sanitize_filename(name: &str) -> String {
    // Normalize Windows backslashes to forward slashes before extracting file name
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

/// Returns `"<source>/<p1>/<p2>/<full-sha256>/<filename>"`, the content-
/// addressed layout described in the module docs.
///
/// # Panics
///
/// Panics if `sha256` is shorter than four bytes, or if byte index 2 or 4 does
/// not fall on a `UTF-8` char boundary (the slicing is unguarded). Validate an
/// untrusted hash with [`is_valid_content_hash`] before calling this.
#[must_use]
pub fn media_path(source: &str, sha256: &str, filename: &str) -> String {
    let p1 = &sha256[..2];
    let p2 = &sha256[2..4];
    format!("{source}/{p1}/{p2}/{sha256}/{filename}")
}

/// Returns `"/media/<source>/<2-hex-p1>/<2-hex-p2>/<full-sha256>/<filename>"`.
#[must_use]
pub fn media_url(source: &str, sha256: &str, filename: &str) -> String {
    format!("/media/{}", media_path(source, sha256, filename))
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

/// Extension-based content type detection. Falls back to `"application/octet-stream"`.
#[must_use]
pub fn detect_content_type(filename: &str) -> &'static str {
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

    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    for (extensions, content_type) in EXTENSIONS {
        if extensions.contains(&ext.as_str()) {
            return content_type;
        }
    }
    "application/octet-stream"
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn media_path_computation() {
        let path = media_path("upload", "a3f2deadbeef1234abcd", "photo.jpg");
        assert_eq!(path, "upload/a3/f2/a3f2deadbeef1234abcd/photo.jpg");
    }

    #[test]
    fn media_url_computation() {
        let url = media_url("upload", "a3f2deadbeef1234abcd", "photo.jpg");
        assert_eq!(url, "/media/upload/a3/f2/a3f2deadbeef1234abcd/photo.jpg");
    }

    #[test]
    fn content_disposition_inline_for_images() {
        assert!(should_inline("image/jpeg"));
        assert!(should_inline("image/png"));
        assert!(should_inline("image/gif"));
        assert!(should_inline("image/webp"));
        assert!(should_inline("image/svg+xml"));
    }

    #[test]
    fn content_disposition_inline_for_media() {
        assert!(should_inline("audio/mpeg"));
        assert!(should_inline("video/mp4"));
        assert!(should_inline("application/pdf"));
    }

    #[test]
    fn content_disposition_attachment_for_others() {
        assert!(!should_inline("application/zip"));
        assert!(!should_inline("text/plain"));
        assert!(!should_inline("application/octet-stream"));
    }

    #[test]
    fn detect_content_type_known_extensions() {
        assert_eq!(detect_content_type("photo.jpg"), "image/jpeg");
        assert_eq!(detect_content_type("photo.jpeg"), "image/jpeg");
        assert_eq!(detect_content_type("image.png"), "image/png");
        assert_eq!(detect_content_type("doc.pdf"), "application/pdf");
        assert_eq!(detect_content_type("video.mp4"), "video/mp4");
    }

    #[test]
    fn detect_content_type_image_formats() {
        assert_eq!(detect_content_type("anim.gif"), "image/gif");
        assert_eq!(detect_content_type("photo.webp"), "image/webp");
        assert_eq!(detect_content_type("icon.svg"), "image/svg+xml");
    }

    #[test]
    fn detect_content_type_audio_formats() {
        assert_eq!(detect_content_type("track.mp3"), "audio/mpeg");
        assert_eq!(detect_content_type("track.ogg"), "audio/ogg");
        assert_eq!(detect_content_type("track.oga"), "audio/ogg");
        assert_eq!(detect_content_type("track.flac"), "audio/flac");
        assert_eq!(detect_content_type("track.wav"), "audio/wav");
    }

    #[test]
    fn detect_content_type_video_webm() {
        assert_eq!(detect_content_type("clip.webm"), "video/webm");
    }

    #[test]
    fn detect_content_type_unknown_extension() {
        assert_eq!(detect_content_type("file.xyz"), "application/octet-stream");
        assert_eq!(detect_content_type("noext"), "application/octet-stream");
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
        // The historical panic trigger: a single byte cannot be sliced at [2..].
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
        // other historical panic source. 21 'é' chars = 42 bytes ... build a
        // 64-byte string whose char boundaries do not align with byte 2.
        let hash = format!("é{}", "a".repeat(62));
        assert!(!is_valid_content_hash(&hash));
    }
}
