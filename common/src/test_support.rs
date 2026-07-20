//! Cross-crate test fixtures for `common`'s domain types, gated by the
//! `test-support` feature (mirroring `storage::test_support`, ADR-0033): `common`'s
//! own tests reach it under `cfg(test)`; `storage`, `server`, and `web` reach it via
//! the `test-support` feature. Kept out of shipped binaries.

// Test scaffolding that deliberately `expect()`s on a fixture parse, so the
// workspace's `expect_used = deny` lint is expected off for this module; `#[expect]`
// self-flags if the scaffolding ever stops using `expect`.
#![expect(clippy::expect_used)]

use crate::audience::AudienceName;
use crate::backup::RetentionCount;
use crate::bio::Bio;
use crate::display_name::DisplayName;
use crate::email::Email;
use crate::feed::{FeedMinDays, FeedMinItems};
use crate::media::{ContentHash, ContentType, Filename, MaxFileSize, UserQuota};
use crate::pagination::PageSize;
use crate::password::Password;
use crate::post_summary::PostSummary;
use crate::post_title::PostTitle;
use crate::slug::Slug;
use crate::tag::{Tag, TagLabel};
use crate::time::UtcInstant;
use crate::token::{RawToken, TokenHash};
use crate::username::Username;

/// Parse `addr` into a valid [`Email`] for tests — the single place a test email
/// literal is parsed, so a malformed fixture fails loudly and the parse isn't
/// re-spelled at every call site across the workspace.
///
/// # Panics
///
/// Panics if `addr` is not a valid email address.
#[must_use]
pub fn parse_email(addr: &str) -> Email {
    addr.parse().expect("valid test email address")
}

/// Parse `name` into a valid [`AudienceName`] for tests — the single place a test
/// audience-name literal is parsed, so a malformed fixture fails loudly and the parse
/// isn't re-spelled at every store-seeding call site across the workspace.
///
/// # Panics
///
/// Panics if `name` is empty or whitespace-only.
#[must_use]
pub fn parse_audience_name(name: &str) -> AudienceName {
    name.parse().expect("valid test audience name")
}

/// Parse `name` into a valid [`DisplayName`] for tests — the single place a test
/// display-name literal is parsed, so a malformed fixture fails loudly and the parse
/// isn't re-spelled at every store-seeding call site across the workspace.
///
/// # Panics
///
/// Panics if `name` is empty, whitespace-only, or longer than the length bound.
#[must_use]
pub fn parse_display_name(name: &str) -> DisplayName {
    name.parse().expect("valid test display name")
}

/// Parse `s` into a valid [`Bio`] for tests — the single place a test bio literal is
/// parsed, so a malformed fixture (empty or over the length bound) fails loudly and the
/// validating `FromStr` isn't re-spelled at every profile fixture across the workspace.
///
/// # Panics
///
/// Panics if `s` is empty/whitespace-only or longer than the length bound.
#[must_use]
pub fn parse_bio(s: &str) -> Bio {
    s.parse().expect("valid test bio")
}

/// Parse `s` into a valid [`RetentionCount`] for tests — the single place a test
/// retention-count literal is parsed, so a malformed fixture (e.g. `"0"`) fails loudly
/// and the parse isn't re-spelled at every `BackupConfig` construction site.
///
/// # Panics
///
/// Panics if `s` is not a whole number of at least 1.
#[must_use]
pub fn parse_retention_count(s: &str) -> RetentionCount {
    s.parse().expect("valid test retention count")
}

/// Parse `s` into a [`PageSize`] for tests — the single place a test page-size literal is
/// parsed, so a malformed fixture (e.g. `"0"`/`"51"`) fails loudly and the parse isn't
/// re-spelled at every pagination call site.
///
/// # Panics
///
/// Panics if `s` is not an integer in `1..=50`.
#[must_use]
pub fn parse_page_size(s: &str) -> PageSize {
    s.parse().expect("valid test page size")
}

/// Parse `s` into a [`FeedMinItems`] for tests — the single place a test feed-min-items
/// literal is parsed, so a malformed fixture (e.g. `"0"`) fails loudly and the parse isn't
/// re-spelled at every `HybridWindow`/`FeedsConfig` construction site.
///
/// # Panics
///
/// Panics if `s` is not a whole number of at least 1.
#[must_use]
pub fn parse_feed_min_items(s: &str) -> FeedMinItems {
    s.parse().expect("valid test feeds.min_items")
}

/// Parse `s` into a [`FeedMinDays`] for tests — the single place a test feed-min-days
/// literal is parsed, so a malformed fixture (e.g. `"0"`) fails loudly and the parse isn't
/// re-spelled at every `HybridWindow`/`FeedsConfig` construction site.
///
/// # Panics
///
/// Panics if `s` is not a whole number of at least 1.
#[must_use]
pub fn parse_feed_min_days(s: &str) -> FeedMinDays {
    s.parse().expect("valid test feeds.min_days")
}

/// Parse `s` into a [`RawToken`] for tests — the single place a test token literal is
/// constructed, so `RawToken::try_from("…".to_string()).unwrap()` isn't re-spelled at
/// every call site. Takes `&str` (no `.to_string()`), routing through `RawToken`'s
/// validating `FromStr`.
///
/// # Panics
///
/// Panics if `s` is empty or not base64url.
#[must_use]
pub fn parse_raw_token(s: &str) -> RawToken {
    s.parse().expect("valid test raw token")
}

/// Build a [`PostTitle`] from `title` for tests — the single place a test title
/// literal is wrapped, so the trimming `From<String>` isn't re-spelled at every feed
/// fixture. `PostTitle` is infallible (no `FromStr`), so this cannot fail.
#[must_use]
pub fn parse_post_title(title: &str) -> PostTitle {
    PostTitle::from(title.to_owned())
}

/// Parse `s` into a valid [`PostSummary`] for tests — the single place a test post-summary
/// literal is parsed, so a malformed fixture (empty or over the length bound) fails loudly
/// and the validating `FromStr` isn't re-spelled at every post/feed fixture across the
/// workspace.
///
/// # Panics
///
/// Panics if `s` is empty/whitespace-only or longer than the length bound.
#[must_use]
pub fn parse_post_summary(s: &str) -> PostSummary {
    s.parse().expect("valid test post summary")
}

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

/// Parse `name` into a valid [`Username`] for tests — the single place a test
/// username literal is parsed, so a malformed fixture fails loudly and the parse
/// isn't re-spelled at every call site across the workspace.
///
/// # Panics
///
/// Panics if `name` is not a valid username (`[a-z0-9_-]+`).
#[must_use]
pub fn parse_username(name: &str) -> Username {
    name.parse().expect("valid test username")
}

/// Parse `s` into a valid [`Slug`] for tests — the single place a test slug literal
/// is parsed, so a malformed fixture fails loudly and the normalizing `FromStr` isn't
/// re-spelled at every post-seeding call site across the workspace.
///
/// # Panics
///
/// Panics if `s` is not a valid slug.
#[must_use]
pub fn parse_slug(s: &str) -> Slug {
    s.parse().expect("valid test slug")
}

/// Parse `s` into a valid [`TokenHash`] for tests — the single place a test
/// token-hash literal is parsed, so a malformed fixture fails loudly and the parse
/// isn't re-spelled at every session-row fixture across the workspace.
///
/// # Panics
///
/// Panics if `s` is not a valid token hash.
#[must_use]
pub fn parse_token_hash(s: &str) -> TokenHash {
    s.parse().expect("valid test token hash")
}

/// Parse `s` into a valid [`Password`] for tests — the single place a test password
/// literal is parsed, so a too-short fixture fails loudly and the validating `FromStr`
/// isn't re-spelled at every `create_user`/`verify_password` call site.
///
/// # Panics
///
/// Panics if `s` does not meet the minimum-length requirement.
#[must_use]
pub fn parse_password(s: &str) -> Password {
    s.parse().expect("valid test password")
}

/// Parse `s` into a valid [`Tag`] (a canonical tag slug) for tests — the single place
/// a test tag-slug literal is parsed, so a malformed fixture fails loudly and the parse
/// isn't re-spelled at every tag fixture across the workspace.
///
/// # Panics
///
/// Panics if `s` is not a valid tag slug.
#[must_use]
pub fn parse_tag(s: &str) -> Tag {
    s.parse().expect("valid test tag slug")
}

/// Parse `s` into a valid [`TagLabel`] (a case-preserving tag label) for tests — the
/// single place a test tag-label literal is parsed, so a malformed fixture fails loudly
/// and the parse isn't re-spelled at every `tag_post`/`apply_post_tag_diff` call site.
///
/// # Panics
///
/// Panics if `s` is not a valid tag label.
#[must_use]
pub fn parse_tag_label(s: &str) -> TagLabel {
    s.parse().expect("valid test tag label")
}

/// Parse `s` into a valid [`UtcInstant`] for tests — the single place a test
/// instant literal is parsed, so a malformed fixture fails loudly and the parse
/// isn't re-spelled at every timeline/post fixture across the workspace.
///
/// # Panics
///
/// Panics if `s` is not a valid RFC3339 instant.
#[must_use]
pub fn parse_utc_instant(s: &str) -> UtcInstant {
    s.parse().expect("valid test UTC instant")
}
