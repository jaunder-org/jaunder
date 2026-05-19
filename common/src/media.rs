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
    if s == "." || s == ".." {
        return String::new();
    }
    let sanitized = s.replace('\0', "_");
    if sanitized.is_empty() {
        return String::new();
    }
    sanitized
}

/// Returns `"<source>/<p1>/<p2>/<full-sha256>/<filename>"`.
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
    }

    #[test]
    fn sanitize_rejects_empty() {
        assert!(sanitize_filename("").is_empty());
        assert!(sanitize_filename("..").is_empty());
        assert!(sanitize_filename(".").is_empty());
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
    fn detect_content_type_unknown_extension() {
        assert_eq!(detect_content_type("file.xyz"), "application/octet-stream");
        assert_eq!(detect_content_type("noext"), "application/octet-stream");
    }
}
