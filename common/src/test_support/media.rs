// Test scaffolding that deliberately `expect()`s on a fixture parse, so the
// workspace's `expect_used = deny` lint is expected off for this module; `#[expect]`
// self-flags if the scaffolding ever stops using `expect`.
// lint-suppression:allow approved in #294; existing expectation documents intentional test-scaffolding or naming exception
#![expect(clippy::expect_used)]

use crate::media::{
    filename::Filename,
    hash::ContentHash,
    mime::ContentType,
    values::{ByteSize, MaxFileSize, UserQuota},
};

/// The content hash every media fixture is stored under: a realistic lowercase
/// SHA-256 hex digest (the digest of the empty input), so the value is a real digest
/// rather than an invented hex string.
///
/// The workspace's one spelling of it. Kept as a `&str` rather than a [`ContentHash`]
/// because most uses interpolate it into a path, URL, or SQL literal; the ones that
/// want the type pass it through [`parse_content_hash`]. `storage::test_support`
/// re-exports it as `MEDIA_TEST_SHA256`, which is what its media fixtures are built on.
pub const MEDIA_TEST_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Parse `s` into a valid [`ContentHash`] for tests — the single place a test
/// media-content-hash literal is parsed, so a malformed fixture fails loudly and
/// the parse isn't re-spelled at every media store-seeding call site.
///
/// # Panics
///
/// Panics if `s` is not 64 lowercase hex characters.
#[must_use]
pub fn parse_content_hash(s: &str) -> ContentHash {
    s.parse().expect("valid test content hash")
}

/// Parse `name` into a valid [`Filename`] for tests — the single place a test
/// filename literal is parsed, so a malformed fixture fails loudly and the parse
/// isn't re-spelled at every media store-seeding call site across the workspace.
///
/// # Panics
///
/// Panics if `name` is not a canonical safe path leaf.
#[must_use]
pub fn parse_filename(name: &str) -> Filename {
    name.parse().expect("valid test filename")
}

/// Parse `s` into a valid [`ContentType`] for tests — the single place a test content-type
/// literal is parsed, so a malformed fixture fails loudly and the parse isn't re-spelled at
/// every media store-seeding call site across the workspace.
///
/// # Panics
///
/// Panics if `s` is not a valid `type/subtype` media type.
#[must_use]
pub fn parse_content_type(s: &str) -> ContentType {
    s.parse().expect("valid test content type")
}

/// Parse `s` into a [`MaxFileSize`] for tests — the single place a test media
/// max-file-size literal is parsed, so a malformed fixture (e.g. `"0"`) fails loudly
/// and the parse isn't re-spelled at every site-config seeding call site.
///
/// # Panics
///
/// Panics if `s` is not a positive number of bytes.
#[must_use]
pub fn parse_max_file_size(s: &str) -> MaxFileSize {
    s.parse().expect("valid test media max file size")
}

/// Parse `s` into a [`UserQuota`] for tests — the single place a test media
/// user-quota literal is parsed, so a malformed fixture (e.g. `"0"`) fails loudly
/// and the parse isn't re-spelled at every site-config seeding call site.
///
/// # Panics
///
/// Panics if `s` is not a positive number of bytes.
#[must_use]
pub fn parse_user_quota(s: &str) -> UserQuota {
    s.parse().expect("valid test media user quota")
}

/// Parse `s` into a [`ByteSize`] for tests — the single place a test byte-count literal is
/// parsed, so a malformed fixture fails loudly and the parse isn't re-spelled at every site.
///
/// # Panics
/// Panics if `s` is not a non-negative number of bytes.
#[must_use]
pub fn parse_byte_size(s: &str) -> ByteSize {
    s.parse().expect("valid test byte size")
}
