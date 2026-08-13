// Test scaffolding that deliberately `expect()`s on a fixture parse, so the
// workspace's `expect_used = deny` lint is expected off for this module; `#[expect]`
// self-flags if the scaffolding ever stops using `expect`.
#![expect(clippy::expect_used)]

use crate::post_body::PostBody;
use crate::post_summary::PostSummary;
use crate::post_title::PostTitle;
use crate::site::SiteTitle;
use crate::slug::Slug;
use crate::tag::{Tag, TagLabel};

/// Parse `title` into a valid [`PostTitle`] for tests — the single place a test title
/// literal is parsed, so the validating `FromStr` isn't re-spelled at every fixture.
///
/// # Panics
///
/// Panics on a blank title, which no test should be constructing (#830).
#[must_use]
pub fn parse_post_title(title: &str) -> PostTitle {
    title.parse().expect("valid test post title")
}

/// Parse `s` into a valid [`SiteTitle`] for tests — the single place a test site-title
/// literal is parsed, so a malformed fixture (empty or whitespace-only) fails loudly and
/// the validating `FromStr` isn't re-spelled at every `SiteIdentity` construction site
/// across the workspace.
///
/// # Panics
///
/// Panics if `s` is empty or whitespace-only.
#[must_use]
pub fn parse_site_title(s: &str) -> SiteTitle {
    s.parse().expect("valid test site title")
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

/// Parse `s` into a valid [`PostBody`] for tests — the single place a test post-body
/// literal is parsed, so a fixture that is nothing but blank lines fails loudly and the
/// validating `FromStr` isn't re-spelled at every post/render fixture across the
/// workspace.
///
/// # Panics
///
/// Panics if `s` has no non-blank line.
#[must_use]
pub fn parse_post_body(s: &str) -> PostBody {
    s.parse().expect("valid test post body")
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
/// and the parse isn't re-spelled at every `set_post_tags` call site.
///
/// # Panics
///
/// Panics if `s` is not a valid tag label.
#[must_use]
pub fn parse_tag_label(s: &str) -> TagLabel {
    s.parse().expect("valid test tag label")
}
